# Stage 2: correctness-preserving performance patch
Base: VIctory_minee_fixedddddd.zip. Preserves release compiler options and 256 MiB normal low-memory snapshot cache.

Implemented: retired snapshot pool trimmed on existing sweep below 4 GiB available physical RAM (64 MiB retained), below 2 GiB (zero retained); live allocations are not evicted. Texture admission checks adapter budget even with an explicit texture cap; NVIDIA profile requests 85% adapter budget. Texture write-watch path hashes complete read data to avoid redundant detiling on identical writes, and hash hits require valid staged data. Hashing adds CPU cost when writes are genuinely different; benchmark before adopting as a final optimization.

Remaining work and validation:
1. Guest comparisons: profile per-frame CPU compare bytes, GPU hash bytes, readback waits. Existing card hash path handles eligible buffers; do not skip GPU-written ranges without exact tracking.
2. Unified memory: run existing VICTORY_INPLACE_PROBE=1 on target, validate heap opening and coherence before enabling VICTORY_INPLACE. Discrete GPU still reads system memory across PCIe.
3. Detiling: current patch skips identical revisions only. Implement GPU compute detile with format/mip/slice parity tests against Layout::detile; preserve sparse mip and fence lifetime behavior. Dirty-page updates require tracking source-page to destination runs and immutable in-flight staging.
4. Synchronization: profile wait categories; preserve relay and exact ordering. Do not disable fences.
5. Allocation: measure pool reuse, private bytes and working set; current trim is reactive, not a hard cap.
6. Wave64: profile shaders on NVIDIA; only lower proven lane-local operations to wave32, preserve cross-half semantics and cache keys.
7. Draws: profile descriptor preparation and pipeline lookup; batch only equivalent state without reordering dependent draws.
8. Streaming: compare cold/warm scenes, verify mip completeness, texture retry and queue backlog. No new copy queue enabled.
9. RAM: collect available physical RAM, private bytes, working set and VRAM usage; target 10–11 GiB process working set as a provisional goal, not an enforced limit. A hard cap without accounting for live guest allocations can crash the game.

Validation: source edits and archive integrity only. Rust compiler, Windows, game assets and target GPU unavailable. No measured FPS or texture correctness guarantee. Existing benchmark is not a baseline for this exact NVIDIA profile.
