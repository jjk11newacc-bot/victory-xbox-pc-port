//! Experimental direct-queue detile of addressable texture subresources. The CPU supplies raw tiled bytes and two axis offset tables
//! derived from the existing address equation; the GPU writes pitched rows.
use std::sync::{OnceLock, atomic::{AtomicU64, Ordering::{Acquire, Release, Relaxed}}};
use victory_formats::texture::{Dimension, Layout, Swizzle};
use windows::Win32::Graphics::Direct3D12::*;
use crate::host::{Host, transition};
use super::{Staging, upload::Upload};

const LIMIT: u64 = 96 << 20;
static RESERVED: AtomicU64 = AtomicU64::new(0);
static RECORDED: AtomicU64 = AtomicU64::new(0);
static REUSED: AtomicU64 = AtomicU64::new(0);
static DIRTY_JOBS_SAVED: AtomicU64 = AtomicU64::new(0);
static DIRTY_BYTES_SAVED: AtomicU64 = AtomicU64::new(0);
static PIPELINE: OnceLock<Option<(ID3D12RootSignature, ID3D12PipelineState)>> = OnceLock::new();

struct Reservation(u64);
impl Drop for Reservation {
    fn drop(&mut self) { RESERVED.fetch_sub(self.0, Relaxed); }
}

pub(super) struct Plan {
    input: ID3D12Resource,
    pub(super) output: ID3D12Resource,
    jobs: Vec<[u32; 9]>,
    // Conservative raw-byte envelopes; may overlap across mips/slices.
    job_sources: Vec<(usize, usize)>,
    pub(super) raw_bytes: usize,
    pub(super) first: u32,
    pub(super) output_base: u64,
    pub(super) input_bytes: u64,
    // CPU fingerprints are tiny compared with keeping another raw texture.
    page_hashes: Vec<blake3::Hash>,
    // None means copy the full input including the address tables. Some
    // means the input has a completed previous revision; copy raw spans only.
    dirty: Option<Vec<(usize, usize)>>,
    submitted: AtomicU64,
    _reservation: Reservation,
}

const RAW_PAGE: usize = 64 << 10;
fn dirty_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_GPU_DETILE_DIRTY_PAGES").is_some_and(|v| v == "1"))
}
#[cfg(test)]
fn raw_hashes(raw: &[u8]) -> Vec<blake3::Hash> {
    raw.chunks(RAW_PAGE).map(blake3::hash).collect()
}
// Hash the same captured bytes that are written into staging, not a
// second read of guest memory. Only one bounded page of scratch is needed.
// SAFETY: dst must be writable for raw.len() bytes and exclusively leased.
unsafe fn capture_raw_pages(raw: &[u8], dst: *mut u8, old: Option<&[blake3::Hash]>) -> Vec<blake3::Hash> {
    let mut scratch = vec![0u8; RAW_PAGE.min(raw.len())];
    let mut hashes = Vec::with_capacity(raw.len().div_ceil(RAW_PAGE));
    for (i, page) in raw.chunks(RAW_PAGE).enumerate() {
        let captured = &mut scratch[..page.len()];
        captured.copy_from_slice(page);
        let hash = blake3::hash(captured);
        if old.and_then(|old| old.get(i)) != Some(&hash) {
            unsafe { std::ptr::copy_nonoverlapping(captured.as_ptr(), dst.add(i * RAW_PAGE), page.len()) };
        }
        hashes.push(hash);
    }
    hashes
}
fn dirty_spans(old: &[blake3::Hash], new: &[blake3::Hash], bytes: usize) -> Option<Vec<(usize, usize)>> {
    if old.len() != new.len() || new.len() != bytes.div_ceil(RAW_PAGE) { return None; }
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (i, (a, b)) in old.iter().zip(new).enumerate() {
        if a == b { continue; }
        let from = i * RAW_PAGE;
        let to = ((i + 1) * RAW_PAGE).min(bytes);
        if let Some(last) = spans.last_mut().filter(|last| last.1 == from) { last.1 = to; }
        else { spans.push((from, to)); }
    }
    Some(spans)
}

fn dirty_jobs_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_GPU_DETILE_DIRTY_JOBS").is_some_and(|v| v == "1"))
}

fn job_needed(source: (usize, usize), dirty: &[(usize, usize)]) -> bool {
    dirty.iter().any(|&(a, b)| a < source.1 && b > source.0)
}

