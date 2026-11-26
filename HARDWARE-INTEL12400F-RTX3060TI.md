# Xbox game runtime on Intel i5-12400F + RTX 3060 Ti

This profile describes the PC host target for the compatibility runtime in this repository. It does not rebuild or retarget the original game executable. That would require the game's source code, assets, build system, and platform SDK. Here, the runtime preserves the Xbox guest interface and translates its CPU/GPU and memory behavior for Windows and D3D12.

## Hardware differences that affect this port

| Component | Xbox Series X target | Your PC host | Port consequence |
| --- | --- | --- | --- |
| CPU | Custom 8-core AMD Zen 2 CPU | Intel Core i5-12400F: 6 Performance cores, 12 threads, up to 4.4 GHz | Both execute x86-64 code, but this PC has two fewer physical cores. Translation, title threads, driver work, and texture preparation compete for six cores. More worker threads are not automatically faster. |
| GPU | Custom AMD RDNA 2, 52 CUs | NVIDIA Ampere RTX 3060 Ti, 8 GB GDDR6/GDDR6X depending on card | Xbox GPU instructions and wave operations cannot be assumed to run natively on NVIDIA. The host uses translated shaders and must query D3D12 wave, shader-model, mesh-shader, heap, and budget capabilities. |
| Memory | 16 GB shared GDDR6 physical pool with separate bandwidth regions and a console title budget | 16 GB system RAM plus a separate 8 GB local VRAM pool | These capacities cannot be added as if they formed one 24 GB pool. Guest pages, CPU shadows, upload staging, and local GPU resources may coexist; host RAM and local VRAM require separate budgets and streaming. |
| Graphics API | Xbox console graphics/runtime contracts | Windows D3D12 and NVIDIA driver | Resource residency, upload/readback timelines, fences, texture layout, shader compilation, and driver behavior need host-side translation and validation. |

Intel's 12400F supports 6 P-cores and 12 threads; it has no E-cores. The present performance launcher derives bounded worker counts from Windows' available parallelism. On a 12-thread report, gameplay defaults to two upload-fill workers and two pipeline workers; shader warmup can use up to four pipeline workers. Environment overrides are honored. This leaves CPU capacity for the runtime/game and avoids treating all 12 logical threads as spare workers.

The loader also emulates Xbox Zen 2 instructions missing on Intel, including AMD SSE4a/CLZERO-family operations used by the guest. The full-source audit fixed an unsafe instruction-byte read in that exception path; bytes are now copied only from queried readable regions and decoded with bounds-checked slices.

The NVIDIA profile selects an NVIDIA adapter for the **host D3D12 device**, queries its wave-lane range, and checks host features instead of pretending the host is an Xbox GPU. The virtual DXGI adapter intentionally continues to report the guest Xbox's AMD identity. That is a compatibility identity presented to the original title; changing it to NVIDIA would make guest vendor checks observe the wrong console. Host selection and guest identity are separate layers.

## Current host settings in this source

- Use **NVIDIA Performance / Low Memory** at native 1080p, with translated ray tracing off. This lowers GPU and memory pressure; indoor lighting can differ from the RT-enabled Xbox output.
- Keep local VRAM and host RAM accounting separate. The profile caps optional card-backed windows at 256 MiB, admits textures below the existing 80% budget threshold, and enables strict admission. These are headroom policies, not a hard cap on all driver/game allocations.
- The PC RAM-pressure policy starts trimming reusable caches below 6 GiB available RAM and uses critical reclamation below 3 GiB. It does not cap guest live memory, guarantee no paging, or ensure a 16 GB fit. If the game needs more live memory than the PC can supply, cache tuning cannot solve that limit.
- Shader model 6.6 and mesh-shader availability are queried on the host. The RTX 3060 Ti generation is expected to support the needed class of features, but the installed driver/runtime is what the program must confirm. Check the startup log for selected adapter, wave lanes, mesh support, and GPU-upload memory support.
- GPU texture detiling remains opt-in until the exact converted layouts pass readback parity on your card. The CPU fallback prioritizes known output; it can cost CPU time and delay texture uploads.
- The Windows install/build should use the NVIDIA driver and a current D3D12-capable Windows SDK. There is no requirement to install an AMD GPU driver for this PC path.

## What needs measurement on this exact machine

Run a warmed scene at the same save, camera, and resolution for the performance profile and baseline fallback. Use a diagnostics build only while collecting counters. Record median and p95 frame time, CPU preparation/guest-memory refresh, texture upload time and resident mip levels, GPU waits/copy-back, process private bytes, available system RAM, local VRAM budget/use, shared GPU memory, and device-removal/shader-refusal messages.

Interpret the bottleneck before changing settings:

- High CPU preparation with spare GPU time points to translation, draw submission, or guest-memory scanning; lowering resolution will have little effect.
- Local VRAM near budget or shared GPU memory growth points to residency/streaming pressure; do not respond by increasing the VRAM window blindly.
- Available system RAM near the critical threshold with paging or long stalls points to host-memory pressure; reducing caches may help, but may add shader/snapshot rebuild stutter.
- Texture artifacts with normal frame pacing need shader/layout and upload parity investigation, not more aggressive detiling.

No source edit can establish your real FPS or texture correctness without the game, driver, and target GPU. The Xbox-to-PC memory model and test method are in [docs/xbox-to-pc-memory.md](docs/xbox-to-pc-memory.md).

## Primary references

- [Xbox Wire: Xbox Series X Zen 2 CPU, RDNA 2 GPU, and memory specifications](https://news.xbox.com/en-us/2020/03/16/xbox-series-x-tech/)
- [Intel: Core i5-12400F specifications](https://www.intel.com/content/www/us/en/products/sku/134587/intel-core-i512400f-processor-18m-cache-up-to-4-40-ghz/specifications.html)
- [NVIDIA: GeForce RTX 30 Series comparison](https://www.nvidia.com/en-us/geforce/graphics-cards/compare/)
- [AMD GPUOpen: RDNA wave32 and wave64 performance notes](https://gpuopen.com/learn/rdna-performance-guide/)
- [NVIDIA CUDA Programming Guide: 32-thread warps](https://docs.nvidia.com/cuda/cuda-programming-guide/01-introduction/programming-model.html)
- [Microsoft Learn: D3D12 residency budgets and discrete memory pools](https://learn.microsoft.com/en-us/windows/win32/direct3d12/residency)
- [Microsoft Learn: D3D12 architecture and UMA properties](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ns-d3d12-feature-data-architecture)
- [Microsoft Learn: D3D12 mesh shader support query](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ns-d3d12-feature-data-d3d12-options7)
