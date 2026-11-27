//! Physical backing for mip tails of reserved, single-layer 2D textures.
//!
//! The tile layout and packed-mip convention follow Neptune VI's `tiles.rs`.
//! A mapping is made before a copy on the same host queue. Callers publish
//! sampled mips only after ordering the upload that initializes them.

use std::collections::BTreeMap;
use std::mem::ManuallyDrop;
use std::ops::Range;
use std::sync::{Mutex, OnceLock};

use windows::Win32::Graphics::Direct3D12::*;
use windows::core::Interface;

const TILE_BYTES: u64 = 64 * 1024;
const HEAP_TILES: u32 = 1024;

fn heap_tiles() -> u32 {
    static TARGET: OnceLock<u32> = OnceLock::new();
    *TARGET.get_or_init(|| {
        if std::env::var_os("VICTORY_LOW_MEMORY").is_some_and(|value| value == "1") {
            512 // 32 MiB chunks reduce unused residency late in a level.
        } else {
            HEAP_TILES
        }
    })
}

/// This path copies into reserved textures and needs defined reads and writes
/// on unmapped tiles. The committed path remains the default.
pub(crate) fn supported(device: &ID3D12Device) -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var_os("VICTORY_LEVELS_ON_DEMAND").is_some_and(|v| v == "1") && tier_two(device)
    })
}

fn tier_two(device: &ID3D12Device) -> bool {
    let mut options = D3D12_FEATURE_DATA_D3D12_OPTIONS::default();
    // SAFETY: the output is the struct this feature query expects.
    let result = unsafe {
        device.CheckFeatureSupport(
            D3D12_FEATURE_D3D12_OPTIONS,
            (&raw mut options).cast(),
            size_of_val(&options) as u32,
        )
    };
    result.is_ok() && options.TiledResourcesTier.0 >= D3D12_TILED_RESOURCES_TIER_2.0
}

/// Activate tiles that may have backed a different texture before this one.
/// Record this before the first copy into newly mapped tiles on the queue
/// that receives the mappings. The caller retains the texture and its tiles.
pub(crate) fn activate(list: &ID3D12GraphicsCommandList, texture: &ID3D12Resource) {
    let barrier = D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_ALIASING,
        Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            Aliasing: ManuallyDrop::new(D3D12_RESOURCE_ALIASING_BARRIER {
                pResourceBefore: ManuallyDrop::new(None),
                // SAFETY: a borrowed pointer the barrier never releases.
                pResourceAfter: unsafe { std::mem::transmute_copy(texture) },
            }),
        },
    };
    // SAFETY: a live texture and a recording command list.
    unsafe { list.ResourceBarrier(&[barrier]) };
}

#[derive(Clone, Copy)]
struct Group {
    first_mip: u32,
    last_mip: u32,
    tiles: u32,
}

/// A physical tile run. The pool holds its heap until all of its runs are
/// returned. A texture keeps the same mappings throughout its lifetime.
#[derive(Clone, Copy)]
struct Allocation {
    heap: usize,
    first: u32,
    tiles: u32,
}

struct Heap {
    resource: ID3D12Heap,
    tiles: u32,
    free: Vec<Range<u32>>,
}

#[derive(Default)]
struct Pool {
    heaps: Vec<Option<Heap>>,
    bytes: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PoolKey {
    Device(usize),
    #[cfg(test)]
    Fixture(usize),
}

// Every heap lives with its device while a run is in use. Tests with a
// private queue can isolate their budget from other users of the device.
static POOLS: Mutex<BTreeMap<PoolKey, Pool>> = Mutex::new(BTreeMap::new());

impl Pool {
    fn take(&mut self, device: &ID3D12Device, tiles: u32, budget: u64) -> Option<Allocation> {
        for (index, heap) in self.heaps.iter_mut().enumerate() {
            if let Some(heap) = heap
                && let Some(first) = take_range(&mut heap.free, tiles)
            {
                return Some(Allocation {
                    heap: index,
                    first,
                    tiles,
                });
            }
        }

        let available = budget.saturating_sub(self.bytes) / TILE_BYTES;
        if available < u64::from(tiles) {
            return None;
        }
        let capacity = u64::from(tiles.max(heap_tiles())).min(available) as u32;
        let size = u64::from(capacity) * TILE_BYTES;
        let desc = D3D12_HEAP_DESC {
            SizeInBytes: size,
            Properties: D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                ..Default::default()
            },
            Flags: D3D12_HEAP_FLAG_ALLOW_ONLY_NON_RT_DS_TEXTURES,
            ..Default::default()
        };
        let mut resource = None;
        // SAFETY: the heap size is a whole number of D3D12 tiles and the
        // output holds the COM heap on success.
        unsafe { device.CreateHeap(&desc, &mut resource).ok()? };
        let heap = Heap {
            resource: resource?,
            tiles: capacity,
            free: (tiles < capacity)
                .then_some(tiles..capacity)
                .into_iter()
                .collect(),
        };
        self.bytes += size;
        let index = self
            .heaps
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.heaps.len());
        if index == self.heaps.len() {
            self.heaps.push(Some(heap));
        } else {
            self.heaps[index] = Some(heap);
        }
        Some(Allocation {
            heap: index,
            first: 0,
            tiles,
        })
    }

    fn release(&mut self, allocation: Allocation) {
        let slot = &mut self.heaps[allocation.heap];
        let heap = slot.as_mut().expect("a live tile run keeps its heap");
        release_range(
            &mut heap.free,
            allocation.first..allocation.first + allocation.tiles,
        );
        if heap.free.as_slice() == std::slice::from_ref(&(0..heap.tiles)) {
            self.bytes -= u64::from(heap.tiles) * TILE_BYTES;
            *slot = None;
        }
    }
}

