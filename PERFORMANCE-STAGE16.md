Stage 16 (uncompiled experimental source)

- Large multi-subresource textures can use two CPU workers for detiling into disjoint staging ranges. The extra worker is globally limited to one, with no second texture-sized buffer. Footprint bounds and overlap are checked before parallel dispatch. Existing serial path is retained. NVIDIA profile enables VICTORY_PARALLEL_DETILE=1; set it to 0 to revert. Thread creation and job sorting can outweigh the gain; a single dominant mip limits speedup. This is CPU parallelism, not GPU compute detiling.
- NVIDIA launcher now defaults to native 1080p when no explicit 1440p mode is requested. Added assertion to existing launcher test.
- No Rust compiler, Windows D3D12 device or game available; no compilation, runtime tests or FPS measurements performed. GPU detiling, full memory redesign, wave64 execution specialization remain outstanding.
