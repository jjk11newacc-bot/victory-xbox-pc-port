//! Suballocate short-lived texture staging uploads from persistently mapped buffers.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
};
use windows::Win32::Graphics::Direct3D12::ID3D12Resource;

use crate::host::Host;

const CHUNK_BYTES: u64 = 2 << 20;
const MIN_SHIFT: u32 = 12;
const MAX_SHIFT: u32 = 19;
const CLASSES: usize = (MAX_SHIFT - MIN_SHIFT + 1) as usize;

struct Chunk {
    buffer: ID3D12Resource,
    address: usize,
}

struct Slots {
    chunk: Arc<Chunk>,
    free: Vec<u32>,
}

static POOL: Mutex<[Vec<Slots>; CLASSES]> = Mutex::new([const { Vec::new() }; CLASSES]);
static GROW: [Mutex<()>; CLASSES] = [const { Mutex::new(()) }; CLASSES];
static REUSE: AtomicU64 = AtomicU64::new(0);
static PRESSURE: AtomicBool = AtomicBool::new(false);
static CREATED: AtomicU64 = AtomicU64::new(0);

/// In the low-memory profile, try local GPU upload storage when Windows
/// is under RAM pressure. The host allocator checks the adapter budget
/// and falls back to a regular upload heap when local storage is unavailable.
fn staging_buffer(host: &Host, bytes: u64) -> Option<(ID3D12Resource, *mut u8)> {
    static LOW_MEMORY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let low_memory = *LOW_MEMORY.get_or_init(|| {
        std::env::var_os("VICTORY_LOW_MEMORY").is_some_and(|v| v == "1")
    });
    if low_memory && crate::budget::ram_pressure(crate::host::available_physical_ram()) != 0 {
        host.shader_input_buffer(bytes)
    } else {
        host.upload_buffer(bytes)
    }
}

/// The lifetime of a staging allocation, including every command list that
/// still holds its `Staging`. Its bytes cannot be recycled until that ends.
pub(super) struct Upload {
    chunk: Arc<Chunk>,
    slot: Option<(usize, u32)>,
    pub(super) offset: u64,
    /// Logical footprint offset represented by the beginning of this lease.
    pub(super) base: u64,
    pub(super) gpu: Option<super::gpu_detile::Plan>,
}

impl Upload {
    pub(super) fn new(host: &Host, bytes: u64) -> Option<Self> {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let enabled = *ENABLED.get_or_init(|| {
            std::env::var_os("VICTORY_TEXTURE_UPLOAD_POOL").is_none_or(|v| v != "0")
        });
        if enabled && (1..=1 << MAX_SHIFT).contains(&bytes) {
            let size = bytes.max(1 << MIN_SHIFT).next_power_of_two();
            let class = (size.ilog2() - MIN_SHIFT) as usize;
            if let Some(upload) = Self::pooled(host, class, size) {
                // SAFETY: the pool reserved this slot exclusively and it has
                // no in-flight readers. A partial or failed refresh can leave
                // some bytes untouched, so start with the same zeros as a new
                // committed upload buffer.
                unsafe { upload.data().write_bytes(0, bytes as usize) };
                return Some(upload);
            }
        }
        let (buffer, data) = staging_buffer(host, bytes)?;
        Some(Self {
            chunk: Arc::new(Chunk {
                buffer,
                address: data as usize,
            }),
            slot: None,
            offset: 0,
            base: 0,
            gpu: None,
        })
    }

    /// Allocate only the suffix holding the mapped coarse mips. Base is
    /// aligned like the original D3D12 placed footprint.
    pub(super) fn suffix(host: &Host, bytes: u64, base: u64) -> Option<Self> {
        if base % 512 != 0 || base >= bytes { return None; }
        let mut upload = Self::new(host, bytes - base)?;
        upload.base = base;
        Some(upload)
    }

