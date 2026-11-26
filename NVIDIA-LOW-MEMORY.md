# NVIDIA RTX 3060 Ti / 16 GB profile

For why this profile budgets separate PC system RAM and VRAM rather than treating them as Xbox-style unified memory, see [Xbox-to-PC memory](docs/xbox-to-pc-memory.md).

For the hardware-specific choices for an Intel Core i5-12400F, RTX 3060 Ti and 16 GB system RAM, see [the Intel/NVIDIA PC target profile](HARDWARE-INTEL12400F-RTX3060TI.md).

The source port preserves the Xbox guest's CPU-visible addresses, page aliases and CPU/GPU write ordering. On a discrete PC, hot shader buffers and textures need local-GPU copies to avoid PCIe reads; tracked writes are merged back to guest memory. This reproduces the guest behavior across PC memory pools, but it cannot make those physical pools unified or remove the cost of copies and synchronization. Optional local buffer/window mirrors now fall back to the ordinary guest shadow if DXGI cannot report the local-memory budget.

This archive contains runtime source, not the game or a ready-to-run Windows binary. These changes do not establish playable FPS or that the entire game fits in 16 GB RAM. NVIDIA shader compatibility and gameplay still need testing on real hardware.

## Install and run

1. Extract the full archive. On Windows x64, install Rust and Visual Studio Build Tools with Desktop development with C++, MSVC x64 and a Windows SDK, as described in README.md.
2. Run Setup.cmd. It downloads the pinned build dependencies and the hash-verified BCPack tables and builds the runtime. Select your existing extracted game's executable. If this ZIP is literally all you have, you cannot launch a game: it contains no game executable or game assets.
3. Open **DexNom RayTime (NVIDIA Low Memory)** from Start. It now selects the NVIDIA performance profile, uses the build-checked 1920x1080 render / output patch and built-in temporal scaling, with ray tracing disabled. No separately supplied FSR provider is needed for this profile.
4. If the native display patch rejects your game build, use **DexNom RayTime (NVIDIA Stock Display)**. That uses the same NVIDIA performance and memory policy without the resolution patch. **NVIDIA Baseline fallback** is available for comparing against the older, smaller set of runtime switches. Do not remove the executable hash checks.

Manual launch after setup:

```powershell
& "$env:LOCALAPPDATA\DexNom Runtime\victory-play.exe" --nvidia-performance --native-1080p
```

Use `--nvidia-performance` without the resolution switch for stock display. The profile also works with `victory-warm.exe` to select the same GPU for shader warmup. Existing saves are retained. Use `--nvidia-low-memory --native-1080p` only to run the baseline fallback.

## What changed

- True native 1080p now updates scanout size/reservation, render scale, display mode, the supported frontend viewport correction, and a separate patched-image cache. The existing 1440p modes remain available. The new mode is tested with synthetic image fields, not with the actual game.
- Changed-byte boundary scans skip equal 64-byte blocks instead of 8-byte words, with immediate first/last-byte checks. Exact byte ownership and synchronization remain unchanged.
- The NVIDIA profile enables up to four existing independent upload-fill workers, limited by available CPU parallelism. An explicit VICTORY_FILL_WORKERS setting is honored. This affects eligible fill batches only, not every submission step.


