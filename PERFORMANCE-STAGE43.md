# Stage 43 — cross-wave exchange, readback dependencies, and render-XOR texture coverage

Base: VIctory_stage42.zip. Target: RTX 3060 Ti 8 GB VRAM / 16 GB system RAM / native 1080p / ray tracing off / 30 FPS. None of the FPS or memory targets has been measured here. This is a source candidate, not a compiled or tested release.

## 1. NVIDIA cross-wave execution

`crates/victory-shader/src/emit/lower.rs`: introduced a paired LDS exchange for two wave-uniform dwords. With `VICTORY_WAVE_PAIR_EXCHANGE=1`, each console wave uses four dwords instead of two in the shared exchange area. Both values are written, one group barrier publishes them, both results are read, and one group barrier protects the slots from reuse. Used by f64 sums and guarded first-lane reads. The two independent exchanges formerly used four barriers; the paired operation uses two. This is an operation-count result, not a measured GPU speedup. The old single-dword exchange remains available; no barrier needed for cross-wave ordering is removed. Cache key includes the switch.

Correctness fix: for groups composed of full host waves that end with only one half of a console wave (96, 160, etc. threads), avoid reading an unwritten upper LDS slot. Return zero for the absent ballot/data half but retain the original local reduction for min/max/AND/f64 so zero is not incorrectly treated as an identity. f64 reductions in single-wave groups or local-wave mode avoid unnecessary exchange and addition of a synthetic zero. This does not solve arbitrary partially populated host waves or divergent cross-wave exchange. The existing approximate local-wave fallback is unchanged by default; `VICTORY_WAVE_LOCAL=idiom` is the stricter diagnostic option (can refuse shaders). Do not turn on unsafe wide-barrier retries indiscriminately.

## 2. Dependent GPU readbacks

`crates/victory-gpu/src/host.rs`: track ordinary pitched copy-backs and bottom-of-pipe writes as well as texture copy-backs in submission dependencies. Previously a subsequent CPU preparation could miss a pending write when the interval was opaque, and splitting dependent lists within a submission considered only tiled texture destinations. Opaque intervals now require a conservative wait; tracked intervals retain existing alias expansion. The owned-present-list fast path cannot discard ordinary readbacks or completion callbacks. These are correctness changes and can add stalls that were previously incorrectly bypassed.

Optional `VICTORY_EXACT_READBACK_ROWS=1` tracks the actual written rows for pitched readbacks of up to 64 rows instead of the enclosing range that includes unused padding. This lets an independent CPU preparation proceed when it touches padding only. Larger copies retain the conservative bound. All existing relay publication order and fences remain. Real CPU reads of shader results must still wait for those results. No claim of eliminating dependent readback latency.

## 3. Xbox texture layout coverage

`crates/victory-formats/src/texture.rs`: added Series X and S SW_64KB_R_X patterns for single-mip, single-layer, single-sample 2D surfaces with 1 or 16 bytes per element. Existing patterns covered only 2/4/8 bytes. `VICTORY_RENDER_XOR_EXTRA=1` enables the new formats for CPU tile/detile and, with the earlier GPU detile options, GPU conversion. The new patterns use the same address oracle as the existing CPU implementation; no new proprietary data. The source's existing AMD MIT license notice applies.

Provenance: AMD GFX10 addrlib's public pattern header as mirrored by ChromiumOS Mesa: https://chromium.googlesource.com/chromiumos/third_party/mesa/+/refs/heads/stabilize-13982.70.B-chromeos-amd/src/amd/addrlib/src/gfx10/gfx10SwizzlePattern.h . PATINFO rows for 32 pipes/8 packers and 8 pipes/4 packers, 1xAA, SW_64K_R_X. Series X: nibble indices (0,307,672) and (7,307,676); Series S: (0,295,219) and (7,299,378). The fourth nibble is zero. Convert each output-bit expression to XOR contributions of X and Y for z=0 and no base pipe XOR. The model script records the transcribed bit equations separately and verifies the Rust tables against them. This confirms table transcription and bijection, not hardware address parity; the external Addrlib executable was not available.

`crates/victory-gpu/src/texture/gpu_detile.rs`: new actual D3D12 readback parity fixtures for 1- and 16-byte render-XOR textures when the switch is enabled. `victory-formats` has a new Rust tile/offset/detile roundtrip test for both console models. Other XOR modes, multi-mip and array render-XOR, BC render-XOR, MSAA and unknown base pipe XOR are still not supported by this change. Do not invent an address equation for them.

## Validation

`STAGE43-MODEL-RESULTS.txt`: the Python models check 2,048,408 address/uniqueness cases against transcribed output-bit equations, 20,000 paired LDS layouts, and 30,000 exact pitched-row dependency cases. Stage 42 models reran successfully. These are independent model checks, not execution of Rust, DXIL or HLSL.

`STAGE43-BUILD-RESULTS.txt`: Cargo and PowerShell unavailable. The pinned Windows check failed with exit 127, and the NVIDIA profile checker failed with missing Cargo. No compiled shader, WARP or GPU runtime parity was executed. `tools/Validate-Stage43.ps1` adds format, shader and GPU suite runs with the options enabled; it is unexecuted and its shader suite can skip some hardware cases. Use isolated warm A/B runs before enabling options in the launcher. The profile and earlier defaults are otherwise unchanged. No FPS, VRAM or private/working-set measurement.

For actual GPU detile use VICTORY_GPU_DETILE=1, VICTORY_GPU_DETILE_EXTENDED=1, VICTORY_GPU_DETILE_SMALL_ELEMENTS=1 with VICTORY_RENDER_XOR_EXTRA=1. Test each experiment independently before combining. New options default OFF. Required readback dependency and unmatched-full-host-wave correctness fixes apply directly. If visual artifacts, shader failures or GPU device removal occur, revert to Stage 42 or isolate the options; there is no verified release yet.
