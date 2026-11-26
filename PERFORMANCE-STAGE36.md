# Stage 36 — audit of the original improvement brief and remaining work

Base: VIctory_stage35.zip. The reuploaded improvement brief is byte-identical to the earlier brief and assesses Stage 17, not Stage 35. Target remains native 1080p, RTX 3060 Ti (8 GiB VRAM), 16 GiB RAM. This is an experimental source revision, not a compiled build or a measured FPS result.

## Changes
- `VICTORY_GPU_DETILE_DIRTY_JOBS=1`: with the previous GPU detile and dirty-page switches, skip compute dispatches for subresources whose conservative source envelope cannot intersect any changed raw page. Only reuse after the existing completed-fence proof. If a bound cannot be proved, use the whole input envelope. First-use/full-refresh paths process every job. Destination copies remain unchanged. Current dynamic partial-plan refresh limitations remain. The existing hash-based dirty detection is probabilistic (BLAKE3); no extra hash assumptions introduced.
- `VICTORY_WAVE_SINGLE_GROUP=1`: for wave64 emulation with explicitly requested host wave32 and compute groups of at most 32 threads, replace wave-base initialization with constant zero. This removes only the translator's unnecessary single-wave LDS counter, group barrier and atomic sequence, not any title barrier or queue fence. Larger groups and unsupported shaders use existing lowering. Include switch in translation cache fingerprint; warmup is required after changing it.
- `VICTORY_BATCH_PAGE_RANGES=1`: when at least eight raw directory owners require recalculation, rebuild affected owner ranges by walking the union of their covered page-tree spans, keeping the legacy clipping, order and contiguous coalescing. Avoid repeated visits for overlapping owner ranges. Not a change to page ownership or writeback.
- `VICTORY_WATCH_CLEAN_FAST=1`: after the existing write-watch query, prove all pages current using the maximum reported generation and the copied generations; otherwise run the legacy path. No kernel query or refresh correctness check removed. Can be slower in mixed/dirty ranges; test separately.
- Extend `VICTORY_MEMORY_PRESSURE_LOG=1` (existing flag) to report texture staging logical bytes and retained raw page-directory backings, including logical bytes for owners with zero current pages. These are overlapping allocation sizes, not resident/private RAM measurements.

## Audit against brief sections
| Brief | Stage 36 status |
|---|---|
| 3.1 VRAM | Adaptive pool, card-window and admission logic present; strict admission opt-in; runtime residency and paging unverified. |
| 3.2 GPU detile | Supported 2D layouts, BC formats, mip chains, eligible arrays and partial tails; dirty input and new dirty-job pruning. Unsupported formats/layouts (including 1/2 byte elements, thick layouts, MSAA) still CPU fallback. Sparse and GPU parity not verified. |
| 3.3 guest sync | Existing shader-aware write declarations (writes_guest_memory), bounded known builder writes, watch, card compare, exact upload, shared compare and capture readback paths. Broad raw stores still require conservative declarations. Historical 2.6 GB comparison problem not shown resolved. In-place probe needs actual hardware. |
| 3.4 draw overhead | Existing root/IA reuse, batch raw work and new page range rebuild. No measured CPU profile; descriptor and command recording overhead still possible. |
| 3.5 RAM | Existing pressure reclaim, memory samples extended. Raw directory retains every numbered shadow (memory.rs Memory::number) and old descriptor runs (descriptor/host.rs raw_page_views). This is a concrete retention risk; not a proven leak. Cannot safely recycle slots without tracking every recorded/in-flight list that references each generation. Requires versioned directory/descriptor leases and fence-aware retirement; no unsafe freeing added. |
| 3.6 shaders | Async pipelines and wait controls already exist; one-shot compute work must not be indiscriminately dropped. Wave-local lowering already exists; new small-group optimization is narrow. Remaining cross-half exchange requires hardware profile and GPU parity. |
| 3.7 waits | queue.rs already splits submissions and cuts command lists at value waits. Existing capture readback may avoid extra fetch, but overflow/dependent reads need synchronization. Cannot eliminate all waits without breaking correctness. |
| 3.8 pacing | Existing adaptive streaming time/byte budgets, demand scheduling and coarse mip handling. Actual stutter and visual parity unverified. |
| 3.9 texture quality | Global anisotropy override not implemented; sampler alone does not establish color vs depth use, so avoid unsafe forced filtering. Demand-based residency needs hardware tuning. |
| 3.10/3.11 | Native 1080p and RT off per low-memory launcher; no additional resolution or visual downgrade. |

All new behavior switches default OFF. Independent Python model: 30,000 source-bound/dirty page checks, 100,000 watched-memory no-copy predicate checks, 10,000 ownership rebuild checks. Rust test adds bounds and overlap assertions. Cargo check and NVIDIA profile checker attempted; both blocked by missing cargo. Real GPU execution, source compilation, memory leak testing and FPS are unverified. No actual game content added.

## Validation
`python tools/check_stage36_model.py`
`cargo check --locked --target x86_64-pc-windows-msvc -p victory --features diagnostics,blake3/pure --bins --tests`
`python tools/check_nvidia_profile.py`
`cargo test --locked -p victory-gpu`

Test individually against a warmed baseline with same save/camera. Do not enable all experimental paths together before parity is established. Use existing memory-pressure and GPU timing logs. If regressions occur, unset the corresponding flag. Remaining critical/high work is NOT complete, especially stable-generation reclamation, broad raw-store provenance, dependent readback synchronization, full detile and wave parity, and measured FPS/RAM verification.
