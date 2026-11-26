# Stage 30 — sparse refresh scheduling and full-pass pacing

Base: the supplied VIctory_stage29.zip. No game assets, executable checks,
GPU fence, write declaration, relay ordering, shader translation, or readback
publication code changed. New behavioral experiments are off by default.
Existing Stage 29 launcher profile remains unchanged.

## Changes

- `VICTORY_SPARSE_REFRESH=1`: the submission Batch retains separated ranges
  for each Shadow instead of always refreshing the complete enclosing span.
  Small gaps (up to 64 KiB) are joined to avoid many small kernel calls; over
  64 separate intervals falls back to the conservative old envelope. One job
  per shadow is retained, so parallel workers do not update the same shadow.
  `flush_one` and full flush preserve their old synchronization boundaries.
  Write-watch prequeries use sparse spans only with no prior flight in the
  air; the old bounding span remains for stale-under-flight checks. This does
  not change the GPU's writeback comparison or readback traffic. The existing
  `VICTORY_SPARSE_DESCRIPTOR_REFRESH` can be tested separately; it is needed
  to preserve separate descriptor ranges already collapsed upstream.
- `VICTORY_STREAMING_FULL_BUDGET=1` (requires `VICTORY_ADAPTIVE_BUDGET=1`):
  account for room checks, remap/sweep, recheck, queue selection and early
  returns in the adaptive streaming clock. Rechecks are limited to the lesser
  of 2 ms and remaining time. Mandatory maintenance is still done; a single
  expensive operation or concurrent passes may exceed this soft budget.
  Texture byte accounting is already cumulative in the old code; not changed.
- `VICTORY_MEMORY_PRESSURE_LOG=1` (diagnostics build): periodic log of working
  set, process commit (PagefileUsage), available RAM, dedicated/shared DXGI
  usage and budgets, shadow/window and retired snapshot pool accounting.
  Categories overlap; do not sum them. Refresh coverage line records legacy
  bounding bytes, scheduled bytes and excluded gaps; not measured PCIe bytes.

## Validation

Python independent model passed 128,000 randomized additions against a sorted
interval reference, plus a synthetic sparse range check (67,112,960 envelope
bytes vs 8,192 scheduled bytes). This is a model, not execution of Rust or a
performance benchmark. Added Rust tests for disjoint, merged, fragmented and
randomized ranges. `cargo check` and the NVIDIA profile checker were attempted
and blocked because cargo/rustc are unavailable. No D3D12 or game test.

## A/B order

1. Baseline Stage 29 defaults after shader warmup, collect latest.txt.
2. Enable sparse refresh only; compare coverage, copy bytes/time, GPU waits,
   screenshots, and settled-scene p95. Then separately test the existing
   sparse descriptor refresh flag if the heap pass dominates.
3. Enable full streaming budget separately; compare p95 and texture arrivals,
   especially decoration bake output. It can delay texture uploads or rechecks.
4. Enable pressure logging for RAM/VRAM diagnosis (not a memory cap).

Disable a flag (set 0) to roll back. Sparse refresh can regress for fragmented
ranges, increase kernel calls, or expose missing descriptor range declarations.
Do not promote until real-hardware parity. There is no verified FPS gain,
no GPU detiling implementation, no elimination of the multi-GB writeback scan,
and no guarantee of operation on 16 GB or 30 FPS.
