//! Texture contents the title writes itself (decision 0007, "Texture
//! memory"): streamed pages, layout-library copies, movie frames, glyph
//! atlases. Such a texture lives in console memory in its tiled layout;
//! before a submission samples its host copy, the memory is detiled into a
//! staging buffer when its content changed, and the staging buffer is
//! copied into the host texture.
//!
//! "Changed" means a page of the memory was written since the last detiling
//! ([`crate::watch`]), in write-watched memory; in physical-page views,
//! which cannot be watched, a content hash of the whole memory at every
//! submission that samples it.
//!
//! shortcut: a change detiles the whole texture, not the pages written.
//! Only textures read through descriptor tables are brought up to date
//! (directly indexed views are not known per draw); plane 0 is uploaded
//! (every mip and array slice), depth and stencil planes are not.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use windows::Win32::Graphics::Direct3D12::*;
use windows_sys::Win32::System::ProcessStatus::{K32QueryWorkingSetEx, PSAPI_WORKING_SET_EX_INFORMATION};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use crate::host::{HostList, host};
use crate::object::This;

pub(crate) static COMPACT_UPLOAD_SAVED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(crate) mod ahead;
mod upload;
mod gpu_detile;

macro_rules! once {
    ($($arg:tt)*) => {{
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| tracing::warn!($($arg)*));
    }};
}

/// A texture's upload state: the staging buffer laid out as the host
/// texture's subresources, and what was last detiled.
pub(crate) struct Staging {
    inline: OnceLock<upload::Upload>,
    /// Its bytes.
    #[cfg(feature = "diagnostics")]
    total: u64,
    footprints: Vec<D3D12_PLACED_SUBRESOURCE_FOOTPRINT>,
    rows: Vec<u32>,
    layout: victory_formats::texture::Layout,
    /// Mips per array slice.
    mips: u32,
    /// The first of them that is uploaded, and the bytes of title memory
    /// read for it and the smaller ones.
    first: u32,
    size: usize,
    /// Bytes covered by a staging lease (counted in `STAGED` per plan).
    bytes: u64,
    memory: *const u8,
    last: Mutex<Last>,
    /// Whether there is a revision to upload and whether a submitted list
    /// has already uploaded one. Record-time inline uploads also use these
    /// flags to avoid copying unchanged texels.
    dirty: std::sync::atomic::AtomicBool,
    uploaded: std::sync::atomic::AtomicBool,
    /// When a list last recorded a use of it (`crate::host::moment`).
    used: std::sync::atomic::AtomicU64,
}

/// A sampled texture copy and the exact staging revision it reads. Both
/// resources remain live until the queue fence signals.
#[derive(Clone)]
pub(crate) struct StagedCopy {
    pub(crate) texture: Arc<crate::resource::HostTexture>,
    staging: Arc<Staging>,
    upload: Arc<upload::Upload>,
    first: u32,
    activate_tiles: bool,
    /// Keeps this revision's copy claim live through preparation and submission.
    sparse_claim: Option<Arc<()>>,
}

impl StagedCopy {
    pub(crate) fn sparse_limit(&self, local: Option<[u64; 4]>) -> u64 {
        const MAX_POOL: u64 = 2 << 30;
        let adaptive = crate::budget::enabled();
        let headroom = local.map_or(0, |[budget, used, ..]| {
            let cap = if adaptive { crate::budget::ceiling(budget, 90) } else { budget / 10 * 9 };
            cap.saturating_sub(used)
        });
        let max_pool = if adaptive { local.map_or(MAX_POOL, |[budget, ..]| crate::budget::pool_cap(budget)) } else { MAX_POOL };
        self.texture.sparse.as_ref().map_or(0, |sparse| {
            sparse.tiles.pool_bytes().saturating_add(headroom).min(max_pool.max(sparse.tiles.pool_bytes()))
        })
    }

    pub(crate) fn prepare_sparse(
        &mut self,
        device: &ID3D12Device,
        queue: &ID3D12CommandQueue,
        limit: u64,
    ) -> bool {
        let Some(sparse) = self.texture.sparse.as_ref() else { return true };
        let first = sparse.tiles.map_from(device, queue, &self.texture.texture, self.first, limit);
        self.texture.account_sparse_bytes(sparse.tiles.backed_bytes());
        let Some(first) = first else { return false };
        // A canceled plan can leave tiles mapped without ever recording the
        // aliasing barrier that makes them safe for this texture.
        self.activate_tiles = sparse.first_submitted().is_none_or(|submitted| first < submitted);
        let mut last = self.staging.last.lock().unwrap_or_else(|e| e.into_inner());
        if last.sampled.as_ref().is_some_and(|upload| Arc::ptr_eq(upload, &self.upload)) {
            self.sparse_claim = last.sparse_copies.claim(first);
            if self.sparse_claim.is_none() {
                return false;
            }
        }
        self.first = first;
        #[cfg(feature = "diagnostics")]
        crate::copystat::add(crate::copystat::C::TexUploads, 1);
        true
    }

    pub(crate) fn activate(&self, list: &ID3D12GraphicsCommandList) {
        if self.activate_tiles {
            crate::tiles::activate(list, &self.texture.texture);
        }
    }

    pub(crate) fn tracks_gpu_input(&self) -> bool {
        self.upload.gpu.as_ref().is_some_and(|plan| plan.tracks_input())
    }

    pub(crate) fn submitted(&self, fence: u64) {
        if let Some(plan) = &self.upload.gpu { plan.submitted(fence); }
        if let Some(sparse) = &self.texture.sparse {
            sparse.submitted(fence, self.first);
            let mut last = self.staging.last.lock().unwrap_or_else(|e| e.into_inner());
            if last.sampled.as_ref().is_some_and(|upload| Arc::ptr_eq(upload, &self.upload)) {
                last.sparse_copies.submitted(self.first);
                if self.first <= last.first {
                    self.staging.mark_uploaded();
                }
            }
        }
    }

    pub(crate) fn record(&self, list: &ID3D12GraphicsCommandList) {
        if let Some(plan) = &self.upload.gpu { plan.record(list, &self.upload); }
        for (i, footprint) in self.staging.footprints.iter().enumerate() {
            if self.staging.part(i).0 < self.first {
                continue;
            }
            let mut footprint = *footprint;
            let buffer = if let Some(plan) = &self.upload.gpu {
                footprint.Offset -= plan.output_base;
                &plan.output
            } else {
                footprint.Offset = footprint.Offset - self.upload.base + self.upload.offset;
                self.upload.buffer()
            };
            let dst = D3D12_TEXTURE_COPY_LOCATION {
                // SAFETY: borrowed pointers, never released by the locations.
                pResource: unsafe { std::mem::transmute_copy(&self.texture.texture) },
                Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: self.staging.subresource(i) },
            };
            let src = D3D12_TEXTURE_COPY_LOCATION {
                pResource: unsafe { std::mem::transmute_copy(buffer) },
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { PlacedFootprint: footprint },
            };
            // SAFETY: an open list and live resources in their copy states.
            unsafe { list.CopyTextureRegion(&dst, 0, 0, 0, &src, None) };
        }
        if let Some(plan) = &self.upload.gpu { plan.finish(list); }
    }
}

/// What a texture's staging buffer holds: the hash of the memory detiled
/// (memory not watched), or the write-watch asking it was detiled after.
#[derive(Default)]
struct Last {
    hash: Option<blake3::Hash>,
    // Fingerprints belong only to the current sampled staging allocation.
    mip_hashes: Vec<Option<blake3::Hash>>,
    asked: u64,
    /// The latest successful detile, if it was staged outside the main list.
    sampled: Option<Arc<upload::Upload>>,
    sparse_copies: SparseCopyProgress,
    /// Finest mip that may be copied by the sampled prelude.
    first: u32,
    has_data: bool,
    inline_current: bool,
    /// The frame a partly mapped texture was last read in.
    partial: Option<u64>,
    partial_time: Option<std::time::Instant>,
    /// The texture was found changed without a write the guard saw
    /// ([`Staging::unseen_write`]): it is hashed at every check until this
    /// frame.
    unguarded: u64,
}

/// Coverage for one immutable sampled staging revision, independent of
/// which of the texture's physical tiles have been mapped.
#[derive(Default)]
struct SparseCopyProgress {
    submitted: Option<u32>,
    prepared: Option<(u32, std::sync::Weak<()>)>,
}

impl SparseCopyProgress {
    fn claim(&mut self, first: u32) -> Option<Arc<()>> {
        if self.submitted.is_some_and(|mip| mip <= first)
            || self.prepared.as_ref().is_some_and(|(mip, claim)| *mip <= first && claim.strong_count() != 0)
        {
            return None;
        }
        let claim = Arc::new(());
        self.prepared = Some((first, Arc::downgrade(&claim)));
        Some(claim)
    }

    fn submitted(&mut self, first: u32) {
        self.submitted = Some(self.submitted.map_or(first, |mip| mip.min(first)));
    }
}

impl Last {
    fn require_cpu_revision(&mut self) {
        if self.sampled.is_none() && !self.inline_current {
            self.has_data = false;
            self.hash = None;
            self.asked = 0;
            self.mip_hashes.clear();
        }
    }

    /// A write published during a submission can have a newer number than
    /// the write-watch query used for that submission.
    fn advance_watch(&mut self, queried: u64, writes: &[(usize, u64)]) -> bool {
        let newest = writes.iter().map(|&(_, stamp)| stamp).max().unwrap_or(0);
        let changed = self.asked == 0 || newest > self.asked;
        self.asked = self.asked.max(queried).max(newest);
        changed
    }

    fn partial_due(&mut self, frame: u64, reported_write: bool) -> bool {
        if !reported_write && self.partial.is_some_and(|last| frame.saturating_sub(last) < 30)
            && self.partial_time.is_some_and(|last| last.elapsed() < std::time::Duration::from_millis(500)) {
            return false;
        }
        self.partial = Some(frame);
        self.partial_time = Some(std::time::Instant::now());
        true
    }
}

// SAFETY: the staging mapping is written only at submission, under
// `last`'s lock; the title memory is only read.
unsafe impl Send for Staging {}
// SAFETY: as above.
unsafe impl Sync for Staging {}

static STAGING: Mutex<Option<HashMap<usize, Arc<Staging>>>> = Mutex::new(None);

/// The buffer a texture the host writes is copied through into the texture
/// of a view the title placed in its memory in another format
/// ([`alias`]): every level of every layer as each of the two formats lays
/// it out for a copy (the same bytes: the texels are as wide), the written
/// texture's layout first.
struct Through {
    buffer: ID3D12Resource,
    parts: Vec<(
        D3D12_PLACED_SUBRESOURCE_FOOTPRINT,
        D3D12_PLACED_SUBRESOURCE_FOOTPRINT,
    )>,
}

/// The most levels times layers [`alias`] copies (a cube of nine levels).
const ALIAS_PARTS: u32 = 54;

// SAFETY: a device-local buffer only lists record copies with.
unsafe impl Send for Through {}
// SAFETY: as above.
unsafe impl Sync for Through {}

/// The copy buffers of [`alias`], by the placed view's texture.
static THROUGH: Mutex<Option<HashMap<usize, Arc<Through>>>> = Mutex::new(None);