fn take_range(free: &mut Vec<Range<u32>>, tiles: u32) -> Option<u32> {
    let index = free
        .iter()
        .position(|range| range.end - range.start >= tiles)?;
    let first = free[index].start;
    free[index].start += tiles;
    if free[index].is_empty() {
        free.remove(index);
    }
    Some(first)
}

fn release_range(free: &mut Vec<Range<u32>>, mut range: Range<u32>) {
    let at = free.partition_point(|other| other.end < range.start);
    while at < free.len() && free[at].start <= range.end {
        let neighbor = free.remove(at);
        debug_assert!(neighbor.start == range.end || neighbor.end == range.start);
        range.start = range.start.min(neighbor.start);
        range.end = range.end.max(neighbor.end);
    }
    free.insert(at, range);
}

/// Owns the physical backing of a reserved texture. Store this beside its
/// D3D12 resource and retain both until all GPU uses have passed their fence.
pub(crate) struct Tiled {
    device: usize,
    groups: Vec<Group>,
    backing: Mutex<Vec<Option<Allocation>>>,
    #[cfg(test)]
    fixture_pool: Option<usize>,
}

impl Tiled {
    /// Read the layout of a reserved texture. Any unexpected tiling is left
    /// to the caller's committed-resource fallback.
    pub(crate) fn new(device: &ID3D12Device, texture: &ID3D12Resource, mips: u32) -> Option<Self> {
        if !(2..=16).contains(&mips) {
            return None;
        }
        // SAFETY: the caller holds a live resource on this device.
        let desc = unsafe { texture.GetDesc() };
        let unsupported = D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET
            | D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL
            | D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS
            | D3D12_RESOURCE_FLAG_DENY_SHADER_RESOURCE;
        if desc.Dimension != D3D12_RESOURCE_DIMENSION_TEXTURE2D
            || desc.DepthOrArraySize != 1
            || desc.SampleDesc.Count != 1
            || u32::from(desc.MipLevels) != mips
            || desc.Layout != D3D12_TEXTURE_LAYOUT_64KB_UNDEFINED_SWIZZLE
            || (desc.Flags & unsupported).0 != 0
        {
            return None;
        }
        let mut total = 0;
        let mut packed = D3D12_PACKED_MIP_INFO::default();
        let mut count = mips;
        let mut tilings = vec![D3D12_SUBRESOURCE_TILING::default(); mips as usize];
        // SAFETY: `count` is the capacity of `tilings` and the other outputs
        // are live local values of the expected types.
        unsafe {
            device.GetResourceTiling(
                texture,
                Some(&mut total),
                Some(&mut packed),
                None,
                Some(&mut count),
                0,
                tilings.as_mut_ptr(),
            );
        }
        let groups = tile_groups(mips, total, packed, &tilings[..count.min(mips) as usize])?;
        let backing = Mutex::new(vec![None; groups.len()]);
        Some(Self {
            device: device.as_raw() as usize,
            groups,
            backing,
            #[cfg(test)]
            fixture_pool: None,
        })
    }

    fn pool_key(&self) -> PoolKey {
        #[cfg(test)]
        if let Some(pool) = self.fixture_pool {
            return PoolKey::Fixture(pool);
        }
        PoolKey::Device(self.device)
    }