struct Axes { columns: Vec<u32>, rows: Vec<u32>, origin: u32, xor: bool }

impl Axes {
    #[cfg(test)]
    fn new(layout: &Layout, bytes: usize) -> Option<Self> {
        Self::for_part(layout, bytes, 0, 0)
    }

    // The same address equation used by the shader. For XOR use the whole
    // possible low 16-bit range; false positives only cause extra work.
    fn source_bounds(&self, element: u32, bytes: usize) -> Option<(usize, usize)> {
        let (a0, a1) = (*self.columns.iter().min()? as u64, *self.columns.iter().max()? as u64);
        let (b0, b1) = (*self.rows.iter().min()? as u64, *self.rows.iter().max()? as u64);
        let (lo, hi) = if self.xor {
            ((a0 & !65535) + (b0 & !65535), (a1 & !65535) + (b1 & !65535) + 65536 + u64::from(element.saturating_sub(4)))
        } else {
            (a0.checked_add(b0)?.checked_sub(self.origin as u64)?,
             a1.checked_add(b1)?.checked_sub(self.origin as u64)?.checked_add(element as u64)?)
        };
        (lo < hi && hi <= bytes as u64).then_some((lo as usize, hi as usize))
    }

    fn for_part(layout: &Layout, bytes: usize, mip: u32, slice: u32) -> Option<Self> {
        let d = &layout.desc;
        let m = layout.mips.get(mip as usize)?;
        let depth = if d.dimension == Dimension::Tex3d { m.depth } else { layout.slices };
        if slice >= depth
            || d.samples != 1 || !matches!(d.block, 1 | 4)
            || !layout.addressable()
            || !matches!(d.element_bytes, 1 | 2 | 4 | 8 | 16) { return None; }
        if m.width == 0 || m.height == 0 || m.width > 16384 || m.height > 16384 { return None; }
        let offset = |x, y| u32::try_from(layout.offset(mip, x, y, slice).ok()?).ok();
        let origin = offset(0, 0)?;
        let columns: Vec<u32> = (0..m.width).map(|x| offset(x, 0)).collect::<Option<_>>()?;
        let rows: Vec<u32> = (0..m.height).map(|y| offset(0, y)).collect::<Option<_>>()?;
        let xor = d.swizzle == Swizzle::SW_64KB_R_X;
        if (xor && origin != 0) || (d.swizzle.xor() && !xor) { return None; }
        let a = u64::from(*columns.iter().max()?);
        let b = u64::from(*rows.iter().max()?);
        let end = if xor { (a & !65535) + (b & !65535) + 65536 }
            else { a.checked_add(b)?.checked_sub(u64::from(origin))?.checked_add(u64::from(d.element_bytes))? };
        if end > bytes as u64 || columns.iter().chain(&rows).any(|v| v % d.element_bytes.min(4) != 0) { return None; }
        Some(Self { columns, rows, origin, xor })
    }
}

fn pipeline(host: &Host) -> Option<&'static (ID3D12RootSignature, ID3D12PipelineState)> {
    PIPELINE.get_or_init(|| {
        let code = crate::host::compile(include_str!("gpu_detile.hlsl"), windows::core::s!("main"), windows::core::s!("cs_5_1")).ok()?;
        // SAFETY: the root signature is embedded in the compiled shader.
        unsafe {
            let root: ID3D12RootSignature = host.device.CreateRootSignature(0, &code).ok()?;
            let pso = host.device.CreateComputePipelineState(&D3D12_COMPUTE_PIPELINE_STATE_DESC {
                pRootSignature: std::mem::transmute_copy(&root),
                CS: D3D12_SHADER_BYTECODE { pShaderBytecode: code.as_ptr().cast(), BytecodeLength: code.len() },
                ..Default::default()
            }).ok()?;
            Some((root, pso))
        }
    }).as_ref()
}

pub(super) fn prepare(staging: &Staging, tiled: &[u8]) -> Option<Upload> {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var_os("VICTORY_GPU_DETILE").is_some_and(|v| v == "1")) { return None; }
    static EXTENDED: OnceLock<bool> = OnceLock::new();
    let extended = *EXTENDED.get_or_init(|| std::env::var_os("VICTORY_GPU_DETILE_EXTENDED").is_some_and(|v| v == "1"));
    prepare_inner(staging, tiled, true, extended, staging.first)
}