/// Brings `placed`, the texture of a view the title placed in the memory
/// of `source` in another format (`crate::resource::alias_of`), up to date
/// with what the host wrote to `source`: the texels of every level and
/// layer are copied as they are, through a buffer (the host copies between
/// two formats of one texel width in no other way). Recorded the first time
/// the list reads the view
/// and again whenever the list used `source` in another way since (wrote
/// it, or read it itself). Leaves `placed` in the copy destination state,
/// as [`upload`] does. False if the copy cannot be made: the caller fills
/// the texture from title memory, as before.
pub(crate) fn alias(list: &HostList, placed: This, source: This) -> bool {
    const TARGET_OR_UNORDERED: u32 = 0x4 | 0x8;
    const COPY_DEST: u32 = 0x400;
    const COPY_SOURCE: u32 = 0x800;
    let (Some(to), Some(from)) = (
        crate::resource::host_resource(placed),
        crate::resource::host_resource(source),
    ) else {
        return false;
    };
    let Some(host) = host() else { return false };
    let through = {
        let mut map = THROUGH.lock().unwrap_or_else(|e| e.into_inner());
        let map = map.get_or_insert_with(HashMap::new);
        match map.get(&(placed as usize)) {
            Some(through) => through.clone(),
            None => {
                // SAFETY: live textures.
                let (of_from, of_to) = unsafe { (from.GetDesc(), to.GetDesc()) };
                // Levels times layers (a volume's slices are one part).
                let count = |d: &D3D12_RESOURCE_DESC| {
                    let layers = if d.Dimension == D3D12_RESOURCE_DIMENSION_TEXTURE3D {
                        1
                    } else {
                        u32::from(d.DepthOrArraySize)
                    };
                    u32::from(d.MipLevels) * layers
                };
                let parts = count(&of_from);
                if parts == 0 || parts > ALIAS_PARTS || parts != count(&of_to) {
                    return false;
                }
                let mut rows = [(); 2]
                    .map(|()| vec![D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default(); parts as usize]);
                let mut bytes = [0u64; 2];
                // SAFETY: plain queries; the out pointers are ours and
                // hold `parts` layouts each.
                unsafe {
                    host.device.GetCopyableFootprints(
                        &of_from,
                        0,
                        parts,
                        0,
                        Some(rows[0].as_mut_ptr()),
                        None,
                        None,
                        Some(&mut bytes[0]),
                    );
                    host.device.GetCopyableFootprints(
                        &of_to,
                        0,
                        parts,
                        0,
                        Some(rows[1].as_mut_ptr()),
                        None,
                        None,
                        Some(&mut bytes[1]),
                    );
                }
                let alike = rows[0].iter().zip(&rows[1]).all(|(a, b)| {
                    let (fa, fb) = (a.Footprint, b.Footprint);
                    a.Offset == b.Offset
                        && (fa.Width, fa.Height, fa.Depth, fa.RowPitch)
                            == (fb.Width, fb.Height, fb.Depth, fb.RowPitch)
                });
                if bytes[0] == 0 || bytes[0] != bytes[1] || !alike {
                    return false;
                }
                let Some(buffer) = host.uav_buffer(bytes[0]) else {
                    return false;
                };
                crate::host::name(&buffer, || {
                    format!(
                        "alias copy {}x{} f{} to f{}",
                        of_from.Width, of_from.Height, of_from.Format.0, of_to.Format.0
                    )
                });
                tracing::info!(
                    view = ?placed,
                    texture = ?source,
                    width = of_from.Width,
                    height = of_from.Height,
                    parts,
                    written_as = of_from.Format.0,
                    read_as = of_to.Format.0,
                    "a view placed in the memory of a texture the host writes, in another format: read from that texture by a copy on the host"
                );
                let [of_written, of_read] = rows;
                let through = Arc::new(Through {
                    buffer,
                    parts: of_written.into_iter().zip(of_read).collect(),
                });
                map.insert(placed as usize, through.clone());
                through
            }
        }
    };
    let first = list.first_use(placed as usize);
    // The source's first use in the list records nothing (its prelude
    // brings it to the state); a use in another way since the last copy
    // shows as a change of state.
    let before = list.transition(source, COPY_SOURCE).map(|(texture, before)| {
        let state = crate::host::legal_state(texture, COPY_SOURCE);
        let barrier = crate::host::transition(
            texture,
            D3D12_RESOURCE_STATES(before as i32),
            D3D12_RESOURCE_STATES(state as i32),
        );
        // SAFETY: an open list and a live texture.
        unsafe { list.list.ResourceBarrier(&[barrier]) };
        before
    });
    if !first && before.is_none() {
        return true;
    }
    crate::list::ensure_state(list, placed, COPY_DEST);
    let part = |texture: &ID3D12Resource, index: usize| D3D12_TEXTURE_COPY_LOCATION {
        // SAFETY: a borrowed pointer the location never releases.
        pResource: unsafe { std::mem::transmute_copy(texture) },
        Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
            SubresourceIndex: index as u32,
        },
    };
    let rows = |row: &D3D12_PLACED_SUBRESOURCE_FOOTPRINT| D3D12_TEXTURE_COPY_LOCATION {
        // SAFETY: as above.
        pResource: unsafe { std::mem::transmute_copy(&through.buffer) },
        Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
            PlacedFootprint: *row,
        },
    };
    let (common, read, written) = (
        D3D12_RESOURCE_STATE_COMMON,
        D3D12_RESOURCE_STATE_COPY_SOURCE,
        D3D12_RESOURCE_STATE_COPY_DEST,
    );
    #[cfg(feature = "diagnostics")]
    list.note_op(|| format!("alias copy of {source:?} into {placed:?}"));
    // SAFETY: an open list and live resources; the buffer is as big as
    // the footprints and rests in the common state between copies.
    unsafe {
        list.list
            .ResourceBarrier(&[crate::host::transition(&through.buffer, common, written)]);
        for (index, (of_written, _)) in through.parts.iter().enumerate() {
            list.list
                .CopyTextureRegion(&rows(of_written), 0, 0, 0, &part(from, index), None);
        }
        list.list
            .ResourceBarrier(&[crate::host::transition(&through.buffer, written, read)]);
        for (index, (_, of_read)) in through.parts.iter().enumerate() {
            list.list
                .CopyTextureRegion(&part(to, index), 0, 0, 0, &rows(of_read), None);
        }
        list.list
            .ResourceBarrier(&[crate::host::transition(&through.buffer, read, common)]);
    }
    list.keep(&through.buffer);
    // A draw that also writes the source (a view of it handled before
    // this one) needs it in that state again; the next read of the alias
    // then copies once more, as it must after a write.
    if let Some(before) = before
        && before & TARGET_OR_UNORDERED != 0
    {
        crate::list::ensure_state(list, source, before);
    }
    true
}

/// Whether a texture refresh that finds the runtime remapping title memory
/// goes on without waiting for it (`VICTORY_TEXTURE_NO_WAIT=1`; off by
/// default).
///
/// A texture whose memory is all mapped is read in place, under a lock
/// the runtime takes alone while it changes a mapping
/// (`crate::watch::title_memory_reading`), so that no page goes away under
/// the read. While the world streams in the runtime remaps all the time,
/// and each of the frame's 150-360 refreshes waited for it: runs 99-101
/// have 4, 15 and 16 frame lines with over 300 ms of "textures", up to
/// 3,708 ms, and on every one of them the wait for this lock is the whole
/// of it (1,345 of 1,381 ms; 3,630 of 3,708; the write-watch askings 3-12
/// ms, the texture's own lock 0).
///
/// With the switch the refresh asks for the lock without waiting. If the
/// runtime has it, the texture is read through the kernel instead, which
/// needs no lock: it fails if a page is not there, and then the texture is
/// left as it is and read at its next refresh, a frame later. A read
/// through the kernel costs about 0.6 ms a megabyte against 0.3 in place.
///
/// What changes for the picture: a texture that arrives while the runtime
/// is remapping may show a frame later. It never shows partly read.
fn no_wait() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_TEXTURE_NO_WAIT").is_some_and(|v| v == "1"))
}

/// Since the last frame-rate line, with [`no_wait`]: refreshes that read
/// through the kernel because title memory was being remapped, and those
/// left for the next refresh because a page was not there.
pub(crate) static UNLOCKED: [std::sync::atomic::AtomicU64; 2] = [const { std::sync::atomic::AtomicU64::new(0) }; 2];

/// Since the last frame-rate line, of texture refreshes on every thread, in
/// microseconds: waiting for the texture's own lock (another thread is
/// refreshing the same texture), asking the write watch and the guards,
/// waiting for title memory to hold still (the lock the runtime takes
/// alone while it remaps, `crate::watch::title_memory_reading`), and the
/// refreshes in all.
///
/// While the world streams in, the frame line's "textures" is seconds a
/// frame in some runs and tenths in others for the same megabytes (2,566
/// ms for 110 MB in run 93, 314 ms for 132 MB in run 91; before these
/// counters, runs 81, 83, 85 and 94 each had lines of 1.5-2.9 s), and the
/// submitting thread is not running while it lasts (run 93: 240-310 ms of
/// its own time a frame beside 1,100-1,800 ms of "textures"): it waits.
/// These say for what.
pub(crate) static REFRESH_WAITS: [std::sync::atomic::AtomicU64; 4] = [const { std::sync::atomic::AtomicU64::new(0) }; 4];

/// `f`'s result, its time added to [`REFRESH_WAITS`]' place `slot`.
fn waited<T>(slot: usize, f: impl FnOnce() -> T) -> T {
    let started = std::time::Instant::now();
    let out = f();
    REFRESH_WAITS[slot].fetch_add(started.elapsed().as_micros() as u64, std::sync::atomic::Ordering::Relaxed);
    out
}

thread_local! {
    /// Room to read a texture's memory into: one buffer per submitting
    /// thread. Each texture kept its own, a second copy in memory of every
    /// write-watched or partly mapped texture ever sampled, for the run.
    static READ: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Calls `f` with `size` bytes of this thread's read buffer (their
/// contents are whatever the last texture left).
fn with_read<R>(size: usize, f: impl FnOnce(&mut [u8]) -> R) -> R {
    READ.with(|read| {
        let mut read = read.borrow_mut();
        if read.len() < size {
            read.resize(size, 0);
        }
        let result = f(&mut read[..size]);
        // Avoid retaining a large streaming allocation on every worker.
        if read.capacity() > 64 * 1024 * 1024
            && crate::budget::ram_pressure(crate::host::available_physical_ram()) > 0 {
            *read = Vec::new();
        }
        result
    })
}

/// Whether a texture all of whose memory is mapped is read through the
/// kernel's copy instead of in place (`VICTORY_TEXTURE_COPY=1`; off by
/// default). Round 3 of the memory work read every such texture that way
/// to be safe from pages taken away under the read; it cost 0.6 ms per MB
/// against 0.3 in place (90-105 ms a frame while the world streams in, runs
/// 72 and 73, against 20-40), and both runs with it lost the graphics
/// device, for a reason not found. The read in place is safe now for
/// another reason (`crate::watch::title_memory_reading`); this switch is
/// left to tell whether the copy was what lost the device.
fn through_copy() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_TEXTURE_COPY").is_some_and(|v| v == "1"))
}

/// Calls `f` with the `size` bytes of title memory at `memory`, read
/// through the kernel into this thread's read buffer; pages that are not
/// there read as zeros. The bytes are those a read in place would see.
fn with_title<R>(memory: *const u8, size: usize, f: impl FnOnce(&[u8]) -> R) -> R {
    with_read(size, |read| {
        crate::host::copy_mapped(read.as_mut_ptr(), memory, size, false);
        f(read)
    })
}

/// Staging buffers alive (also those only a list still holds) and their
/// bytes.
static STAGED: [std::sync::atomic::AtomicU64; 2] = [const { std::sync::atomic::AtomicU64::new(0) }; 2];

impl Drop for Staging {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering::Relaxed;
        STAGED[0].fetch_sub(1, Relaxed);
        STAGED[1].fetch_sub(self.bytes, Relaxed);
    }
}

/// For the memory line: staging plans alive, their lease sizes, and how many
/// of them their texture still has (the others wait for lists or fences).
#[cfg(feature = "diagnostics")]
pub(crate) fn memory() -> [u64; 3] {
    use std::sync::atomic::Ordering::Relaxed;
    let kept = STAGING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map_or(0, |map| map.len() as u64);
    [STAGED[0].load(Relaxed), STAGED[1].load(Relaxed), kept]
}

#[cfg(feature = "diagnostics")]
pub(crate) fn upload_pool_totals() -> (u64, u64) {
    upload::totals()
}

/// Staging buffers let go as idle since the start, and their bytes.
static LET_GO: [std::sync::atomic::AtomicU64; 2] = [const { std::sync::atomic::AtomicU64::new(0) }; 2];

/// For the memory line: staging buffers let go as idle, and their bytes.
#[cfg(feature = "diagnostics")]
pub(crate) fn let_go() -> [u64; 2] {
    use std::sync::atomic::Ordering::Relaxed;
    [LET_GO[0].load(Relaxed), LET_GO[1].load(Relaxed)]
}

/// Lets go of the staging buffers no list recorded a use of for `IDLE_MS`
/// and `IDLE_FRAMES` frames and no list holds: a staging buffer is system memory the size of its
/// texture, kept so that an unchanged texture is not uploaded again, and
/// those of textures no longer sampled were kept for the run (2,246 of
/// them, 2.1 GB, at the end of run 24). The next list that samples such a
/// texture makes a staging buffer anew, detiles and uploads once more.
/// Called before a submission starts, by the thread that submits.
pub(crate) fn let_go_idle() {
    use std::sync::atomic::Ordering::Relaxed;
    const IDLE_FRAMES: u64 = 4;
    let available = crate::host::available_physical_ram();
    upload::trim_idle_under_pressure(available);
    let mut guard = STAGING.lock().unwrap_or_else(|e| e.into_inner());
    let Some(map) = guard.as_mut() else {
        return;
    };
    if map.is_empty() {
        return;
    }
    let idle_ms = crate::host::ram_idle_ms(available);
    // A list that recorded an upload or a check holds the staging until it
    // is reset: one only the table holds is named by no list, recorded or
    // running.
    let (mut went, mut bytes) = (0u64, 0u64);
    let mut retired = Vec::new();
    let mut released_uploads = Vec::new();
    let mut released_bytes = 0u64;
    static RECLAIM: OnceLock<bool> = OnceLock::new();
    let reclaim = *RECLAIM.get_or_init(|| std::env::var_os("VICTORY_GPU_ONLY_TEXTURES").is_some_and(|v| v == "1"));
    static PRESSURE: OnceLock<bool> = OnceLock::new();
    let pressure_reclaim = *PRESSURE.get_or_init(|| std::env::var_os("VICTORY_TEXTURE_PRESSURE_RECLAIM").is_some_and(|v| v == "1"))
        && (crate::budget::ram_pressure(available) > 0
            || host().and_then(|h| h.video_memory()).is_some_and(|[budget, used, ..]| budget.saturating_sub(used) < 768 << 20));
    let reclaim = reclaim || pressure_reclaim;
    map.retain(|_, s| {
        let keep = Arc::strong_count(s) > 1
            || !crate::host::long_ago(s.used.load(Relaxed), idle_ms, IDLE_FRAMES);
        // Keep the revision fingerprint and watch state, but release the
        // redundant CPU upload after all recorded/in-flight users are gone.
        // Never touch inline allocations or incomplete sparse revisions.
        if keep && reclaim && Arc::strong_count(s) == 1
            && s.inline.get().is_none() && s.uploaded.load(Relaxed)
            && !s.dirty.load(Relaxed) {
            let mut last = s.last.lock().unwrap_or_else(|e| e.into_inner());
            if last.first == s.first && last.has_data && !last.inline_current
                && last.sampled.as_ref().is_some_and(|u| Arc::strong_count(u) == 1) {
                if let Some(upload) = last.sampled.take() {
                    released_bytes += upload.gpu.as_ref().map_or(s.bytes - upload.base, |plan| plan.input_bytes);
                    released_uploads.push(upload);
                    last.mip_hashes.clear();
                    last.sparse_copies = Default::default();
                }
            }
        }
        if !keep {
            LET_GO[0].fetch_add(1, Relaxed);
            LET_GO[1].fetch_add(s.bytes, Relaxed);
            went += 1;
            bytes += s.bytes;
            retired.push(Arc::clone(s));
        }
        keep
    });
    drop(guard);
    drop(retired);
    let released_count = released_uploads.len();
    drop(released_uploads);
    if released_count > 0 {
        upload::trim_idle_under_pressure(available);
        crate::host::trail(|| format!("GPU-only textures: released {released_count} CPU staging leases ({} KiB)", released_bytes >> 10));
    }
    if went > 0 {
        crate::host::trail(|| format!("sweep: {went} staging buffers of textures let go ({} KiB), held by no list", bytes >> 10));
    }
}

