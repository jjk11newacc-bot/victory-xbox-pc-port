# Stage 37 — raw memory lifetime reclamation, smaller GPU texels, pipeline wait control

Base: VIctory_stage36.zip. Target: RTX 3060 Ti 8 GiB VRAM, 16 GiB system RAM, native 1080p, ray tracing off. The two supplied improvement briefs have the same SHA-256 and describe Stage 17. This revision is source only, not a compiled executable, not a measured FPS result and NOT a complete resolution of all outstanding issues.

## Source changes and activation

### Raw memory backing and descriptor retention (`VICTORY_RECLAIM_RAW_SLOTS=1`)
- `host.rs`: deduplicated Arc lifetime tokens in Deferred, retained through closed-list replay, reset and the existing flight lifetime. Raw directory and heap run tokens are distinct.
- `descriptor/host.rs`: raw descriptor runs carry an epoch and lease. Superseded runs are retired, rather than permanently reserved. At subsequent leased raw binds, clear and free a retired run only if the heap is its sole lease owner. The underlying host resource references are released through the existing retire mechanism. No descriptor currently referenced by a recorded list is intentionally rewritten by reclamation.
- `memory.rs`: after ownership reconciliation, compact numbered backings only if no recorded raw list has a lease (including closed, never-submitted lists). Preserve page addresses, backing identity, byte offsets, live ranges and partial-copy expansion; build a new zeroed directory, renumber only live owners, reset per-epoch counters, and require each heap to rebuild its raw run on the next bind. Trigger when at least 16 slots are unused, or at least 64 MiB of logical backing is unused, or physical RAM free is below 2 GiB. One scan per frame when quiescent. New allocation failure leaves the previous epoch intact. Keep all existing fences/relay ordering.
- Limits: persistent replayable raw lists can prevent directory compaction indefinitely. Heap run reclamation is lazy, at the next raw bind to that heap, or heap destruction under the existing lifetime contract. Live guest allocations remain non-evictable; other references can keep an otherwise retired backing alive. No guarantee of a hard RAM cap or complete memory-leak fix. The raw directory may not be a major workload with RT disabled; measure. `VICTORY_MEMORY_PRESSURE_LOG=1` now reports `raw_epoch` and `raw_list_leases` to diagnose blockers. The logical bytes are not committed or resident RAM.
- Added Windows tests for directory mapping preservation and closed-list lease retention. They are not executed here. The descriptor test simulates an epoch change and does not execute old GPU commands.

### GPU detile for 1- and 2-byte elements (`VICTORY_GPU_DETILE_SMALL_ELEMENTS=1`)
Requires `VICTORY_GPU_DETILE=1` and `VICTORY_GPU_DETILE_EXTENDED=1` as before. `texture/gpu_detile.rs` + `texture/gpu_detile.hlsl` add eligible 2D small-element layouts, including non-XOR 8/16-bit and known 16-bit render-XOR equations. The GPU gathers aligned source dwords, extracts 1/2-byte texels, then packs each destination dword with one thread, including odd-width tails. Existing >=4-byte path still reads one dword. Footprint bounds, existing scratch and headroom caps, and CPU fallback retained. No global format support assertion: unsupported swizzles (including unknown XOR), thick and MSAA layouts, too-small/too-large uploads, and allocation failures remain on CPU. Linear layouts retain existing fast CPU copies. Added actual GPU readback parity cases and expanded CPU address equation tests; unexecuted. The independent model checks packing/address arithmetic only.

### Shader compile stutter controls (explicit graphics-only overrides)
`pipeline/make.rs`: `VICTORY_GRAPHICS_PIPELINE_WAIT_MS` and `VICTORY_GRAPHICS_PIPELINE_GATE_S` override only graphics wait budget and loading allowance. Existing compute budgets remain unchanged; default behavior remains unchanged if unset. Useful for an A/B after shader warmup (e.g. graphics wait 5 ms and graphics gate 0); can temporarily omit draws, including one-shot graphics passes. Do not use until baseline shader warmup and correctness established. This is not a solution to shader compilation coverage or wave64 execution overhead.

## Remaining audit against original brief
| Section | Status and unresolved requirement |
|---|---|
| 3.1 VRAM budget | Adaptive pool/window/admission paths present; new raw run reclamation can release unused GPU refs. Must measure dedicated/shared residency, eviction and spikes; no hard global VRAM cap. |
| 3.2 GPU detile | Eligible 2D swizzles, BC formats, partial mips and arrays, small elements, dirty pages/jobs implemented experimentally. Still missing unsupported layout equations, thick/3D/MSAA support and full GPU parity. |
| 3.3 guest memory | Range union, exact unwatched card uploads, watch, page compare sharing, dirty readback/capture paths exist. Broad indirect raw writes still require conservative ownership, snapshots and synchronization. No proof historical 2.6 GB/frame is resolved. |
| 3.4 draw overhead | Batch raw work, root/IA reuse, declaration index and range rebuild present. Real 12,500-draw CPU profile still needed; no unsafe reordering. |
| 3.5 RAM | Pressure trimming and experimental raw epoch/run reclamation. Persistent lists, lazy heap cleanup, other live allocations remain. Must measure private bytes/working set over long sessions; no guarantee 10–11 GiB or zero leaks. |
| 3.6 shaders | Async compilation, cache fingerprint and wave-local/single-group paths exist; new graphics-only wait overrides. Cross-half wave64 behavior requires semantic parity and NVIDIA GPU profiling. |
| 3.7 waits | Wait splitting exists; dependent readback and relay ordering remain required. No general safe removal of waits. |
| 3.8 pacing | Adaptive time/byte/demand scheduling implemented previously, still requires same-scene frame p95 and texture loading tests. |
| 3.9 filtering | Not implemented: a sampler alone doesn't establish color vs depth use, so global override is unsafe. Demand-based residency requires testing. |
| 3.10/11 | Existing native 1080p/RT-off profile unchanged; no new visual downgrades or DXR rewrite. |

## Validation performed in this environment
- `python tools/check_stage37_model.py`: 20,000 directory mapping cases, 50,000 lifetime model events, 10,000 packed-row cases (976,978 byte comparisons), 20,000 render-XOR arithmetic cases passed.
- All seven existing Stage 30–36 model scripts passed (as of this revision). These are independent Python models, NOT compilation or GPU execution.
- Required `cargo check --locked --target x86_64-pc-windows-msvc -p victory --features diagnostics,blake3/pure --bins --tests`: exit 127, cargo not installed. `python tools/check_nvidia_profile.py`: exit 1, FileNotFoundError for cargo. Windows Rust/HLSL compile and GPU tests NOT executed.
- Run `cargo test --locked -p victory-gpu` on Windows with WARP and real NVIDIA hardware; confirm no D3D12 validation errors, including repeated closed-list replay and long flights, and bit-exact texture readback. For the new memory paths run with/without the new gate, log raw counts and pressure; do not enable all flags simultaneously. Test graphics-only overrides separately with a warmed baseline and verify one-shot passes. Capture same save/camera after warmup, median/p95 FPS and frame time, private bytes, working set, dedicated/shared VRAM, draw drops, GPU timing, and texture work.

No game content, fences, executable hash checks or default-on new behavior intentionally added/removed. No claim of stable 30 FPS, 16 GiB suitability or full correctness. All new behavior remains opt-in per the brief. The earlier patches also remain unverified by the real compiler or hardware.
