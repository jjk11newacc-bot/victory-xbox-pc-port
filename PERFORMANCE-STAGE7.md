# Stage 7 — targeted static revision
- Serialize texture upload pool growth per size class, rechecking free slots after waiting. This avoids redundant concurrent 2 MiB driver allocations without holding the global pool lock during allocation. Growth within one class can now wait on another worker.
- Trim fully idle staging chunks on pool growth under low physical RAM in low-memory mode. Active allocations are not capped or evicted.
- Retry watched textures with a previously staged coarse mip on the existing timed partial retry schedule, even without a new write notification. Do not promote an existing coarse mip to full coverage if the backing range is still not committed. This is a conservative correctness change; it may increase polling/copy work.
- No change to shader semantics, draw ordering, guest-memory coherency, compiler flags or resolution.
- Not implemented: GPU compute detiling, unified-memory redesign, wave64 rewrite, draw batching, hard RAM cap, complete dirty tracking. No FPS claim.
- No Rust toolchain or Windows GPU test environment available. Source review and ZIP integrity only.
