"""Reference models of changed-run limiting, hazard union, directory lifetime.
Not an execution of the Rust runtime or GPU shaders."""
import random
from bisect import bisect_left
r = random.Random(42)

def union(ranges):
    out = []
    for a,b in sorted(ranges):
        if a >= b: continue
        if out and out[-1][1] >= a: out[-1] = (out[-1][0], max(out[-1][1],b))
        else: out.append((a,b))
    return out

def overlaps(u,a,b):
    if a>=b:return False
    i=bisect_left(u,(b,))-1
    return i>=0 and u[i][1]>a

P=16
for _ in range(30000):
    ranges=[(a,a+r.randrange(80)) for a in (r.randrange(240) for _ in range(r.randrange(24)))]
    a=r.randrange(240); b=a+r.randrange(65)
    u=union(ranges)
    busy=[(max(a,x),min(b,y)) for x,y in ranges if x<y and x<b and y>a] if a<b else []
    assert overlaps(u,a,b)==bool(busy)
    copied=[r.randrange(12) for _ in range(20)]
    writes={p:r.randrange(1,16) for p in range(20) if r.randrange(3)}
    old=any(writes.get(p,0)>copied[p] for x,y in busy for p in range(x//P,(y-1)//P+1))
    floor=min(copied[a//P:(b-1)//P+1],default=0)
    new=any(w>floor and w>copied[p] and overlaps(u,max(a,p*P),min(b,(p+1)*P)) for p,w in writes.items())
    assert old==new
print('30,000 overlapping hazard-query coverage/generation cases passed')

def bounded(a,b,lim):
    runs=[]; i=0
    while i<min(len(a),len(b)):
        if a[i]==b[i]:i+=1;continue
        if len(runs)==lim:return None
        first=i
        while i<min(len(a),len(b)) and a[i]!=b[i]:i+=1
        runs.append((first,i-first))
    return runs
for _ in range(10000):
    n=r.randrange(256); a=bytes(r.randrange(4) for _ in range(n)); b=bytes(r.randrange(4) for _ in range(n)); lim=r.randrange(100)
    ref=[]
    for i,(x,y) in enumerate(zip(a,b)):
        if x==y:continue
        if ref and ref[-1][0]+ref[-1][1]==i:ref[-1]=(ref[-1][0],ref[-1][1]+1)
        else:ref.append((i,1))
    assert bounded(a,b,lim)==(ref if len(ref)<=lim else None)
assert bounded(bytes(65536),bytes([0,1])*32768,512) is None
print('10,000 bounded changed-run cases and 64 KiB fragmented-page case passed')
# Existing lists capture immutable directory snapshots. In-place mutation
# must only occur when the directory has no outstanding list references.
current={}; live={}; reused=0; cloned=0
for i in range(15000):
    action=r.randrange(3)
    if action==0:live[i]=(current,current.copy())
    elif action==1 and live:del live[r.choice(list(live))]
    else:
        if live:current=current.copy();cloned+=1
        else:reused+=1
        current[r.randrange(64)]=r.randrange(256)
    for actual,expected in live.values():assert actual==expected
print(f'15,000 directory lifetime events passed ({reused} idle reuses, {cloned} protected replacements)')
