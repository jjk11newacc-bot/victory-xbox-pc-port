Stage 3 (based on stage 2)
- Exact 4 KiB region comparisons before cache-line boundary searches; expanded boundary tests. This targets changed-piece writeback searches, not the full guest scan. Performance unmeasured and may regress on dense changes.
- Fix skipped all-zero mapped texture refresh incorrectly publishing a hash for bytes never staged; invalidate hash on failed inline detile. Preserve previous image and retry.
- Release fully idle texture upload pool chunks below 2 GiB available physical RAM; no active allocations evicted. At most 16 MiB additional idle pool retention removed.
- No GPU detiler, unified memory redesign, wave64 rewrite, draw batching, hard RAM cap or new synchronization architecture implemented. Existing in-place probe requires target hardware. No Rust compiler or Windows GPU available; source review and ZIP integrity only. No FPS guarantee.