/// Textures the host GPU wrote by a copy or resolve: their contents are on
/// the host, not in title memory, which stays as it was.
static HOST_WRITTEN: Mutex<Option<std::collections::HashSet<usize>>> = Mutex::new(None);

/// A host copy or resolve writes `resource`.
pub(crate) fn host_written(resource: This) {
    let new = {
        let mut set = HOST_WRITTEN.lock().unwrap_or_else(|e| e.into_inner());
        set.get_or_insert_with(Default::default).insert(resource as usize)
    };
    // Diagnostics (run 177): where its title memory is, for placed views
    // the title reads that memory through ([`placed_in_host_written`]).
    if new && let Some((at, desc)) = crate::resource::texture_memory(resource) {
        // SAFETY: 56 bytes in the description's layout.
        let words = unsafe { std::ptr::from_ref(&desc).cast::<[u32; 14]>().read_unaligned() };
        let bytes = crate::resource::placed_bytes(&words).unwrap_or(u64::MAX);
        if !at.is_null() && bytes > 0 {
            HOST_WRITTEN_AT
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert((at as u64, resource as usize), (bytes, words[4], words[6], words[7], words[8]));
        }
    }
}

/// The title memory of textures the host copied into, by address: bytes
/// and description words 4, 6, 7 and 8 (width, height, layers and levels,
/// format).
static HOST_WRITTEN_AT: Mutex<std::collections::BTreeMap<(u64, usize), (u64, u32, u32, u32, u32)>> =
    Mutex::new(std::collections::BTreeMap::new());

pub(crate) fn host_written_overlaps(start: u64, end: u64) -> bool {
    let writes = HOST_WRITTEN_AT.lock().unwrap_or_else(|e| e.into_inner());
    writes.range(..(end, 0)).rev().any(|(&(base, _), entry)| base.saturating_add(entry.0) > start)
}

/// A texture the title placed at `address` (description words `desc`) was
/// made: says so once if it lies in the memory of a texture the host
/// copied into ([`host_written`]). Its own host copy is filled from title
/// memory, which the copy never reached: the copied texels are on the
/// card in the other texture, where no shader that reads this view finds
/// them.
pub(crate) fn placed_in_host_written(address: u64, desc: &[u32; 14]) {
    let hit = {
        let map = HOST_WRITTEN_AT.lock().unwrap_or_else(|e| e.into_inner());
        map.range(..=(address, usize::MAX)).next_back().and_then(|(&(at, _), _)| {
            map.range((at, 0)..=(at, usize::MAX)).find_map(|(_, &d)| {
                (address < at.saturating_add(d.0)).then_some((at, d))
            })
        })
    };
    if let Some((at, (bytes, width, height, layers_levels, format))) = hit {
        static SAID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        if SAID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 200 {
            tracing::warn!(
                view_address = format_args!("{address:#x}"),
                view_offset = address - at,
                view_width = desc[4],
                view_height = desc[6],
                view_layers_levels = format_args!("{:#x}", desc[7]),
                view_format = desc[8],
                copied_address = format_args!("{at:#x}"),
                copied_bytes = bytes,
                copied_width = width,
                copied_height = height,
                copied_layers_levels = format_args!("{layers_levels:#x}"),
                copied_format = format,
                "HERO LEAD: a placed texture view lies in the memory of a texture the host copied into: it is filled from title memory, which the copy never reached"
            );
        }
    }
}

/// Whether the host GPU wrote `resource` by a copy or resolve
/// ([`host_written`]): its host copy cannot be made again from title
/// memory.
pub(crate) fn host_wrote(resource: usize) -> bool {
    HOST_WRITTEN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|set| set.contains(&resource))
}

/// The title released a resource.
pub(crate) fn forget(resource: usize, address: u64) {
    let mut map = STAGING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = map.as_mut() {
        map.remove(&resource);
    }
    if HOST_WRITTEN.lock().unwrap_or_else(|e| e.into_inner()).as_mut().is_some_and(|set| set.remove(&resource)) {
        HOST_WRITTEN_AT.lock().unwrap_or_else(|e| e.into_inner()).remove(&(address, resource));
    }
    let mut through = THROUGH.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(through) = through.as_mut() {
        through.remove(&resource);
    }
}

/// Whether `resource`'s host copy takes its contents from title memory:
/// a texture the host GPU does not render to or write.
pub(crate) fn title_written(texture: &ID3D12Resource) -> bool {
    const TARGET_DEPTH_UNORDERED: i32 = 0x7;
    // SAFETY: a live resource.
    unsafe { texture.GetDesc() }.Flags.0 & TARGET_DEPTH_UNORDERED == 0
}

/// Records, once per list, the copy of `resource`'s staging buffer into
/// its host copy, and the detiling that refreshes the staging buffer at
/// submission if the memory changed. Leaves the texture in the copy
/// destination state.
pub(crate) fn upload(list: &HostList, resource: This, texture: &ID3D12Resource) {
    upload_texture(list, resource, texture, 0, false);
}

/// [`upload`] of the mips from `first` on, the ones a view shows: of a
/// placed texture the title may have no memory for the larger ones, and
/// what lies there is another texture's.
pub(crate) fn upload_from(list: &HostList, resource: This, texture: &ID3D12Resource, first: u32) -> bool {
    upload_texture(list, resource, texture, first, true)
}

fn upload_texture(list: &HostList, resource: This, texture: &ID3D12Resource, first: u32, sampled: bool) -> bool {
    if ahead::enabled()
        && let Some(copy) = crate::resource::existing_host_copy(resource)
        && ahead::claim_normal(&copy) == ahead::Status::Pending
    {
        return false;
    }
    let host_written = HOST_WRITTEN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|set| set.contains(&(resource as usize)));
    // shortcut: a texture the host copied into keeps the host's contents
    // even if the title later writes its memory too.
    if host_written || list.texture_covers(texture, first) {
        return true;
    }
    let Some(staging) = staging(resource, texture, first) else {
        return false;
    };
    use std::sync::atomic::Ordering::Relaxed;
    staging.used.store(crate::host::moment(), Relaxed);
    if sampled && let Some(copy) = list.first_texture_upload(resource, texture) {
        list.note_texture_mips(texture, staging.first);
        list.defer_sampled_texture(staging, copy);
        return true;
    }
    if host().is_some_and(|host| crate::tiles::supported(&host.device))
        && crate::resource::sparse_sample(resource).is_some()
    {
        // Inline copies have no point at which to map their tiles. Keep the
        // last ready view and let the background pass record a prelude.
        return false;
    }
    // The host texture has the staging buffer's contents: only the check
    // at submission is left, and what it finds changed the next list that
    // samples the texture uploads (a frame late where every list of the
    // frame was recorded before that check).
    if staging.uploaded.load(Relaxed) && !staging.dirty.load(Relaxed) {
        #[cfg(feature = "diagnostics")]
        crate::copystat::add(crate::copystat::C::TexSkipped, 1);
        list.note_texture_mips(texture, staging.first);
        list.defer_texture(staging, false);
        return true;
    }
    let Some(upload) = staging.inline_upload() else { return false };
    list.note_texture_mips(texture, staging.first);
    #[cfg(feature = "diagnostics")]
    crate::copystat::add(crate::copystat::C::TexUploads, 1);
    const COPY_DEST: u32 = 0x400;
    crate::list::ensure_state(list, resource, COPY_DEST);
    // The staging buffer lives as long as `staging`, which the deferred
    // refresh below holds until the list is reset.
    for (i, footprint) in staging.footprints.iter().enumerate() {
        let mut footprint = *footprint;
        footprint.Offset += upload.offset;
        #[cfg(feature = "diagnostics")]
        {
            let f = footprint.Footprint;
            // SAFETY: live resources.
            let (gpu, d) = unsafe { (upload.buffer().GetGPUVirtualAddress(), texture.GetDesc()) };
            let len = u64::from(f.RowPitch) * u64::from(staging.rows[i]) * u64::from(f.Depth);
            list.note_read(
                || {
                    format!(
                        "texture upload {resource:?} ({}x{}x{} f{} mips {}) sub {i} of {} from staging {gpu:#x}+{:#x}: offset {:#x} {}x{}x{} pitch {}",
                        d.Width,
                        d.Height,
                        d.DepthOrArraySize,
                        d.Format.0,
                        d.MipLevels,
                        staging.footprints.len(),
                        staging.total,
                        footprint.Offset,
                        f.Width,
                        f.Height,
                        f.Depth,
                        f.RowPitch
                    )
                },
                (gpu + footprint.Offset, len),
            );
        }
        let to = D3D12_TEXTURE_COPY_LOCATION {
            // SAFETY: a borrowed pointer the location never releases.
            pResource: unsafe { std::mem::transmute_copy(texture) },
            Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                SubresourceIndex: staging.subresource(i),
            },
        };
        let from = D3D12_TEXTURE_COPY_LOCATION {
            // SAFETY: as above.
            pResource: unsafe { std::mem::transmute_copy(upload.buffer()) },
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: footprint,
            },
        };
        // SAFETY: an open list; both locations name live resources.
        unsafe { list.list.CopyTextureRegion(&to, 0, 0, 0, &from, None) };
    }
    // The copy reads the bytes refreshed for this submission. Recording
    // its upload after the refresh lets subsequent lists reuse those bytes.
    list.defer_texture(staging, true);
    true
}

// A compact lease represents [base, bytes) in the full footprint space.
// Reject invalid geometry rather than constructing an out-of-bounds pointer.
fn compact_suffix_base(bytes: u64, selected: impl Iterator<Item = (u64, Option<u64>)>) -> u64 {
    let mut base = bytes;
    for (offset, len) in selected {
        if offset % 512 != 0 || len.is_none_or(|n| n == 0 || offset.checked_add(n).is_none_or(|end| end > bytes)) {
            return 0;
        }
        base = base.min(offset);
    }
    if base >= bytes { 0 } else { base }
}

#[cfg(test)]
#[test]
fn compact_suffix_preserves_padded_footprints() {
    let bytes = 16 * 1024;
    assert_eq!(compact_suffix_base(bytes, [(8192, Some(4096)), (12288, Some(4096))].into_iter()), 8192);
    assert_eq!(compact_suffix_base(bytes, [(8193, Some(4096))].into_iter()), 0);
    assert_eq!(compact_suffix_base(bytes, [(12288, Some(4097))].into_iter()), 0);
    assert_eq!(compact_suffix_base(bytes, [(8192, None)].into_iter()), 0);
    assert_eq!(compact_suffix_base(bytes, std::iter::empty()), 0);
}

impl Staging {
    fn inline_upload(&self) -> Option<&upload::Upload> {
        if self.inline.get().is_none() {
            let _ = self.inline.set(upload::Upload::new(host()?, self.bytes)?);
        }
        self.inline.get()
    }

    fn mark_uploaded(&self) {
        use std::sync::atomic::Ordering::Relaxed;
        self.dirty.store(false, Relaxed);
        self.uploaded.store(true, Relaxed);
    }

    pub(crate) fn finish_inline_upload(&self) {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if !last.has_data {
            return;
        }
        if !last.inline_current {
            let from = last.sampled.as_ref().expect("latest sampled staging");
            let to = self.inline.get().expect("inline staging is recorded");
            if let Some(plan) = &from.gpu {
                // A raw GPU revision may also be needed by an inline copy.
                // Reconstruct linear rows from its immutable raw snapshot,
                // not from guest memory that could already have changed.
                let tiled = unsafe { std::slice::from_raw_parts(from.data(), plan.raw_bytes) };
                if plan.first > self.first {
                    // Match the legacy partial CPU revision's zero-filled fine
                    // footprints when a previously recorded inline copy needs it.
                    unsafe { to.data().write_bytes(0, self.bytes as usize); }
                    if !self.detile_selected(tiled, to.data(), &[], &[], plan.first, 0) { return; }
                } else if !self.detile_into(tiled, to.data()) { return; }
            } else {
                // SAFETY: distinct slots, and this submission owns both CPU mappings.
                unsafe {
                    let base = from.base as usize;
                    if base != 0 { to.data().write_bytes(0, base); }
                    std::ptr::copy_nonoverlapping(from.data(), to.data().add(base), self.bytes as usize - base);
                };
            }
            last.inline_current = true;
        }
        self.mark_uploaded();
    }

    pub(crate) fn sampled_copy(self: &Arc<Self>, texture: &Arc<crate::resource::HostTexture>) -> Option<StagedCopy> {
        use std::sync::atomic::Ordering::Relaxed;
        #[cfg(feature = "diagnostics")]
        use crate::copystat::{C, add};

        if self.uploaded.load(Relaxed) && !self.dirty.load(Relaxed) {
            #[cfg(feature = "diagnostics")]
            add(C::TexSkipped, 1);
            return None;
        }
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if !last.has_data {
            return None;
        }
        if last.sampled.is_none() {
            let inline = self.inline.get().filter(|_| last.inline_current)?;
            let upload = upload::Upload::new(host()?, self.bytes)?;
            // SAFETY: a new exclusive slot; the inline staging is CPU-stable.
            unsafe { std::ptr::copy_nonoverlapping(inline.data(), upload.data(), self.bytes as usize) };
            last.sampled = Some(Arc::new(upload));
            last.sparse_copies = Default::default();
        }
        let upload = last.sampled.as_ref().unwrap().clone();
        if texture.sparse.is_none() {
            self.mark_uploaded();
            #[cfg(feature = "diagnostics")]
            add(C::TexUploads, 1);
        }
        Some(StagedCopy {
            texture: texture.clone(), staging: self.clone(), upload, first: last.first, activate_tiles: false,
            sparse_claim: None,
        })
    }

