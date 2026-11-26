# Stage 33 — expanded GPU texture detiling

Base: VIctory_stage32.zip. Targets improvement brief section 3.2. This extends the actual GPU compute upload path rather than changing quality settings.

## What changed

- GPU-detile plans now hold one nine-dword dispatch record per eligible D3D12 placed subresource, with its own mip/slice source axes, row pitch, and destination byte offset. Raw bytes and concatenated axis tables are transferred once per staged revision. The compute shader writes separate output footprint ranges; the existing copy/transition and immutable Arc lifetimes remain in use.
- `VICTORY_GPU_DETILE=1` continues to enable the original single-mip subset. Set **both** `VICTORY_GPU_DETILE=1` and `VICTORY_GPU_DETILE_EXTENDED=1` to test addressable BC block layouts (8/16 bytes per block), multi-mip, array layers, and single-slice partial mip suffixes. Existing supported 4/8/16-byte uncompressed layouts are retained. Neither flag is enabled automatically. Linear remains CPU copy; unsupported XOR, MSAA, 3D, 1/2-byte elements, mismatched footprints and allocation failures remain CPU paths.
- Validate every job against the known Layout offset equation, raw mapped size, row count, footprint dimensions in block units, 512-byte offset alignment, and nonoverlapping pitched output bounds. Restrict to at most 192 subresources, 2 MiB of address metadata, 1–16 MiB output and 16 MiB raw bytes. Existing 96 MiB requested device-local scratch cap, 512 MiB additional VRAM headroom and low-RAM fallback retained. These are not whole-process caps or guarantees of lower RAM consumption.
- No changes to guest ownership, relay fences, inline CPU fallback, copy-only queue exclusion, asset checks, resolution or ray-tracing settings. Compressed texture blocks are copied byte-for-byte; no quality reduction or decoding is intended. Render-XOR multi-mip/BC layouts remain unsupported by the existing Layout oracle.

## Validation

- `python tools/check_stage33_detile_model.py`: 1,391,338 element and 3,183,242 dword comparisons across 600 synthetic multi-job plans, using the source microtile patterns. This checks the model, not compiled Rust/HLSL or actual Xbox asset layouts.
- Previous Stage 30, 31 and 32 independent models pass (128,000 + 30,000 + 110,000 cases).
- Extended the actual GPU readback regression test with BC8, BC16 array, mip tail and partial single-slice cases. These tests use synthetic footprints and compare visible output rows against `Layout::detile`. Not executed here; D3D12/Windows and pinned Rust toolchain unavailable. NVIDIA checker attempted, blocked by missing cargo.
- Run on Windows with the pinned toolchain:

      cargo check --locked --target x86_64-pc-windows-msvc -p victory --features diagnostics,blake3/pure --bins --tests
      cargo test --locked -p victory-gpu gpu_detile -- --test-threads=1
      python tools/check_nvidia_profile.py

Compare warmed same-save/same-camera runs: both flags off, base flag only, both flags on. Observe CPU texture time, GPU pass time, VRAM/private bytes, visual parity and frame p95. Roll back by unsetting extended or both flags. Additional dispatches and raw PCIe upload/scratch may regress GPU-bound scenes. This stage does not implement dirty-page GPU uploads or guarantee texture coverage in the game. No measured FPS or memory improvement.

## Next major work
Guest memory copy/readback reduction and per-draw CPU cost; GPU-detile dirty-page updates and scratch retirement; wave64 profiling. Full compile and gameplay validation required before enabling experiments by default.
