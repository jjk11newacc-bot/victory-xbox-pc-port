"""Independent models; no Rust, Windows or GPU execution."""
from random import Random
from bisect import bisect_right
r=Random(40)
PAGE_CHUNK=256
writes={}; maxima={}; comparisons=0
for i in range(1,24001):
    p=r.randrange(8192); stamp=i if i%7 else i//2
    writes[p]=stamp; c=p//PAGE_CHUNK; maxima[c]=max(stamp,maxima.get(c,0))
    if i%8: continue
    a=r.randrange(8192); b=min(8192,a+r.randrange(1,4097)); floor=r.randrange(i+1)
    old=sorted((p,w) for p,w in writes.items() if a<=p<b and w>floor)
    new=sorted((p,w) for p,w in writes.items() if a<=p<b and maxima[p//PAGE_CHUNK]>floor and w>floor)
    assert old==new
    # A buffer's minimum copied generation never omits a stale page.
    generations={p:r.choice((0,floor,r.randrange(i+1))) for p in range(a,b)}
    low=min(generations.values()); old_stale=[p for p in range(a,b) if not generations[p] or writes.get(p,0)>generations[p]]
    filtered={p:w for p,w in writes.items() if a<=p<b and w>low}
    new_stale=[p for p in range(a,b) if not generations[p] or filtered.get(p,0)>generations[p]]
    assert old_stale==new_stale
    # Also exercise the useful nonzero floor path.
    generations={p:r.randrange(max(1,floor),i+1) for p in range(a,b)}
    low=min(generations.values()); old_stale=[p for p in range(a,b) if writes.get(p,0)>generations[p]]
    filtered={p:w for p,w in writes.items() if a<=p<b and w>low}
    assert old_stale==[p for p in range(a,b) if filtered.get(p,0)>generations[p]]
    comparisons+=1
print(f'{comparisons:,} journal queries and {comparisons*2:,} stale-page decisions match the full journal')
for _ in range(10000):
    groups=sorted((a:=r.randrange(100),a+r.randrange(1,80)) for _ in range(r.randrange(30)))
    candidates=sorted(set((a:=r.randrange(100),a+r.randrange(1,80)) for _ in range(r.randrange(1,50))))
    success={p:r.choice((True,False)) for p in candidates}
    old=list(groups); calls=[]
    for a,b in candidates:
        if not any(x<=a and b<=y for x,y in old):
            calls.append((a,b))
            if success[a,b]: old.append((a,b))
    starts=[]; ends=[]
    for a,b in groups: starts.append(a); ends.append(max(b,ends[-1] if ends else 0))
    last=0; new=list(groups); calls2=[]
    for a,b in candidates:
        i=bisect_right(starts,a)
        if b>last and not (i and ends[i-1]>=b):
            calls2.append((a,b))
            if success[a,b]: new.append((a,b)); last=max(last,b)
    assert calls==calls2 and old==new
print('10,000 singleton query schedules match, including kernel failures')
