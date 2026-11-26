# Stage 39 — volume detile, graphics bindings and retention

Base: VIctory_stage38.zip. Target RTX 3060 Ti 8 GiB, 16 GiB RAM, native 1080p, RT off. This is an uncompiled source candidate, not a complete fix or FPS result. The supplied brief requires opt-in changes and measured parity before promotion. No synchronization or executable integrity check intentionally removed.

## Changes
- `texture/gpu_detile.rs`: opt-in `VICTORY_GPU_DETILE_VOLUME=1` (requires GPU_DETILE and GPU_DETILE_EXTENDED, plus GPU_DETILE_SMALL_ELEMENTS for sub-dword elements). Accept known addressable single-sample tiled volume layouts. Each depth slice uses existing Layout::offset as its oracle and a separate disjoint output plane of the D3D12 placed footprint. Check complete volume depth, full pitched extent, subresource count, bounds, and caps on jobs/metadata. Existing HLSL writes rows unchanged, and CopyTextureRegion copies the volume. Partial 3D revisions, unknown swizzles, multisample surfaces, small/oversized uploads, allocation failures retain CPU/previous fallback. Extra dispatch and table overhead could outweigh benefit; this is not full layout coverage. Added actual CPU-oracle volume tests and GPU readback volume cases, unexecuted here.
- `draw.rs`: `VICTORY_REUSE_GRAPHICS_BINDINGS=1` retains exact typed root GPU addresses/descriptor table handles; avoid identical setter calls. Existing resource lookup, memory refresh/write declarations, ownership, and descriptor preparation still execute. Cache invalidation follows existing root constant cache on heap/signature changes, external passes, replay, and indirect paths. Compute unchanged. Added unit test of exact key and invalidation; D3D12 command parity not executed.
- `memory.rs`: under existing `VICTORY_RECLAIM_RAW_SLOTS=1`, a quiescent directory at/above initial speculative slot limit can compact even when only a small number of entries are dead. The lease guard and existing live-owner remap remain; persistent recorded lists can still prevent compaction. This is not a universal leak fix.
- `pipeline/make.rs`: under existing `VICTORY_TRIM_PIPELINE_REGISTRY=1`, prune superseded early queue references periodically. Promoted jobs retain their requested queue entries; no unique job is cancelled and WAITING accounting is unchanged. Retired Arc references drop outside queue lock. The queue regression test now checks promotion cleanup too. Does not shorten driver compilation time.

## Remaining 12-item audit
| Issue | Status |
|---|---|
| Old memory backing reclamation | Conditional compaction improved; replayable lists can pin resources. |
| Excessive guest memory comparisons | Prior selective refresh/upload and compare paths exist; untracked writes and broad raw writes remain; no new fundamental comparison rewrite. |
| GPU readback stalls | Prior packed readbacks and relay exist; required dependencies remain; no new readback change. |
| Full GPU texture detiling coverage | Known tiled volume layouts added; unknown XOR, MSAA and other unsupported layouts remain; actual GPU parity untested. |
| NVIDIA wave64 emulation | Previous narrow lowering remains; cross-half optimization unresolved. |
| Draw-call processing overhead | Exact graphics root binding reuse added; draw count and remaining CPU overhead unmeasured. |
| VRAM overcommit | Existing caps/admission; no proof of 8 GiB residency or absence of spill. |
| System RAM | Conditional backing and queue retention improved; live memory remains, no verified RAM limit. |
| Shader compilation stuttering | Stale queue retention addressed, previous wait controls unchanged; warmup and runtime compilation untested. |
| Texture streaming | Prior adaptive pacing, dirty mip and page paths; eligible 3D GPU path added; p95 and image quality untested. |
| Memory leak verification | Requires actual long-session workload; not performed. |
| Command-list wait splitting | Existing implementation retained, not verified on GPU. |

## Validation evidence
- Exact source diff against Stage 38: STAGE39-SOURCE-DIFF.patch.
- Stages 30–37 independent model scripts completed successfully in this run. Stage 38 rerun hit tool time limit. New model: 48,000 synthetic volume-address/plane checks and 20,000 synthetic graphics binding events passed. These are not executing the Rust source. See STAGE39-MODEL-RESULTS.txt.
- Required cargo check exited 127; NVIDIA profile checker exited 1 because cargo does not exist in this environment. See STAGE39-BUILD-RESULTS.txt.
- tools/Validate-Stage39.ps1 runs the required Windows build check, profile check and new regression tests; must be run on Windows with pinned toolchain. A hardware test that skips because no adapter exists is not hardware validation. Use WARP and a real NVIDIA adapter, monitor D3D12 debug messages and parity, then run actual game tests separately. Script not executed here.
- Test new switches separately, using same save and camera after shader warmup. No measured FPS, RAM, VRAM or stutter result. Do not combine unverified switches blindly. Keep 1080p and RT off; measure private bytes, working set, dedicated/shared memory, median/p95 frame time, dropped draws and visual parity. No promise of stable 30 FPS.
