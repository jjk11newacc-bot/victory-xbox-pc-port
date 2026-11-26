# Stage 49: Intel/NVIDIA hardware target profile

Target: Intel Core i5-12400F (6 P-cores / 12 threads), GeForce RTX 3060 Ti (8 GB local VRAM), 16 GB system RAM.

## Research findings

- Xbox Series X uses a custom 8-core AMD Zen 2 CPU, an AMD RDNA 2 GPU with 52 CUs, and a shared physical GDDR6 system. The PC instead has an Intel x86-64 CPU and NVIDIA Ampere GPU with separate host RAM and local VRAM budgets.
- The game binary and guest APIs still describe the Xbox environment. The PC compatibility layer selects the host NVIDIA D3D12 adapter and translates guest shaders/resources. Pretending the guest's virtual GPU is NVIDIA would alter guest-visible vendor checks and is not the correct porting fix.
- Host GPU wave widths and optional features are queried at runtime. This matters because cross-lane shader operations and mesh-shader support cannot safely be inferred from the guest's AMD GPU or a generic feature-level number.
- The i5-12400F has 6 P-cores and no E-cores. Existing NVIDIA launcher worker policy uses available host parallelism and, on a 12-thread report, chooses 2 fill workers + 2 gameplay pipeline workers; shader warmup can use up to 4 pipeline workers. This is conservative so CPU-heavy guest execution retains cores.
- 16 GB system RAM and 8 GB VRAM are separate budgets. Windows/D3D12 residency can fluctuate; over-budget allocations can stall or fail. An aggregate “24 GB” assumption is invalid.

## Source changes

- Added `HARDWARE-INTEL12400F-RTX3060TI.md` with CPU/GPU/memory mapping, current launcher choices, bottleneck interpretation, and target-PC measurement steps.
- Linked that target profile from the README and NVIDIA setup guide.
- Documented in `dxgi.rs` that vendor 0x1002 is the guest-facing Xbox AMD adapter identity; the real host NVIDIA device is selected separately in `host.rs`.
- Extended the Stage 44 static integration checker to protect that guest/host separation and the queried NVIDIA wave width.
- No unsafe guest adapter spoof or new vendor-specific shader assumption was added. The hardware profile retains the earlier native 1080p, 8 GB VRAM admission, 16 GB host-RAM pressure, and GPU-detile opt-in choices.

## Validation and remaining work

Source review confirms the NVIDIA launcher sets `VICTORY_NVIDIA`, host adapter enumeration filters for NVIDIA vendor 0x10DE, host wave limits feed shader emission, and mesh/shader model capabilities are queried. These checks do not prove every shader or game frame is correct on the installed driver.

PowerShell setup parsing and source assertions can be run locally. Python/Cargo/MSVC, D3D12 hardware tests, game output and FPS tests are unavailable in this environment. On the target PC, build the normal runtime and run the same warmed scene under the performance and baseline profiles. Use `Setup.cmd -Diagnostics` only for captures. GPU detile remains off until readback parity passes on the 3060 Ti.

## Primary sources

- [Xbox Wire: Series X Zen 2, RDNA 2, 52 CUs and memory regions](https://news.xbox.com/en-us/2020/03/16/xbox-series-x-tech/)
- [Intel: Core i5-12400F specifications](https://www.intel.com/content/www/us/en/products/sku/134587/intel-core-i512400f-processor-18m-cache-up-to-4-40-ghz/specifications.html)
- [NVIDIA: GeForce 30 Series specifications](https://www.nvidia.com/en-us/geforce/graphics-cards/compare/)
- [AMD GPUOpen: RDNA wave32/wave64](https://gpuopen.com/learn/rdna-performance-guide/)
- [Microsoft Learn: D3D12 residency budgets](https://learn.microsoft.com/en-us/windows/win32/direct3d12/residency)
- [Microsoft Learn: D3D12 adapter architecture properties](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ns-d3d12-feature-data-architecture)
- [Microsoft Learn: D3D12 mesh shader capability query](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ns-d3d12-d3d12_feature_data_d3d12_options7)
- [NVIDIA CUDA Programming Guide: 32-thread warps](https://docs.nvidia.com/cuda/cuda-programming-guide/01-introduction/programming-model.html)
