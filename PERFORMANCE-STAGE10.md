# Stage 10

Watched texture refresh now requires a successful complete kernel read before hashing or staging. Failed reads reset the watch cursor for retry and retain the previous image. This prevents zero-filled partial copies from replacing valid textures; incomplete watched textures can remain stale until readable.

Render-XOR CPU detiling/tiling precomputes column byte bases and run lengths, and computes row byte bases once per row. The address equation is unchanged. The existing independent render-XOR offset and padded-row tests cover this path but were not run. The column vector may use slightly more memory.

No GPU compute detiling, unified-memory redesign or wave64 execution optimization is implemented in this revision. No Rust compiler or Windows GPU testing available. No FPS or RAM guarantee.