    /// Map an available contiguous tail, smallest mips first. Returns the
    /// finest requested level whose tiles are ready for an upload. The caller
    /// must activate and initialize the returned tail before exposing it to a shader.
    /// `budget` caps the total heap bytes in this device's texture tile pool.
    pub(crate) fn map_from(
        &self,
        device: &ID3D12Device,
        queue: &ID3D12CommandQueue,
        texture: &ID3D12Resource,
        requested_first: u32,
        budget: u64,
    ) -> Option<u32> {
        if device.as_raw() as usize != self.device {
            return None;
        }
        let mut backing = self.backing.lock().unwrap_or_else(|e| e.into_inner());
        let mut pools = POOLS.lock().unwrap_or_else(|e| e.into_inner());
        let pool = pools.entry(self.pool_key()).or_default();
        let mut first = None;
        for (group, backed) in self.groups.iter().zip(backing.iter_mut()).rev() {
            if group.last_mip < requested_first {
                break;
            }
            if backed.is_none() {
                let Some(allocation) = pool.take(device, group.tiles, budget) else {
                    break;
                };
                let heap = pool.heaps[allocation.heap].as_ref().unwrap();
                let coordinate = D3D12_TILED_RESOURCE_COORDINATE {
                    Subresource: group.first_mip,
                    ..Default::default()
                };
                let region = D3D12_TILE_REGION_SIZE {
                    NumTiles: group.tiles,
                    ..Default::default()
                };
                // SAFETY: the heap run stays live with the reserved texture.
                // The queue orders this one whole-subresource mapping before
                // the copying list; a packed mip tail is one region.
                unsafe {
                    queue.UpdateTileMappings(
                        texture,
                        1,
                        Some(&coordinate),
                        Some(&region),
                        &heap.resource,
                        1,
                        Some(&D3D12_TILE_RANGE_FLAG_NONE),
                        Some(&allocation.first),
                        Some(&allocation.tiles),
                        D3D12_TILE_MAPPING_FLAG_NONE,
                    );
                }
                *backed = Some(allocation);
            }
            first = Some(requested_first.max(group.first_mip));
        }
        first
    }

    pub(crate) fn backed_bytes(&self) -> u64 {
        self.backing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .flatten()
            .map(|allocation| u64::from(allocation.tiles) * TILE_BYTES)
            .sum()
    }

    pub(crate) fn pool_bytes(&self) -> u64 {
        POOLS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&self.pool_key())
            .map_or(0, |pool| pool.bytes)
    }
}

impl Drop for Tiled {
    fn drop(&mut self) {
        let key = self.pool_key();
        let backing = self.backing.get_mut().unwrap_or_else(|e| e.into_inner());
        let mut pools = POOLS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pool) = pools.get_mut(&key) {
            for allocation in backing.iter().flatten() {
                pool.release(*allocation);
            }
            if pool.bytes == 0 {
                pools.remove(&key);
            }
        }
    }
}