/// A streaming revision can start at a coarser mip than the original view.
/// Only fully mapped single-slice mip tails are accepted, exactly as on CPU.
pub(super) fn prepare_partial(staging: &Staging, tiled: &[u8], first: u32) -> Option<Upload> {
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| ["VICTORY_GPU_DETILE", "VICTORY_GPU_DETILE_EXTENDED", "VICTORY_GPU_DETILE_PARTIAL"]
        .iter().all(|n| std::env::var_os(n).is_some_and(|v| v == "1"))) { return None; }
    prepare_inner(staging, tiled, true, true, first)
}

fn prepare_inner(staging: &Staging, tiled: &[u8], check_budget: bool, extended: bool, first: u32) -> Option<Upload> {
    static LARGE: OnceLock<bool> = OnceLock::new();
    let large = extended && *LARGE.get_or_init(|| std::env::var_os("VICTORY_GPU_DETILE_LARGE").is_some_and(|v| v == "1"));
    let max_bytes = if large { 32 << 20 } else { 16 << 20 };
    if first < staging.first || first >= staging.mips
        || tiled.len() as u64 > max_bytes || tiled.len() % 4 != 0 { return None; }
    if first == staging.first {
        if tiled.len() != staging.size { return None; }
    } else if !extended || staging.layout.slices != 1 || staging.layout.thick
        || u64::try_from(tiled.len()).ok()? != staging.layout.bytes_from(first) { return None; }
    // Preserve D3D12's 512-byte placed-footprint alignment when rebasing.
    let output_base = if first == staging.first { 0 } else {
        staging.footprints.iter().enumerate().filter(|(i, _)| staging.part(*i).0 >= first)
            .map(|(_, f)| f.Offset).min()?
    };
    let output_bytes = staging.bytes.checked_sub(output_base)?;
    if output_base % 512 != 0 || output_bytes < 1 << 20 || output_bytes > max_bytes { return None; }
    let layout = &staging.layout;
    let d = &layout.desc;
    let volume = d.dimension == Dimension::Tex3d;
    // Linear rows already have a cheap CPU path. Allow the GPU path only
    // for explicit parity/profiling, not as a default performance choice.
    static LINEAR_1D: OnceLock<bool> = OnceLock::new();
    if (d.swizzle == Swizzle::LINEAR || d.dimension == Dimension::Tex1d)
        && (!extended || (check_budget && !*LINEAR_1D.get_or_init(||
            std::env::var_os("VICTORY_GPU_DETILE_LINEAR_1D").is_some_and(|v| v == "1")))) { return None; }
    static VOLUME: OnceLock<bool> = OnceLock::new();
    if volume && (!extended || first != 0 || staging.first != 0 || (check_budget && !*VOLUME.get_or_init(||
        std::env::var_os("VICTORY_GPU_DETILE_VOLUME").is_some_and(|v| v == "1")))) { return None; }
    static SMALL_ELEMENTS: OnceLock<bool> = OnceLock::new();
    if d.element_bytes < 4 && (!extended || (check_budget && !*SMALL_ELEMENTS.get_or_init(||
        std::env::var_os("VICTORY_GPU_DETILE_SMALL_ELEMENTS").is_some_and(|v| v == "1")))) { return None; }
    if !extended && (staging.first != 0 || staging.mips != 1 || staging.footprints.len() != 1
        || d.block != 1 || d.depth != 1) { return None; }
    if staging.first >= staging.mips || staging.footprints.len() != staging.rows.len()
        || staging.mips != d.mips || (staging.first != 0 && layout.slices != 1)
        || staging.footprints.len() as u32 != (staging.mips - staging.first).checked_mul(if volume { 1 } else { layout.slices })?
        || staging.footprints.len() > 192 { return None; }
    // A job describes one disjoint placed footprint. BCn uses block units,
    // not texel units. The existing Layout oracle supplies the guest addresses.
    let mut tables = Vec::<u32>::new();
    let mut jobs = Vec::with_capacity(staging.footprints.len());
    let mut job_sources = Vec::with_capacity(staging.footprints.len());
    let raw = u32::try_from(tiled.len()).ok()?;
    let mut end = 0u64;
    for (i, (footprint, &rows)) in staging.footprints.iter().zip(&staging.rows).enumerate() {
        let (mip, slice) = staging.part(i);
        if mip < first { continue; }
        let m = layout.mips.get(mip as usize)?;
        let f = footprint.Footprint;
        let row_bytes = m.width.checked_mul(d.element_bytes)?;
        let depth = if volume { m.depth } else { 1 };
        let depth_pitch = u64::from(f.RowPitch).checked_mul(u64::from(rows))?;
        let next = footprint.Offset.checked_add(depth_pitch.checked_mul(u64::from(depth))?)?;
        if f.Depth != depth || depth == 0 || f.RowPitch < row_bytes || f.RowPitch % 4 != 0
            || rows != m.height || f.Width.div_ceil(d.block) != m.width
            || f.Height.div_ceil(d.block) != m.height || footprint.Offset % 512 != 0
            || footprint.Offset < end || next > staging.bytes { return None; }
        end = next;
        // One independent job per depth plane; the existing shader writes
        // pitched rows and the footprint's Depth handles the final copy.
        // Bound metadata and dispatch count before allocating GPU memory.
        for z in 0..depth {
            if jobs.len() >= 2048 { return None; }
            let axes = Axes::for_part(layout, tiled.len(), mip, if volume { z } else { slice })?;
            let col_at = raw.checked_add(u32::try_from(tables.len()).ok()?.checked_mul(4)?)?;
            tables.extend_from_slice(&axes.columns);
            let row_at = raw.checked_add(u32::try_from(tables.len()).ok()?.checked_mul(4)?)?;
            tables.extend_from_slice(&axes.rows);
            // Prevent an oversized CPU metadata table for pathological arrays.
            if tables.len() > (2 << 20) / 4 { return None; }
            // An unprovable envelope disables pruning for this job, never the upload.
            job_sources.push(axes.source_bounds(d.element_bytes, tiled.len()).unwrap_or((0, tiled.len())));
            jobs.push([row_bytes, m.height, f.RowPitch, d.element_bytes, col_at, row_at,
                axes.origin, u32::from(axes.xor), u32::try_from(footprint.Offset.checked_sub(output_base)?.checked_add(depth_pitch.checked_mul(u64::from(z))?)?).ok()?]);
        }
    }
    if jobs.is_empty() { return None; }
    let input_bytes = u64::from(raw) + tables.len() as u64 * 4;
    let host = crate::host::host()?;
    let bytes = input_bytes.next_multiple_of(64 << 10) + output_bytes.next_multiple_of(64 << 10);
    if check_budget {
        let [budget, used, ..] = host.video_memory()?;
        if budget.saturating_sub(used) < bytes + (512 << 20) { return None; }
        if crate::budget::ram_pressure(crate::host::available_physical_ram()) == 2 { return None; }
    }
    RESERVED.fetch_update(Relaxed, Relaxed, |old| old.checked_add(bytes).filter(|&n| n <= LIMIT)).ok()?;
    let reservation = Reservation(bytes);
    pipeline(host)?;
    let input = host.uav_buffer(input_bytes)?;
    let output = host.uav_buffer(output_bytes)?;
    let mut upload = Upload::new(host, input_bytes)?;
    // SAFETY: the fresh lease holds raw bytes and both nonoverlapping u32 tables.
    let page_hashes = unsafe {
        let hashes = if dirty_enabled() { capture_raw_pages(tiled, upload.data(), None) }
            else { std::ptr::copy_nonoverlapping(tiled.as_ptr(), upload.data(), tiled.len()); Vec::new() };
        std::ptr::copy_nonoverlapping(tables.as_ptr().cast::<u8>(), upload.data().add(raw as usize), tables.len() * 4);
        hashes
    };
    upload.gpu = Some(Plan { input, output, input_bytes, _reservation: reservation,
        jobs, job_sources, raw_bytes: tiled.len(), first, output_base,
        page_hashes,
        dirty: None, submitted: AtomicU64::new(0) });
    static FIRST: std::sync::Once = std::sync::Once::new();
    FIRST.call_once(|| tracing::info!(input_bytes, output_bytes, "experimental GPU texture detile active"));
    Some(upload)
}

