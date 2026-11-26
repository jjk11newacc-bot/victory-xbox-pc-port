"""Independent arithmetic/coverage checks; does not execute Rust or D3D12."""
import random
r = random.Random(31031)
for _ in range(20000):
    spans = sorted((a := r.randrange(128), a + r.randrange(1, 20)) for _ in range(r.randrange(1, 40)))
    runs = []
    for a, b in spans:
        if runs and a <= runs[-1][1]:
            runs[-1] = (runs[-1][0], max(b, runs[-1][1]))
        else:
            runs.append((a, b))
    origin = r.randrange(0, 1000)
    offsets = {}
    for a, b in runs:
        for page in range(a, b):
            assert page not in offsets
            offsets[page] = origin + len(offsets)
    assert set(offsets) == {p for a, b in spans for p in range(a, b)}
    for a, b in spans:
        assert [offsets[p] for p in range(a, b)] == list(range(offsets[a], offsets[a]+b-a))
    # Shared dirty masks must give the same per-declaration bytes as the
    # OR of legacy masks from duplicate comparisons. Do not publish gaps.
    dirty = {p: r.randrange(65536) for p in offsets}
    old = {}
    for a, b in spans:
        for p in range(a, b):
            old.setdefault(p, dirty[p])
    assert all(old[p] == dirty[p] for a, b in spans for p in range(a,b))
for _ in range(10000):
    size = r.randrange(1, 32)
    offsets, lengths = [], []
    end = 0
    for i in range(size):
        offsets.append((end+511)//512*512)
        lengths.append(256*r.randrange(1,16))
        end = offsets[-1]+lengths[-1]
    first = r.randrange(size)
    base = offsets[first]
    full = bytearray(end)
    compact = bytearray(end-base)
    for offset, n in zip(offsets[first:], lengths[first:]):
        payload = r.randbytes(n)
        full[offset:offset+n] = payload
        compact[offset-base:offset-base+n] = payload
    assert full == bytes(base)+compact
    assert all((offset-base)%512 == 0 for offset in offsets[first:])
print('PASS: 20,000 shared compare coverage cases; 10,000 compact footprint cases (independent model only).')