    /// The title memory the texture is read from: address and bytes.
    pub(crate) fn memory(&self) -> (usize, usize) {
        (self.memory as usize, self.size)
    }

    /// The mip and the array slice of the `i`th part of the staging buffer.
    fn part(&self, i: usize) -> (u32, u32) {
        let mips = self.mips - self.first;
        (self.first + i as u32 % mips, i as u32 / mips)
    }

    /// The texture's subresource the `i`th part of the staging buffer is.
    fn subresource(&self, i: usize) -> u32 {
        let (mip, slice) = self.part(i);
        slice * self.mips + mip
    }

    /// Detiles the texture memory into the staging buffer if it changed
    /// since the last time (and is mapped at all).
    pub(crate) fn refresh(&self, sampled: bool) {
        use std::sync::atomic::Ordering::Relaxed;
        let started = std::time::Instant::now();
        let read = self.refresh_now(sampled);
        let preparing = &crate::host::PREPARING;
        preparing[2].fetch_add(started.elapsed().as_micros() as u64, Relaxed);
        preparing[3].fetch_add(read as u64, Relaxed);
    }

    /// Returns the bytes of title memory read.
    fn refresh_now(&self, sampled: bool) -> usize {
        let whole = std::time::Instant::now();
        let read = self.refresh_timed(sampled);
        REFRESH_WAITS[3].fetch_add(whole.elapsed().as_micros() as u64, std::sync::atomic::Ordering::Relaxed);
        read
    }

    fn refresh_timed(&self, sampled: bool) -> usize {
        let size = self.size;
        let start = self.memory as usize;
        let Some(end) = start.checked_add(size) else { return 0 };
        let mut last = waited(0, || self.last.lock().unwrap_or_else(|e| e.into_inner()));
        let last = &mut *last;
        if !sampled {
            last.require_cpu_revision();
        }
        if let Some((now, written)) = waited(1, || crate::watch::writes_since(start, end, false, if crate::watch::indexed_generations() { last.asked } else { 0 })) {
            let changed = last.advance_watch(now, &written);
            if !changed && last.has_data {
                if last.first == self.first || !last.partial_due(crate::frame::current(), false) {
                    return 0;
                }
            }
            if changed {
                last.partial = None;
                last.partial_time = None;
            }
            // Require a complete kernel read for watched memory. A partial
            // copy with zero-filled holes must not replace a valid revision.
            with_read(size, |read| {
                if !crate::host::read_into(self.memory, read) {
                    last.asked = 0;
                    return;
                }
                // A write notification can cover padding or unchanged bytes.
                // Avoid detiling again if the complete staged revision matches.
                let hash = blake3::hash(read);
                if !last.has_data || last.hash != Some(hash) || last.first != self.first {
                    self.detile_hashed(last, read, hash, sampled);
                }
            });
            #[cfg(feature = "diagnostics")]
            crate::copystat::add(crate::copystat::C::TexWatched, size as u64);
            return size;
        }
        // View memory has no write-watch, and a texture there was hashed
        // whole at every check: 229 MB and 51 ms a frame in a gameplay view
        // where two textures change (run 58). It is guarded now, as buffers
        // in view memory are: a write of the title's to it faults once and
        // is noted, a mapping is announced by the runtime. While neither
        // happened the texture is as it was. What the guard cannot see (a
        // write to the same pages through another view) is still found by
        // the hash, which is taken every eighth frame only, by the
        // texture's address, so the textures take turns.
        // `VICTORY_TEXTURE_GUARD=0` hashes at every check, as before.
        static GUARD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        // A texture that changed without the guard seeing it is hashed at
        // every check for a while (the intro's movie: three pictures in turn
        // that only changed every eighth frame, one old picture after
        // another on the screen).
        let mut unseen = false;
        let mut reported_write = false;
        if *GUARD.get_or_init(|| std::env::var_os("VICTORY_TEXTURE_GUARD").is_none_or(|v| v != "0"))
            && let Some((now, written)) = waited(1, || crate::watch::writes_since(start, end, true, if crate::watch::indexed_generations() { last.asked } else { 0 }))
        {
            const HASH_EVERY: u64 = 8;
            reported_write = last.advance_watch(now, &written);
            let frame = crate::frame::current();
            let turn = frame.wrapping_add((start >> 16) as u64).is_multiple_of(HASH_EVERY);
            // Linear pictures of one level are what the processor draws (the
            // planes of a movie), through a view of its own.
            let desc = &self.layout.desc;
            let film = desc.swizzle.0 == 0 && desc.mips == 1;
            let partial_retry = last.partial_time.is_some_and(|t| {
                t.elapsed() >= std::time::Duration::from_millis(500)
            });
            if !reported_write && !turn && !film && !partial_retry && last.has_data && frame >= last.unguarded {
                return 0;
            }
            unseen = !reported_write;
        }
        // Pin the exact span through the committed check, hash and detile.
        // Windows temporarily removes whole physical views on a partial
        // remap; xmem includes those old extents in its writer range.
        let _mapped = if no_wait() {
            // Not waited for: while the runtime remaps title memory the
            // texture is read through the kernel instead, whole or not at
            // all (`no_wait`).
            let Some(mapped) = crate::watch::title_memory_reading_now(start..end) else {
                return self.refresh_unlocked(last, size, sampled);
            };
            mapped
        } else {
            waited(2, || crate::watch::title_memory_reading_range(start..end))
        };
        if !committed(self.memory, size) {
            // Partly mapped: a streamed texture with only some mips' pages
            // there, the usual state while it streams in. Read through the
            // copy, which stands in for unmapped pages with zeros. An
            // eligible sampled prelude copies only complete coarse mips.
            // Any missing fine mip keeps its earlier texels.
            // Poll unchanged partial textures infrequently. A page write
            // or remap reported by the guard makes the texture due now.
            let frame = crate::frame::current();
            if !last.partial_due(frame, reported_write) {
                return 0;
            }
            let use_mip_tail = sampled && self.layout.slices == 1 && !self.layout.thick
                && self.layout.desc.dimension == victory_formats::texture::Dimension::Tex2d;
            let first = if use_mip_tail {
                let Some(first) = (self.first + 1..self.mips).find(|&level| {
                    committed(self.memory, self.layout.bytes_from(level) as usize)
                }) else {
                    // Keep the timed partial poll, but do not let the guard
                    // suppress it for eight more frames after a failed read.
                    last.unguarded = frame.saturating_add(8);
                    last.asked = 0;
                    return 0;
                };
                first
            } else {
                // A non-tail layout cannot safely expose a partially mapped
                // revision. Preserve the previous image until it is complete.
                last.asked = 0;
                return 0;
            };
            // The committed coarse prefix is sufficient for these mips.
            // Do not read, hash, zero-fill or detile missing fine levels.
            let prefix = self.layout.bytes_from(first) as usize;
            // SAFETY: the prefix passed committed() above and the mapping
            // read guard remains held until this function returns.
            let tiled = unsafe { std::slice::from_raw_parts(self.memory, prefix) };
            let hash = blake3::hash(tiled);
            #[cfg(feature = "diagnostics")]
            crate::copystat::add(crate::copystat::C::TexHashed, prefix as u64);
            if last.has_data && last.hash == Some(hash) && last.first == first {
                return prefix;
            }
            // Publish only after every coarse subresource succeeds. A fresh
            // revision protects the previous image and any in-flight copies.
            if let Some(upload) = gpu_detile::prepare_partial(self, tiled, first) {
                last.sampled = Some(Arc::new(upload));
                last.mip_hashes.clear();
                last.inline_current = false;
                last.has_data = true;
                last.sparse_copies = Default::default();
                last.first = first;
                last.hash = Some(hash);
                self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
                #[cfg(feature = "diagnostics")]
                crate::copystat::add(crate::copystat::C::TexChanged, prefix as u64);
                return prefix;
            }
            let base = self.partial_upload_base(first);
            let Some(upload) = host().and_then(|host| upload::Upload::suffix(host, self.bytes, base)) else {
                last.asked = 0;
                return prefix;
            };
            // Inline fallback may copy the entire staging slot. Initialize
            // it so skipped footprints never expose stale pooled bytes.
            // SAFETY: this fresh upload owns self.bytes writable bytes.
            unsafe { upload.data().write_bytes(0, (self.bytes - base) as usize) };
            if !self.detile_selected(tiled, upload.data(), &[], &[], first, base) {
                last.asked = 0;
                return prefix;
            }
            if base != 0 {
                COMPACT_UPLOAD_SAVED.fetch_add(base, std::sync::atomic::Ordering::Relaxed);
            }
            last.sampled = Some(Arc::new(upload));
            last.mip_hashes.clear();
            last.inline_current = false;
            last.has_data = true;
            last.sparse_copies = Default::default();
            last.first = first;
            last.hash = Some(hash);
            self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
            #[cfg(feature = "diagnostics")]
            crate::copystat::add(crate::copystat::C::TexChanged, prefix as u64);
            return prefix;
        }
        last.partial = None;
        last.partial_time = None;
        if through_copy() {
            return with_title(self.memory, size, |tiled| {
                let hash = blake3::hash(tiled);
                #[cfg(feature = "diagnostics")]
                crate::copystat::add(crate::copystat::C::TexHashed, size as u64);
                if last.has_data && last.hash == Some(hash) && last.first == self.first {
                    return size;
                }
                if unseen && last.hash.is_some() {
                    self.unseen_write(last);
                }
                let changed = self.detile_hashed(last, tiled, hash, sampled);
                #[cfg(feature = "diagnostics")]
                if changed {
                    crate::copystat::add(crate::copystat::C::TexChanged, size as u64);
                }
                let _ = changed;
                size
            });
        }
        // SAFETY: committed memory of the texture's layout size, and it
        // stays committed while `_mapped` is held.
        let tiled = unsafe { std::slice::from_raw_parts(self.memory, size) };
        let hash = blake3::hash(tiled);
        #[cfg(feature = "diagnostics")]
        crate::copystat::add(crate::copystat::C::TexHashed, size as u64);
        if last.has_data && last.hash == Some(hash) && last.first == self.first {
            return size;
        }
        if unseen && last.hash.is_some() {
            self.unseen_write(last);
        }
        let changed = self.detile_hashed(last, tiled, hash, sampled);
        #[cfg(feature = "diagnostics")]
        if changed {
            crate::copystat::add(crate::copystat::C::TexChanged, size as u64);
        }
        let _ = changed;
        size
    }

    /// The texture's memory changed and the guard says nothing wrote it: it
    /// is hashed at every check for the next frames, the longer the more
    /// often this happens (a texture streamed in once is soon guarded
    /// again, a movie's is not).
    fn unseen_write(&self, last: &mut Last) {
        let frame = crate::frame::current();
        let more = if frame < last.unguarded { 120 } else { 16 };
        #[cfg(feature = "diagnostics")]
        {
            use windows_sys::Win32::System::Memory::{MEMORY_BASIC_INFORMATION, VirtualQuery};
            static SEEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if SEEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 24 {
                // SAFETY: plain data filled in by the query.
                let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
                // SAFETY: a valid out pointer of the right size.
                unsafe { VirtualQuery(self.memory.cast(), &mut info, size_of_val(&info)) };
                tracing::info!(
                    frame,
                    memory = ?self.memory,
                    size = self.layout.size,
                    protection = format_args!("{:#x}", info.Protect),
                    kind = format_args!("{:#x}", info.Type),
                    run = info.RegionSize,
                    desc = ?self.layout.desc,
                    "a guarded texture changed and no write to it was seen"
                );
            }
        }
        last.unguarded = frame + more;
    }

    /// A refresh while the runtime is remapping title memory ([`no_wait`]):
    /// the texture is read through the kernel, which fails if a page of it
    /// is not there at this moment. Then nothing is taken and the texture
    /// is looked at again at its next refresh (what was learnt about writes
    /// to it is forgotten, so that it is not passed over as unwritten).
    /// Returns the bytes read.
    fn refresh_unlocked(&self, last: &mut Last, size: usize, sampled: bool) -> usize {
        use std::sync::atomic::Ordering::Relaxed;
        let read = with_read(size, |read| {
            if !crate::host::read_into(self.memory, read) {
                return false;
            }
            let hash = blake3::hash(read);
            if !last.has_data || last.hash != Some(hash) || last.first != self.first {
                return self.detile_hashed(last, read, hash, sampled);
            }
            true
        });
        if read {
            UNLOCKED[0].fetch_add(1, Relaxed);
            size
        } else {
            UNLOCKED[1].fetch_add(1, Relaxed);
            last.asked = 0;
            0
        }
    }

    fn detile_hashed(&self, last: &mut Last, tiled: &[u8], hash: blake3::Hash, sampled: bool) -> bool {
        if self.detile(last, tiled, sampled) {
            last.hash = Some(hash);
            true
        } else {
            last.asked = 0;
            false
        }
    }