    fn pooled(host: &Host, class: usize, size: u64) -> Option<Self> {
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        let slots = &mut pool[class];
        if let Some(page) = slots.iter_mut().find(|page| !page.free.is_empty()) {
            let index = page.free.pop()?;
            REUSE.fetch_add(1, Relaxed);
            return Some(Self::from_slot(page, class, index, size));
        }

        // Driver allocation can block. No slot is reserved yet, so release
        // the global pool lock while creating a new chunk.
        drop(pool);
        // Serialize growth only within this size class. A concurrent worker
        // may have supplied free slots while we waited; recheck before allocating.
        let _grow = GROW[class].lock().unwrap_or_else(|e| e.into_inner());
        {
            let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(page) = pool[class].iter_mut().find(|page| !page.free.is_empty()) {
                let index = page.free.pop()?;
                REUSE.fetch_add(1, Relaxed);
                return Some(Self::from_slot(page, class, index, size));
            }
        }
        if std::env::var_os("VICTORY_LOW_MEMORY").is_some_and(|v| v == "1") {
            trim_idle_under_pressure(crate::host::available_physical_ram());
        }
        let (buffer, data) = staging_buffer(host, CHUNK_BYTES)?;
        crate::host::name(&buffer, || "texture staging pool".into());
        let page = Slots {
            chunk: Arc::new(Chunk {
                buffer,
                address: data as usize,
            }),
            free: (1..(CHUNK_BYTES / size) as u32).rev().collect(),
        };
        let upload = Self::from_slot(&page, class, 0, size);
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        pool[class].push(page);
        CREATED.fetch_add(1, Relaxed);
        Some(upload)
    }

    fn from_slot(page: &Slots, class: usize, index: u32, size: u64) -> Self {
        Self {
            chunk: page.chunk.clone(),
            slot: Some((class, index)),
            offset: u64::from(index) * size,
            base: 0,
            gpu: None,
        }
    }

    pub(super) fn data(&self) -> *mut u8 {
        (self.chunk.address + self.offset as usize) as *mut u8
    }

    pub(super) fn buffer(&self) -> &ID3D12Resource {
        &self.chunk.buffer
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        let Some((class, index)) = self.slot else {
            return;
        };
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        let slots = &mut pool[class];
        let page_index = slots
            .iter()
            .position(|page| Arc::ptr_eq(&page.chunk, &self.chunk))
            .expect("a leased texture staging page must be in its size class");
        let capacity = (CHUNK_BYTES >> (MIN_SHIFT as usize + class)) as usize;
        slots[page_index].free.push(index);
        if slots[page_index].free.len() == capacity
            && (PRESSURE.load(Relaxed) || slots
                .iter()
                .enumerate()
                .any(|(i, page)| i != page_index && page.free.len() == capacity))
        {
            // Keep at most one idle chunk per size class. This Upload still
            // holds the chunk, so final COM release occurs outside the lock.
            slots.swap_remove(page_index);
        }
    }
}

#[cfg(feature = "diagnostics")]
pub(super) fn totals() -> (u64, u64) {
    (CREATED.load(Relaxed), REUSE.load(Relaxed))
}

/// Release only fully free chunks under physical RAM pressure. Drop COM
/// resources after releasing the pool lock; leased slots are untouched.
pub(super) fn trim_idle_under_pressure(available: Option<u64>) {
    let Some(available) = available else { return };
    // Idle staging chunks are cheap to recreate, so release them only at
    // critical RAM pressure; the earlier tier trims larger caches first.
    let pressured = crate::budget::ram_pressure(Some(available)) == 2;
    if crate::budget::sticky_pressure() {
        // Hysteresis avoids repeatedly freeing and recreating a chunk near
        // the low-memory threshold. Only idle chunks are ever retired.
        if pressured { PRESSURE.store(true, Relaxed); }
        else if crate::budget::ram_pressure(Some(available)) == 0 { PRESSURE.store(false, Relaxed); }
    }
    if !pressured { return; }
    let mut retired = Vec::new();
    {
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        for (class, slots) in pool.iter_mut().enumerate() {
            let capacity = (CHUNK_BYTES >> (MIN_SHIFT as usize + class)) as usize;
            let mut i = 0;
            while i < slots.len() {
                if slots[i].free.len() == capacity {
                    retired.push(slots.swap_remove(i));
                } else {
                    i += 1;
                }
            }
        }
    }
    drop(retired);
}
