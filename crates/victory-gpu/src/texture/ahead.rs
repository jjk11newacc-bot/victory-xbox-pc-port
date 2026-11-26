//! First uploads of previously unused bindless textures on a D3D12 copy queue.
//! Submission preparation still snapshots title memory in command order. Only
//! the resulting immutable staging bytes cross to the copy worker.

use std::collections::VecDeque;
use std::sync::atomic::{
    AtomicU64,
    Ordering::{AcqRel, Acquire, Relaxed, Release},
};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D12::*;
use windows::core::Interface;

use super::{StagedCopy, Staging};
use crate::host::HostList;
use crate::inplace::ReadbackMapping;
use crate::object::This;
use crate::resource::HostTexture;

const FRESH: u8 = 0;
const NORMAL: u8 = 1;
const ARMED: u8 = 2;
const COPYING: u8 = 3;
const READY: u8 = 4;
const CANCELED: u8 = 5;

const MAX_BYTES: u64 = 96 << 20;
const BATCH_BYTES: u64 = 16 << 20;
const BATCH_COUNT: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Unavailable,
    Pending,
    Ready,
}

fn state(phase: u8) -> Status {
    match phase {
        ARMED | COPYING | CANCELED => Status::Pending,
        READY => Status::Ready,
        _ => Status::Unavailable,
    }
}

pub(crate) fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_UPLOAD_AHEAD").is_some_and(|v| v == "1"))
}

pub(crate) fn status(copy: &HostTexture) -> Status {
    let phase = copy.ahead.load(Acquire);
    if phase != READY {
        return state(phase);
    }
    let mut ready = copy.ahead_ready.lock().unwrap_or_else(|e| e.into_inner());
    if checked_ready(copy, &mut ready) {
        Status::Ready
    } else {
        Status::Unavailable
    }
}

/// Before naming a texture from an ordinary descriptor or upload path, make
/// its first queue choice atomic with the copy-ahead admission check.
pub(crate) fn claim_normal(copy: &HostTexture) -> Status {
    if !enabled() {
        return Status::Unavailable;
    }
    match copy.ahead.compare_exchange(FRESH, NORMAL, AcqRel, Acquire) {
        Ok(_) => Status::Unavailable,
        Err(READY) => status(copy),
        Err(phase) => state(phase),
    }
}

/// After the descriptor actually names this ready copy, all updates use the
/// normal upload path. This allocation cannot be admitted again.
pub(crate) fn retire(copy: &HostTexture) -> Option<u64> {
    let mut ready = copy.ahead_ready.lock().unwrap_or_else(|e| e.into_inner());
    if !checked_ready(copy, &mut ready) {
        return None;
    }
    let original = ready.take().expect("checked ready upload").fingerprint;
    copy.ahead.store(NORMAL, Release);
    Some(original)
}

/// A pending descriptor discovered a newer texture revision. Leave submitted
/// work hidden through its fence, then restart using the ordinary upload path.
pub(crate) fn discard(copy: &HostTexture) {
    let mut ready = copy.ahead_ready.lock().unwrap_or_else(|e| e.into_inner());
    let _ = copy
        .ahead
        .fetch_update(AcqRel, Acquire, |phase| match phase {
            ARMED => Some(NORMAL),
            COPYING => Some(CANCELED),
            _ => None,
        });
    if copy.ahead.load(Acquire) == READY {
        clear_ready(copy, &mut ready);
    }
}

fn checked_ready(copy: &HostTexture, ready: &mut Option<Ready>) -> bool {
    if copy.ahead.load(Acquire) != READY {
        return false;
    }
    if ready.as_ref().is_some_and(|ready| ready.mapping.live()) {
        return true;
    }
    clear_ready(copy, ready);
    false
}

fn clear_ready(copy: &HostTexture, ready: &mut Option<Ready>) {
    if let Some(staging) = ready.take().and_then(|ready| ready.staging.upgrade()) {
        staging_reset(&staging);
    }
    copy.ahead.store(NORMAL, Release);
}

