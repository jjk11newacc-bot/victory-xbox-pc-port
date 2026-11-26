# Adapting Xbox memory behavior to a discrete PC

## What the Xbox memory model means

Xbox Series X has 16 GB of GDDR6 in a single physical memory system attached to its SoC. Microsoft publishes two bandwidth regions: 10 GB at 560 GB/s and 6 GB at 336 GB/s. The CPU and GPU draw from the console's common physical pool, and the Xbox game OS gives a title a predictable platform budget. That does **not** mean every CPU/GPU access is automatically coherent or free of synchronization; the title still follows the console API's resource and fence rules.

An RTX 3060 Ti desktop instead has separate system DDR4 and 8 GB of local GDDR6. CPU allocations are in system memory; a D3D12 default-heap resource on a discrete adapter normally lives in local video memory. GPU access to system memory crosses PCIe and is much slower than local VRAM. Windows also supplies dynamic per-process budgets for local and non-local memory. If local VRAM is over budget, Windows may move allocations into system memory, which can create the exact frame-time spikes this port needs to avoid.

There is no PC setting that turns those two physical pools into Xbox unified memory. The port must reproduce the Xbox guest's observable address, aliasing, update, and ordering behavior while choosing a storage path suitable for each PC resource.

## How this runtime translates the guest memory

The compatibility layer preserves Xbox guest CPU-visible addresses and page mappings. Translated GPU work cannot safely treat arbitrary Windows guest pages as ordinary discrete-GPU VRAM, so the runtime uses these paths:

| Guest data/use | PC representation in this source | Why |
| --- | --- | --- |
| CPU-visible title pages and aliases | Windows system memory with the guest address/page map retained | CPU code and guest remaps must see the title's bytes and address semantics. |
| Shader-visible title buffers | Host shadow buffer refreshed from guest memory; eligible, budgeted buffers also get a GPU-local side | The GPU reads hot data locally instead of repeatedly fetching it over PCIe. Declared GPU writes are copied back in queue order so CPU code and aliases see them. |
| Xbox tiled texture resources | D3D12 texture resources plus tile/mip residency and upload staging | Xbox tiling and D3D12 texture layouts are not interchangeable; the runtime translates the contents and streams resident pieces. |
| CPU-written, GPU-read small shader inputs | GPU-upload memory only when the device advertises support, it maps to local memory, and the local budget has headroom; otherwise ordinary upload memory | GPU-upload heaps can avoid a copy on supported discrete adapters, but are not a universal resource type or a substitute for writable default heaps. |
| Directly opened title pages | Opt-in path for private, committed, stable pages only | On a discrete GPU this is still system memory from the GPU's perspective. It can avoid one software copy, but GPU reads still cross PCIe and page lifetime/remapping restrictions apply. |

The main implementation is in `crates/victory-gpu/src/memory.rs`, `shadow.rs`, `host.rs`, `resource.rs`, `texture.rs`, and `inplace.rs`. The NVIDIA profile favors local GPU resources for hot shader data and texture residency, but leaves enough budget for render targets, the driver, and Windows. Its budgets are independent: local VRAM, non-local/shared GPU-visible RAM, and host physical RAM.

## Fix in this revision

Local-side buffer/window mirrors are optional optimizations; the guest shadow path remains the correctness fallback. They now require a successful DXGI local-memory budget query before allocation. If Windows cannot provide a trustworthy local budget, the runtime keeps the normal guest shadow and does not risk an unbudgeted VRAM mirror. This closes a fail-open case in the previous code. Existing checks also gate GPU-upload heaps on the adapter capability and local-pool preference.

The earlier memory-pressure policy addresses host RAM exhaustion separately. It cannot remove the inherent PC cost of maintaining a CPU-visible guest copy plus a GPU-local copy when data needs both, or the upload/readback synchronization needed to preserve guest behavior. A direct alias of system pages is not a general replacement: it may make GPU traffic slower and cannot cover sparse/remapped page-pool views.

## How to verify on the target PC

Use the same scene, camera, resolution, driver, and warmed shader cache. Capture, per frame, local budget/use, non-local budget/use, available host RAM, CPU refresh bytes/time, bytes copied into local mirrors, GPU readback/copy-back bytes/time, texture resident mips, and frame-time median/p95. When memory pressure appears, determine which pool is full before changing the other pool's limits. A fix is successful only if image correctness and memory behavior hold together with better frame times.

## References

- [Xbox Wire: Xbox Series X system and memory specifications](https://news.xbox.com/en-us/2020/03/16/xbox-series-x-tech/)
- [Microsoft GDK: Xbox game OS resource guarantees and memory model overview](https://learn.microsoft.com/en-us/gaming/gdk/docs/gdk-dev/intro/introduction?view=gdk-2604)
- [Microsoft Learn: D3D12 architecture and cache-coherent UMA](https://learn.microsoft.com/en-us/windows/desktop/api/d3d12/ns-d3d12-feature-data-architecture)
- [Microsoft Learn: Direct3D 12 memory management strategies](https://learn.microsoft.com/en-us/windows/win32/direct3d12/memory-management-strategies)
- [Microsoft Learn: D3D12 residency and per-process budgets](https://learn.microsoft.com/en-us/windows/win32/direct3d12/residency)
- [Microsoft Learn: D3D12 heap types and adapter-neutral memory behavior](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ne-d3d12-d3d12_heap_type)
- [Microsoft DirectX specification: resource heaps, aliases, and discrete versus UMA memory](https://microsoft.github.io/DirectX-Specs/d3d/ResourceHeaps.html)
- [Microsoft Learn: GPU-upload heap properties](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12device-getcustomheapproperties%28uint_d3d12_heap_type%29)
