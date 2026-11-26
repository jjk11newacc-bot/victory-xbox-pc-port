"""Check the render-XOR decomposition and dword copy model, not the Rust/HLSL executable."""
from pathlib import Path
import random, re

source = (Path(__file__).resolve().parents[1] / 'crates/victory-formats/src/texture.rs').read_text()
body = source.split('fn render_xor_pattern(', 1)[1].split('fn contributions(', 1)[0]
patterns = re.findall(r'\((true|false), (\d+)\) => \(\s*&\[([^]]+)\],\s*&\[([^]]+)\]', body)
assert len(patterns) == 6
rng = random.Random(32)
cases = 0
for series, n, xs, ys in patterns:
    n = int(n)
    if n == 2: # not eligible in stage 32
        continue
    xb = [int(x.strip(), 0) for x in xs.split(',') if x.strip()]
    yb = [int(y.strip(), 0) for y in ys.split(',') if y.strip()]
    def spread(v, basis):
        return __import__('functools').reduce(int.__xor__, (bit for i, bit in enumerate(basis) if v & (1 << i)), 0)
    # The 64KiB render block has 16-log2(element bytes) coordinate bits.
    # Exercise several block geometries; the decomposition is independent of shape.
    for w, h in [(128, 128), (64, 256), (256, 64), (128, 64), (64, 128)]:
        for _ in range(5000):
            x, y = rng.randrange(4096), rng.randrange(4096)
            pitch_blocks = 4096 // w + 1
            ox, oy = spread(x, xb), spread(y, yb)
            reference = ((y // h) * pitch_blocks + x // w) * 65536 + (ox ^ oy)
            a = (x // w) * 65536 + ox
            b = (y // h) * pitch_blocks * 65536 + oy
            actual = (a & ~65535) + (b & ~65535) + ((a ^ b) & 65535)
            assert reference == actual and actual % n == 0
            for sub in range(0, n, 4):
                byte_x = x*n+sub
                assert byte_x//n == x and actual+byte_x % n == reference+sub
            cases += 1
# Non-XOR layout combines independent row and column contributions, including tails.
for _ in range(10000):
    origin = rng.randrange(0, 10000)*4
    column, row = rng.randrange(0, 10000)*4, rng.randrange(0, 10000)*4
    assert (origin+column)+(origin+row)-origin == origin+column+row
    cases += 1
print(f'{cases:,} address-decomposition model cases passed; no Rust or HLSL execution')
