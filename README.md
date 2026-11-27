# DexNom RayTime

DexNom RayTime is a Windows x64 fork of Marvisak’s VIctory. It translates Xbox Series GPU work to Direct3D 12 for a game installation you supply. Executables, game assets, saves, and generated caches stay on your machine.

## Run it on your PC

1. Use 64-bit Windows and a current Direct3D 12 driver. The game uses mesh shaders and Shader Model 6.6.
2. Install Rust for Windows and Visual Studio Build Tools. In the Visual Studio installer, select **Desktop development with C++**, including the x64 MSVC toolset and a Windows 10 or 11 SDK. [Rust installer](https://rustup.rs/) · [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
3. Download this repository using **Code → Download ZIP**, extract the ZIP to a writable folder, and double-click **Setup.cmd**. Select the Series X game executable inside your own extracted game installation. Keep its accompanying files together. Setup downloads the pinned build dependencies, verifies the BCPack format tables, and compiles the three Windows programs locally without diagnostic instrumentation. Use `Setup.cmd -Diagnostics` when collecting detailed runtime counters. [Download source ZIP](https://github.com/jjk11newacc-bot/victory-xbox-pc-port/archive/refs/heads/main.zip)
4. Open **DexNom RayTime** from the Windows Start menu. Use a controller for the game's first-time setup. The launcher and your local data are in `%LOCALAPPDATA%\DexNom Runtime`. Saves are in `state`, shader caches in `cache`, and per-run logs in `logs`. The file `logs\latest.txt` points to the latest game log.

To update, close the runtime, extract the new source ZIP, and run **Setup.cmd** again. It reuses the saved game path and the same save directory. To select a different installation, pass its executable path to Setup from PowerShell:

```powershell
.\Setup.cmd -GamePath "D:\My Games\Series X\game.exe"
```

To warm the shader cache, open `victory-warm.exe` in the installed runtime directory. The log lists translated and refused shaders. Graphics pipelines also depend on draw state encountered during play. If you see a missing `VCRUNTIME140.dll` error, install Microsoft's x64 redistributable. [Microsoft Visual C++ Redistributable](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)

For a manual performance build, run `.\Initialize-FormatTables.ps1`, then `cargo build --locked --release -p victory --bins`. Add `--features diagnostics` when you need detailed counters. The release binaries are in `target\release`. Place all three together outside the checkout and pass the game executable path to `victory-play.exe`. Custom data and cache directories are documented in the configuration example. [Configuration example](victory.example.toml)

## RTX 3060 Ti launch profile

The stock Series X display path has been tested on an Intel Core i5-12400F with 16 GB RAM and an RTX 3060 Ti. The intro opens, but some world pipelines still fail and the 3D scene can render incorrectly.

After setup, this command uses the NVIDIA low-memory profile, disables ray tracing, and keeps a 1 GiB memory guard active.

```powershell
$env:VICTORY_MEMORY_GUARD = '1'
$env:VICTORY_MEMORY_GUARD_MIB = '1024'
& "$env:LOCALAPPDATA\DexNom Runtime\victory-play.exe" --nvidia-low-memory --no-rt --console series-x
```

See the [loading and memory notes](LOADING_MEMORY_NOTE.md), [Xbox-to-PC memory design](docs/xbox-to-pc-memory.md), and [diagnostic logs](diagnostic-logs/README.md).

## Credits

- **Marvisak and the VIctory contributors** created the original Windows compatibility runtime that this fork builds on. [VIctory](https://github.com/Marvisak/VIctory)
- **ZSlayerHQ and the victory-world contributors**, including **triippywhale** and **asdaasdsaddf**, supplied the Radeon rendering work used as the basis for this fork. [victory-world-runtime](https://github.com/ZSlayerHQ/victory-world-runtime) · [Integrated revision](https://github.com/ZSlayerHQ/victory-world-runtime/commit/99f8834aa16b98d7f3943b8de262a8d4857bda7b)
- **The XCL contributors** published an independent Xbox Series compatibility layer that informed our work on shader translation and CPU/GPU memory synchronization. [XCL](https://github.com/xcl-dev/xcl)
- **AMD** supplies the FSR 4.1.2 upscaler used by the Quality 4K profile. [AMD FSR SDK](https://gpuopen.com/amd-fsr-sdk/)



## Licensing

VIctory source is licensed under GPL-3.0-or-later. The upstream project identifies the required BCPack format tables as separately supplied data outside that license. This source archive omits that file. The setup script verifies a local or upstream copy before use. [License](LICENSE) · [Upstream asset policy and table notice](docs/asset-policy.md)
