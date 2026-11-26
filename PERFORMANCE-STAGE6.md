# Stage 6 (static source revision)
- Exact changed-run scanning now uses the existing region comparison helper, avoiding the eight-byte loop for equal spans. Dense changes may see overhead; benchmark before claiming a gain.
- Texture unstage releases resources outside the global staging mutex.
- Texture pool allocates new driver buffers outside its mutex. Concurrent misses may allocate extra chunks; existing idle trimming remains active.
- Existing memory and shader semantics, 1080p profile and compiler settings retained.
- No Rust compiler or Windows GPU available. Tests not executed. ZIP integrity only.
- Not implemented: unified memory redesign, GPU detiling, wave64 rewrite, draw batching, hard RAM cap. No 30 FPS claim.
