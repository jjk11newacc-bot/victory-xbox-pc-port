# Stage 42 — remaining bottleneck and validation pass

Base: VIctory_stage41_final.zip. Target unchanged: RTX 3060 Ti 8 GB, 16 GB system RAM, 1080p native, ray tracing off, 30 FPS. The 30 FPS goal is not verified. A numeric percentage of source work cannot establish performance readiness. No Windows GPU or compiler was available.

## Actual changes

- `shadow.rs`: `VICTORY_BATCH_HAZARD_QUERY=1` together with `VICTORY_INDEXED_PENDING=1` uses the existing union index for a fast no-overlap decision. When the requested span is covered by one successful write-watch ahead query, test the written pages against pending intervals with one journal query rather than scanning each exact declaration. If it is not covered, use the original per-declaration path. Never infer clean memory from missing journal coverage. Actual merge/declaration/reference-count order stays unchanged. `VICTORY_INDEXED_WATCH=1` can further narrow the journal by the minimum copied generation; it is optional. Added overlap assertions in existing Rust test. Empty declarations cannot hold GPU-written bytes and are excluded from both reference hazard and merge checks.
- `shadow.rs`: limit changed-run collection to the already-existing per-submission upload limit (512 runs at the caller) before allocating and scanning all fragmented runs. If the next run would exceed the limit, take the existing settlement path without partially applying this page. This unconditional implementation change preserves the old limit and all required waits. A 64 KiB alternating page can contain 32,768 runs; the new collector stores at most the remaining run allowance. Added Rust regression test. This does not limit the separate shared-memory merge path.
- `memory.rs`: `VICTORY_REUSE_IDLE_DIRECTORY=1` with `VICTORY_RECLAIM_RAW_SLOTS=1` avoids allocating/copying another 16 MiB raw address directory during reconciliation when no recorded/in-flight raw-list lease survives. Uses the same quiescence condition as existing compaction. Otherwise retains original clone-on-remap behavior. The proof is based on the existing lease design and needs actual runtime validation. Does not reclaim live backings or change mapping or GPU address semantics.
- `texture/gpu_detile.rs` and validation scripts: prevent the mandatory GPU test from silently returning when no host device exists; explicitly select NVIDIA hardware and require nonsoftware when requested; verify WARP mode and software adapter when requested. Earlier Stage39 validation commands used WARP environment without compiling `diagnostics`, even though `host::on_warp` requires that feature. Correct both calls. New full validation script includes actual Rust suites and an exact WARP parity test with a success sentinel. Scripts are unexecuted here. Shader suite contains existing tests which can skip unsupported hardware cases, so a passing suite is not full shader parity.

## Remaining audit

| Issue | State after Stage 42 |
|---|---|
| Old backing reclamation | Existing conditional compaction + directory allocation reduction. Live/replayable lists still pin old state. |
| Excessive guest comparisons | New indexed hazard path reduces duplicate journal processing. Unwatched memory and GPU-written ranges still require comparison/transfer. |
| GPU readback stalls | Bounded run bookkeeping preserves existing settle fallback. Dependent CPU reads still must wait. |
| GPU detile | Prior coverage retained; unknown XOR, MSAA, unsupported layouts require an address oracle and parity. |
| NVIDIA wave64 | Prior optimizations retained. Existing approximate fallback (`VICTORY_WAVE_LOCAL` default behavior) may compute per-half results where cross-half results are needed; strict `idiom` mode exists but may refuse shaders. No safe blanket wave32 replacement. |
| Draw overhead | Prior batching/binding reuse; no measured hot path on 3060 Ti. |
| VRAM budget | Prior admission protections, not a verified cap. |
| System RAM | Prior adaptive reclamation plus less directory churn and bounded run metadata. Live guest memory cannot be blindly freed. |
| Shader compilation | Prior worker/wait controls. Actual shader warmup coverage and CPU/GPU timings unknown. |
| Texture streaming | Prior dirty-page/mip processing. Hardware parity gate corrected; not run. |
| Memory leaks | Retention improvements, no long-duration hardware data. |
| Command-list wait splitting | Existing implementation preserved. Duplicate split-list replay remains a documented shortcut in list.rs; not changed because ordering and lifetime need testing. |

## Validation

See STAGE42-MODEL-RESULTS.txt for independent model output and STAGE42-BUILD-RESULTS.txt for actual build attempts. Models do not compile or execute the Rust implementation. Cargo unavailable (exit 127), profile checker blocked by missing Cargo (exit 1), PowerShell unavailable. No GPU test was run. No FPS, working-set or VRAM measurements. No game files added. New experimental behaviors default off; do not enable every prior experimental setting at once. Correctness/validation changes are not performance claims.

On Windows with the pinned toolchain and verified format tables initialized, run `powershell -File tools/Validate-Stage42.ps1`. It checks compilation and runs the GPU/Shader suites, then repeats GPU tests with the new options and runs WARP parity. Even if all pass, collect warm baseline and isolated A/B runs in the same scene, actual GPU timings, frame p95, process private bytes/working set, dedicated/shared GPU memory, and long-duration leak/visual regression checks before promoting any experimental defaults. Cannot close performance targets or unresolved architecture based on offline model tests.