    /// A sampled refresh keeps any slot still in use by the queue immutable.
    /// Inline copies already name their slot in the main list.
    fn detile(&self, last: &mut Last, tiled: &[u8], sampled: bool) -> bool {
        if sampled {
            let reused_gpu = last.sampled.as_mut().and_then(Arc::get_mut)
                .is_some_and(|upload| gpu_detile::refresh(self, tiled, upload));
            let gpu_ready = reused_gpu || if let Some(upload) = gpu_detile::prepare(self, tiled) {
                last.sampled = Some(Arc::new(upload));
                true
            } else { false };
            if gpu_ready {
                last.mip_hashes.clear();
                last.inline_current = false;
                last.has_data = true;
                last.sparse_copies = Default::default();
                last.first = self.first;
                self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
                return true;
            }
            let hashes = self.mip_hashes(tiled);
            if let Some(upload) = last.sampled.as_mut().filter(|u| u.base == 0 && u.gpu.is_none()).and_then(Arc::get_mut) {
                // Reuse the slot only after all queue references are gone.
                if !self.detile_selected(tiled, upload.data(), &last.mip_hashes, &hashes, self.first, 0) {
                    last.mip_hashes.clear();
                    last.sampled = None;
                    last.sparse_copies = Default::default();
                    last.has_data = last.inline_current;
                    last.hash = None;
                    return false;
                }
            } else {
                let Some(upload) = host().and_then(|host| upload::Upload::new(host, self.bytes)) else { return false };
                if !self.detile_into(tiled, upload.data()) {
                    return false;
                }
                last.sampled = Some(Arc::new(upload));
            }
            last.mip_hashes = hashes;
            last.inline_current = false;
        } else {
            last.mip_hashes.clear();
            let Some(upload) = self.inline_upload() else { return false };
            if !self.detile_into(tiled, upload.data()) {
                last.inline_current = false;
                last.hash = None;
                last.has_data = last.sampled.is_some();
                return false;
            }
            last.sampled = None;
            last.inline_current = true;
        }
        last.has_data = true;
        last.sparse_copies = Default::default();
        last.first = self.first;
        self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        true
    }

    // Only single-slice, non-thick layouts with multiple mips have a
    // proven contiguous mip span here. Tail mips share the entire tail block.
    fn mip_hashes(&self, tiled: &[u8]) -> Vec<Option<blake3::Hash>> {
        use victory_formats::texture::{Dimension, Swizzle};
        static ENABLED: OnceLock<bool> = OnceLock::new();
        if !*ENABLED.get_or_init(|| std::env::var_os("VICTORY_MIP_DIFF").is_some_and(|v| v == "1")) {
            return Vec::new();
        }
        let l = &self.layout;
        if l.slices != 1 || l.thick || l.desc.dimension == Dimension::Tex3d
            || l.desc.swizzle == Swizzle::SW_64KB_R_X || self.footprints.len() < 2 {
            return Vec::new();
        }
        self.footprints.iter().enumerate().map(|(i, _)| {
            let (mip, _) = self.part(i);
            let m = l.mips.get(mip as usize)?;
            let start = if m.tail.is_some() { 0 } else { m.offset };
            let end = l.bytes_from(mip);
            let span = tiled.get(usize::try_from(start).ok()?..usize::try_from(end).ok()?)?;
            if span.is_empty() { return None; }
            Some(blake3::hash(span))
        }).collect()
    }

    fn detile_into(&self, tiled: &[u8], data: *mut u8) -> bool {
        self.detile_selected(tiled, data, &[], &[], self.first, 0)
    }

    fn partial_upload_base(&self, first: u32) -> u64 {
        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON.get_or_init(|| std::env::var_os("VICTORY_COMPACT_MIP_UPLOAD").is_some_and(|v| v == "1")) {
            return 0;
        }
        compact_suffix_base(self.bytes, self.footprints.iter().zip(&self.rows).enumerate()
            .filter(|(i, _)| self.part(*i).0 >= first)
            .map(|(_, (f, rows))| {
                let len = u64::from(f.Footprint.RowPitch).checked_mul(u64::from(*rows))
                    .and_then(|n| n.checked_mul(u64::from(f.Footprint.Depth)));
                (f.Offset, len)
            }))
    }

    fn detile_selected(&self, tiled: &[u8], data: *mut u8,
        old: &[Option<blake3::Hash>], new: &[Option<blake3::Hash>], first: u32, base: u64) -> bool {
        #[cfg(feature = "diagnostics")]
        if tracing::enabled!(tracing::Level::DEBUG) && tiled.iter().all(|&b| b == 0) {
            tracing::debug!(
                memory = ?self.memory,
                desc = ?self.layout.desc,
                "a sampled texture's memory is all zero (not streamed yet, or a failed BCPack read)"
            );
        }
        // Large mip chains can be converted concurrently without duplicating
        // guest data or allocating another staging buffer. Limit the entire
        // process to one extra worker, even when refreshes are already parallel.
        static PARALLEL: OnceLock<bool> = OnceLock::new();
        static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        use std::sync::atomic::Ordering::{Acquire, Release, Relaxed};
        if base == 0 && self.bytes >= 4 << 20
            && *PARALLEL.get_or_init(|| std::env::var_os("VICTORY_PARALLEL_DETILE").is_some_and(|v| v == "1"))
            && BUSY.compare_exchange(false, true, Acquire, Relaxed).is_ok()
        {
            struct Permit<'a>(&'a std::sync::atomic::AtomicBool);
            impl Drop for Permit<'_> {
                fn drop(&mut self) { self.0.store(false, Release); }
            }
            let _permit = Permit(&BUSY);
            // Validate every footprint before constructing disjoint slices.
            let mut ranges = Vec::new();
            let mut end = 0usize;
            let mut valid = true;
            for (i, (footprint, &rows)) in self.footprints.iter().zip(&self.rows).enumerate() {
                let f = footprint.Footprint;
                let pitch = f.RowPitch as usize;
                let Some(depth_pitch) = pitch.checked_mul(rows as usize) else { valid = false; break };
                let Some(len) = depth_pitch.checked_mul(f.Depth as usize) else { valid = false; break };
                let Ok(start) = usize::try_from(footprint.Offset) else { valid = false; break };
                let Some(next) = start.checked_add(len) else { valid = false; break };
                if start < end || next as u64 > self.bytes || len == 0 { valid = false; break; }
                end = next;
                if self.part(i).0 < first { continue; }
                if let Some(Some(hash)) = new.get(i) {
                    if old.get(i) == Some(&Some(*hash)) { continue; }
                }
                ranges.push((i, start, len, pitch, depth_pitch));
            }
            if valid && !ranges.is_empty() {
                let layout = &self.layout;
                let mut jobs = Vec::with_capacity(ranges.len());
                for (i, start, len, pitch, depth_pitch) in ranges {
                    let (mip, slice) = self.part(i);
                    // SAFETY: all ranges were checked for bounds and overlap;
                    // the caller owns this staging revision until we join.
                    let dst = unsafe { std::slice::from_raw_parts_mut(data.add(start), len) };
                    jobs.push((mip, slice, pitch, depth_pitch, dst));
                }
                // Greedy size balancing, so a large top mip does not leave
                // the second worker with only tiny tail mips unnecessarily.
                jobs.sort_unstable_by_key(|job| std::cmp::Reverse(job.4.len()));
                let total: usize = jobs.iter().map(|job| job.4.len()).sum();
                if jobs[0].4.len() >= total - total / 4 {
                    for (index, (mip, slice, pitch, depth_pitch, dst)) in jobs.into_iter().enumerate() {
                        let result = if index == 0 {
                            layout.detile_parallel_rows(tiled, mip, slice, dst, pitch, depth_pitch)
                        } else {
                            layout.detile(tiled, mip, slice, dst, pitch, depth_pitch)
                        };
                        if let Err(e) = result {
                            tracing::warn!(?e, mip, slice, "row-band texture not detiled");
                            return false;
                        }
                    }
                    return true;
                }
                let (mut a, mut b) = (Vec::new(), Vec::new());
                let (mut na, mut nb) = (0usize, 0usize);
                for job in jobs {
                    if na <= nb { na += job.4.len(); a.push(job); }
                    else { nb += job.4.len(); b.push(job); }
                }
                let convert = |jobs: Vec<(u32, u32, usize, usize, &mut [u8])>| {
                    let mut ok = true;
                    for (mip, slice, pitch, depth_pitch, dst) in jobs {
                        if let Err(e) = layout.detile(tiled, mip, slice, dst, pitch, depth_pitch) {
                            tracing::warn!(?e, mip, slice, "parallel texture not detiled");
                            ok = false;
                        }
                    }
                    ok
                };
                return std::thread::scope(|scope| {
                    let convert_ref = &convert;
                    let worker = scope.spawn(move || convert_ref(b));
                    let first = convert(a);
                    let second = worker.join().unwrap_or(false);
                    first && second
                });
            }
        }
        for (i, (footprint, &rows)) in self.footprints.iter().zip(&self.rows).enumerate() {
            if self.part(i).0 < first { continue; }
            if let Some(Some(hash)) = new.get(i) {
                if old.get(i) == Some(&Some(*hash)) { continue; }
            }
            let (mip, slice) = self.part(i);
            let f = footprint.Footprint;
            let pitch = f.RowPitch as usize;
            let len = pitch * rows as usize * f.Depth as usize;
            // SAFETY: the subresource's part of the mapped staging buffer.
            let linear = unsafe {
                std::slice::from_raw_parts_mut(data.add((footprint.Offset - base) as usize), len)
            };
            if let Err(e) =
                self.layout
                    .detile(tiled, mip, slice, linear, pitch, pitch * rows as usize)
            {
                tracing::warn!(?e, mip, slice, "texture not detiled");
                return false;
            }
        }
        true
    }
}

/// Lets go of a title texture's staging buffer (lists that recorded an
/// upload from it hold it until they have run); the next upload makes one
/// anew.
pub(crate) fn unstage(resource: This) {
    let retired = {
        let mut map = STAGING.lock().unwrap_or_else(|e| e.into_inner());
        map.as_mut().and_then(|map| map.remove(&(resource as usize)))
    };
    // Releasing a staging resource may release COM objects and upload slots.
    // Do not serialize unrelated texture lookups behind that destruction.
    drop(retired);
}

/// Has the next list that uploads a title texture with a staging buffer
/// copy it whole, changed or not since: what the check at a submission
/// finds changed is copied by the next list that samples the texture
/// through a table, and there may be none for one that shaders now pick by
/// index alone.
pub(crate) fn upload_again(resource: This) {
    let staging = {
        let map = STAGING.lock().unwrap_or_else(|e| e.into_inner());
        map.as_ref().and_then(|map| map.get(&(resource as usize)).cloned())
    };
    if let Some(staging) = staging {
        let mut last = staging.last.lock().unwrap_or_else(|e| e.into_inner());
        staging.uploaded.store(false, std::sync::atomic::Ordering::Relaxed);
        // A GPU-only revision has no CPU bytes to replay. Force the next
        // refresh to rebuild it rather than treating its old hash as data.
        last.require_cpu_revision();
        last.partial = None;
        last.sparse_copies = Default::default();
    }
}

/// Where a staged title texture's memory is: address and bytes.
pub(crate) fn staged_memory(resource: This) -> Option<(u64, u64)> {
    let map = STAGING.lock().unwrap_or_else(|e| e.into_inner());
    let (at, len) = map.as_ref()?.get(&(resource as usize))?.memory();
    Some((at as u64, len as u64))
}

/// A fingerprint of the title memory `[at, at + len)`, never 0: of all of
/// it up to 48 KiB, else of twelve pieces of 4 KiB spread evenly over it,
/// the first and the last included. Read through the kernel, so pages not
/// there count as zeros. (A texture whose change falls between the pieces
/// is missed.)
pub(crate) fn fingerprint(at: u64, len: u64) -> u64 {
    const PIECE: usize = 4 << 10;
    const PIECES: usize = 12;
    thread_local! {
        static READ: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    let len = len as usize;
    READ.with(|read| {
        let mut read = read.borrow_mut();
        read.resize(PIECES * PIECE, 0);
        let mut hasher = blake3::Hasher::new();
        let mut take = |from: usize, bytes: usize| {
            read[..bytes].fill(0);
            crate::host::copy_mapped(read.as_mut_ptr(), (at as usize + from) as *const u8, bytes, false);
            hasher.update(&read[..bytes]);
        };
        if len <= PIECES * PIECE {
            take(0, len);
        } else {
            for n in 0..PIECES - 1 {
                take(((len - PIECE) / (PIECES - 1) * n) & !0xfff, PIECE);
            }
            take(len - PIECE, PIECE);
        }
        let hash = hasher.finalize();
        let mut first = [0u8; 8];
        first.copy_from_slice(&hash.as_bytes()[..8]);
        // 0 says "none" to the texture queue (`Recheck`, `checked`).
        u64::from_le_bytes(first) | 1
    })
}

/// Whether a title texture has a staging buffer: a list recorded its
/// upload, and the lists that sample it check it at submission.
/// Whether a list has uploaded `resource`'s staging buffer and whether it
/// was detiled since. None: staging has not been allocated.
pub(crate) fn staged_state(resource: This) -> Option<(bool, bool)> {
    use std::sync::atomic::Ordering::Relaxed;
    let map = STAGING.lock().unwrap_or_else(|e| e.into_inner());
    let staging = map.as_ref()?.get(&(resource as usize))?;
    Some((staging.uploaded.load(Relaxed), staging.dirty.load(Relaxed)))
}

pub(crate) fn is_staged(resource: This) -> bool {
    let map = STAGING.lock().unwrap_or_else(|e| e.into_inner());
    map.as_ref().is_some_and(|map| map.contains_key(&(resource as usize)))
}

/// The bytes of a title texture's memory, and whether all of it is mapped,
/// if the texture has been staged for upload.
pub(crate) fn staged(resource: This) -> Option<(u64, bool)> {
    let s = {
        let map = STAGING.lock().unwrap_or_else(|e| e.into_inner());
        map.as_ref()?.get(&(resource as usize))?.clone()
    };
    Some((s.size as u64, committed(s.memory, s.size)))
}

/// Whether `[at, at + len)` is committed memory (streamed textures may be
/// unmapped until their pages arrive).
pub(crate) fn committed(at: *const u8, len: usize) -> bool {
    if by_query() {
        return committed_by_query(at, len);
    }
    if len == 0 {
        return true;
    }
    let (there, of) = readable(at as usize, len, true);
    there == of
}

/// Whether the two questions above and below are put to `VirtualQuery`, as
/// they were (`VICTORY_COMMITTED_QUERY=1`). `VirtualQuery` answers with the
/// length of the run of pages in the same state, which it finds by walking
/// the page tables to the run's end: 8.8 ms for one question in the title's
/// 1.1 GB block (round 6). The pass over the heap's views asks three or
/// more of them for every texture slot that is due, 3,000 slots a frame
/// when the card has no room (run 75): that pass then ran 208-264 ms, and
/// while it ran the submitting thread's write-watch askings waited behind
/// the walks (run 76: 8-22 askings a frame of 6-21 ms each, 46-172 ms).
fn by_query() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_COMMITTED_QUERY").is_some_and(|v| v == "1"))
}