pub(crate) struct Ready {
    staging: std::sync::Weak<Staging>,
    mapping: Arc<ReadbackMapping>,
    range: std::ops::Range<usize>,
    fingerprint: u64,
}

/// Keep the source mapping identity fixed through publication. The lookup
/// can become invalid without a view's contents or generation changing.
pub(crate) fn while_ready(copy: &HostTexture, publish: impl FnOnce()) -> bool {
    if copy.ahead.load(Acquire) != READY {
        return false;
    }
    let range = {
        let mut ready = copy.ahead_ready.lock().unwrap_or_else(|e| e.into_inner());
        checked_ready(copy, &mut ready).then(|| ready.as_ref().unwrap().range.clone())
    };
    let Some(range) = range else { return false };
    let Some(_mapped) = crate::watch::title_memory_reading_now(range) else {
        return false;
    };
    let mut ready = copy.ahead_ready.lock().unwrap_or_else(|e| e.into_inner());
    if !checked_ready(copy, &mut ready) {
        return false;
    }
    let image = ready.as_ref().expect("checked ready upload");
    if super::fingerprint(image.range.start as u64, image.range.len() as u64) != image.fingerprint {
        clear_ready(copy, &mut ready);
        rejected();
        return false;
    }
    publish();
    true
}

/// Claims an unpublished allocation before its queued descriptor can expose it.
pub(crate) fn request(resource: This, copy: &Arc<HostTexture>, first: u32) -> Status {
    if !enabled() {
        return Status::Unavailable;
    }
    let phase = copy.ahead.load(Acquire);
    if phase != FRESH {
        return status(copy);
    }
    if !eligible(resource, copy, first) || !start_worker() {
        rejected();
        return claim_normal(copy);
    }
    match copy.ahead.compare_exchange(FRESH, ARMED, AcqRel, Acquire) {
        Ok(_) => Status::Pending,
        Err(READY) => status(copy),
        Err(phase) => state(phase),
    }
}

fn eligible(resource: This, copy: &HostTexture, first: u32) -> bool {
    if first != 0
        || !crate::resource::ahead_eligible(resource)
        || copy.sparse.is_some()
        || copy.state.load(Acquire) != D3D12_RESOURCE_STATE_COMMON.0 as u32
        || super::host_wrote(resource as usize)
        || super::is_staged(resource)
        || crate::resource::alias_of(resource).is_some()
    {
        return false;
    }
    // SAFETY: the shared host texture is alive.
    let desc = unsafe { copy.texture.GetDesc() };
    desc.Dimension == D3D12_RESOURCE_DIMENSION_TEXTURE2D
        && desc.DepthOrArraySize == 1
        && desc.SampleDesc.Count == 1
        && desc.Flags.0 & 7 == 0
        && matches!(desc.Format.0, 70..=84 | 94..=99)
}

/// Register the title-memory dependency in the list even though the GPU
/// copy itself will run on another queue. Never transition this texture here.
pub(crate) fn plan(list: &HostList, resource: This, copy: &Arc<HostTexture>) -> Option<u64> {
    match copy.ahead.load(Acquire) {
        COPYING | CANCELED => return Some(0),
        ARMED => {}
        _ => return None,
    }
    let staging = super::staging(resource, &copy.texture, 0).filter(|staging| {
        staging.bytes <= BATCH_BYTES && super::committed(staging.memory, staging.size)
    });
    let Some(staging) = staging else {
        if copy
            .ahead
            .compare_exchange(ARMED, NORMAL, AcqRel, Acquire)
            .is_ok()
        {
            rejected();
        }
        return None;
    };
    let bytes = staging.bytes;
    list.defer_ahead_texture(staging, copy.clone());
    Some(bytes)
}

