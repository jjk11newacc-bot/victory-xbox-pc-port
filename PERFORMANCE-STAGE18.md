# Stage 18: coarse mip streaming

Partial single-slice 2D sampled texture refreshes previously copied and hashed the full guest span, including missing fine mip pages, and detiled every footprint before uploading only coarse mips. Now the existing mapping guard protects a committed coarse prefix, which is hashed directly; only eligible coarse footprints are detiled. A fresh staging revision is published only on success. The slot is initialized for inline fallback safety. Full refreshes still detile all levels; mip hashes are cleared on partial publication. The existing sampled_partial_mip_refresh_keeps_the_previous_fine_level regression is relevant but has not been executed.

Tradeoffs: fresh full-sized staging allocation and initialization on changed partial revisions; no hard RAM cap. No GPU compute detiling, wave64 redesign or full unified memory rewrite. No Rust compiler or Windows GPU test available. FPS and texture correctness unverified.