- NVIDIA selection is explicit for this profile: enumerate high-performance adapters, skip software and non-NVIDIA adapters, then create a D3D12 device. Fail instead of silently selecting integrated graphics. Systems with multiple NVIDIA GPUs use the first capable NVIDIA adapter in DXGI preference order.
- Retired CPU snapshot retention is capped at 256 MiB instead of 512 MiB (raised from 64 MiB: a raw-address dispatch retires hundreds of 4 MiB windows at once, and a 16-window pool forced fresh page commits every frame). This can reduce idle cache retention by up to 256 MiB; it is not a measured reduction in total RAM. Live snapshots and GPU fence ownership are unchanged. Reallocation may cost performance in some workloads.
- Enable the existing sparse-mip and budget-checked GPU-upload input paths. GPU-upload heaps are used only when the device reports support and local-memory headroom is available; otherwise the original system-memory upload fallback remains. This does not force all guest allocations into VRAM.
- Base GPU texture detiling is opt-in through `--nvidia-gpu-detile`; the default NVIDIA profile stays on the CPU path until the D3D12 parity check passes on the target RTX 3060 Ti. Unsupported or ineligible layouts always retain the CPU fallback. Extended layouts, partial GPU detiling, dirty-page GPU jobs, and volume/linear special modes remain separately gated.
- Disable translated ray tracing, enable the existing ambient material fallback, disable speculative upload-ahead, and override inherited FSR/forced-WARP/display settings for this profile. Lighting can differ, particularly indoors.
- The NVIDIA baseline fallback retains VICTORY_JOIN_COPIES, VICTORY_WIDE_REFRESH and VICTORY_TEXTURE_NO_WAIT from the earlier profile. The optimized profile keeps the first two as configured but sets VICTORY_TEXTURE_NO_WAIT=0 so remap-dependent refreshes wait for stable guest memory instead of being deferred. This may increase waits but avoids stale texture revisions.
- The NVIDIA defaults now limit card-backed windows to 256 MiB, lower the texture room threshold to 80%, and enable strict admission. This leaves more adapter budget for render targets and the driver on an 8 GB card. It is still an admission policy, not a hard cap on total VRAM; set any of these variables explicitly before launch to override the default.
- Run the new **DexNom Shader Warmup (NVIDIA) - run first** shortcut once and let it finish before playing; see FPS-FIX-PLAN.md for the remaining, larger fixes.
- Existing launch modes remain available. Conflicting WARP, RT, FSR 4K and upload-ahead flags are rejected for the new profile.

## Why memory cannot simply be moved to the GPU

The runtime executes guest CPU code that accesses guest addresses directly. CPU-visible game memory, uploads and GPU results needed by the CPU still need valid synchronized backing. Replacing these with GPU-only allocations would break those accesses. The existing implementation already uses separate host GPU resources and performs synchronization; this patch preserves that ownership model.

The 2 GiB constant in texture.rs limits a sparse GPU tile pool, not a system-RAM staging cache. Texture staging suballocation already releases surplus idle chunks. No unsafe global VRAM conversion, forced wave size, disabled synchronization or blanket readback removal has been applied.

## Test on your PC

The latest log path is recorded in `%LOCALAPPDATA%\DexNom Runtime\logs\latest.txt`. Check for `selected NVIDIA adapter`, the wave-lane report, and whether local GPU-upload memory is supported. Compare the same save and camera after shader compilation settles. Record frame time, dedicated/shared GPU memory, process RAM, visual errors and any refused shaders/device-removal messages. Share the log if launch fails; source inspection alone cannot identify the failure on your particular NVIDIA system.

Microsoft references used for the adapter/memory implementation:
- https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_6/nf-dxgi1_6-idxgifactory6-enumadapterbygpupreference
- https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-d3d12createdevice
- https://microsoft.github.io/DirectX-Specs/d3d/D3D12GPUUploadHeaps.html

See VALIDATION.md for the checks actually completed in this environment.

See PERFORMANCE-FINDINGS.md for the archive’s bottleneck evidence and the limits of the local CPU microbenchmark. No 30 FPS result has been established.

## NVIDIA profile selection

Setup's **NVIDIA Low Memory** and **NVIDIA Performance** shortcuts use the optimized profile, and the NVIDIA warmup shortcut uses matching worker settings. GPU detiling is an explicit opt-in until target-GPU parity is verified. `--native-1080p` selects the native-resolution patch; omit it to keep the game's stock display mode. **NVIDIA Baseline fallback** retains the smaller switch set for comparison or rollback. Each flag can be overridden by an environment value (including `0`). Extended GPU detiling modes remain separately gated. See PERFORMANCE-STAGE47.md for Xbox-to-PC memory adaptation and remaining target-PC validation.
