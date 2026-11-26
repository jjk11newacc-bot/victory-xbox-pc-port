# Texture retry and memory retention revision

Based on NVIDIA experimental archive. Source changes only, not compiled or run.
- Partially mapped textures retry after 30 frames OR 500 ms (at the next refresh), whichever comes first; reported writes still trigger immediate retry. Previously 30 frames meant 7.5–30 seconds at 1–4 FPS.
- Never skip initial guarded texture refresh solely due to the eight-frame hash schedule.
- Clear stale hash after successful un-hashed write-watch detile, so switching read paths cannot falsely identify an old texture revision as current.
- Low-memory mode releases per-thread texture read scratch buffers whose capacity exceeds 64 MiB after use. This bounds retained scratch, not peak allocation or total process RAM; repeated large reads can cost more allocations.
- Added a regression assertion for elapsed-time partial retry. Not executed (Rust toolchain and Windows/D3D12 unavailable).
- Existing 1080p NVIDIA shortcut, RT-off, sparse textures and GPU-local settings preserved.
- No GPU detiling, unified-memory rewrite, shader wave rewrite or draw batching implemented. Existing directory growth code and regression tests already present; historical 1023-slot exhaustion is not proof of a current fixed cap. No FPS or RAM guarantee.
