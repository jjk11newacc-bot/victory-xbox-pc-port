# Stage 32 — experimental GPU compute texture detile

Base: VIctory_stage31.zip. This implements the first bounded runtime GPU-detile path
from section 3.2 of the improvement brief, not the entire texture conversion rewrite.
Existing 1080p output, RT settings, game assets policy, executable checks and queue
fences are unchanged. New behavior defaults OFF; enable `VICTORY_GPU_DETILE=1` for
an isolated hardware A/B. Disable with `VICTORY_GPU_DETILE=0`.

## Runtime implementation

- `texture/gpu_detile.rs` derives small X and Y offset tables using the existing
  `Layout::offset` address equations. The render-XOR equation combines high block
  offsets additively and low 16-bit offsets with XOR; other addressable tiled
  modes use separable row/column contributions. The CPU captures the same tiled
  revision the existing refresh path hashed, but does not detile it.
- `texture/gpu_detile.hlsl` dispatches 64 threads per group, each writing one
  aligned dword into a device-local pitched buffer. The main direct-queue prelude
  copies the raw snapshot and tables to device-local input, dispatches compute,
  transitions the output to COPY_SOURCE and copies the texture as before.
  Both scratch buffers are restored to COMMON. The existing StagedCopy's Arc
  lifetime retains every resource through GPU execution. The copy-only ahead
  queue explicitly rejects compute-backed revisions.
- If a later inline upload needs that revision, the original CPU converter
  reconstructs it from the raw snapshot rather than copying raw tiled bytes as
  linear pixels. Exclusive staging revisions can reuse their scratch allocations;
  recorded/in-flight revisions stay immutable. CPU fallback is also used for
  unsupported geometry, shader compilation or allocation failure, or pressure.
- Conservative eligibility: addressable uncompressed 2D, single sample, single
  mip, single layer, 4/8/16-byte elements (R_X supports 4/8 here), fully staged,
  1–16 MiB pitched output, at most 16 MiB raw bytes. Linear is already a fast CPU
  copy path and stays there. Multi-mip, BCn, arrays, 3D, 1/2-byte elements and
  partial residency remain CPU paths. This limits real scene coverage.
- Temporary device-local input/output requested bytes (rounded to 64 KiB) share
  a process-wide 96 MiB cap. A new plan requires 512 MiB additional adapter
  headroom after its allocation, and falls back if known free system RAM is
  below 2 GiB. This is not a whole-game memory cap. The raw CPU snapshot can be
  larger than linear staging; the GPU experiment is not guaranteed to save RAM.
  Existing GPU-only texture reclamation can release these leases. Scratch is
  retained with a staging revision to avoid premature release; this can exhaust
  the experiment's cap and cause CPU fallback until old revisions retire.
- `GPU texture detile dispatches` logs at powers of two report dispatches
  recorded, reuse count, and reserved scratch bytes. These are not GPU timings.
  This path adds a bulk PCIe copy, compute dispatch and VRAM scratch, so it may
  regress GPU-bound scenes. First use also compiles a compute pipeline.

## Validation

- `python tools/check_stage32_detile_model.py`: 110,000 mathematical address
  decomposition cases passed, including the actual render-XOR basis tables
  extracted from the formats source. This does NOT execute Rust or HLSL.
- Stage 30 and 31 independent models still pass (128,000 and 30,000 cases).
- Added `axis_equation_matches_existing_layout` (real Rust Layout oracle) and
  `gpu_detile_readback_matches_cpu_rows` (actual compute dispatch and readback,
  compares visible row bytes with CPU detile for three swizzle/element cases).
  Neither could run here; Cargo/rustc, D3D compiler, Windows and D3D12 unavailable.
- NVIDIA checker attempted but blocked by missing Cargo. Compile with the pinned
  toolchain and run the real GPU parity test on Windows, optionally with WARP:

      cargo check --locked --target x86_64-pc-windows-msvc -p victory --features diagnostics,blake3/pure --bins --tests
      cargo test --locked -p victory-gpu gpu_detile -- --test-threads=1
      python tools/check_nvidia_profile.py

- Test same warmed scene and camera with flag 0/1; inspect CPU texture preparation,
  GPU timings, private bytes/working set, dedicated/shared memory, dispatch counts
  and screenshots. Confirm inline fallback and changing textures remain correct.
  If dispatch count remains low, the scene uses layouts not covered by this stage.
  Keep the flag off if any corruption, device removal, RAM or frame-time regression.

## Remaining work

BC/multi-mip/array/partial residency GPU detile, dirty-page uploads, scratch
pooling/retirement refinements, raw guest-memory transfer/writeback architecture,
per-draw processing, wave64 shader overhead and full hardware validation remain.
No measured FPS, stutter, memory improvement or claim of stable 30 FPS.
