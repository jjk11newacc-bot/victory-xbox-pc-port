"""Independent algorithm models, not tests of compiled Rust or D3D12."""
import random
from itertools import permutations
r = random.Random(41)
ordered=[]; exact=set(); union=[]
def add(a,b):
    if a>=b: return
    out=[]
    for x,y in union:
        if y<a or x>b: out.append((x,y))
        else: a,b=min(a,x),max(b,y)
    union[:]=sorted(out+[(a,b)])
for i in range(20000):
    a=r.randrange(256); b=a+r.randrange(25)
    if i%9==0:
        ordered[:]=[p for p in ordered if p[0]%4!=i%4]
        exact=set(ordered); union.clear()
        for x,y in ordered: add(x,y)
    elif (a,b) not in exact:
        ordered.append((a,b)); exact.add((a,b)); add(a,b)
    start=r.randrange(280); end=min(280,start+r.randrange(65))
    expected=[p for p in range(start,end) if not any(x<=p<y for x,y in ordered)]
    actual=[p for p in range(start,end) if not any(x<=p<y for x,y in union)]
    assert expected==actual and exact==set(ordered)
print('20,000 pending-range model events passed')
old=[]; new={}; live={}
for i in range(10000):
    ranges=list(dict.fromkeys((a,a+4) for a in (r.randrange(32) for _ in range(r.randrange(1,9)))))
    live[i]=ranges
    for key in ranges+ranges:
        for entry in old:
            if entry[0]==key:
                if entry[2]!=i: entry[1]+=1; entry[2]=i
                break
        else: old.append([key,1,i])
        n,last=new.get(key,(0,i))
        new[key]=(n+1,i) if n==0 or last!=i else (n,last)
    if len(live)>10 or (live and r.random()<.7):
        ranges=live.pop(r.choice(list(live)))
        free1=[];free2=[]
        for k in ranges:
            for j,e in enumerate(old):
                if e[0]==k:
                    e[1]-=1
                    if not e[1]: free1.append(k);old.pop(j)
                    break
            n,last=new[k]
            if n==1: free2.append(k);del new[k]
            else:new[k]=(n-1,last)
        assert free1==free2
    assert new=={k:(n,last) for k,n,last in old}
print('10,000 flight reference-count model events passed')
count=0
for order in permutations('ABCD'):
    if order.index('A')>order.index('B') or order.index('C')>order.index('D'):continue
    done=False; queued=False; initial=False
    for op in order:
        if op=='A': initial=done
        if op=='B': queued=not(initial or done)
        if op=='C': done=True
        if op=='D': queued=False
    assert done and not queued
    count+=1
print(f'{count} retirement interleavings passed (serialized enqueue/sweep)')
for _ in range(10000):
    high=0; since=[]
    for _ in range(16):
        a=r.randrange(128); b=a+r.randrange(1,16)
        if a<high: since=[]
        assert not any(a<y and x<b for x,y in since)
        since.append((a,b)); high=max(high,b)
print('10,000 hash ordering sequences passed (abstract barriers)')
