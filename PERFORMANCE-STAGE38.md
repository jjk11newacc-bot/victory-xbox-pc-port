# Stage 38 — settled readback, lifetime retention, compilation diagnostics

Base: VIctory_stage37.zip. Source-only; no measured FPS, Windows build or GPU parity result. Target remains RTX 3060 Ti 8 GiB, 16 GiB RAM, native 1080p, RT off. All newly enabled behavior is experimental and defaults off (diagnostic compile counters are collected without changing rendering).

## Changes

- `host.rs` (`VICTORY_PACK_SYNC_READBACK=1`): the non-relay/settled path now unions dirty masks for every declaration covering the same backing and piece, just like the existing relay path. Each declared range is still clipped by `changed_ranges`. Claim each byte of a backing at most once in a settlement, including prior settled flights, using a disjoint interval union; repeated settled merges could otherwise mistake preserved CPU writes for new GPU writes. Pack adjacent transfer runs by backing via the existing packer, preserving first-claim merge order. Keep merges serial in this mode: distinct backing IDs may alias guest addresses. Mandatory waits, ownership publication, and fence ordering remain. Extra coverage relative to the former implementation and serial merging may cost performance. Not a removal of dependent readback waits; default relay path unchanged.
- `descriptor/host.rs`: `VICTORY_EAGER_DESCRIPTOR_RELEASE=1` clears GPU views and resource references when the existing lifetime contract marks a dynamic allocation discarded, even without texture pressure. Replayable runs unchanged. With existing `VICTORY_RECLAIM_RAW_SLOTS=1`, normal descriptor allocations can collect retired raw runs once per frame (nonblocking retired lock), even without subsequent raw bindings. Only the sole-owner lease may be collected. Trade-off: additional descriptor writes and collection overhead.
- `memory.rs` and `host.rs`: existing raw-slot reclamation also attempts idle compaction in the existing quiescent submission sweep. Nonblocking memory lock, sole epoch lease, current ownership generation; do not reconcile stale maps inside submission lock. Stale directories still need the normal raw-bind reconciliation. Live replayable lists block compaction; current descriptor runs can still pin old resources until their next raw bind or destruction.
- `texture/gpu_detile.rs`: existing dirty-job gate now bypasses GPU compute setup and input transitions entirely when the completed previous detile revision has no dirty raw pages; output is still transitioned and copied into the texture, retaining existing destination correctness. Unsupported layouts and other revisions unchanged.
- `pipeline/make.rs` and `pipeline.rs`: `VICTORY_TRIM_PIPELINE_REGISTRY=1` prunes expired weak pipeline-cache entries during the existing idle sweep (nonblocking registry lock). No live state is removed. The registry previously retained weak control blocks and key allocations for the entire process lifetime. Compiled/failed pipeline attempt counts and shader/library memory totals now appear in the existing memory pressure diagnostics.

## All 12 requested issues: remaining obligations

| Issue | Source status | Remaining |
|---|---|---|
| Old memory backing reclamation | Stage 37 conditional epoch compaction; Stage 38 additional idle attempt and retired descriptor collection | Persistent replay lists, stale generations, current raw heap refs and live resources may block reclamation; verify actual lifetime on GPU |
| Excessive guest memory comparisons | Prior watch/journal, sparse merge, exact card uploads, shared GPU hashes, range coverage | Broad indirect writes and untracked CPU writes still require comparison; measure actual 2.6 GB/frame baseline |
| GPU readback stalls | New non-relay dirty-mask union, unique byte claims and packing; existing relay capture | Required producer/consumer waits cannot be discarded; profile GPU and CPU, compare on target |
| Full GPU detile coverage | Existing eligible 2D layouts, BC, small elements, arrays/mips; new no-change setup bypass | Unknown swizzles, thick/3D/MSAA, small/large uploads outside limits and GPU parity remain |
| NVIDIA wave64 emulation | Prior wave-local and single-wave group lowering | No new cross-half rewrite; semantic parity and NVIDIA profiling needed |
| Draw-call overhead | Prior root/IA reuse, batched raw work; idle cache cleanup | No new draw batching; descriptor clearing can increase CPU cost; measure exact 12,500-draw bottleneck |
| VRAM overcommit | Prior adaptive admission, pool/window caps | No new global hard cap; actual residency and shared VRAM unverified |
| System RAM | New weak registry pruning, descriptor resource cleanup and backing sweep | Mandatory live allocations cannot be evicted; no guarantee 10-11 GiB |
| Shader compilation stutter | Prior async compilation and wait controls; new per-sample attempt counters | No new wait policy; full-game warmup coverage and one-shot pass tests needed |
| Texture streaming | Prior adaptive pacing/demand/partial uploads; clean detile fast path | Actual p95 latency and quality unverified |
| Memory leaks | Concrete retention fixes | Long-session RAM/VRAM measurement, D3D12 debug, replay tests required |
| Command-list wait splitting | Existing implementation preserved | Windows and real GPU verification still required |

## Tests performed
- All eight independent Stage 30–37 model scripts passed; exact outputs are in STAGE38-MODEL-RESULTS.txt.
- Stage 38 independent interval model: 100,000 exact byte-coverage claims, and a CPU-only write preservation example passed. Rust unit regression added; not executed.
- Required cargo check: command not found (exit 127). NVIDIA profile checker: failed because cargo not installed (exit 1). No Rust compiler, HLSL execution, game or D3D12 adapter available. No new claimed performance numbers.
- Test each gate separately against the same save/camera after warmup. In particular force the non-relay path for PACK_SYNC_READBACK tests, verify overlapping dirty declarations with CPU changes, recorded-list replay after descriptor reclamation, and GPU texture parity. Monitor private bytes, working set, dedicated/shared GPU memory, median/p95 frame time and draw drops. Do not enable every flag at once. Leave RT off and preserve all mandatory synchronization.
