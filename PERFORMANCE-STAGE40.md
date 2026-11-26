# Stage 40 — generation-indexed write watch and pressure-safe cache retirement

Base: VIctory_stage39.zip. Target 3060 Ti, 16 GB RAM, native 1080p, RT off. This source candidate is not compiled or GPU-tested. The supplied Stage 17 improvement brief calls for default-off experimental behavior and parity/hardware A/B. No waits, fences, executable checks, or relay ordering are deliberately removed.

## Changes
- `watch.rs`: `VICTORY_INDEXED_WATCH=1` indexes the maximum recorded generation per 256 guest pages (16 MiB). A query with a nonzero caller watermark can skip chunks with no newer entry; within candidate chunks it filters the exact original page journal. All kernel write-watch/guard queries still happen as before. Every journal write site updates the conservative maximum. The submission's singleton query pass also uses the group containment index plus the maximum end of prior successful singleton calls; exact same candidate ordering and containment, not a union of adjacent successful queries.
- `shadow.rs`: buffer refresh and in-flight merge paths use the minimum copied page generation as a conservative watermark (zero for uninitialized pages). Clean initialized ranges avoid a second page walk. No page with a newer journal entry is intentionally skipped. Merge now records `max(now, write)` rather than an older ahead-query timestamp, matching the existing `copy_stale` rule for explicit writes after the query. This can reduce repeated consideration of CPU writes, not GPU-written byte comparisons. Actual affected workload size unmeasured.
- `texture.rs`: use its existing `Last::asked` watermark for the same journal filtering. Existing texture retry, guard, hash and detile rules remain intact.
- `shadow.rs`, `budget.rs`: `VICTORY_STICKY_RAM_PRESSURE=1` keeps the retired snapshot pool at 0 bytes under 2 GiB free physical RAM, or at most 64 MiB between 2 and 4 GiB. The normal cap resumes at 4 GiB free. No live snapshot is evicted. Retired buffers no longer immediately refill a cache after a pressure sweep.
- `texture/upload.rs`: same pressure gate stops keeping idle staging chunks after the last lease drops, while available RAM is low. Hysteresis restores normal caching at 4 GiB free. Only unleased chunks are removed; no active resource lifetime is shortened. Possible more allocations when memory is scarce, and periodic RAM measurements can lag real demand.

## Audit of the twelve requested issues
| Issue | Status |
|---|---|
| Old backing reclamation | Existing conditional reclamation unchanged; replayable lists may pin resources. |
| Excessive guest memory comparisons | New indexed watch filters CPU-side historical journal work; GPU comparisons, unwatched regions and broad declarations remain. |
| GPU readback stalls | Existing packed readbacks and relay unchanged; required dependencies remain. |
| Full GPU detiling | Prior expanded coverage, unsupported layouts remain, parity untested. |
| NVIDIA wave64 | Existing safe lowering remains; cross-wave optimization unresolved. |
| Draw overhead | Faster watch scheduling; prior draw state caching; no measured draw bottleneck. |
| VRAM spill | Existing caps and admission unchanged; unverified. |
| System RAM | Sticky retirement caps added; live guest memory may exceed target. |
| Compilation stutter | Existing controls and warmup; untested. |
| Texture streaming | Filter historical journal work and reclaim idle staging; no measured pacing or quality. |
| Memory leak verification | Retention policy improvement, no long-session runtime test. |
| Wait splitting | Existing implementation unchanged, unverified. |

## Validation and usage
Both new gates default OFF. Test separately after shader warmup on same scene. The independent Python model checks exact filtered journal results, stale-page equivalence, and successful/failed singleton query scheduling. Stage 39 model rerun also passed. New Rust unit tests added but not executed. Required Windows Cargo check and profile checker failed because Cargo is not present. `tools/Validate-Stage40.ps1` invokes prior validation and the new unit tests. Exact source diff in STAGE40-SOURCE-DIFF.patch. No measured FPS, RAM or VRAM results. Target 30 FPS remains unverified; neither a finished build nor complete resolution of the twelve issues is claimed.