/// Captures the first revision during normal submission preparation. A copy
/// ahead job owns its staging allocation and destination until its fence.
pub(crate) fn stage(staging: &Arc<Staging>, copy: &Arc<HostTexture>) {
    if copy
        .ahead
        .compare_exchange(ARMED, COPYING, AcqRel, Acquire)
        .is_err()
    {
        return;
    }
    let job = prepare(staging, copy);
    if copy.ahead.load(Acquire) == CANCELED || job.is_none_or(|job| !enqueue(job)) {
        failed(staging, copy);
    }
}

fn staging_reset(staging: &Staging) {
    *staging.last.lock().unwrap_or_else(|e| e.into_inner()) = super::Last::default();
    staging.uploaded.store(false, Relaxed);
    staging.dirty.store(true, Relaxed);
}

fn failed(staging: &Staging, copy: &HostTexture) {
    staging_reset(staging);
    copy.ahead.store(NORMAL, Release);
    rejected();
}

fn prepare(staging: &Arc<Staging>, copy: &Arc<HostTexture>) -> Option<Job> {
    let (at, size) = staging.memory();
    let mapping = Arc::new(ReadbackMapping::record(at, size)?);
    if !super::committed(staging.memory, size) {
        return None;
    }
    // Keep the sampled identity from the ordered staging step. A new sample
    // taken at retirement would acknowledge guest writes the GPU never read.
    let fingerprint = super::fingerprint(at as u64, size as u64);
    staging.refresh(true);
    if !mapping.live() || super::fingerprint(at as u64, size as u64) != fingerprint {
        return None;
    }
    let last = staging.last.lock().unwrap_or_else(|e| e.into_inner());
    if !last.has_data || last.first != 0 || !super::committed(staging.memory, size) {
        return None;
    }
    Some(Job {
        copy: StagedCopy {
            texture: copy.clone(),
            staging: staging.clone(),
            // The copy-only queue cannot dispatch a GPU detile.
            upload: last.sampled.as_ref().filter(|u| u.gpu.is_none())?.clone(),
            first: 0,
            activate_tiles: false,
            sparse_claim: None,
        },
        mapping,
        fingerprint,
    })
}

struct Job {
    copy: StagedCopy,
    mapping: Arc<ReadbackMapping>,
    fingerprint: u64,
}

impl Job {
    fn finish(&self) -> bool {
        let staging = &self.copy.staging;
        let (at, bytes) = staging.memory();
        let range = at..at + bytes;
        let _mapped = crate::watch::title_memory_reading_range(range.clone());
        if !self.mapping.live() {
            return false;
        }
        let mut ready = self
            .copy
            .texture
            .ahead_ready
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.copy.texture.ahead.load(Acquire) != COPYING {
            return false;
        }
        let last = staging.last.lock().unwrap_or_else(|e| e.into_inner());
        if !last
            .sampled
            .as_ref()
            .is_some_and(|upload| Arc::ptr_eq(upload, &self.copy.upload))
        {
            return false;
        }
        staging.mark_uploaded();
        *ready = Some(Ready {
            staging: Arc::downgrade(staging),
            mapping: self.mapping.clone(),
            range,
            fingerprint: self.fingerprint,
        });
        self.copy.texture.ahead.store(READY, Release);
        true
    }
}

struct Pending {
    jobs: VecDeque<Job>,
    bytes: u64,
    accepting: bool,
}

static QUEUE: (Mutex<Pending>, Condvar) = (
    Mutex::new(Pending {
        jobs: VecDeque::new(),
        bytes: 0,
        accepting: true,
    }),
    Condvar::new(),
);
static QUEUED: AtomicU64 = AtomicU64::new(0);
static COMPLETED: AtomicU64 = AtomicU64::new(0);
static PUBLISHED: AtomicU64 = AtomicU64::new(0);
static REJECTED: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

fn rejected() {
    let n = REJECTED.fetch_add(1, Relaxed) + 1;
    if n.is_power_of_two() {
        log_stats();
    }
}

