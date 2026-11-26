"""Independent reference models; NOT execution of the Rust or HLSL patches."""
from pathlib import Path
from functools import reduce
from operator import xor
import random, re

r = random.Random(37)
# Directory compaction must preserve the resolved backing and byte offset of
# each page and the partly populated owner, not the numeric descriptor index.
for _ in range(20_000):
    n = r.randrange(1, 130)
    backing = r.sample(range(1, 1000000), n)
    pages = {p: (r.randrange(n), r.randrange(1 << 16) * 65536) for p in r.sample(range(400), r.randrange(180))}
    partial = r.choice([None, r.randrange(n)])
    live = {i for i, _ in pages.values()}
    if partial is not None:
        live.add(partial)
    keep = [i for i in range(n) if i in live]
    remap = {i: j for j, i in enumerate(keep)}
    compacted = [backing[i] for i in keep]
    updated = {p: (remap[i], offset) for p, (i, offset) in pages.items()}
    assert {p: (backing[i], o) for p, (i, o) in pages.items()} == {p: (compacted[i], o) for p, (i, o) in updated.items()}
    assert all(backing[i] == compacted[j] for i, j in remap.items())
    if partial is not None:
        assert compacted[remap[partial]] == backing[partial]
print('directory compaction: 20,000 mapping reference cases passed')

# A raw run has a heap-owned lease and one lease per list (including
# closed/never-submitted lists and repeated executions). Simulate a growing
# heap and release only ranges no remaining list can replay.
runs = {0: (0, 16)}
current = 0
recorded = {}
retired = set()
next_id = 1
checks = 0
for _ in range(50_000):
    cmd = r.randrange(4)
    if cmd == 0:
        list_id = r.randrange(64)
        recorded.setdefault(list_id, set()).add(current)
    elif cmd == 1:
        recorded.pop(r.randrange(64), None)
    elif cmd == 2:
        retired.add(current)
        occupied = {slot for start, end in runs.values() for slot in range(start, end)}
        start = 0
        while any(slot in occupied for slot in range(start, start + 16)):
            start += 16
        runs[next_id] = (start, start+16)
        current, next_id = next_id, next_id + 1
    else:
        pinned = set().union(*recorded.values()) if recorded else set()
        for dead in retired - pinned:
            del runs[dead]
        retired &= pinned
    assert all(i in runs for values in recorded.values() for i in values)
    checks += 1
print(f'descriptor retirement: {checks:,} lifetime model events passed')

# For sub-dword texels a thread gathers from aligned source dwords and
# writes one destination dword. Check every actual row byte; padding is not
# copied to the texture. Random permutations exercise arbitrary tiled order.
checks = 0
for _ in range(10_000):
    e = r.choice([1,2])
    width = r.randrange(1,130)
    storage = (width * e + 127 + 3) // 4 * 4
    raw = r.randbytes(storage)
    offsets = r.sample(range(0, storage-e+1, e), width)
    expected = b''.join(raw[o:o+e] for o in offsets)
    actual = bytearray()
    for x in range(0, width*e, 4):
        value = 0
        for j in range(0,4,e):
            if x+j >= width*e: break
            at = offsets[(x+j)//e]
            aligned = at & ~3
            word = int.from_bytes(raw[aligned:aligned+4], 'little')
            value |= ((word >> ((at&3)*8)) & ((1<<(8*e))-1)) << (j*8)
        actual.extend(value.to_bytes(4,'little'))
    assert actual[:len(expected)] == expected
    checks += len(expected)
print(f'packed sub-dword detile: 10,000 row cases, {checks:,} byte comparisons passed')

# Check the actual 16-bit render-XOR basis used by each console model.
src = (Path(__file__).resolve().parents[1] / 'crates/victory-formats/src/texture.rs').read_text()
body = src.split('fn render_xor_pattern(',1)[1].split('fn contributions(',1)[0]
patterns = re.findall(r'\((true|false), 2\) => \(\s*&\[([^]]+)\],\s*&\[([^]]+)\]', body)
assert len(patterns)==2
for _, xs, ys in patterns:
    xb = [int(x.strip(),0) for x in xs.split(',') if x.strip()]
    yb = [int(x.strip(),0) for x in ys.split(',') if x.strip()]
    def spread(v,basis): return reduce(xor,(x for i,x in enumerate(basis) if v & (1<<i)),0)
    for _ in range(10_000):
        x,y=r.randrange(4096),r.randrange(4096)
        ox,oy=spread(x,xb),spread(y,yb)
        a=(x//128)*65536+ox; b=(y//256)*33*65536+oy
        offset=(a&~65535)+(b&~65535)+((a^b)&65535)
        assert offset==((x//128)+(y//256)*33)*65536+(ox^oy)
        assert offset%2==0 and (offset&3) in (0,2)
print('16-bit render-XOR: 20,000 address/alignment model cases passed')
