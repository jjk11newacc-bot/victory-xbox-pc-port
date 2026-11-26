# Stage 34 — raw memory recording and per-draw state reuse

Base: VIctory_stage33.zip. Targets improvement brief sections 3.3 and 3.4, especially host CPU overhead. No resolution, lighting, texture-quality, GPU-detile coverage or memory limits changed. Both new switches default off.

## Changes

- `VICTORY_BATCH_RAW_WORK=1`: raw directory bindings now record the same ordered refresh and writeback steps under one deferred-work mutex acquisition for the nonempty range block, rather than one acquisition per range. The existing raw directory coverage, precise bounded writes, owner resolution, in-place exclusions, exact deduplication, clamping and title-write invalidation are preserved. Scalar and bulk paths share the same Deferred step recorders. A bind naming 4,100 ranges would take one rather than 4,100 acquisitions for this block (other locks and lookups still exist). The bytes scheduled are deliberately unchanged. This is not a PCIe transfer elimination or a readback elimination. A very large block can hold the mutex longer and compete with other threads; benchmark.
- `VICTORY_REUSE_ROOT_DATA=1`: use 64-DWORD stack storage for ordinary translated root-constant construction instead of a fresh allocation at each shader stage. Cache exact graphics constant values by host parameter (no hash collision), avoiding repeated identical setter calls. Compute setters are not cached because split dispatches can patch constants. Cache invalidated on heap/signature changes, internal/external pass, compute bind, replay, reset, table retirement and indirect execution. Still translate each stage's arguments, record material demand, resolve descriptors, refresh resources and declare writes.
- Same flag enables exact translated vertex/index binding reuse. Compare GPU addresses, lengths, strides/formats and vertex array length, not just guest addresses. Ownership lookup and refresh still happen per draw. Invalidate on rectangle expansion (including partial failure), indirect execution, external state loss, internal passes, replay and table retirement. No draw merging or reordering.

## Validation

- `python tools/check_stage34_model.py`: 10,000 independent randomized scheduling comparisons (dedup on/off, in-place exclusion, bounds, duplicate ranges, and invalidation boundaries) and 100,000 simulated binding events passed. Model skipped 4,955 redundant setters; this is not a real game trace or a measured performance improvement.
- Stage 30, 31, 32, 33 independent model scripts rerun successfully.
- Added Rust tests `batched_raw_work_matches_scalar_step_order` (actual deferred command order) and `exact_graphics_constants_only_skip_identical_words` (actual cache logic). Not executed. Windows D3D12 unavailable, `cargo check` and `tools/check_nvidia_profile.py` attempted but cargo missing. Archive is source only, no tested executable.
- Windows check:

      cargo check --locked --target x86_64-pc-windows-msvc -p victory --features diagnostics,blake3/pure --bins --tests
      cargo test --locked -p victory-gpu batched_raw_work_matches_scalar_step_order -- --test-threads=1
      cargo test --locked -p victory-gpu exact_graphics_constants_only_skip_identical_words -- --test-threads=1
      python tools/check_nvidia_profile.py

Run warmed, same-save and same-camera tests with each switch separately, then both. Compare recording CPU time, frame p50/p95, dropped draws, memory use, and rendered output. Unset either variable for rollback. Exact state comparisons can cost more on draws where every binding changes; extra small per-list cache memory is retained until invalidation/reset. GPU-dependent visual parity and performance are unverified.

## Remaining work

The dominant multi-GB compare/transfer path and GPU-to-CPU waits have not been eliminated. Need runtime counters and same-scene profiles to choose the next large optimization; GPU detile dirty-region uploads, shader wave64 lowering, dynamic raw-memory lifetime management and RAM/VRAM pressure testing also remain. No guarantee of 30 FPS or stable 16 GB operation.