fn log_stats() {
    tracing::info!(
        queued = QUEUED.load(Relaxed),
        completed = COMPLETED.load(Relaxed),
        published = PUBLISHED.load(Relaxed),
        rejected = REJECTED.load(Relaxed),
        bytes = BYTES.load(Relaxed),
        "asynchronous first texture uploads"
    );
}

/// Count only the current descriptor revision actually finalized by the draw pass.
pub(crate) fn note_published() {
    let n = PUBLISHED.fetch_add(1, Relaxed) + 1;
    if n.is_power_of_two() || n.is_multiple_of(256) {
        log_stats();
    }
}

fn enqueue(job: Job) -> bool {
    let bytes = job.copy.staging.bytes;
    let mut queue = QUEUE.0.lock().unwrap_or_else(|e| e.into_inner());
    if !queue.accepting || bytes > BATCH_BYTES || queue.bytes + bytes > MAX_BYTES {
        return false;
    }
    queue.bytes += bytes;
    queue.jobs.push_back(job);
    QUEUED.fetch_add(1, Relaxed);
    QUEUE.1.notify_one();
    true
}

fn start_worker() -> bool {
    static STARTED: OnceLock<bool> = OnceLock::new();
    *STARTED.get_or_init(|| {
        let Some(host) = crate::host::host() else {
            return false;
        };
        match Worker::new(&host.device) {
            Ok(worker) => std::thread::Builder::new()
                .name("texture copy queue".into())
                .spawn(move || worker.run())
                .is_ok(),
            Err(error) => {
                tracing::warn!(%error, "asynchronous texture copy queue is unavailable");
                false
            }
        }
    }) && QUEUE.0.lock().unwrap_or_else(|e| e.into_inner()).accepting
}

struct Worker {
    queue: ID3D12CommandQueue,
    allocator: ID3D12CommandAllocator,
    list: ID3D12GraphicsCommandList,
    fence: ID3D12Fence,
}

// SAFETY: the queue is free-threaded and the other objects belong exclusively
// to the worker after it is moved to that thread.
unsafe impl Send for Worker {}

enum Copied {
    Complete,
    Rejected(windows::core::Error),
    Uncertain(windows::core::Error),
}

impl Worker {
    fn new(device: &ID3D12Device) -> windows::core::Result<Self> {
        // SAFETY: typed creation, with each output owned by this worker.
        unsafe {
            let queue = device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                Type: D3D12_COMMAND_LIST_TYPE_COPY,
                ..Default::default()
            })?;
            let allocator = device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_COPY)?;
            let list: ID3D12GraphicsCommandList =
                device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_COPY, &allocator, None)?;
            list.Close()?;
            Ok(Self {
                queue,
                allocator,
                list,
                fence: device.CreateFence(0, D3D12_FENCE_FLAG_NONE)?,
            })
        }
    }

    fn run(self) {
        for value in 1.. {
            let (jobs, bytes) = next_batch();
            match self.copy(&jobs, value) {
                Copied::Complete => finish(jobs, true),
                Copied::Rejected(error) => {
                    tracing::warn!(%error, "asynchronous texture copies could not be recorded");
                    finish(jobs, false);
                }
                Copied::Uncertain(error) => {
                    tracing::error!(%error, "texture copy queue lost its completion fence; submitted allocations remain hidden");
                    let queued = {
                        let mut queue = QUEUE.0.lock().unwrap_or_else(|e| e.into_inner());
                        queue.accepting = false;
                        std::mem::take(&mut queue.jobs)
                    };
                    finish(queued.into(), false);
                    // Execute succeeded but no fence can establish when the
                    // GPU stops using these resources. Keep the leases and
                    // command objects until process exit; never reuse them.
                    std::mem::forget(jobs);
                    std::mem::forget(self);
                    return;
                }
            }
            QUEUE.0.lock().unwrap_or_else(|e| e.into_inner()).bytes -= bytes;
        }
    }

    fn copy(&self, jobs: &[Job], value: u64) -> Copied {
        // SAFETY: exclusive allocator and list; every previous command has
        // finished. Destination textures start in COMMON, promote for the copy,
        // then decay to COMMON before the completion fence on this queue.
        unsafe {
            let commands = (|| -> windows::core::Result<ID3D12CommandList> {
                self.allocator.Reset()?;
                self.list.Reset(&self.allocator, None)?;
                for job in jobs {
                    job.copy.record(&self.list);
                }
                self.list.Close()?;
                self.list.cast()
            })();
            let commands = match commands {
                Ok(commands) => commands,
                Err(error) => return Copied::Rejected(error),
            };
            self.queue.ExecuteCommandLists(&[Some(commands)]);
            if let Err(error) = self.queue.Signal(&self.fence, value) {
                return Copied::Uncertain(error);
            }
            let wait = self.fence.SetEventOnCompletion(value, HANDLE::default());
            let seen = self.fence.GetCompletedValue();
            if seen != u64::MAX && seen >= value {
                Copied::Complete
            } else {
                Copied::Uncertain(wait.err().unwrap_or_else(windows::core::Error::empty))
            }
        }
    }
}

