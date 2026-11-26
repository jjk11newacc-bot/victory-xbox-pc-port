# Stage 14: bounded descriptor scheduling and experimental mip-level detiling

Based on Stage 13. No Windows/D3D12 runtime or Rust compiler available.

* Descriptor retry/recheck scheduling now partitions candidates before sorting. A pass of size K sorts at most K retries, 2K fresh candidates and K rechecks. The original alternating policy is retained; the full queue is still scanned. Write numbers determine priority.
* Experimental `VICTORY_MIP_DIFF=1`: sampled textures with a single non-thick non-3D slice, multiple mips and supported non-render-XOR layout fingerprint conservative mip spans. Tail mips share the entire tail block. Only an exclusively owned existing sampled staging allocation can skip unchanged mip conversion. New allocations and inline uploads always convert all subresources. Failed conversion invalidates the fingerprints. This does not skip GPU copies or implement GPU detiling. Extra hashing may cost more than it saves; disabled by default pending correctness and performance testing.
* No wave64 execution change or unified-memory redesign. Existing NVIDIA profile unchanged.

Validation: archive integrity only, source review, and independent scheduling model check. No compilation or FPS measurement. Test mip conversion against full conversion before enabling in gameplay.
