"""Stage 43 independent address/exchange/dependency models, not Rust or GPU execution."""
from pathlib import Path
from random import Random
import re

source = (Path(__file__).resolve().parents[1] / 'crates/victory-formats/src/texture.rs').read_text()
# Upstream GFX10 PATINFO / NIBBLE01,2,3: no base pipe XOR, z=0.
# Each expression is one output address bit, independent of the Rust axis tables.
refs = {
    (False, 1): 'X0 X1 X2 X3 Y0 Y1 Y2 Y3 Y4^X7^Y7 X4^Y4 Y5^X6 X5^Y6 X6^X8^Y8 Y6 X7 Y7^X8',
    (False, 16): '0 0 0 0 X0 Y0 X1 Y1 Y4^X7^Y7 X4^Y4 Y5^X6 X5^Y6 X3^X8^Y8 X2 Y2 Y3^X6',
    (True, 1): 'X0 X1 X2 X3 Y0 Y1 Y2 Y3 Y4^X6^Y6 X4^Y4 X5^Y5 X5 X6 Y6^X9 X7^Y8 Y7^X8',
    (True, 16): '0 0 0 0 X0 Y0 X1 Y1 Y4^X6^Y6 X4^Y4 X5^Y5 X2 Y2 X3^Y7 Y3^X6 X5^Y6',
}

def axis(basis, coord):
    v = 0
    for i,b in enumerate(basis):
        if (coord >> i)&1: v ^= b
    return v

def oracle(exprs, x, y):
    out = 0
    for i,terms in enumerate(exprs):
        bit = 0
        for term in terms.split('^'):
            if term != '0': bit ^= ((x if term[0] == 'X' else y) >> int(term[1:]))&1
        out |= bit << i
    return out

checks = 0
for (ss,n), ref in refs.items():
    m = re.search(r'\('+str(ss).lower()+r', '+str(n)+r'\) => \(\s*&\[([^]]+)\],\s*&\[([^]]+)\]',source)
    assert m
    ax, ay = [[int(x.strip(),0) for x in part.split(',')] for part in m.groups()]
    w = 1 << ((16-(n.bit_length()-1)+1)//2)
    h = 65536//(w*n)
    for width,height in [(256,256),(513,357),(1025,257)]:
        blocks = (width+w-1)//w
        size=blocks*((height+h-1)//h)*65536
        cols=[axis(ax,x)+(x//w)*65536 for x in range(width)]
        rows=[axis(ay,y)+(y//h)*blocks*65536 for y in range(height)]
        seen=set()
        for y,b in enumerate(rows):
            for x,a in enumerate(cols):
                off=(a&~65535)+(b&~65535)+((a^b)&65535)
                assert off==((y//h)*blocks+x//w)*65536+oracle(ref.split(),x,y)
                assert 0<=off<=size-n and off%n==0 and off not in seen
                seen.add(off)
                if x%(16//n)==0:
                    for t in range(min(16//n,width-x)):
                        assert (axis(ax,x+t)^axis(ay,y)) == ((a^b)&65535)+t*n
                checks+=1
print(f'render-XOR: {checks:,} address/uniqueness checks against transcribed output-bit equations')

rng=Random(43)
for _ in range(20000):
    group=rng.randrange(2,33)*32
    waves=group//32
    values=[(rng.getrandbits(32),rng.getrandbits(32)) for _ in range(waves)]
    lds=[None]*(((waves+1)//2)*4)
    order=list(range(waves));rng.shuffle(order)
    for i in order:
        base=(i//2)*4+i%2
        lds[base],lds[base+2]=values[i]
    for i in range(waves):
        base=(i//2)*4
        a,b=values[i//2*2]
        hi=values[i//2*2+1] if i//2*2+1<waves else (0,0)
        assert (lds[base],lds[base+2])==(a,b)
        assert (lds[base+1],lds[base+3])==(hi if i//2*2+1<waves else (None,None))
        if i//2*2+1==waves:
            assert i%2==0 and group%64==32
            # The single existing half is the result even for nonzero identities.
            assert a==(a & 0xffffffff) and (a|0)==a
print('cross-wave: 20,000 randomized paired LDS layouts including unmatched halves')

for _ in range(30000):
    base=rng.randrange(1024);row=rng.randrange(1,65);pitch=row+rng.randrange(1,128);count=rng.randrange(1,65)
    rows=[(base+i*pitch,base+i*pitch+row) for i in range(count)]
    bound=(base,base+(count-1)*pitch+row)
    lo=rng.randrange(0,bound[1]+200);hi=lo+rng.randrange(1,150)
    overlap=lambda x: x[0]<hi and lo<x[1]
    exact=any(map(overlap,rows))
    reference=any(lo<=at<hi for i in range(count) for at in range(base+i*pitch,base+i*pitch+row))
    assert exact==reference and (not exact or overlap(bound))
print('readbacks: 30,000 exact-row dependency checks; enclosing range remains conservative')
