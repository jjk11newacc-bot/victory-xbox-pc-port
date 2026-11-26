"""Independent models only; not Rust or D3D12 execution."""
import random
r=random.Random(36)
for _ in range(30000):
    xor=r.choice([True, False]); e=r.choice([4,8,16]); a=[r.randrange(0,1<<20,4) for _ in range(r.randrange(1,8))]; b=[r.randrange(0,1<<20,4) for _ in range(r.randrange(1,8))]
    origin=0 if xor else min(a)+min(b)
    if xor:
        lo=(min(a)&~65535)+(min(b)&~65535); hi=(max(a)&~65535)+(max(b)&~65535)+65536+e-4
        refs=[(x&~65535)+(y&~65535)+((x^y)&65535) for x in a for y in b]
    else:
        lo=min(a)+min(b)-origin;hi=max(a)+max(b)-origin+e;refs=[x+y-origin for x in a for y in b]
    assert all(lo<=x and x+e<=hi for x in refs)
    d=r.randrange(0,2<<20,65536); end=d+65536
    if not (d<hi and end>lo):assert not any(d<x+e and end>x for x in refs)
print('dirty detile: 30000 conservative bounds/dirty page cases passed')
for _ in range(100000):
    n=r.randrange(1,100); copied=[r.randrange(0,100) for _ in range(n)]; writes={p:r.randrange(1,100) for p in range(n) if r.randrange(4)==0}
    clean=all(c!=0 and c>=max(writes.values(),default=0) for c in copied)
    if clean:assert not any(c==0 or writes.get(i,0)>c for i,c in enumerate(copied))
print('watched memory: 100000 no-copy predicate cases passed')
for _ in range(10000):
    count=r.randrange(8,40); spans=[(a:=r.randrange(100),a+r.randrange(1,100)) for _ in range(count)]
    pages={p:r.randrange(count) for p in range(200) if r.randrange(3)}; changed=set(r.sample(range(count),r.randrange(8,count+1)))
    def add(out,p):
        if out and out[-1][0]+out[-1][1]==p:out[-1]=(out[-1][0],out[-1][1]+1)
        else:out.append((p,1))
    old={i:[] for i in changed};new={i:[] for i in changed}
    for i in changed:
        a,b=spans[i]
        for p in range(a,b):
            if pages.get(p)==i:add(old[i],p)
    for p,i in sorted(pages.items()):
        if i in changed and spans[i][0]<=p<spans[i][1]:add(new[i],p)
    assert old==new
print('page ownership ranges: 10000 exact order/coverage cases passed')