fn tile_groups(
    mips: u32,
    total: u32,
    packed: D3D12_PACKED_MIP_INFO,
    tilings: &[D3D12_SUBRESOURCE_TILING],
) -> Option<Vec<Group>> {
    let standard = u32::from(packed.NumStandardMips);
    let packed_mips = u32::from(packed.NumPackedMips);
    if standard.checked_add(packed_mips)? != mips || total == 0 {
        return None;
    }
    let mut groups = Vec::with_capacity(mips as usize);
    let mut offset = 0u32;
    for (index, tiling) in tilings.get(..standard as usize)?.iter().enumerate() {
        let tiles = tiling
            .WidthInTiles
            .checked_mul(u32::from(tiling.HeightInTiles))?
            .checked_mul(u32::from(tiling.DepthInTiles))?;
        if tiles == 0 || tiling.StartTileIndexInOverallResource != offset {
            return None;
        }
        offset = offset.checked_add(tiles)?;
        let mip = index as u32;
        groups.push(Group {
            first_mip: mip,
            last_mip: mip,
            tiles,
        });
    }
    if packed_mips > 0 {
        let tiles = packed.NumTilesForPackedMips;
        if tiles == 0 || packed.StartTileIndexInOverallResource != offset {
            return None;
        }
        offset = offset.checked_add(tiles)?;
        groups.push(Group {
            first_mip: standard,
            last_mip: mips - 1,
            tiles,
        });
    }
    (offset == total).then_some(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_runs_are_reused_and_coalesce_when_freed() {
        let mut free: Vec<_> = std::iter::once(0..20).collect();
        assert_eq!(take_range(&mut free, 8), Some(0));
        assert_eq!(take_range(&mut free, 4), Some(8));
        assert_eq!(take_range(&mut free, 4), Some(12));
        release_range(&mut free, 0..8);
        release_range(&mut free, 12..16);
        assert_eq!(free, [0..8, 12..20]);
        assert_eq!(take_range(&mut free, 9), None);
        release_range(&mut free, 8..12);
        assert_eq!(free.as_slice(), std::slice::from_ref(&(0..20)));
        assert_eq!(take_range(&mut free, 20), Some(0));
        assert!(free.is_empty());
    }

    #[test]
    fn packed_mips_share_one_backing_group() {
        let layout = [D3D12_SUBRESOURCE_TILING {
            WidthInTiles: 2,
            HeightInTiles: 2,
            DepthInTiles: 1,
            StartTileIndexInOverallResource: 0,
        }];
        let packed = D3D12_PACKED_MIP_INFO {
            NumStandardMips: 1,
            NumPackedMips: 3,
            NumTilesForPackedMips: 1,
            StartTileIndexInOverallResource: 4,
        };
        let groups = tile_groups(4, 5, packed, &layout).unwrap();
        assert_eq!(
            groups
                .iter()
                .map(|g| (g.first_mip, g.last_mip, g.tiles))
                .collect::<Vec<_>>(),
            [(0, 0, 4), (1, 3, 1)]
        );
        assert!(tile_groups(4, 6, packed, &layout).is_none());
    }

    #[test]
    fn reserved_textures_map_a_coarse_tail_within_budget_and_reuse_tiles() {
        use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows::Win32::Graphics::Dxgi::Common::{
            DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
        };
        use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

        let Some(host) = crate::host::host() else {
            return;
        };
        let device = &host.device;
        let tier_two = tier_two(device);
        assert_eq!(
            supported(device),
            std::env::var_os("VICTORY_LEVELS_ON_DEMAND").is_some_and(|v| v == "1") && tier_two
        );
        if !tier_two {
            eprintln!("Skipping sparse D3D12 test on a device without tiled resources Tier 2");
            return;
        }
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Width: 512,
            Height: 512,
            DepthOrArraySize: 1,
            MipLevels: 10,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_64KB_UNDEFINED_SWIZZLE,
            ..Default::default()
        };
        // SAFETY: a private direct queue used only by this test.
        let queue: ID3D12CommandQueue = unsafe {
            device
                .CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC::default())
                .unwrap()
        };
        let reserve = || {
            let mut texture = None;
            // SAFETY: a reserved, non-target 2D texture with one array slice.
            unsafe {
                device
                    .CreateReservedResource(
                        &desc,
                        D3D12_RESOURCE_STATE_COPY_DEST,
                        None,
                        &mut texture,
                    )
                    .unwrap()
            };
            let texture = texture.unwrap();
            let mut tiled = Tiled::new(device, &texture, u32::from(desc.MipLevels)).unwrap();
            tiled.fixture_pool = Some(queue.as_raw() as usize);
            (texture, tiled)
        };
        let wait = || {
            // SAFETY: the test owns the fence and event, and closes the event after waiting.
            unsafe {
                let fence: ID3D12Fence = device.CreateFence(0, D3D12_FENCE_FLAG_NONE).unwrap();
                let event = CreateEventW(None, false, false, None).unwrap();
                queue.Signal(&fence, 1).unwrap();
                fence.SetEventOnCompletion(1, event).unwrap();
                let waited = WaitForSingleObject(event, 5_000);
                CloseHandle(event).unwrap();
                assert_eq!(waited, WAIT_OBJECT_0);
            }
        };

        let (first, tiled) = reserve();
        let tail = *tiled.groups.last().unwrap();
        let tail_bytes = u64::from(tail.tiles) * TILE_BYTES;
        assert_eq!(tiled.map_from(device, &queue, &first, 0, 0), None);
        assert_eq!(tiled.backed_bytes(), 0);
        assert_eq!(
            tiled.map_from(device, &queue, &first, 0, tail_bytes),
            Some(tail.first_mip)
        );
        assert_eq!(tiled.backed_bytes(), tail_bytes);
        assert_eq!(
            tiled.map_from(device, &queue, &first, tail.last_mip, tail_bytes),
            Some(tail.last_mip)
        );
        wait();
        drop(first);
        drop(tiled);

        let (first, first_tiles) = reserve();
        let (second, second_tiles) = reserve();
        let (third, third_tiles) = reserve();
        let budget = 2 * tail_bytes;
        for (texture, tiled) in [(&first, &first_tiles), (&second, &second_tiles)] {
            assert_eq!(
                tiled.map_from(device, &queue, texture, tail.last_mip, budget),
                Some(tail.last_mip)
            );
        }
        assert_eq!(
            third_tiles.map_from(device, &queue, &third, tail.last_mip, budget),
            None
        );
        let run = first_tiles.backing.lock().unwrap().last().unwrap().unwrap();
        wait();
        drop(first);
        drop(first_tiles);
        assert_eq!(
            third_tiles.map_from(device, &queue, &third, tail.last_mip, budget),
            Some(tail.last_mip)
        );
        let reused = third_tiles.backing.lock().unwrap().last().unwrap().unwrap();
        assert_eq!((reused.heap, reused.first), (run.heap, run.first));
        let list = host.new_list().unwrap();
        activate(&list.list, &third);
        list.close();
        // SAFETY: the list is closed and all three live objects outlast its fence.
        unsafe { queue.ExecuteCommandLists(&[Some(list.list.cast().unwrap())]) };
        wait();
    }
}
