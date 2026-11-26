> **Intel Core i5-12400F / RTX 3060 Ti / 16 GB PC profile:** See [the hardware-specific target notes](HARDWARE-INTEL12400F-RTX3060TI.md), [NVIDIA-LOW-MEMORY.md](NVIDIA-LOW-MEMORY.md), [Xbox-to-PC memory notes](docs/xbox-to-pc-memory.md). The RTX gameplay path still needs validation on the target PC.

# DexNom RayTime (AMD)

DexNom RayTime is a community fork of Marvisak's VIctory for Windows x64. It runs an Xbox Series executable from your own game installation and translates its GPU work for Direct3D 12. This is our most complete tested build so far and the source for the FSR Quality 4K baseline. Game files, executables, saves and generated shader caches stay on your own computer.

AMD Radeon is the primary target and the best-tested GPU path for this build. See [NVIDIA RTX PC profile](#nvidia-rtx-pc-profile) for the RTX 3060 Ti launch settings. The current performance priorities are reducing VRAM use and improving FPS and frame-time stability.

The Radeon FSR Quality 4K build is our most stable tested baseline for shaders and texture streaming. See [Things to Be Done](#things-to-be-done) for open work and how to measure a contribution.

## Tested PC

| Component | Test system |
| --- | --- |
| GPU | AMD Radeon RX 7900 XTX, 24 GB GDDR6 |
| CPU | Intel Core Ultra 9 285K, 24 cores |
| System memory | 64 GB DDR5 |
| OS | Windows 11 Pro x64, build 26200 |
| Render profile | Series X, ray tracing on, 2560×1440 render, FSR 4.1.2 Quality, 3840×2160 output |

[AMD Radeon RX 7900 XTX specifications](https://www.amd.com/en/products/graphics/desktops/radeon/7000-series/amd-radeon-rx-7900xtx.html)

## Run it on your PC

1. Use 64-bit Windows and a current Direct3D 12 driver. The game uses mesh shaders and Shader Model 6.6. The Radeon gameplay results and known issues are listed below.
2. Install Rust for Windows and Visual Studio Build Tools. In the Visual Studio installer, select **Desktop development with C++**, including the x64 MSVC toolset and a Windows 10 or 11 SDK. [Rust installer](https://rustup.rs/) · [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
3. Download this repository using **Code → Download ZIP**, extract the ZIP to a writable folder, and double-click **Setup.cmd**. Select the Series X game executable inside your own extracted game installation. Keep its accompanying files together. Setup downloads the pinned build dependencies, verifies the BCPack format tables, and compiles the three Windows programs locally without diagnostic instrumentation. Use `Setup.cmd -Diagnostics` when collecting detailed runtime counters. [Download source ZIP](https://github.com/DexNom/VIctory/archive/refs/heads/main.zip)
4. Open **DexNom RayTime** from the Windows Start menu. Use a controller for the game's first-time setup. The launcher and your local data are in `%LOCALAPPDATA%\DexNom Runtime`. Saves are in `state`, shader caches in `cache`, and per-run logs in `logs`. The file `logs\latest.txt` points to the latest game log.

To update, close the runtime, extract the new source ZIP, and run **Setup.cmd** again. It reuses the saved game path and the same save directory. To select a different installation, pass its executable path to Setup from PowerShell:

```powershell
.\Setup.cmd -GamePath "D:\My Games\Series X\game.exe"
```

To warm the shader cache, open `victory-warm.exe` in the installed runtime directory. The log lists translated and refused shaders. Graphics pipelines also depend on draw state encountered during play. If you see a missing `VCRUNTIME140.dll` error, install Microsoft's x64 redistributable. [Microsoft Visual C++ Redistributable](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)

For a manual performance build, run `.\Initialize-FormatTables.ps1`, then `cargo build --locked --release -p victory --bins`. Add `--features diagnostics` when you need detailed counters. The release binaries are in `target\release`. Place all three together outside the checkout and pass the game executable path to `victory-play.exe`. Custom data and cache directories are documented in the configuration example. [Configuration example](victory.example.toml)

## FSR Quality 4K baseline

Open **DexNom RayTime (FSR Quality 4K)** to use the tested Series X profile. It renders at 2560×1440 and presents at 3840×2160 with ray tracing enabled. FSR 4.1.2 takes the place of the game's TAA upscaler in this profile.

The baseline was tested with a PC FidelityFX 4.1.2 provider. Put the provider and its dependencies in the installed runtime's `fidelityfx` subfolder or set `VICTORY_FIDELITYFX_DIR` to the folder containing `amd_fidelityfx_upscaler_dx12.dll`. The provider is supplied separately.

For a manual launch, use the same flags as the Quality shortcut.

```powershell
& "$env:LOCALAPPDATA\DexNom Runtime\victory-play.exe" --rt --console series-x --local-gpu-memory --fsr-quality --sparse-textures --no-upload-ahead --capture-readbacks --rt-write-stats
```

The **DexNom RayTime (Experimental RT)** shortcut enables ray tracing with the game's display path. The shortcuts share your local saves.

Chapter 1 City produced in-range ray and triangle intersections on an RX 7900 XTX. The game builds and traverses its acceleration structures through translated Direct3D 12 compute shaders. Frame times can still reach several seconds. Interior lighting, shadows and performance vary by scene. [Ray-tracing implementation](docs/fork-ray-tracing.md) · [City testing](BENCHMARK.md)

## NVIDIA RTX PC profile

NVIDIA gameplay is untested. Start with ray tracing off and the game's display path. Replace `--rt` with `--no-rt` and leave out `--fsr-quality`, `--fsr-balanced`, and `--fsr-performance`. After setup, run the following in PowerShell.

```powershell
$env:VICTORY_FSR_4K = '0'
$env:VICTORY_AMBIENT_FALLBACK = '1'
& "$env:LOCALAPPDATA\DexNom Runtime\victory-play.exe" --no-rt --console series-x
```

`--no-rt` skips the ray-tracing builder, traversal, and lighting resolve. Supported material shaders use a lightweight diffuse ambient fill based on their base color. Some interiors can still look darker. [RT pass selection](crates/victory-gpu/src/pipeline/ray_tracing.rs) · [Ambient fill](crates/victory-shader/src/emit/mod.rs)

For a separate partial RT experiment, replace `--no-rt` with `--rt-static-bvh` and keep the FSR preset off. This skips rebuilding the acceleration structure for dynamic geometry and still traces rays through the prebuilt world structure. Ray traversal and lighting resolve can still be expensive, and moving objects may have incomplete shadows or reflections. Baked lighting or a probe-based indirect-light fallback is future work. [Static BVH mode](crates/victory-gpu/src/pipeline/ray_tracing.rs)

Keep the scene and resolution the same when comparing the two modes. Include the GPU actually used, its driver, frame times, and any lighting issues. The file `logs\latest.txt` points to the run log, which records the requested ray-tracing mode.

## Things to Be Done

A fresh public download reached the City waterfront on an RX 7900 XTX with FSR 4.1.2 and ray tracing enabled. The first in-game frames took longer while pipelines compiled. Warm samples reached about 1.1–1.3 FPS with zero dropped draws. Some materials still rendered black.

- **Reduce buffer copy-back.** Track which pages the GPU actually writes and read back the ranges needed by CPU consumers. Start with the ray-tracing header builder, which declares much broader write ranges than it uses. Check fence ordering, CPU writes, aliases, and remaps when narrowing synchronization. [Submission and readback](crates/victory-gpu/src/host.rs) · [Buffer snapshots](crates/victory-gpu/src/shadow.rs) · [Guest memory directory](crates/victory-gpu/src/memory.rs)
- **Reduce VRAM use and stabilize texture streaming.** Measure local VRAM and GPU-shared system memory separately. Look for duplicate resident resources, oversized staging allocations, and repeated mip uploads. Keep a valid resident mip available until its replacement has finished uploading. For black materials, trace the draw's base color back to the bound texture and mip. [Texture residency](crates/victory-gpu/src/texture.rs) · [Texture descriptors](crates/victory-gpu/src/descriptor/host.rs)
- **Improve frame-time stability.** Measure cold pipeline compilation and warm gameplay separately. Look for CPU waits on buffer publication, repeated uploads, and pipeline work that can be prepared before the first draw. Compare pipeline counts and the rendered scene to check for regressions. [Shader translation](crates/victory-shader/src/emit) · [Pipeline creation](crates/victory-gpu/src/pipeline.rs)

For a before-and-after test, use the same scene, camera, and driver with ray tracing on and FSR Quality at 1440p → 4K. Report median and 95th-percentile frame times, dedicated and shared GPU memory, upload and copy-back time and bytes, and dropped draws or pending pipelines. Include the commit, hardware, and relevant log lines. [Benchmark notes](BENCHMARK.md)

## Changes in this snapshot

- Pixel shaders that read neighboring pixels in a 2×2 group now preserve the values at triangle edges. On an RX 7900 XTX, this reduced incorrect edge pixels from 32 to zero in each of two adjoining-triangle tests. Cube-map sampling also selects the correct mip level for explicit gradients and honors the shader's minimum mip level.
- The ray-tracing memory directory can grow beyond 1,023 backing buffers. Each recorded draw or dispatch uses only the views available when it was recorded. Older command lists keep their original views.
- Streamed textures can upload when only their middle pages are resident. The runtime merges overlapping memory notifications and rescans tracked textures if the notification queue fills. Previously used depth-only pipelines now warm ahead of speculative material pipelines to reduce missing shadows during loading.
- Some dock objects still appear black in the City save. The same distant dock fixtures appeared dark with the ray-structure builder disabled. The effect of these changes on those objects and on gameplay FPS is still being tested.
- GPU readback identifies changed pages and combines adjacent copy ranges. This targets CPU/GPU synchronization and transfer overhead.
- Ray-structure builder dispatches now honor the game's thread counts and mask excess lanes in the final group. Read-only traversal no longer declares the entire guest memory directory writable when the shader sends its output to another buffer or texture.
- Texture handling searches for contiguous descriptor slots and admits already-resident textures under memory pressure. Earlier changes address publication order, placed-texture ownership and supported render-XOR layouts.
- Shaders can choose between two static samplers across a uniform branch. This recovered 40 shader variants from the tested archive. Another fix recovered one voxel visibility shader by recognizing a saved and restored wave64 lane mask. Mixed render targets retain their individual write masks when an integer target bypasses blending.
- The runtime preserves the game's vertex output interpolation and fixes output signature ordering. All 2,191 archive vertex shaders passed the Microsoft DXIL validator, including 18 water shaders. New textures and retries share the upload budget, and a previously refused buffer view is checked again when its backing allocation changes.
- The runtime resolves guest addresses from ray-tracing descriptors, preserves raw buffer offsets and prepares the memory each command list reads. Texture uploads can proceed while Windows remaps unrelated memory. In-game ray-traced lighting is still experimental.
- Five additional shadow compute shaders compile and create pipelines on the RX 7900 XTX. The translated ray-node regression also exercises three reflections through the guest-page directory. Changes confined to the standalone execution tests now preserve cached shader translations.
- Shader translation preserves the guest wave width for lane permutations and handles heap-indexed texture-size queries. Save handling flushes copied blobs before publication and can recover a complete checkpoint.

The source ZIP builds with the Windows setup script. The test report covers shader validation, a Radeon pipeline check, gameplay frame times and the remaining interior lighting issue. [Test and benchmark report](BENCHMARK.md)

### Testing CPU submission workers

For a local experiment, open PowerShell and run the following after setup. The process uses four CPU workers for eligible upload batches and can record independent transition command lists on the shared worker pool. The final GPU queue, fence signals and resource transitions retain the game's submission order. Compare the same save and camera after the initial streaming period. The default launcher uses the baseline settings.

```powershell
$env:VICTORY_FILL_WORKERS = '4'
$env:VICTORY_PARALLEL_PRELUDES = '1'
& "$env:LOCALAPPDATA\DexNom Runtime\victory-play.exe"
```

## Credits

- **Marvisak and the VIctory contributors** created the original Windows compatibility runtime that this fork builds on. [VIctory](https://github.com/Marvisak/VIctory)
- **ZSlayerHQ and the victory-world contributors**, including **triippywhale** and **asdaasdsaddf**, supplied the Radeon rendering work used as the basis for this fork. [victory-world-runtime](https://github.com/ZSlayerHQ/victory-world-runtime) · [Integrated revision](https://github.com/ZSlayerHQ/victory-world-runtime/commit/99f8834aa16b98d7f3943b8de262a8d4857bda7b)
- **The XCL contributors** published an independent Xbox Series compatibility layer that informed our work on shader translation and CPU/GPU memory synchronization. [XCL](https://github.com/xcl-dev/xcl)
- **AMD** supplies the FSR 4.1.2 upscaler used by the Quality 4K profile. [AMD FSR SDK](https://gpuopen.com/amd-fsr-sdk/)



## Licensing

VIctory source is licensed under GPL-3.0-or-later. The upstream project identifies the required BCPack format tables as separately supplied data outside that license. This source archive omits that file. The setup script verifies a local or upstream copy before use. [License](LICENSE) · [Upstream asset policy and table notice](docs/asset-policy.md)





