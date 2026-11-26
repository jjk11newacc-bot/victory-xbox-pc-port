# FPS fix plan

This plan records work and remaining bottlenecks from source review and
`BENCHMARK.md`. The NVIDIA Low Memory shortcut now selects the optimized
profile described in `PERFORMANCE-STAGE45.md`; the source still needs the
Windows validation and target-PC capture listed below.

## Done in this archive
- NVIDIA optimized profile enables indexed tracking, batching, binding/root
  reuse, bounded streaming recovery, strict memory admission and base GPU
  texture detiling. It sets `VICTORY_TEXTURE_NO_WAIT=0` to avoid deferring
  remap-dependent texture refreshes. Explicit user values still win.
- New Start Menu shortcut "DexNom Shader Warmup (NVIDIA) - run first"
  (victory-warm.exe, same optimized flags). Run it to completion before
  playing so shaders are translated and cached; a cold cache gives seconds
  per frame.
- Already on by default upstream, so not changed: VICTORY_WIDE_BACK and the
  relay (CPU/GPU overlap).
- Not enabled on purpose: VICTORY_KEEP_SNAPSHOTS (uses RAM, turns the relay
  off), VICTORY_CUT_WRITES (splits lists, more submissions).

## Remaining fixes, by expected payoff
1. Game memory shared with the GPU, not copied (victory-gpu/src/inplace.rs,
   shadow.rs, host.rs). Run the existing probe on the PC (VICTORY_INPLACE_PROBE)
   to see what the NVIDIA driver allows; then place big title buffers in
   opened heaps. Highest gain, highest risk of corrupted rendering.
2. Texture detiling (texture.rs, texture/upload.rs): base GPU detiling now
   covers eligible layouts. Dirty-page GPU detiling and additional layouts
   remain gated; measure CPU time and parity before expanding them.
3. Per-draw CPU cost (draw.rs, descriptor/host.rs): binding/root reuse and
   batching are enabled in the optimized profile. A target frame capture is
   needed to see what still dominates among descriptor tables, pipeline
   lookup, translation and submission.
4. Shader cost on 32-wide NVIDIA (victory-shader emit/wave*.rs): wave64
   emulation and wave-op simulation. Needs a GPU profile of the heaviest
   shaders (PIX or Nsight).
5. Ray tracing: compute traversal of the title's BVH is far slower than DXR
   hardware. Profile stays RT-off until a DXR-based path exists.
6. Waits hold a whole list (queue.rs submit_split, list.rs unmet_waits):
   split lists at the wait. Modest gain, moderate risk.

## What to send back
%LOCALAPPDATA%\DexNom Runtime\logs\latest.txt from a settled scene after
warmup. Its per-frame line splits CPU preparation, copies, texture work, GPU
waits, copy-back, dropped draws and RAM, which decides which item to do first.
