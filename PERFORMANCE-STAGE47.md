# Stage 47: Xbox guest memory on discrete PC

This stage corrects the focus of the Stage 46 pass. The task is to preserve the Xbox guest's memory behavior while translating it to a PC with separate system RAM and local VRAM, not to pretend a PC has Xbox unified memory.

## Architecture audit and source change

- Xbox Series X has 16 GB GDDR6 in a shared physical system, with 10 GB at 560 GB/s and 6 GB at 336 GB/s. A discrete RTX PC has separate CPU system RAM and GPU-local VRAM. D3D12 exposes distinct local/non-local budgets which may change while the game runs.
- The runtime already preserves guest CPU-visible addresses/page mappings, makes host shadows for shader-visible title buffers, refreshes them from guest memory, and merges declared GPU writes back in order. It translates Xbox texture/tile contents into D3D12 texture residency and staging. Local default-heap copies reduce repeated PCIe reads; these are the PC replacement for direct use of one physical Xbox pool.
- Directly opening title system pages is not an equivalent universal solution. On a discrete GPU those pages remain in system memory; GPU reads still cross PCIe, and the host cannot safely open sparse/remapped pages or allow their lifetime to change while a heap refers to them. The in-place route therefore remains a constrained opt-in.
- Fixed a fail-open in the optional local buffer/window mirror path: it previously allocated when `QueryVideoMemoryInfo` returned no local budget. The path now declines that optimization and keeps the correctness-preserving ordinary guest shadow. GPU-upload heaps already require the feature capability, local-pool preference, and local headroom.
- The separate host-RAM pressure policy from Stage 46 remains in place. Local VRAM, shared/non-local GPU-visible RAM, and host physical RAM are treated as separate budgets.

## Validation and limits

Static source assertions and ZIP integrity checks were run. Rust compilation/tests, real D3D12 GPU parity, game image comparison, and RTX 3060 Ti FPS and memory captures were not run in this environment. The source architecture is Xbox-aware, but this cannot prove that a particular game build has no memory aliasing or rendering mismatch. Use `docs/xbox-to-pc-memory.md` for the source map and target-PC measurement list.

## Primary references

- [Xbox Wire: Series X memory capacity and bandwidth regions](https://news.xbox.com/en-us/2020/03/16/xbox-series-x-tech/)
- [Microsoft GDK: console resource guarantees and Xbox memory model](https://learn.microsoft.com/en-us/gaming/gdk/docs/gdk-dev/intro/introduction?view=gdk-2604)
- [Microsoft Learn: D3D12 UMA and cache-coherent UMA properties](https://learn.microsoft.com/en-us/windows/desktop/api/d3d12/ns-d3d12-feature-data-architecture)
- [Microsoft Learn: D3D12 residency budgets](https://learn.microsoft.com/en-us/windows/win32/direct3d12/residency)
- [Microsoft DirectX specification: D3D12 resource heaps, aliasing and separated PC memory](https://microsoft.github.io/DirectX-Specs/d3d/ResourceHeaps.html)
