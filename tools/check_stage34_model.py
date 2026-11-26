"""Independent models of Stage 34 ordering and exact host state reuse.
Not a Rust, D3D12, FPS, or hardware test.
"""
import random
r = random.Random(34)

def scalar(groups, dedup):
    steps, seen_r, seen_w = [], set(), set()
    for event, reads, writes in groups:
        if event:
            steps.append(('boundary', event))
            seen_r.clear(); seen_w.clear()
        for shadow, address, length, size, placed in reads:
            key = (shadow, address, min(length, size-address))
            if key not in seen_r:
                seen_r.add(key); steps.append(('read', key))
        for shadow, address, length, size, placed in writes:
            key = (shadow, address, min(length, size-address))
            if not placed and (not dedup or key not in seen_w):
                seen_w.add(key); steps.append(('write', key))
    return steps

def batch(groups, dedup):
    steps, seen = [], {'read': set(), 'write': set()}
    for event, reads, writes in groups:
        if event:
            steps.append(('boundary', event))
            for keys in seen.values(): keys.clear()
        ordered = [('read', row) for row in reads] + [('write', row) for row in writes]
        for kind, (shadow, address, length, size, placed) in ordered:
            if kind == 'write' and placed: continue
            key = (shadow, address, min(length, size-address))
            if kind == 'write' and not dedup or key not in seen[kind]:
                seen[kind].add(key); steps.append((kind, key))
    return steps

for trial in range(5000):
    sizes = [r.randrange(32, 512) for _ in range(8)]
    placed = [r.randrange(3) == 0 for _ in sizes]
    pool = []
    for _ in range(24):
        i = r.randrange(len(sizes)); size = sizes[i]
        pool.append((i, r.randrange(size), r.randrange(1, 600), size, placed[i]))
    groups = [(r.choice([0,0,0,1,2]), r.choices(pool, k=r.randrange(20)), r.choices(pool, k=r.randrange(20))) for _ in range(8)]
    for dedup in (False, True):
        assert scalar(groups, dedup) == batch(groups, dedup)

# Model setter elision, invalidation and overlapping IA slot writes. Root
# signatures, descriptor heaps, replay and external passes invalidate caches.
cache, actual, reference = {}, {}, {}
vertex_cache, index_cache = None, None
vertex, ref_vertex = {}, {}
index = ref_index = None
skipped = 0
for _ in range(100000):
    event = r.randrange(12)
    if event == 0:
        cache.clear(); actual.clear(); reference.clear()
        vertex_cache = index_cache = None
        vertex.clear(); ref_vertex.clear(); index = ref_index = None
    elif event == 1:
        # Rectangle expansion or indirect command invalidates IA.
        slot = r.randrange(8); value = (r.randrange(8), 16, 4)
        vertex[slot] = ref_vertex[slot] = value
        vertex_cache = index_cache = None
    elif event < 5:
        param = r.randrange(4); words = tuple(r.randrange(3) for _ in range(r.randrange(1,4)))
        reference[param] = words
        if cache.get(param) != words:
            actual[param] = cache[param] = words
        else: skipped += 1
    elif event < 9:
        views = tuple((r.randrange(3), 64, 4) for _ in range(r.randrange(1,5)))
        ref_vertex.update(enumerate(views))
        if vertex_cache != views:
            vertex.update(enumerate(views)); vertex_cache = views
        else: skipped += 1
    else:
        view = (r.randrange(4), 128, 57)
        ref_index = view
        if index_cache != view:
            index = index_cache = view
        else: skipped += 1
    assert actual == reference and vertex == ref_vertex and index == ref_index
print(f'10,000 scheduling comparisons; 100,000 state events; {skipped:,} model setter elisions; PASS')
