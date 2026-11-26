Stage 5: static source revision from Stage 4
- Narrow reverse changed-byte searches to the suffix beginning at the first changed byte (four writeback paths). Exact byte semantics retained; benefit workload dependent.
- Deduplicate identical rounded guest-page ranges in both watch batching paths; retain distinct/overlapping ranges and fallback behavior. Added regression test.
- Hoist render-XOR texture column offsets out of row loop, retaining the exact original address formula and copy lengths. Extra temporary column vector; CPU detiling remains.
- Release retired staging resources outside the global staging lock; no in-flight references evicted.
- Existing NVIDIA profile, shader semantics, compiler settings and synchronization preserved.
- No GPU compute detiling, unified memory redesign, wave64 rewrite, hard RAM cap or draw batching implemented. No Windows compiler or GPU/game validation. No FPS claim.
