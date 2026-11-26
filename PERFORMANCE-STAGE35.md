# Stage 35 — coordinated memory-transfer, texture and residency changes

Base: VIctory_stage34.zip. Target: RTX 3060 Ti (8 GiB VRAM), 16 GiB RAM, native 1080p, 30 FPS. Source-only experimental revision; no verified FPS improvement. All new switches are off by default. Keep synchronization, executable identity checks and original fallbacks.

## Changes and switches

- `VICTORY_EXACT_CARD_UPLOADS=1`: exact byte comparisons against initialized CPU staging for unwatched guest card gaps; skip PCIe uploads for unchanged 64 KiB blocks. Still reads guest memory and compares the bytes, so CPU time can increase. First touch and failed reads use the existing conservative path. Reports examined/skipped/failed candidates (not measured PCIe traffic). Example: after initialization, 64 MiB of eligible data with one changed 64 KiB block can schedule 64 KiB instead of 64 MiB; not a gameplay result.
- `VICTORY_INDEX_WRITE_GROUPS=1`: indexed exact write declaration deduplication, preserving original declaration order and ownership identity. No removal of writeback requirements.
- `VICTORY_TRIM_READBACKS=1`: release oversized cached serial/relay readbacks only after GPU fence and CPU merge completion, under RAM pressure. Does not eliminate readback synchronization.
- `VICTORY_GPU_DETILE=1`, `VICTORY_GPU_DETILE_EXTENDED=1`, `VICTORY_GPU_DETILE_PARTIAL=1`: experimental GPU detile for dynamically resident single-slice mip suffixes with compact output footprints, existing CPU fallback, inline replay support, and resource bounds.
- `VICTORY_GPU_DETILE_DIRTY_PAGES=1` (requires GPU detile): reuse GPU raw inputs only after exclusive staging ownership and actual previous fence completion; BLAKE3 page fingerprints compare the bytes captured in staging; upload only changed adjacent 64 KiB regions, retaining full upload fallback. Hash collisions are theoretically possible. Detile dispatch still processes the selected mip chain. Not yet supported for updating a dynamically partial plan.
- `VICTORY_TEXTURE_PRESSURE_RECLAIM=1`: expand eligible idle GPU staging reclaim under system/VRAM pressure. Existing resource lifetime guards preserved.
- Existing `VICTORY_STRICT_ADMISSION=1`: adds a reserve for estimated card hash storage plus 1 GiB headroom for new card allocations. Does not enforce a total VRAM cap.

## Tests and limits

`python tools/check_stage35_model.py` passed 100,000 unwatched upload model comparisons, 150,000 declaration-order events, 60,000 texture revision transitions and 10,000 footprint plans. Prior Stage 30–34 model scripts passed. These independent Python models do not execute the changed Rust or HLSL. Rust regression tests were added/expanded. Both Cargo check and NVIDIA profile checker could not run because Cargo is absent. Windows, D3D12 and gameplay are untested. No measured FPS, RAM or VRAM result.

On Windows with the pinned Rust toolchain and dependencies:

```
cargo check --locked --target x86_64-pc-windows-msvc -p victory --features diagnostics,blake3/pure --bins --tests
python tools/check_nvidia_profile.py
cargo test --locked -p victory-gpu
```

Use the project warmup launcher; record a warmed same-save/same-camera baseline, then toggle one experimental path at a time. For the partial path enable its dependencies first and test against that baseline. Compare frame mean/median/p95, texture correctness, private bytes and working set, dedicated/shared GPU memory, and copy/readback timings. Turn a switch off if visual or stability regressions appear. Existing `VICTORY_CAPTURE_READBACKS=1` is a separate prior experimental readback optimization; validate separately, including overflow fallback. Keep relay enabled.

## Outstanding critical/high work

- Reduce guest memory comparison/snapshot volume itself, including watched ranges; the exact upload path still scans eligible guest ranges.
- Eliminate only provably redundant GPU readbacks and waits; dependent guest reads and dense/overflow capture still require synchronization.
- Profile remaining draw submission/state translation costs and optimize bottlenecks.
- Profile translated wave64 shaders on NVIDIA; no unconditional wave32 substitution because cross-half operations change semantics.
- Add unsupported layout support and restrict compute detile to proven dirty regions; current changes reduce input transfers, not all compute.
- Validate RAM/VRAM pressure, paging, leak/lifetime and texture parity in real long runs. No hard RAM cap or guarantee of stable 30 FPS.