/// Of the kernel's pages that hold `[at, at + len)` (`len > 0`): how many
/// can be read, and how many there are; with `all`, it stops at the first
/// that cannot. One question to the kernel for up to 256 pages at once
/// (`QueryWorkingSetEx`: the state of each page asked about, no walk); a
/// page not in the working set may still be there (never touched, or
/// trimmed), and a one-byte read through the kernel says.
/// A page that is committed and may not be read counts as not there, which
/// is what the callers ask for: they read or write the memory next.
fn readable(at: usize, len: usize, all: bool) -> (usize, usize) {
    const KERNEL_PAGE: usize = 4096;
    let (first, last) = (at / KERNEL_PAGE, at.saturating_add(len - 1) / KERNEL_PAGE);
    let of = last - first + 1;
    let mut there = 0;
    // SAFETY: plain data.
    let mut info: [PSAPI_WORKING_SET_EX_INFORMATION; 256] = unsafe { std::mem::zeroed() };
    let mut page = first;
    while page <= last {
        let n = (last - page + 1).min(info.len());
        let asked = query_working_set(&mut info[..n], page * KERNEL_PAGE, KERNEL_PAGE);
        for entry in &info[..n] {
            if page_readable(entry, asked) {
                there += 1;
            } else if all {
                return (there, of);
            }
        }
        page += n;
    }
    (there, of)
}

fn query_working_set(info: &mut [PSAPI_WORKING_SET_EX_INFORMATION], first: usize, stride: usize) -> bool {
    for (i, entry) in info.iter_mut().enumerate() {
        entry.VirtualAddress = (first + i * stride) as *mut _;
    }
    // SAFETY: the entries describe addresses in our own process and the
    // byte count is the length of the supplied buffer.
    unsafe { K32QueryWorkingSetEx(GetCurrentProcess(), info.as_mut_ptr().cast(), size_of_val(info) as u32) != 0 }
}

fn page_readable(info: &PSAPI_WORKING_SET_EX_INFORMATION, asked: bool) -> bool {
    // SAFETY: bit 0 of a successful query's returned flags is Valid.
    let valid = asked && unsafe { info.VirtualAttributes.Flags } & 1 != 0;
    valid || crate::host::read_into(info.VirtualAddress.cast(), &mut [0u8])
}

fn committed_by_query(at: *const u8, len: usize) -> bool {
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEMORY_BASIC_INFORMATION, VirtualQuery};
    let mut p = at as usize;
    let end = p + len;
    while p < end {
        // SAFETY: plain data filled in by the query.
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a valid out pointer of the right size.
        let n = unsafe { VirtualQuery(p as *const _, &mut info, size_of_val(&info)) };
        if n == 0 || info.State != MEM_COMMIT {
            return false;
        }
        p = info.BaseAddress as usize + info.RegionSize;
    }
    true
}

/// How many bytes of `[at, at + len)` are committed memory: all of a
/// texture that is there, some of one streaming in (its smaller mips lie
/// at the start of its memory, and their pages are mapped first), none of
/// one not there. A texture can also have only interior mips present while
/// streaming. Windows can skip each uncommitted run in a single query.
pub(crate) fn mapped_bytes(at: *const u8, len: usize) -> u64 {
    let start = at as usize;
    let end = start.saturating_add(len);
    if start == end {
        return 0;
    }
    let query = by_query();
    let first_readable = !query && committed(at, 1);
    let ends_missing = !query && !first_readable && !committed((end - 1) as *const u8, 1);
    if ends_missing && (end - 1) / 4096 - start / 4096 <= 1 {
        return 0;
    }
    if !query && !ends_missing {
        static BY_PAGE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *BY_PAGE.get_or_init(|| std::env::var_os("VICTORY_MAPPED_BY_PAGE").is_some_and(|v| v == "1")) {
            // By page, without the walk: whole pages, cut to the range.
            let (there, _) = readable(start, len, false);
            return (there as u64 * 4096).min(len as u64);
        }
        // By 64 KiB block, one page asked of each: the title's memory is
        // mapped in blocks of that size (the console's page), and the
        // answer is a measure of how much of a texture has arrived, not a
        // promise that every byte can be read (the upload reads through the
        // kernel). By page, a texture half there cost a failed kernel read
        // for each of its missing pages, every frame it stayed due: 1,663
        // textures "uploaded in part" kept the pass over the heap's views at
        // 90-160 ms a frame (run 81). `VICTORY_MAPPED_BY_PAGE=1`: by page.
        return mapped_blocks(start, end, first_readable);
    }
    mapped_regions(start, end, false)
}

/// Asks about one sample per full guest block, in groups rather than a
/// kernel call per block. The first block's sample was already checked.
fn mapped_blocks(start: usize, end: usize, first_readable: bool) -> u64 {
    const BLOCK: usize = 64 << 10;
    let mut block = (start & !(BLOCK - 1)).saturating_add(BLOCK).min(end);
    let mut there = if first_readable { (block - start) as u64 } else { 0 };
    if block == end {
        return there;
    }
    // SAFETY: zero is a valid initial value for these query structures.
    let mut info: [PSAPI_WORKING_SET_EX_INFORMATION; 256] = unsafe { std::mem::zeroed() };
    while block < end {
        let count = (end - block).div_ceil(BLOCK).min(info.len());
        let asked = query_working_set(&mut info[..count], block, BLOCK);
        for entry in &info[..count] {
            let next = block.saturating_add(BLOCK).min(end);
            if page_readable(entry, asked) {
                there += (next - block) as u64;
            }
            block = next;
        }
    }
    there
}

/// The sweep only needs to discover whether a texture has any data to stage.
pub(crate) fn any_mapped(start: usize, len: usize) -> bool {
    let end = start.saturating_add(len);
    if start == end {
        return false;
    }
    if committed(start as *const u8, 1) {
        return true;
    }
    (end - 1) / 4096 > start / 4096 && mapped_regions(start, end, true) > 0
}

fn mapped_regions(start: usize, end: usize, first_only: bool) -> u64 {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
    };
    let (mut p, mut there) = (start, 0u64);
    while p < end {
        // SAFETY: plain data filled in by the query.
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a valid out pointer of the right size.
        let n = unsafe { VirtualQuery(p as *const _, &mut info, size_of_val(&info)) };
        let next = (info.BaseAddress as usize).saturating_add(info.RegionSize).min(end);
        if n == 0 || next <= p {
            break;
        }
        if info.State == MEM_COMMIT && info.Protect & (PAGE_NOACCESS | PAGE_GUARD) == 0 {
            there += (next - p) as u64;
            if first_only {
                break;
            }
        }
        p = next;
    }
    there
}

/// The first mip a staging buffer holds and the bytes of title memory read
/// for it and the smaller ones, for a texture of `mips` mips a view shows
/// from `first` on: from `first` only where the layout can end behind the
/// smaller mips (one 2-D slice: `bytes_from` is the whole layout for any
/// other), else all of it.
fn staged_part(layout: &victory_formats::texture::Layout, mips: u32, first: u32) -> (u32, usize) {
    let first = if first < mips && layout.bytes_from(first) < layout.size { first } else { 0 };
    (first, layout.bytes_from(first) as usize)
}