fn next_batch() -> (Vec<Job>, u64) {
    let mut queue = QUEUE.0.lock().unwrap_or_else(|e| e.into_inner());
    while queue.jobs.is_empty() {
        queue = QUEUE.1.wait(queue).unwrap_or_else(|e| e.into_inner());
    }
    let mut jobs = Vec::new();
    let mut bytes = 0;
    while jobs.len() < BATCH_COUNT
        && let Some(job) = queue.jobs.front()
        && bytes + job.copy.staging.bytes <= BATCH_BYTES
    {
        bytes += job.copy.staging.bytes;
        jobs.push(queue.jobs.pop_front().expect("first queued upload"));
    }
    (jobs, bytes)
}

fn finish(jobs: Vec<Job>, copied: bool) {
    for job in jobs {
        if copied && job.finish() {
            BYTES.fetch_add(job.copy.staging.bytes, Relaxed);
            let n = COMPLETED.fetch_add(1, Relaxed) + 1;
            if n.is_power_of_two() || n.is_multiple_of(256) {
                log_stats();
            }
        } else {
            failed(&job.copy.staging, &job.copy.texture);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::*;

    #[test]
    fn first_upload_waits_for_its_copy_fence_and_preserves_texture_bytes() {
        let Some(host) = crate::host::host() else {
            return;
        };
        let words = [
            3u32,
            0,
            0,
            0,
            256,
            0,
            256,
            9 << 16 | 1,
            98,
            1,
            0,
            0x105,
            0,
            0,
        ];
        let bytes = crate::resource::placed_bytes(&words).unwrap() as usize;
        // SAFETY: this test owns the private, committed texture source.
        let source = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                bytes,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_READWRITE,
            )
            .cast::<u8>()
        };
        assert!(!source.is_null());
        unsafe { (0..bytes).for_each(|i| source.add(i).write((i * 13 + i / 509) as u8)) };
        let title = crate::resource::texture_at(source as u64, &words, 0).unwrap();
        let copy = crate::resource::host_copy(title).unwrap();
        assert!(eligible(title, &copy, 0));
        assert!(!eligible(title, &copy, 1));
        assert_eq!(
            copy.ahead.compare_exchange(FRESH, COPYING, AcqRel, Acquire),
            Ok(FRESH)
        );
        let staging = super::super::staging(title, &copy.texture, 0).unwrap();
        let job = prepare(&staging, &copy).unwrap();

        let worker = Worker::new(&host.device).unwrap();
        // SAFETY: private queue and fence, held until the worker joins.
        let gate: ID3D12Fence =
            unsafe { host.device.CreateFence(0, D3D12_FENCE_FLAG_NONE).unwrap() };
        unsafe { worker.queue.Wait(&gate, 1).unwrap() };
        let (started, running) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let _ = started.send(());
            match worker.copy(std::slice::from_ref(&job), 1) {
                Copied::Complete => assert!(job.finish()),
                Copied::Rejected(error) | Copied::Uncertain(error) => {
                    panic!("copy queue failed: {error}")
                }
            }
        });
        let began = running.recv_timeout(std::time::Duration::from_secs(5));
        let before = (status(&copy), staging.uploaded.load(Relaxed));
        assert!(!while_ready(&copy, || panic!("copy has not completed")));
        unsafe { gate.Signal(1).unwrap() };
        thread.join().unwrap();
        assert_eq!(began, Ok(()));
        assert_eq!(before, (Status::Pending, false));
        assert_eq!(status(&copy), Status::Ready);
        assert!(staging.uploaded.load(Acquire));
        // An unrelated mapping may momentarily hold the mapping lock; the
        // descriptor scheduler will retry rather than block a draw on it.
        assert!((0..100).any(|_| while_ready(&copy, || {})));

        // A direct queue submission sees the contents only after the copy
        // fence returned. Compare actual GPU data with the guest layout.
        let footprint = staging.footprints[0];
        let pitch = footprint.Footprint.RowPitch as usize;
        let linear_bytes = pitch * staging.rows[0] as usize;
        let back = host.read_back_buffer(linear_bytes as u64).unwrap();
        let list = host.new_list().unwrap();
        crate::list::ensure_state(&list, title, D3D12_RESOURCE_STATE_COPY_SOURCE.0 as u32);
        let to = D3D12_TEXTURE_COPY_LOCATION {
            pResource: unsafe { std::mem::transmute_copy(&back) },
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: footprint,
            },
        };
        let from = D3D12_TEXTURE_COPY_LOCATION {
            pResource: unsafe { std::mem::transmute_copy(&copy.texture) },
            Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                SubresourceIndex: 0,
            },
        };
        // SAFETY: both resources are alive and have the required copy states.
        unsafe { list.list.CopyTextureRegion(&to, 0, 0, 0, &from, None) };
        list.close();
        host.execute(&[&list]);
        let mut expected = vec![0; linear_bytes];
        let tiled = unsafe { std::slice::from_raw_parts(source, bytes) };
        staging
            .layout
            .detile(tiled, 0, 0, &mut expected, pitch, linear_bytes)
            .unwrap();
        // SAFETY: the direct submission finished and the map covers this buffer.
        unsafe {
            let mut data = std::ptr::null_mut();
            back.Map(0, None, Some(&mut data)).unwrap();
            assert_eq!(
                std::slice::from_raw_parts(data.cast::<u8>(), linear_bytes),
                expected
            );
            back.Unmap(0, Some(&D3D12_RANGE::default()));
        }
        let ready_mapping = copy
            .ahead_ready
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .mapping
            .clone();
        let staged_fingerprint = copy
            .ahead_ready
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .fingerprint;
        assert_eq!(
            staged_fingerprint,
            super::super::fingerprint(source as u64, bytes as u64)
        );
        // SAFETY: this test owns the backing. This changes a sampled byte
        // directly, with no remap or descriptor rewrite.
        unsafe { source.write(0xdd) };
        assert!(ready_mapping.live());
        assert!(!(0..100).any(|_| while_ready(&copy, || panic!("stale upload published"))));
        assert_eq!(retire(&copy), None);
        assert!(!staging.uploaded.load(Acquire));
        assert!(!staging.last.lock().unwrap().has_data);
        assert_eq!(status(&copy), Status::Unavailable);
        assert_eq!(
            copy.ahead.compare_exchange(FRESH, ARMED, AcqRel, Acquire),
            Err(NORMAL)
        );
        super::super::unstage(title);
        assert_ne!(unsafe { VirtualFree(source.cast(), 0, MEM_RELEASE) }, 0);
    }
}
