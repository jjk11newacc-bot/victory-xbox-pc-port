# Stage 11

Write-watch journal coverage queries now use a sorted prefix-maximum index after each batch, replacing linear scans in asked() and writes(). The predicate still requires containment within one successful kernel asking; adjacent ranges are not merged. Batch construction retains its original linear checks. Added a regression test against the original predicate. The index adds two machine words per successful range and sorting overhead per batch.

Texture diagnostics now avoid scanning entire tiled inputs for all-zero content when DEBUG tracing is disabled. No change to texture addressing or upload semantics.

No GPU compute detiling, full unified-memory redesign, or wave64 execution optimization. Rust compiler and Windows GPU unavailable; tests not executed. No FPS or RAM guarantee.
