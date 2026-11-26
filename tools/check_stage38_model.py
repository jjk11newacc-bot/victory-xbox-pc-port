"""Independent interval/readback model; no Rust or D3D12 execution."""
import random

def claim(covered, lo, hi):
    if lo >= hi:
        return []
    start, end, nxt = lo, hi, lo
    missing = []
    prior = max((a for a in covered if a <= lo), default=None)
    if prior is not None and covered[prior] >= lo:
        b = covered.pop(prior)
        start, end, nxt = prior, max(end,b), max(nxt,b)
    while True:
        a = min((a for a in covered if start <= a <= end), default=None)
        if a is None:
            break
        b = covered.pop(a)
        if nxt < a and nxt < hi:
            missing.append((nxt,min(a,hi)))
        nxt, end = max(nxt,b), max(end,b)
    if nxt < hi:
        missing.append((nxt,hi))
    covered[start] = end
    return missing

rng = random.Random(38)
count = 0
for trial in range(2000):
    maps = [{} for _ in range(4)]
    reference = [set() for _ in maps]
    for _ in range(50):
        k = rng.randrange(4)
        lo = rng.randrange(512)
        hi = lo+rng.randrange(100)
        fresh = claim(maps[k],lo,hi)
        got = [i for a,b in fresh for i in range(a,b)]
        assert len(got)==len(set(got))
        assert set(got)==set(range(lo,hi))-reference[k]
        reference[k].update(range(lo,hi))
        ordered = sorted(maps[k].items())
        assert all(b<c for (_,b),(c,_) in zip(ordered,ordered[1:]))
        assert {i for a,b in ordered for i in range(a,b)} == reference[k]
        count+=1
print(f'{count:,} interval claims matched exact byte coverage')
# Overlapping readback must not re-merge CPU writes after the first merge.
# The CPU modifies byte 1 while the GPU changes byte 0.
base = [0,0,0]; gpu=[9,0,0]; title=[0,7,0]
for a,b in [(0,2),(1,3)]:
    # use unique ranges and preserve CPU-only byte 1
    if a==0: covered={}
    for x,y in claim(covered,a,b):
        for i in range(x,y):
            if gpu[i]!=base[i]: title[i]=gpu[i]
            base[i]=title[i]
assert title==[9,7,0]
print('overlapping settled readback preserves CPU-only writes')
