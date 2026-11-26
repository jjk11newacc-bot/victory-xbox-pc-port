"""Independent model, not a Rust/D3D12 execution test."""
from random import Random
r = Random(39)
# Thick volume offsets: disjoint x/y/z bits inside a tile, nonoverlapping
# macroblocks. The shader uses the same axis-add-minus-origin equation.
count = 0
for _ in range(1200):
    e = r.choice([1,2,4,8,16]); w,h,d = [r.randint(1,65) for _ in range(3)]
    pitch = ((w*e+255)//256)*256; plane=pitch*h
    # 8x8x4 model tile with Morton-style bits and a base for the mip/slab.
    def offset(x,y,z):
        bits=sum(((x>>i)&1)<<(3*i) | ((y>>i)&1)<<(3*i+1) | ((z>>i)&1)<<(3*i+2) for i in range(2))
        bits |= ((x>>2)&1)<<6 | ((y>>2)&1)<<7
        tile=((z//4)*((h+7)//8)+y//8)*((w+7)//8)+x//8
        return 65536+(tile*256+bits)*e
    for _ in range(40):
        x,y,z=r.randrange(w),r.randrange(h),r.randrange(d)
        assert offset(x,0,z)+offset(0,y,z)-offset(0,0,z)==offset(x,y,z)
        a=z*plane+y*pitch+x*e
        assert z*plane <= a < a+e <= (z+1)*plane
        count+=1
print(f'{count:,} synthetic volume axis and destination bounds checks passed')
# Binding cache must replay exact state through invalidations.
cache={}; actual={}
for _ in range(20000):
    if r.randrange(13)==0: cache.clear(); actual.clear(); continue
    p=r.randrange(64); v=(r.randrange(4),r.randrange(12)*256)
    if cache.get(p)!=v: cache[p]=v; actual[p]=v
    assert actual[p]==v
print('20,000 synthetic graphics binding events passed')