/// The caller owns the sole Arc reference, so no recorded or running copy
/// can still read these bytes. The geometry and tables belong to this staging.
pub(super) fn refresh(staging: &Staging, tiled: &[u8], upload: &mut Upload) -> bool {
    let data = upload.data();
    let Some(plan) = upload.gpu.as_mut() else { return false };
    if plan.first != staging.first || tiled.len() != staging.size || tiled.len() != plan.raw_bytes { return false; }
    let mut dirty = None;
    if dirty_enabled() {
        let submitted = plan.submitted.load(Acquire);
        // Arc uniqueness rules out recorded readers. Additionally require
        // the actual GPU fence to have completed before reusing GPU input.
        if submitted != 0 && crate::host::completed_texture_uploads().is_none_or(|done| done < submitted) {
            return false;
        }
        let eligible = submitted != 0 && plan.page_hashes.len() == tiled.len().div_ceil(RAW_PAGE);
        let hashes = unsafe { capture_raw_pages(tiled, data, eligible.then_some(plan.page_hashes.as_slice())) };
        if eligible { dirty = dirty_spans(&plan.page_hashes, &hashes, tiled.len()); }
        plan.page_hashes = hashes;
    } else {
        unsafe { std::ptr::copy_nonoverlapping(tiled.as_ptr(), data, tiled.len()) };
    }
    plan.dirty = dirty;
    plan.submitted.store(0, Release);
    REUSED.fetch_add(1, Relaxed);
    true
}

