# Stage 31 — GPU compare coalescing and compact texture staging

Base: VIctory_stage30.zip. Three Rust source files changed. Existing launcher,
1080p output, RT settings, guest writeback ownership, fences, executable checks,
and assets policy are unchanged. New experiments default OFF.

## 1. VICTORY_SHARED_COMPARE_RUNS=1

A shader write declaration is clipped to its actual byte range, but CARD_COMPARE
hashes complete 64 KiB pieces. Disjoint declarations in the same piece used to
repeat hashing, consume multiple result dwords, and potentially repeat capture
planning. The new planner joins only overlapping/adjacent piece intervals per
shadow, dispatches each piece once, and maps each original byte declaration onto
the appropriate shared result offsets. Publication and ownership still use
original byte ranges, not the joined envelope. No change to HLSL hash or fence
logic. If the shared plan cannot fit the existing result arena or exceeds the
D3D12 group count limit, fall back to the original per-range plan. Log reports
logical duplicate 64 KiB coverage avoided, not measured PCIe traffic.

## 2. VICTORY_COMPACT_MIP_UPLOAD=1

Partly mapped 2D textures previously allocated and zeroed a full-size upload
lease even if only the small/coarse mips were committed. For the existing
eligible partial-mip path, allocate just the suffix of the pitched footprints
needed by the selected mips. Validate alignment and padded bounds, and rebase
copy offsets and CPU detiling destinations. A finer/full revision must not
reuse a smaller lease. Inline full-size fallback restores zeroed prefix and
copies the suffix; the existing in-flight Arc lifetime is unchanged. This
reduces requested staging allocation and zeroing, not the texture's GPU
allocation or the original guest allocation. Pool granularity can reduce the
actual memory saving. Logs report requested allocation avoided, not RAM usage.

## Validation and next steps

- Independent Python model: 20,000 shared comparison coverage cases and 10,000
  compact footprint cases passed. These are arithmetic/coverage models, not
  Rust execution or GPU validation. Run `python tools/check_stage31_model.py`.
- Rust tests added for compare layout/arena capacity and suffix alignment and
  bounds. Cargo unavailable: Windows cargo check and the profile checker both
  attempted but blocked. No gameplay, GPU, compile or visual results.
- A/B each flag individually, with the same warmed scene and camera; compare
  latest.txt, GPU timing, working set, private bytes, texture arrival and image
  correctness. Check dirty bytes on overlapping-piece disjoint declarations,
  captured and fallback readbacks, and partially resident textures transitioning
  to full mip chains. Disable flag (0) to roll back. Do not promote to defaults
  until real hardware parity. Shared compare can add CPU planning cost; compact
  staging can reduce reuse and require a full allocation on promotion.
- GPU compute detile, conservative write declaration audit, wave64 optimization,
  and actual frame-time/RAM verification remain unresolved. No claim of 30 FPS.
