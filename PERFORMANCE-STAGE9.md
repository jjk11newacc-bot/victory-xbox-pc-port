# Stage 9

Texture correctness: preserve the previous image when a non-tail texture is only partially mapped, rather than uploading zero-filled missing pages. Reset the watch cursor after a failed coarse-mip mapping so it is retried. Trade-off: incomplete non-tail textures may remain absent or stale until fully mapped; retries may increase CPU work.

No GPU detiling, unified-memory redesign or wave64 execution optimization implemented. No compilation or runtime testing performed.