/// The staging state of a title texture from mip `first` on, made on first
/// use (and made anew for larger mips than the one there holds).
fn staging(resource: This, texture: &ID3D12Resource, first: u32) -> Option<Arc<Staging>> {
    let mut map = STAGING.lock().unwrap_or_else(|e| e.into_inner());
    let map = map.get_or_insert_with(HashMap::new);
    if let Some(s) = map.get(&(resource as usize))
        && s.first <= first
    {
        return Some(s.clone());
    }
    let (memory, desc) = crate::resource::texture_memory(resource)?;
    let Some(layout) = crate::resource::texture_layout(&desc).map(|t| t.planes[0].layout.clone())
    else {
        once!(
            ?desc,
            "texture contents without a console layout not uploaded"
        );
        return None;
    };
    // A buffer-to-texture copy needs a single sample and a known guest
    // address equation. Some render-XOR layouts satisfy both.
    if layout.desc.samples != 1 || !layout.addressable() {
        once!(?desc, "texture contents without a supported staging layout not uploaded");
        return None;
    }
    let host = host()?;
    // SAFETY: a live resource.
    let d = unsafe { texture.GetDesc() };
    let (first, size) = staged_part(&layout, u32::from(d.MipLevels), first);
    let count = if d.Dimension == D3D12_RESOURCE_DIMENSION_TEXTURE3D {
        u32::from(d.MipLevels)
    } else {
        u32::from(d.MipLevels) * u32::from(d.DepthOrArraySize) - first
    };
    let mut footprints = vec![D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default(); count as usize];
    let mut rows = vec![0u32; count as usize];
    let mut total = 0;
    // SAFETY: outputs for `count` subresources.
    unsafe {
        host.device.GetCopyableFootprints(
            &d,
            first,
            count,
            0,
            Some(footprints.as_mut_ptr()),
            Some(rows.as_mut_ptr()),
            None,
            Some(&mut total),
        );
    }
    if total == 0 || total == u64::MAX {
        return None;
    }
    // Detiling addresses complete pitched rows, including the last row's
    // padding, which GetCopyableFootprints can omit from its byte total.
    let mut bytes = total;
    for (footprint, &rows) in footprints.iter().zip(&rows) {
        let f = footprint.Footprint;
        let padded = u64::from(f.RowPitch).checked_mul(u64::from(rows))?.checked_mul(u64::from(f.Depth))?;
        bytes = bytes.max(footprint.Offset.checked_add(padded)?);
    }
    tracing::debug!(?desc, total, "texture contents uploaded from title memory");
    STAGED[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    STAGED[1].fetch_add(bytes, std::sync::atomic::Ordering::Relaxed);
    let s = Arc::new(Staging {
        inline: OnceLock::new(),
        #[cfg(feature = "diagnostics")]
        total,
        footprints,
        rows,
        layout,
        mips: u32::from(d.MipLevels),
        first,
        size,
        bytes,
        memory,
        last: Mutex::default(),
        dirty: std::sync::atomic::AtomicBool::new(true),
        uploaded: std::sync::atomic::AtomicBool::new(false),
        used: std::sync::atomic::AtomicU64::new(crate::host::moment()),
    });
    map.insert(resource as usize, s.clone());
    Some(s)
}

#[cfg(test)]
mod first_mip_tests {
    use victory_formats::texture::{Dimension, Swizzle, Texture, TextureDesc};

    fn layout(depth_or_array: u32) -> victory_formats::texture::Layout {
        let desc = TextureDesc {
            dimension: Dimension::Tex2d,
            width: 1024,
            height: 1024,
            depth_or_array,
            mips: 9,
            format: 71,
            samples: 1,
            swizzle: Swizzle::SW_4KB_S,
        };
        Texture::new(&desc).unwrap().planes[0].layout.clone()
    }

    /// A view from mip 4 of a 1024x1024 BC1 texture of 9 mips stages mips
    /// 4 to 8 and reads only the memory they take; a view of all of it, an
    /// array's view, or a first mip past the chain stage all of it.
    #[test]
    fn a_view_of_the_smaller_mips_stages_only_their_memory() {
        let l = layout(1);
        assert_eq!(super::staged_part(&l, 9, 4), (4, l.bytes_from(4) as usize));
        assert!(l.bytes_from(4) < l.size);
        assert_eq!(super::staged_part(&l, 9, 0), (0, l.size as usize));
        assert_eq!(super::staged_part(&l, 9, 9), (0, l.size as usize), "past the chain");
        let array = layout(2);
        assert_eq!(super::staged_part(&array, 9, 4), (0, array.size as usize), "slices each hold their chain");
    }

    #[test]
    fn partial_mips_refresh_on_a_new_write_without_waiting_for_the_poll() {
        let mut last = super::Last::default();
        assert!(last.partial_due(40, true));
        assert!(!last.partial_due(41, false));
        assert!(last.advance_watch(2, &[(17, 3)]));
        assert!(last.partial_due(42, true));
        assert!(!last.advance_watch(2, &[(17, 3)]));
        assert!(!last.partial_due(43, false));
        assert!(last.partial_due(72, false));
        last.partial_time = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        assert!(last.partial_due(73, false));
    }

    #[test]
    fn unchanged_sparse_tail_is_copied_once_per_revision() {
        let mut copies = super::SparseCopyProgress::default();
        let canceled = copies.claim(3).unwrap();
        assert!(copies.claim(3).is_none());
        assert!(copies.claim(4).is_none());
        drop(canceled);

        // Dropping a prelude before submitting it must leave the upload due.
        let coarse = copies.claim(3).unwrap();
        copies.submitted(3);
        drop(coarse);
        assert!(copies.claim(3).is_none());

        // A later budget increase can map finer tiles for the same bytes.
        let sharper = copies.claim(1).unwrap();
        assert!(copies.claim(2).is_none());
        copies.submitted(1);
        copies.submitted(3);
        drop(sharper);
        assert_eq!(copies.submitted, Some(1));
        assert!(copies.claim(0).is_some());

        // Updating the CPU staging lease makes even the same tail due again.
        copies = Default::default();
        assert!(copies.claim(3).is_some());
    }

    /// A 2-D view's first mip is its fifth word; other views show all mips.
    #[test]
    fn a_2d_view_names_its_first_mip() {
        assert_eq!(crate::resource::first_mip(&[71, 4, 0x1688, 0, 4, 5, 0, 0, 0, 0]), 4);
        assert_eq!(crate::resource::first_mip(&[71, 5, 0x1688, 0, 4, 5, 0, 0, 0, 0]), 0, "an array view");
        assert_eq!(crate::resource::first_mip(&[71, 4, 0x1688, 0, 99, 5, 0, 0, 0, 0]), 15);
    }
}

#[cfg(test)]
mod title_memory_tests {
    use super::with_title;

    #[test]
    fn batched_residency_samples_keep_holes_and_clip_edge_blocks() {
        use windows_sys::Win32::System::Memory::*;
        const BLOCK: usize = 64 << 10;
        // SAFETY: this reservation is private to the test. The committed
        // blocks include an untouched page so the query's read fallback runs.
        unsafe {
            let base = VirtualAlloc(std::ptr::null(), 260 * BLOCK, MEM_RESERVE, PAGE_NOACCESS).cast::<u8>();
            assert!(!base.is_null());
            for block in [0, 2, 255, 256, 258] {
                let at = base.add(block * BLOCK);
                assert_eq!(VirtualAlloc(at.cast(), BLOCK, MEM_COMMIT, PAGE_READWRITE), at.cast());
                if block != 256 {
                    at.write(0x57);
                }
            }
            let start = base.add(21);
            let end = base.add(258 * BLOCK + 117);
            assert_eq!(
                super::mapped_blocks(start as usize, end as usize, true),
                (4 * BLOCK - 21 + 117) as u64,
            );
            assert_eq!(
                super::mapped_blocks(base.add(BLOCK + 9) as usize, end as usize, false),
                (3 * BLOCK + 117) as u64,
            );
            assert_eq!(super::mapped_bytes(start, end.offset_from(start) as usize), (4 * BLOCK - 21 + 117) as u64);
            assert_ne!(VirtualFree(base.cast(), 0, MEM_RELEASE), 0);
        }
    }

    #[test]
    fn pooled_uploads_copy_their_own_texels_from_nonzero_offsets() {
        use super::*;
        use windows::core::{IUnknown, Interface};

        let Some(host) = host() else { return };
        let words = [3u32, 0, 0, 0, 16, 0, 4, 65537, 10, 1, 0, 256, 3_145_728, 0];
        // SAFETY: a known 16x4, single-mip title texture description.
        let desc = unsafe { std::mem::transmute::<[u32; 14], crate::resource::ResourceDesc>(words) };
        let sources = [vec![0x57u8; 65536], vec![0xa9u8; 65536]];
        let list = host.new_list().unwrap();
        const PITCHED: u64 = 1024;
        let readback = host.read_back_buffer(2 * PITCHED).unwrap();
        let mut owners = Vec::new();
        let mut offsets = Vec::new();
        let mut staged = Vec::new();
        for (i, source) in sources.iter().enumerate() {
            let mut title = std::ptr::null();
            assert_eq!(crate::resource::create_at_address(std::ptr::null_mut(), source.as_ptr() as u64, &desc,
                0, std::ptr::null(), &crate::resource::PAGEABLE_IIDS[0], &mut title), 0);
            // SAFETY: take the title resource's initial reference.
            owners.push(unsafe { IUnknown::from_raw(title.cast_mut().cast()) });
            let copy = crate::resource::host_copy(title).unwrap();
            upload(&list, title, &copy.texture);
            let staging = staging(title, &copy.texture, 0).unwrap();
            offsets.push(staging.inline.get().unwrap().offset);
            // Release the registry's reference before submission. The list
            // still owns the staging lease and must retain its bytes.
            unstage(title);
            upload(&list, title, &copy.texture);
            assert!(!is_staged(title), "another view of this mip must reuse the recorded upload");
            crate::list::ensure_state(&list, title, D3D12_RESOURCE_STATE_COPY_SOURCE.0 as u32);
            let mut footprint = staging.footprints[0];
            footprint.Offset = i as u64 * PITCHED;
            let from = D3D12_TEXTURE_COPY_LOCATION {
                pResource: unsafe { std::mem::transmute_copy(&copy.texture) },
                Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
            };
            let to = D3D12_TEXTURE_COPY_LOCATION {
                pResource: unsafe { std::mem::transmute_copy(&readback) },
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { PlacedFootprint: footprint },
            };
            // SAFETY: a live list, two disjoint readback offsets, and the
            // default texture now in copy-source state.
            unsafe { list.list.CopyTextureRegion(&to, 0, 0, 0, &from, None) };
            staged.push(staging);
        }
        if std::env::var_os("VICTORY_TEXTURE_UPLOAD_POOL").is_none_or(|v| v != "0") {
            assert!(offsets.iter().any(|&offset| offset != 0));
        }
        list.close();
        host.execute(&[&list]);
        for staging in staged {
            use std::sync::atomic::Ordering::Relaxed;
            assert!(staging.uploaded.load(Relaxed));
            assert!(!staging.dirty.load(Relaxed), "the submitted upload consumed the refreshed texels");
        }
        // SAFETY: the fence has completed and both regions are mapped.
        unsafe {
            let mut data = std::ptr::null_mut();
            readback.Map(0, None, Some(&mut data)).unwrap();
            assert_eq!(*data.cast::<u8>(), 0x57);
            assert_eq!(*data.cast::<u8>().add(PITCHED as usize), 0xa9);
            readback.Unmap(0, Some(&D3D12_RANGE::default()));
        }
    }

    #[test]
    fn sparse_texture_backs_only_the_sampled_tail_until_finer_mips_arrive() {
        use super::*;
        use windows_sys::Win32::System::Memory::*;

        let Some(host) = host() else { return };
        if !crate::tiles::supported(&host.device) {
            return;
        }
        let words = [3u32, 0, 0, 0, 1024, 0, 1024, 4 << 16 | 1, 71, 1, 0, 256, 0, 0];
        // SAFETY: the title descriptor layout and a single-slice linear BC1 test texture.
        let desc = unsafe { std::mem::transmute::<[u32; 14], crate::resource::ResourceDesc>(words) };
        let layout = &crate::resource::texture_layout(&desc).unwrap().planes[0].layout;
        let bytes = layout.size as usize;
        let coarse = (layout.bytes_from(2) as usize).next_multiple_of(4096);
        assert!(coarse < bytes);
        // SAFETY: one private allocation with only the coarse mip tail committed.
        let source = unsafe {
            let base = VirtualAlloc(std::ptr::null(), bytes, MEM_RESERVE, PAGE_NOACCESS).cast::<u8>();
            assert!(!base.is_null());
            assert_eq!(VirtualAlloc(base.cast(), coarse, MEM_COMMIT, PAGE_READWRITE), base.cast());
            base.write_bytes(0x37, coarse);
            base
        };
        let title = crate::resource::texture_at(source as u64, &words, 2).unwrap();
        let copy = crate::resource::host_copy(title).unwrap();
        let sparse = copy.sparse.as_ref().expect("eligible synthetic texture is reserved");
        assert_eq!(crate::resource::sparse_sample(title).unwrap().1, None);
        let submit = |first| {
            let list = host.new_list().unwrap();
            assert!(upload_from(&list, title, &copy.texture, first));
            list.close();
            host.execute(&[&list]);
        };
        submit(2);
        assert_eq!(crate::resource::sparse_sample(title).unwrap().1, Some(2));
        assert_eq!(staged_state(title), Some((true, false)));
        let tail = sparse.tiles.backed_bytes();
        assert_ne!(tail, 0);
        // SAFETY: storage belongs to this test and the first submission finished.
        unsafe {
            let fine = source.add(coarse).cast();
            assert_eq!(VirtualAlloc(fine, bytes - coarse, MEM_COMMIT, PAGE_READWRITE), fine);
        }
        crate::watch::title_memory_remapped(source as usize, bytes);
        submit(0);
        assert_eq!(crate::resource::sparse_sample(title).unwrap().1, Some(0));
        assert_eq!(staged_state(title), Some((true, false)));
        let full = sparse.tiles.backed_bytes();
        assert!(tail < full);
        eprintln!("reserved BC1 texture resident bytes: coarse {tail}, full {full}");
        unstage(title);
        assert_ne!(unsafe { VirtualFree(source.cast(), 0, MEM_RELEASE) }, 0);
    }

    #[test]
    fn sampled_partial_mip_refresh_keeps_the_previous_fine_level() {
        use super::*;
        use windows::core::{IUnknown, Interface};
        use windows_sys::Win32::System::Memory::*;

        let Some(host) = host() else { return };
        let words = [3u32, 0, 0, 0, 512, 0, 512, 3 << 16 | 1, 10, 1, 0, 256, 3_145_728, 0];
        // SAFETY: the title resource description for a single-slice, three-mip texture.
        let desc = unsafe { std::mem::transmute::<[u32; 14], crate::resource::ResourceDesc>(words) };
        let layout = &crate::resource::texture_layout(&desc).unwrap().planes[0].layout;
        let bytes = layout.size as usize;
        let coarse = (layout.bytes_from(1) as usize).next_multiple_of(64 << 10);
        assert!(layout.mips[0].offset as usize >= coarse && bytes > coarse);
        // SAFETY: private committed memory owned by this test.
        let source = unsafe {
            VirtualAlloc(std::ptr::null(), bytes, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE).cast::<u8>()
        };
        assert!(!source.is_null());
        unsafe { source.write_bytes(0x56, bytes) };
        let mut title = std::ptr::null();
        assert_eq!(crate::resource::create_at_address(std::ptr::null_mut(), source as u64, &desc,
            0, std::ptr::null(), &crate::resource::PAGEABLE_IIDS[0], &mut title), 0);
        // SAFETY: take the title object's initial reference.
        let owner = unsafe { IUnknown::from_raw(title.cast_mut().cast()) };
        let copy = crate::resource::host_copy(title).unwrap();
        let staging = staging(title, &copy.texture, 0).unwrap();
        let sample = || {
            let readback = host.read_back_buffer(staging.bytes).unwrap();
            let list = host.new_list().unwrap();
            assert!(upload_from(&list, title, &copy.texture, 0));
            crate::list::ensure_state(&list, title, D3D12_RESOURCE_STATE_COPY_SOURCE.0 as u32);
            for (mip, &footprint) in staging.footprints.iter().take(2).enumerate() {
                let src = D3D12_TEXTURE_COPY_LOCATION {
                    pResource: unsafe { std::mem::transmute_copy(&copy.texture) },
                    Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: mip as u32 },
                };
                let dst = D3D12_TEXTURE_COPY_LOCATION {
                    pResource: unsafe { std::mem::transmute_copy(&readback) },
                    Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { PlacedFootprint: footprint },
                };
                // SAFETY: live resources in their copy states and disjoint footprints.
                unsafe { list.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None) };
            }
            list.close();
            host.execute(&[&list]);
            // SAFETY: the list completed and the readback buffer remains live.
            unsafe {
                let mut data = std::ptr::null_mut();
                readback.Map(0, None, Some(&mut data)).unwrap();
                let pair = [0, 1].map(|mip| *data.cast::<u8>().add(staging.footprints[mip].Offset as usize));
                readback.Unmap(0, Some(&D3D12_RANGE::default()));
                pair
            }
        };
        assert_eq!(sample(), [0x56, 0x56]);
        // SAFETY: the first submission has completed and only the fine mip's pages leave.
        unsafe {
            assert_ne!(VirtualFree(source.add(coarse).cast(), bytes - coarse, MEM_DECOMMIT), 0);
            source.write_bytes(0x9b, coarse);
        }
        crate::watch::title_memory_remapped(source as usize, bytes);
        upload_again(title);
        assert_eq!(sample(), [0x56, 0x9b]);
        // SAFETY: recommit the missing level. Its valid zero texels match
        // the prior sparse snapshot's hash but must now reach the GPU.
        unsafe {
            let fine = source.add(coarse).cast();
            assert_eq!(VirtualAlloc(fine, bytes - coarse, MEM_COMMIT, PAGE_READWRITE), fine);
        }
        crate::watch::title_memory_remapped(source as usize, bytes);
        upload_again(title);
        assert_eq!(sample(), [0x00, 0x9b]);
        drop(staging);
        drop(copy);
        drop(owner);
        assert_ne!(unsafe { VirtualFree(source.cast(), 0, MEM_RELEASE) }, 0);
    }

    #[test]
    fn sampled_uploads_follow_guest_writes_before_the_first_read() {
        use super::*;
        use windows::core::{IUnknown, Interface};

        let Some(host) = host() else { return };
        let words = [3u32, 0, 0, 0, 16, 0, 4, 65537, 10, 1, 0, 256, 3_145_728, 0];
        // SAFETY: a 16x4 single-mip title texture, as in the pooled-copy test.
        let desc = unsafe { std::mem::transmute::<[u32; 14], crate::resource::ResourceDesc>(words) };
        let mut source = vec![0x35u8; 65536];
        let mut title = std::ptr::null();
        assert_eq!(crate::resource::create_at_address(std::ptr::null_mut(), source.as_ptr() as u64,
            &desc, 0, std::ptr::null(), &crate::resource::PAGEABLE_IIDS[0], &mut title), 0);
        // SAFETY: own the returned title resource until the test ends.
        let _owner = unsafe { IUnknown::from_raw(title.cast_mut().cast()) };
        let copy = crate::resource::host_copy(title).unwrap();
        let staging = staging(title, &copy.texture, 0).unwrap();
        const PITCHED: u64 = 1024;
        let readback = host.read_back_buffer(6 * PITCHED).unwrap();
        let record = |index: u64, change: Option<(&[u8], *mut u8)>| {
            let list = host.new_list().unwrap();
            if let Some((bytes, dst)) = change {
                list.defer_copy(dst, bytes.as_ptr(), bytes.len());
            }
            assert!(upload_from(&list, title, &copy.texture, 0));
            crate::list::ensure_state(&list, title, D3D12_RESOURCE_STATE_COPY_SOURCE.0 as u32);
            let mut footprint = staging.footprints[0];
            footprint.Offset = index * PITCHED;
            let src = D3D12_TEXTURE_COPY_LOCATION {
                // SAFETY: live borrowed resource pointers.
                pResource: unsafe { std::mem::transmute_copy(&copy.texture) },
                Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
            };
            let dst = D3D12_TEXTURE_COPY_LOCATION {
                pResource: unsafe { std::mem::transmute_copy(&readback) },
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { PlacedFootprint: footprint },
            };
            // SAFETY: a live list, a disjoint readback slot, and copy states.
            unsafe { list.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None) };
            list.close();
            list
        };
        host.execute(&[&record(0, None)]);
        let clean_at_recording = record(1, None);
        source.fill(0x58);
        crate::watch::wrote(source.as_ptr() as usize, source.len());
        host.execute(&[&clean_at_recording]);

        let bytes = vec![0x8bu8; source.len()];
        let first = record(2, None);
        let second = record(3, Some((&bytes, source.as_mut_ptr())));
        source.fill(0x72);
        crate::watch::wrote(source.as_ptr() as usize, source.len());
        host.execute(&[&first, &second]);

        let count = if crate::host::late() {
            let gate = host.gate_queue_for_test();
            let (sent, ready) = std::sync::mpsc::channel();
            let signal = std::thread::spawn(move || {
                // Let both lists prepare before the queue reads either slot.
                // The timeout also releases the queue if a test panics.
                let _ = ready.recv_timeout(std::time::Duration::from_secs(1));
                unsafe { gate.Signal(1).unwrap() };
            });
            source.fill(0x9b);
            crate::watch::wrote(source.as_ptr() as usize, source.len());
            host.execute_late(vec![crate::list::Detached::for_test(record(4, None))], vec![0]);
            source.fill(0xc4);
            crate::watch::wrote(source.as_ptr() as usize, source.len());
            host.execute_late(vec![crate::list::Detached::for_test(record(5, None))], vec![0]);
            let _ = sent.send(());
            signal.join().unwrap();
            crate::host::settle_for_fence();
            6
        } else {
            4
        };

        // SAFETY: all submissions have completed; each offset starts a readback.
        unsafe {
            let mut data = std::ptr::null_mut();
            readback.Map(0, None, Some(&mut data)).unwrap();
            for (i, pixel) in [0x35, 0x58, 0x72, 0x8b, 0x9b, 0xc4].into_iter().take(count).enumerate() {
                assert_eq!(*data.cast::<u8>().add(i * PITCHED as usize), pixel, "read {i}");
            }
            readback.Unmap(0, Some(&D3D12_RANGE::default()));
        }
    }

    #[test]
    fn a_texture_refresh_waits_only_for_its_own_mapping() {
        use std::sync::{atomic::{AtomicBool, Ordering}, mpsc};
        use std::time::{Duration, Instant};
        use windows::core::{IUnknown, Interface};
        use windows_sys::Win32::System::Memory::*;

        if crate::host::host().is_none() {
            return; // no D3D12 device
        }
        let words: [u32; 14] = [3, 0, 0, 0, 16, 0, 4, 65537, 10, 1, 0, 256, 3_145_728, 0];
        // SAFETY: a known title texture description. These private pages are
        // committed and live until its resource and staging have been released.
        let desc = unsafe { std::mem::transmute::<[u32; 14], crate::resource::ResourceDesc>(words) };
        let page = 64 << 10;
        let memory = unsafe {
            VirtualAlloc(std::ptr::null(), 2 * page, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE)
        };
        assert!(!memory.is_null());
        let at = memory as usize;
        unsafe { memory.cast::<u8>().write_bytes(0x57, 2 * page) };
        let mut title = std::ptr::null();
        assert_eq!(crate::resource::create_at_address(std::ptr::null_mut(), at as u64, &desc,
            0, std::ptr::null(), &crate::resource::PAGEABLE_IIDS[0], &mut title), 0);
        // SAFETY: take ownership of the resource's initial COM reference.
        let owner = unsafe { IUnknown::from_raw(title.cast_mut().cast()) };
        let host = crate::resource::host_copy(title).unwrap();
        let staging = super::staging(title, &host.texture, 0).unwrap();
        assert!(staging.size <= page);
        for (range, overlapping) in [(at + page..at + 2 * page, false), (at..at + page, true)] {
            *staging.last.lock().unwrap() = super::Last::default();
            let finished = AtomicBool::new(false);
            std::thread::scope(|s| {
                let (started, ready) = mpsc::channel();
                let finished = &finished;
                s.spawn(move || {
                    let _writer = crate::watch::title_memory_remapping_range(range);
                    started.send(()).unwrap();
                    std::thread::sleep(Duration::from_secs(1));
                    finished.store(true, Ordering::Release);
                });
                ready.recv().unwrap();
                let start = Instant::now();
                assert_eq!(staging.refresh_timed(false), staging.size);
                eprintln!("texture refresh, overlap={overlapping}: {:?}", start.elapsed());
                assert_eq!(finished.load(Ordering::Acquire), overlapping && !super::no_wait());
                // SAFETY: the first texel has been staged, and nothing is writing it.
                assert_eq!(unsafe { *staging.inline.get().unwrap().data() }, 0x57);
            });
        }
        drop(staging);
        drop(host);
        drop(owner);
        crate::watch::title_memory_remapped(at, 2 * page);
        // SAFETY: no remaining resource, reader, or writer uses this reservation.
        assert_ne!(unsafe { VirtualFree(memory, 0, MEM_RELEASE) }, 0);
    }

    #[test]
    fn a_texture_with_only_interior_pages_counts_its_resident_bytes() {
        use windows_sys::Win32::System::Memory::*;
        const BLOCK: usize = 64 << 10;
        // SAFETY: this private reservation is ours throughout the test.
        unsafe {
            let base = VirtualAlloc(std::ptr::null(), 5 * BLOCK, MEM_RESERVE, PAGE_NOACCESS).cast::<u8>();
            assert!(!base.is_null());
            let middle = base.add(2 * BLOCK);
            assert_eq!(VirtualAlloc(middle.cast(), BLOCK, MEM_COMMIT, PAGE_READWRITE), middle.cast());
            assert_eq!(super::mapped_bytes(base, 5 * BLOCK), BLOCK as u64);
            assert!(super::any_mapped(base as usize, 5 * BLOCK));
            assert_eq!(super::mapped_bytes(base.add(BLOCK - 3), 2 * BLOCK + 9), BLOCK as u64);
            assert_eq!(super::mapped_bytes(base, 2 * BLOCK), 0);
            assert_ne!(VirtualFree(middle.cast(), BLOCK, MEM_DECOMMIT), 0);
            assert_eq!(super::mapped_bytes(base, 5 * BLOCK), 0);
            assert!(!super::any_mapped(base as usize, 5 * BLOCK));
            assert_ne!(VirtualFree(base.cast(), 0, MEM_RELEASE), 0);
        }
    }

    /// Title memory whose tail is reserved and not committed reads through
    /// the copy as its bytes and then zeros, without a fault; so does
    /// memory a page of which is taken away between two reads, and the
    /// zeros are zeros whatever the read buffer held before.
    #[test]
    fn an_uncommitted_tail_reads_as_zeros() {
        use windows_sys::Win32::System::Memory::*;
        const HEAD: usize = 0xb_0000;
        const SIZE: usize = 0x20_0000;
        // SAFETY: a private reservation of our own; only committed pages
        // are written in place, and nothing else reads them in place.
        unsafe {
            // The read buffer holds something else first.
            let other = vec![0x77u8; SIZE];
            with_title(other.as_ptr(), SIZE, |bytes| assert!(bytes.iter().all(|&b| b == 0x77)));
            let base = VirtualAlloc(std::ptr::null(), SIZE, MEM_RESERVE, PAGE_NOACCESS).cast::<u8>();
            assert!(!base.is_null());
            assert_eq!(VirtualAlloc(base.cast(), HEAD, MEM_COMMIT, PAGE_READWRITE), base.cast());
            base.write_bytes(0x5a, HEAD);
            with_title(base, SIZE, |bytes| {
                assert!(bytes[..HEAD].iter().all(|&b| b == 0x5a), "what is mapped");
                assert!(bytes[HEAD..].iter().all(|&b| b == 0), "what is not: zeros");
            });
            // The title takes a page in the middle away: zeros from then
            // on, the rest as before.
            assert_ne!(VirtualFree(base.add(0x4_0000).cast(), 0x1_0000, MEM_DECOMMIT), 0);
            with_title(base, SIZE, |bytes| {
                assert!(bytes[..0x4_0000].iter().all(|&b| b == 0x5a));
                assert!(bytes[0x4_0000..0x5_0000].iter().all(|&b| b == 0));
                assert!(bytes[0x5_0000..HEAD].iter().all(|&b| b == 0x5a));
                assert!(bytes[HEAD..].iter().all(|&b| b == 0));
            });
            assert_ne!(VirtualFree(base.cast(), 0, MEM_RELEASE), 0);
        }
    }

    /// The texture both device-loss runs named last (16 texels wide, 4 or 8
    /// high, format 10, layout 0x100, console flags 0x300000; its memory
    /// began 0x300 into a page in run 73 and 0x700 in run 72, and here also
    /// 0xf00, so that a page line runs through it): what the detiling is
    /// given and what it writes are the same read in place and read through
    /// the copy, and the rows it writes end where the staging buffer the
    /// host sizes for it ends (896 bytes for 4 rows of 128 bytes 256 apart,
    /// 1,920 for 8).
    #[test]
    fn the_small_texture_of_runs_72_and_73_detiles_the_same_both_ways() {
        use windows_sys::Win32::System::Memory::*;
        for (height, within) in [(4u32, 0x300usize), (8, 0x700), (4, 0xf00), (8, 0xf00)] {
            // The description as the log printed it, word for word.
            let words: [u32; 14] = [3, 0, 0, 0, 16, 0, height, 65537, 10, 1, 0, 256, 3_145_728, 0];
            // SAFETY: fourteen words are a resource description (56 bytes,
            // plain numbers).
            let desc: crate::resource::ResourceDesc = unsafe { std::mem::transmute(words) };
            let layout = crate::resource::texture_layout(&desc)
                .expect("a console layout")
                .planes[0]
                .layout
                .clone();
            let size = layout.size as usize;
            let (row, pitch) = (16 * 8, 256usize);
            assert!(size >= row * height as usize, "the layout holds every texel");
            // Committed pages and a reserved one after them; the texture
            // begins `within` bytes into the first, as the title placed it.
            // SAFETY: our own reservation; the committed part is written
            // and read in place, by this thread alone.
            unsafe {
                let pages = size.div_ceil(4096) + 1;
                let base = VirtualAlloc(std::ptr::null(), (pages + 1) * 4096, MEM_RESERVE, PAGE_NOACCESS).cast::<u8>();
                assert!(!base.is_null());
                assert_eq!(VirtualAlloc(base.cast(), pages * 4096, MEM_COMMIT, PAGE_READWRITE), base.cast());
                for i in 0..pages * 4096 {
                    base.add(i).write((i * 7 + i / 251) as u8);
                }
                let memory = base.add(within);
                let in_place = std::slice::from_raw_parts(memory, size);
                // The bytes: the same in place and through the copy.
                with_title(memory, size, |copied| assert_eq!(copied, in_place));
                // What the detiling writes: the same, and inside the bytes
                // the host's staging buffer for this texture has.
                let staged = (height as usize - 1) * pitch + row;
                assert_eq!(staged, if height == 4 { 896 } else { 1920 });
                let detile = |tiled: &[u8]| {
                    // The slice `Staging::detile` hands over: whole rows.
                    let mut linear = vec![0xeeu8; pitch * height as usize];
                    layout
                        .detile(tiled, 0, 0, &mut linear, pitch, pitch * height as usize)
                        .expect("detiled");
                    linear
                };
                let direct = detile(in_place);
                let through = with_title(memory, size, |copied| detile(copied));
                assert_eq!(direct, through);
                for y in 0..height as usize {
                    assert!(direct[y * pitch..][..row].iter().any(|&b| b != 0xee), "row {y} written");
                    assert!(direct[y * pitch + row..(y + 1) * pitch].iter().all(|&b| b == 0xee), "nothing past row {y}");
                }
                // A buffer of exactly the staged bytes is enough.
                let mut exact = vec![0u8; staged];
                layout
                    .detile(in_place, 0, 0, &mut exact, pitch, pitch * height as usize)
                    .expect("the staging buffer's bytes are enough");
                assert_ne!(VirtualFree(base.cast(), 0, MEM_RELEASE), 0);
            }
        }
    }
}

#[cfg(test)]
mod gpu_only_revision_tests {
    use super::*;
    #[test]
    fn invalidation_requires_rebuilding_gpu_only_revision() {
        let mut last = Last { has_data: true, asked: 42, hash: Some(blake3::hash(b"texture")), ..Default::default() };
        last.require_cpu_revision();
        assert!(!last.has_data);
        assert_eq!(last.asked, 0);
        assert!(last.hash.is_none());
    }
    #[test]
    fn inline_revision_is_not_invalidated() {
        let mut last = Last { has_data: true, inline_current: true, asked: 42, ..Default::default() };
        last.require_cpu_revision();
        assert!(last.has_data);
        assert_eq!(last.asked, 42);
    }
}
