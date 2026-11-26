# Stage 41 — final offline source candidate

Target: RTX 3060 Ti (8 GB), 16 GB DDR4, native 1080p, RT disabled, 30+ FPS. This is not a compiled executable or a validated finished port. No Windows/D3D12 gameplay or shader execution was available. The historical Radeon benchmark is not a measurement of these changes.

## Changes against Stage 40

- `shadow.rs`: `VICTORY_INDEXED_PENDING=1` maintains exact declarations in original order, a membership set and a coalesced interval index. Refresh gaps use the index rather than repeatedly sorting pending ranges. Flight reference counts have an indexed path. Settlements still operate on exact declarations; rebuild the index after deletion. This trades some metadata memory and rebuild work for reduced repeated scans. Default off.
- `host.rs`: recheck completion while holding the retirement queue lock so a concurrent completion cannot miss an enqueue. Release outside the queue lock. Guest destination sparse-copy fallback uses kernel-mediated writes rather than raw pointer writes into potentially inaccessible pages. Add conservative UAV barriers for overlapping hash dispatches in the same shadow, including the rehash and relay compare/store paths. These are unconditional correctness changes rather than optional performance policies; this is a deliberate exception to the brief's opt-in rule. No required waits or barriers removed. Barriers may cost GPU time.
- `texture/gpu_detile.rs`: optional `VICTORY_GPU_DETILE_LINEAR_1D=1` enables eligible linear and 1D layouts in the extended GPU path; requires the existing GPU detile/extended flags. Linear CPU copies may actually be faster. `VICTORY_GPU_DETILE_LARGE=1` permits 32 MiB per input/output instead of 16 MiB; existing global 96 MiB scratch budget and headroom check remain. Unsupported layouts, footprints or memory conditions retain the CPU fallback. Added CPU address parity test and linear GPU readback test case; neither executed here.
- `pipeline/make.rs`: optional `VICTORY_PIPELINE_WORKERS` caps compilation concurrency to between one and the normal worker count, e.g. 2 for an A/B on a 16 GB PC. No compilation jobs deliberately discarded. May increase warmup time. No override by default.

## Closure audit

| Issue | Actual state / outstanding verification |
| --- | --- |
| Old memory backing reclamation | Conditional directory compaction and descriptor reclamation from prior stages, retirement race addressed here. Live/replayable lists can still pin backings. Needs lifetime and long-play testing. |
| Excessive guest memory comparisons | Indexed pending declarations and journal, selective transfer and GPU comparison paths. Still no measured elimination of bulk comparisons; unavoidable for unwatched or uncertain data without stronger coherence guarantees. |
| GPU readback stalls | Packed/shared readbacks in prior stages, overlap ordering corrected here. Required dependent reads still block. Need GPU timing and correctness traces. |
| Full GPU detiling | Expanded addressable layouts and limits; unknown layouts, unsupported cases and memory-limited paths fall back. No full hardware parity. |
| NVIDIA wave64 | Prior safe lane-local/single-wave lowering; cross-half semantics still require emulation. Needs shader profiling and actual parity; not completed. |
| Draw overhead | Prior binding reuse, batched work, and new pending declaration indexing. Further savings and bottleneck not measured. |
| VRAM overcommit | Existing budget/admission safeguards, not a guarantee or global hard cap. Need live dedicated/shared telemetry. |
| System RAM | Cache pressure limits and reclamation, optional compiler worker cap. Live guest memory cannot be blindly evicted. 10–11 GiB process working-set target unverified. |
| Shader stutter | Warmup and graphics wait controls, worker cap. Shader coverage and runtime compilation rate unverified. |
| Texture streaming | Prior adaptive budgets, dirty page/mip processing, expanded GPU detile eligibility. Visual and latency tests pending. |
| Memory leaks | Multiple retention fixes, no leak-free guarantee; need long-duration allocation and fence telemetry. |
| Command-list wait splitting | Existing implementation; ordering and performance not verified in gameplay. |

## Validation and decision rules

See STAGE41-MODEL-RESULTS.txt and STAGE41-BUILD-RESULTS.txt. Python independent models test algorithms, not the actual Rust functions. Stage 39 model rerun passed; Stage 40 rerun exceeded the 12-second limit (exit 124), not a passing result. Cargo check and NVIDIA profile checker blocked by missing Cargo. All new Rust tests and the Windows script are unexecuted. No FPS/RAM/VRAM claims are made. Earlier experimental switches stay default-off; do not turn them all on together without isolated parity and A/B tests. Correctness fixes in host.rs require compilation and regression validation too.

On a Windows MSVC environment with the pinned Rust toolchain, run `powershell -File tools/Validate-Stage41.ps1`. It includes earlier checks and new targeted tests. Ensure the actual hardware GPU test executes rather than silently returning without an adapter. Collect warm baseline and individual A/B latest.txt, GPU timings, mean/median/p95 frame time, private bytes/working set, dedicated/shared VRAM, compile count and visual regressions. Include long-duration scene transitions and replayable lists for leak verification. Do not promote experimental defaults until parity and hardware testing pass. No game assets added and executable hash validation unchanged.

This is the final offline candidate, not a claim that every issue has been solved.
