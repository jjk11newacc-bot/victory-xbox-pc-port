"""Independent byte-copy model for extended GPU detile; not Rust/HLSL execution."""
import random, re
from pathlib import Path
rng = random.Random(33)
src = (Path(__file__).resolve().parents[1] / 'crates/victory-formats/src/texture.rs').read_text()
patterns = re.findall(r'b"([xyz0-9-]{16})"', src.split('const MICRO:',1)[1].split('fn pattern',1)[0])
assert len(patterns) == 15
cases = words = 0
for _ in range(600):
    n = rng.choice([4, 8, 16]); log = n.bit_length()-1
    tile_log = rng.choice([8, 12, 16]); tile = 1 << tile_log
    pat = patterns[rng.randrange(2)*5+log]
    axes = [[(int(pat[2*b+1]), b) for b in range(8) if pat[2*b] == a] for a in 'xy']
    for bit in range(8,tile_log):
        a = axes[(bit-8+1)%2]; a.append((len(a),bit))
    bw,bh = [1 << len(a) for a in axes]
    def spread(v,a): return sum(((v>>i)&1)<<b for i,b in a)
    # Distinct bases exercise mip offsets and array-slice offsets. Nonzero
    # shifted coordinates model packed mip tails as in Layout::row/column.
    jobs=[]; end=0
    for j in range(rng.randint(2,6)):
        w,h = rng.randint(1,65),rng.randint(1,35)
        tx,ty = rng.randrange(bw),rng.randrange(bh)
        blocks = (tx+w+bw-1)//bw + 1
        base = rng.randrange(1,64)*tile
        def addr(x,y):
            x+=tx;y+=ty
            return base+((y//bh)*blocks+x//bw)*tile+spread(x%bw,axes[0])+spread(y%bh,axes[1])
        origin=addr(0,0); cols=[addr(x,0) for x in range(w)]; rows=[addr(0,y) for y in range(h)]
        bound=max(cols)+max(rows)-origin+n
        pitch=((w*n+255)//256)*256; out=(end+511)//512*512; end=out+pitch*h
        jobs.append((w,h,n,cols,rows,origin,bound,pitch,out,[[addr(x,y) for x in range(w)] for y in range(h)]))
    output=bytearray([0xcc])*end; expected=bytearray(output)
    def val(at): return ((at*31)^(at>>8)^(at>>16))&255
    for w,h,n,cols,rows,origin,bound,pitch,out,ref in jobs:
        for y in range(h):
            for x in range(w):
                at=cols[x]+rows[y]-origin
                assert at==ref[y][x] and at+n<=bound and at%4==0
                pos=out+y*pitch+x*n
                expected[pos:pos+n]=bytes(val(ref[y][x]+k) for k in range(n))
                for sub in range(0,n,4):
                    bx=x*n+sub; offset=cols[bx//n]+rows[y]-origin+bx%n
                    dest=out+y*pitch+bx
                    output[dest:dest+4]=bytes(val(offset+k) for k in range(4));words+=1
                cases+=1
    assert output==expected
print(f'{cases:,} element and {words:,} dword comparisons passed across 600 multi-job plans; no Rust/HLSL execution')