impl Plan {
    pub(super) fn tracks_input(&self) -> bool { !self.page_hashes.is_empty() }
    pub(super) fn submitted(&self, fence: u64) {
        if self.tracks_input() { self.submitted.fetch_max(fence, Release); }
    }
    /// Direct queue only. Each record restores both private buffers to COMMON.
    pub(super) fn record(&self, list: &ID3D12GraphicsCommandList, upload: &Upload) {
        let (root, pso) = PIPELINE.get().and_then(Option::as_ref).expect("GPU detile pipeline");
        let recorded = RECORDED.fetch_add(1, Relaxed) + 1;
        if recorded.is_power_of_two() {
            tracing::info!(recorded, reused = REUSED.load(Relaxed), reserved = RESERVED.load(Relaxed), dirty_upload_bytes_avoided = DIRTY_BYTES_SAVED.load(Relaxed), dirty_detile_jobs_avoided = DIRTY_JOBS_SAVED.load(Relaxed), "GPU texture detile dispatches");
        }
        let common = D3D12_RESOURCE_STATE_COMMON;
        // No changed raw bytes and a completed previous output: the caller
        // still copies the private output into the destination texture. Avoid
        // touching the input or binding/dispatching compute at all.
        if dirty_jobs_enabled() && self.dirty.as_ref().is_some_and(Vec::is_empty) {
            DIRTY_JOBS_SAVED.fetch_add(self.jobs.len() as u64, Relaxed);
            DIRTY_BYTES_SAVED.fetch_add(self.input_bytes, Relaxed);
            unsafe { list.ResourceBarrier(&[transition(&self.output, common, D3D12_RESOURCE_STATE_COPY_SOURCE)]) };
            return;
        }
        let dest = D3D12_RESOURCE_STATE_COPY_DEST;
        let srv = D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE;
        let uav = D3D12_RESOURCE_STATE_UNORDERED_ACCESS;
        let source = D3D12_RESOURCE_STATE_COPY_SOURCE;
        // SAFETY: prelude owns the direct list; the upload Arc retains all
        // resources through its fence, and transitions order the compute/copy.
        unsafe {
            list.ResourceBarrier(&[transition(&self.input, common, dest), transition(&self.output, common, uav)]);
            if let Some(spans) = &self.dirty {
                let mut copied = 0u64;
                for &(from, to) in spans {
                    let n = (to - from) as u64;
                    list.CopyBufferRegion(&self.input, from as u64, upload.buffer(), upload.offset + from as u64, n);
                    copied += n;
                }
                DIRTY_BYTES_SAVED.fetch_add(self.input_bytes.saturating_sub(copied), Relaxed);
            } else {
                list.CopyBufferRegion(&self.input, 0, upload.buffer(), upload.offset, self.input_bytes);
            }
            list.ResourceBarrier(&[transition(&self.input, dest, srv)]);
            list.SetComputeRootSignature(root);
            list.SetPipelineState(pso);
            list.SetComputeRootShaderResourceView(0, self.input.GetGPUVirtualAddress());
            list.SetComputeRootUnorderedAccessView(1, self.output.GetGPUVirtualAddress());
            // Each job writes a separate footprint; no UAV ordering dependency.
            for (i, args) in self.jobs.iter().enumerate() {
                // `dirty` is Some only after a previous revision completed.
                // Output is private to this plan and preserved between records.
                if dirty_jobs_enabled() && self.dirty.as_ref().is_some_and(|spans| !job_needed(self.job_sources[i], spans)) {
                    DIRTY_JOBS_SAVED.fetch_add(1, Relaxed);
                    continue;
                }
                list.SetComputeRoot32BitConstants(2, 9, args.as_ptr().cast(), 0);
                list.Dispatch(args[0].div_ceil(4).div_ceil(64), args[1], 1);
            }
            list.ResourceBarrier(&[transition(&self.input, srv, common), transition(&self.output, uav, source)]);
        }
    }
    pub(super) fn finish(&self, list: &ID3D12GraphicsCommandList) {
        unsafe { list.ResourceBarrier(&[transition(&self.output, D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_COMMON)]) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use victory_formats::texture::Desc;
    #[test]
    fn dirty_raw_pages_preserve_tail_and_join_adjacent_pages() {
        let old = vec![0u8; RAW_PAGE * 5 + 8];
        let mut new = old.clone();
        new[RAW_PAGE + 1] = 3;
        new[RAW_PAGE * 2 + 5] = 9;
        new[RAW_PAGE * 5 + 7] = 2;
        let spans = dirty_spans(&raw_hashes(&old), &raw_hashes(&new), old.len()).unwrap();
        assert_eq!(spans, [(RAW_PAGE, RAW_PAGE * 3), (RAW_PAGE * 5, RAW_PAGE * 5 + 8)]);
        let mut patched = old.clone();
        for (a,b) in spans { patched[a..b].copy_from_slice(&new[a..b]); }
        assert_eq!(patched, new);
        assert!(dirty_spans(&[], &raw_hashes(&new), new.len()).is_none());
    }

    #[test]
    fn dirty_jobs_keep_every_intersecting_source() {
        assert!(!job_needed((64, 128), &[(0, 64), (128, 256)]));
        assert!(job_needed((64, 128), &[(127, 256)]));
        assert!(!job_needed((64, 128), &[]));
        assert!(job_needed((64, 128), &[(0, 256)]));
    }

    // Exercises the real HLSL, transitions, resource lifetime and GPU output.
    // Can be run with VICTORY_D3D12_WARP=1 or a real GPU; no game assets.
    #[test]
    fn gpu_detile_readback_matches_cpu_rows() {
        let Some(host) = crate::host::host() else {
            assert!(std::env::var_os("VICTORY_D3D12_WARP").is_none()
                && std::env::var_os("VICTORY_REQUIRE_GPU_TEST").is_none(), "requested D3D12 device unavailable");
            return;
        };
        if std::env::var_os("VICTORY_D3D12_WARP").is_some() {
            assert!(crate::host::on_warp() && host.is_software(), "test must execute on WARP; enable diagnostics");
        }
        if std::env::var_os("VICTORY_REQUIRE_GPU_TEST").is_some() {
            assert!(!host.is_software(), "hardware test unexpectedly uses software");
            assert!(std::env::var_os("VICTORY_NVIDIA").is_some_and(|v| v == "1"), "hardware test must select NVIDIA explicitly");
        }
        for (swizzle, element_bytes, block, width, height, mips, depth, first, revision_first, volume) in [
            (Swizzle::SW_4KB_D, 1, 1, 1537, 1025, 1, 1, 0, 0, false),
            (Swizzle::SW_64KB_S, 2, 1, 1025, 769, 9, 1, 0, 0, false),
            (Swizzle::SW_64KB_R_X, 1, 1, 1537, 1025, 1, 1, 0, 0, false),
            (Swizzle::SW_64KB_R_X, 16, 1, 513, 517, 1, 1, 0, 0, false),
            (Swizzle::SW_64KB_R_X, 2, 1, 1025, 769, 1, 1, 0, 0, false),
            (Swizzle::SW_64KB_R_X, 4, 1, 513, 517, 1, 1, 0, 0, false),
            (Swizzle::SW_64KB_R_X, 8, 1, 513, 517, 1, 1, 0, 0, false),
            (Swizzle::SW_4KB_D, 16, 1, 513, 517, 1, 1, 0, 0, false),
            (Swizzle::SW_64KB_S, 8, 4, 2049, 1537, 10, 1, 0, 0, false),
            (Swizzle::SW_4KB_D, 16, 4, 1025, 1025, 10, 2, 0, 0, false),
            (Swizzle::SW_64KB_S, 4, 1, 2049, 1025, 11, 1, 1, 1, false),
            (Swizzle::SW_64KB_S, 4, 1, 4097, 2049, 12, 1, 0, 2, false),
            (Swizzle::LINEAR, 4, 1, 2049, 1025, 4, 1, 0, 0, false),
            (Swizzle::SW_4KB_S, 4, 1, 128, 96, 6, 32, 0, 0, true),
            (Swizzle::SW_64KB_S, 2, 1, 129, 97, 5, 32, 0, 0, true),
        ] {
            if swizzle == Swizzle::SW_64KB_R_X && matches!(element_bytes, 1 | 16)
                && !std::env::var_os("VICTORY_RENDER_XOR_EXTRA").is_some_and(|v| v == "1") { continue; }
            let desc = Desc { dimension: if volume { Dimension::Tex3d } else { Dimension::Tex2d }, width, height, depth,
                mips, element_bytes, block, samples: 1, swizzle };
            let layout = Layout::new(&desc).unwrap();
            let size = layout.bytes_from(first) as usize;
            let tiled: Vec<u8> = (0..layout.bytes_from(revision_first) as usize).map(|i| ((i.wrapping_mul(31) ^ (i >> 8) ^ (i >> 16)) & 255) as u8).collect();
            let mut footprints = Vec::new();
            let mut rows = Vec::new();
            let mut bytes = 0u64;
            for _slice in 0..if volume { 1 } else { depth } {
                for mip in first..mips {
                    let m = &layout.mips[mip as usize];
                    let pitch = (m.width * element_bytes).next_multiple_of(256);
                    let offset = bytes.next_multiple_of(512);
                    footprints.push(D3D12_PLACED_SUBRESOURCE_FOOTPRINT { Offset: offset,
                        Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                            Width: (width >> mip).max(1), Height: (height >> mip).max(1),
                            Depth: m.depth, RowPitch: pitch, ..Default::default() } });
                    rows.push(m.height);
                    bytes = offset + u64::from(pitch) * u64::from(m.height) * u64::from(m.depth);
                }
            }
            super::super::STAGED[0].fetch_add(1, Relaxed);
            super::super::STAGED[1].fetch_add(bytes, Relaxed);
            let staging = Staging {
                inline: OnceLock::new(),
                #[cfg(feature = "diagnostics")]
                total: bytes,
                footprints, rows, layout, mips, first, size, bytes,
                memory: tiled.as_ptr(), last: Default::default(), dirty: true.into(),
                uploaded: false.into(), used: 0.into(),
            };
            let extended = element_bytes < 4 || block != 1 || mips != 1 || depth != 1;
            if extended { assert!(prepare_inner(&staging, &tiled, false, false, revision_first).is_none()); }
            let upload = prepare_inner(&staging, &tiled, false, extended, revision_first).expect("eligible test GPU detile");
            let plan = upload.gpu.as_ref().unwrap();
            let list = host.new_list().unwrap();
            let out_bytes = bytes - plan.output_base;
            let readback = host.read_back_buffer(out_bytes).unwrap();
            plan.record(&list.list, &upload);
            unsafe { list.list.CopyBufferRegion(&readback, 0, &plan.output, 0, out_bytes) };
            plan.finish(&list.list);
            list.close();
            host.execute(&[&list]);
            let mut expected = vec![0; out_bytes as usize];
            for (i, fp) in staging.footprints.iter().enumerate() {
                let (mip, slice) = staging.part(i);
                if mip < revision_first { continue; }
                let pitch = fp.Footprint.RowPitch as usize;
                let len = pitch * staging.rows[i] as usize;
                let offset = (fp.Offset - plan.output_base) as usize;
                staging.layout.detile(&tiled, mip, slice, &mut expected[offset..offset + len * fp.Footprint.Depth as usize], pitch, len).unwrap();
            }
            unsafe {
                let mut data = std::ptr::null_mut();
                readback.Map(0, None, Some(&mut data)).unwrap();
                let actual = std::slice::from_raw_parts(data.cast::<u8>(), out_bytes as usize);
                for (i, fp) in staging.footprints.iter().enumerate() {
                    let (mip, slice) = staging.part(i);
                    if mip < revision_first { continue; }
                    let m = &staging.layout.mips[mip as usize];
                    let row = (m.width * element_bytes) as usize;
                    for z in 0..m.depth as usize { for y in 0..m.height as usize {
                        let start = (fp.Offset - plan.output_base) as usize + (z * m.height as usize + y) * fp.Footprint.RowPitch as usize;
                        assert_eq!(&actual[start..start + row], &expected[start..start + row], "{swizzle:?}, mip {mip}, slice {slice}, depth {z}, row {y}");
                    }}
                }
                readback.Unmap(0, Some(&D3D12_RANGE::default()));
            }
        }
        eprintln!("STAGE42_GPU_DETILE_PARITY_OK");
    }

    #[test]
    fn linear_and_one_dimensional_axes_match_cpu_addresses() {
        for (dimension, height, swizzle) in [
            (Dimension::Tex1d, 1, Swizzle::LINEAR),
            (Dimension::Tex1d, 1, Swizzle::SW_4KB_D),
            (Dimension::Tex2d, 37, Swizzle::LINEAR),
        ] {
            for element_bytes in [1, 2, 4, 8, 16] {
                let desc = Desc { dimension, width: 513, height, depth: 2,
                    mips: 5, element_bytes, block: 1, samples: 1, swizzle };
                let layout = Layout::new(&desc).unwrap();
                for (mip, m) in layout.mips.iter().enumerate() {
                    for slice in 0..layout.slices {
                        let axes = Axes::for_part(&layout, layout.size as usize, mip as u32, slice).unwrap();
                        for y in 0..m.height { for x in 0..m.width {
                            let at = u64::from(axes.columns[x as usize]) + u64::from(axes.rows[y as usize]) - u64::from(axes.origin);
                            assert_eq!(at, layout.offset(mip as u32, x, y, slice).unwrap());
                        }}
                    }
                }
            }
        }
    }

    #[test]
    fn volume_axes_match_every_cpu_address() {
        for swizzle in [Swizzle::SW_4KB_S, Swizzle::SW_64KB_S] {
            for element_bytes in [1, 2, 4, 8, 16] {
                let desc = Desc { dimension: Dimension::Tex3d, width: 33, height: 19, depth: 9,
                    mips: 5, element_bytes, block: 1, samples: 1, swizzle };
                let layout = Layout::new(&desc).unwrap();
                for (mip, m) in layout.mips.iter().enumerate() {
                    for z in 0..m.depth {
                        let axes = Axes::for_part(&layout, layout.size as usize, mip as u32, z).unwrap();
                        let (lo, hi) = axes.source_bounds(element_bytes, layout.size as usize).unwrap();
                        for y in 0..m.height { for x in 0..m.width {
                            let at = u64::from(axes.columns[x as usize]) + u64::from(axes.rows[y as usize]) - u64::from(axes.origin);
                            assert_eq!(at, layout.offset(mip as u32, x, y, z).unwrap());
                            assert!(lo as u64 <= at && at + u64::from(element_bytes) <= hi as u64);
                        }}
                    }
                }
            }
        }
    }

    #[test]
    fn axis_equation_matches_existing_layout() {
        for swizzle in [Swizzle::SW_256B_S, Swizzle::SW_4KB_D, Swizzle::SW_64KB_S, Swizzle::SW_64KB_R_X] {
            for element_bytes in [1, 2, 4, 8, 16] {
                let desc = Desc { dimension: Dimension::Tex2d, width: 513, height: 259, depth: 1,
                    mips: 1, element_bytes, block: 1, samples: 1, swizzle };
                let layout = Layout::new(&desc).unwrap();
                let Some(axes) = Axes::new(&layout, layout.size as usize) else {
                    assert!(!layout.addressable() || (swizzle == Swizzle::SW_64KB_R_X && matches!(element_bytes, 1 | 16))); continue;
                };
                let (lo, hi) = axes.source_bounds(element_bytes, layout.size as usize).unwrap_or((0, layout.size as usize));
                for y in 0..desc.height { for x in 0..desc.width {
                    let a = axes.columns[x as usize]; let b = axes.rows[y as usize];
                    let offset = if axes.xor { (a & !65535) + (b & !65535) + ((a ^ b) & 65535) }
                        else { a + b - axes.origin };
                    assert_eq!(u64::from(offset), layout.offset(0, x, y, 0).unwrap());
                    assert!(lo <= offset as usize && offset as usize + element_bytes as usize <= hi);
                }}
            }
        }
    }
}
