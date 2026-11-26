"""Independent Python model of Coverage's policy; does not execute Rust.
Use the Rust unit tests for actual implementation validation.
"""
import random
GAP=65536
MAX_PARTS=64

def add(bounds, parts, sparse, a, b):
    old=bounds
    bounds=min(old[0],a),max(old[1],b)
    if not sparse: return bounds,parts,sparse
    if not parts:
        if a>old[1]+GAP: parts=[old,(a,b)]
        elif old[0]>b+GAP: parts=[(a,b),old]
        return bounds,parts,sparse
    first=next((i for i,(_,y) in enumerate(parts) if y+GAP>=a),len(parts))
    last=next((i for i in range(first,len(parts)) if parts[i][0]>b+GAP),len(parts))
    if first==last:
        if len(parts)==MAX_PARTS: return bounds,[],False
        parts.insert(first,(a,b))
    else:
        parts[first:last]=[(min(parts[first][0],a),max(parts[last-1][1],b))]
        if len(parts)==1: parts=[]
    return bounds,parts,sparse

def reference(requests):
    result=[]
    for a,b in sorted(requests):
        if result and a<=result[-1][1]+GAP:
            result[-1]=(result[-1][0],max(result[-1][1],b))
        else: result.append((a,b))
    return result

rng=random.Random(30)
checks=0
for _ in range(1000):
    requests=[(0,4096)]; bounds=requests[0]; parts=[]; sparse=True
    for _ in range(128):
        a=rng.randrange(0,1<<24); b=a+rng.randrange(1,1<<15)
        requests.append((a,b))
        bounds,parts,sparse=add(bounds,parts,sparse,a,b)
        actual=parts or [bounds]
        assert actual==(reference(requests) if sparse else [bounds])
        assert len(actual)<=MAX_PARTS
        checks+=1
bounds,parts,sparse=add((0,4096),[],True,64*1024*1024,64*1024*1024+4096)
assert sum(b-a for a,b in parts)==8192
print(f'{checks:,} model steps passed; synthetic distant ranges: {bounds[1]-bounds[0]:,} envelope bytes -> 8,192 scheduled bytes (not a runtime measurement).')
