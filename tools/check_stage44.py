"""Static launcher integration checks + independent pacing/queue reference models.
Run --hardware on Windows to execute the actual Rust test suite under the new profile.
No FPS claims. The Python portion does not compile Rust or execute D3D12.
"""
from pathlib import Path
import os, random, re, subprocess, sys
root = Path(__file__).resolve().parents[1]
launcher = (root / 'apps/victory/src/play_launcher.rs').read_text()
setup = (root / 'Setup-DexNomRuntime.ps1').read_text()
profile = dict(re.findall(r'\("(VICTORY_[A-Z_0-9]+)", "([0-9]+)"\)', launcher.split('const NVIDIA_PERFORMANCE:', 1)[1].split('];', 1)[0]))
assert len(profile) == 30, len(profile)
source = '\n'.join(p.read_text(errors='replace') for p in (root / 'crates').rglob('*.rs'))
for name in profile:
    assert f'"{name}"' in source, name
assert 'if !options.native_1440p { options.native_1080p = true; }' not in launcher
assert "Name = 'DexNom RayTime (NVIDIA Low Memory)'; Arguments = '--nvidia-performance --native-1080p'" in setup
assert "Name = 'DexNom RayTime (NVIDIA Baseline fallback)'; Arguments = '--nvidia-low-memory --native-1080p'" in setup
assert "Name = 'DexNom RayTime (NVIDIA Stock Display)'; Arguments = '--nvidia-performance'" in setup
assert "Name = 'DexNom RayTime (NVIDIA Performance)'; Arguments = '--nvidia-performance --native-1080p'" in setup
assert "Name = 'DexNom Shader Warmup (NVIDIA) - run first'; Arguments = '--nvidia-performance --native-1080p'" in setup
assert 'VICTORY_GPU_DETILE' not in profile
assert 'if nvidia_gpu_detile' in launcher
assert "if ($Diagnostics) {\n    $shortcuts += @{ Name = 'DexNom RayTime (NVIDIA Performance - diagnostic)'" in setup
assert "if ([IO.File]::Exists($diagnosticShortcut)) { [IO.File]::Delete($diagnosticShortcut) }" in setup
assert profile['VICTORY_WATCH_CLEAN_FAST'] == '1'
assert profile['VICTORY_TEXTURE_NO_WAIT'] == '0'
dxgi = (root / 'crates/victory-gpu/src/dxgi.rs').read_text()
host = (root / 'crates/victory-gpu/src/host.rs').read_text()
assert 'const AMD: u32 = 0x1002;' in dxgi
assert 'guest-facing Xbox adapter identity, not the physical host GPU' in dxgi
assert 'desc.VendorId != 0x10de' in host
assert 'set_host_max_wave(o1.WaveLaneCountMax)' in host
assert not {'VICTORY_WAVE_WIDE', 'VICTORY_RECLAIM_RAW_SLOTS', 'VICTORY_GPU_ONLY_TEXTURES'} & profile.keys()
assert profile['VICTORY_TEXTURE_STAGE_MB'] == '96' and profile['VICTORY_TEXTURE_STAGE_MS'] == '24'
low_memory_defaults = launcher.split('// Reuse existing dependency-aware parallel paths', 1)[1].split('] {', 1)[0]
assert '("VICTORY_CARD_WINDOWS_MB", "256")' in low_memory_defaults
assert '("VICTORY_TEXTURE_ROOM_PERCENT", "80")' in low_memory_defaults
assert '("VICTORY_STRICT_ADMISSION", "1")' in low_memory_defaults
assert 'if nvidia_low_memory && !nvidia_profile_log && std::env::var_os("VICTORY_LOG").is_none()' in launcher
assert 'if nvidia_low_memory && !nvidia_profile_log { "0" } else { "1" }' in launcher
host = (root / 'crates/victory-gpu/src/host.rs').read_text()
inplace = (root / 'crates/victory-gpu/src/inplace.rs').read_text()
assert 'options.GPUUploadHeapSupported.as_bool()' in host
assert '== D3D12_MEMORY_POOL_L1' in host
assert 'self.upload_buffer(size)' in host
assert 'fn level() -> u8' in inplace and 'VICTORY_INPLACE' in inplace
print(f'Static integration: {len(profile)} runtime switches found, display and GPU profile separation passed')
print('PC memory path: GPU-upload capability and local-pool checks, normal-upload fallback, and opt-in in-place heaps verified')

def allowance(ms, backlog):
    if not ms: return 32, 8
    if ms <= 40: return 16, 3
    if ms <= 100: return 24, 5
    if backlog > 2000: return 32, 8
    return 24, 5

def recover(ms, backlog):
    if ms > 100 and backlog > 2000:
        return max(32, min(96, ms//4)), max(8, min(24, ms//12))
    return allowance(ms, backlog)

rng = random.Random(44)
for _ in range(30000):
    ms, backlog = rng.randrange(2001), rng.randrange(50000)
    mb, time = recover(ms, backlog)
    assert 0 < mb <= 96 and 0 < time <= 24
    if ms <= 100 or backlog <= 2000: assert (mb,time) == allowance(ms,backlog)
    else: assert time <= ms//12
    retry = max(1, min(120, 2000//max(1, ms)))
    assert 1 <= retry <= 120
    if ms: assert retry*ms <= max(2000, ms)
print('30,000 independent pacing / retry model cases passed')

def select(retry, fresh, cap, frame, fair, bounded):
    retry, fresh = sorted(retry), sorted(fresh)
    if bounded:
        if fair and len(retry)>2*cap: retry=retry[:cap]+retry[-cap:]
        elif not fair and len(retry)>cap: retry=retry[-cap:]
        if len(fresh)>2*cap: fresh=fresh[:cap]+fresh[-cap:]
    from collections import deque
    retry,fresh=deque(retry),deque(fresh)
    turn = frame%2 == 0; newest=not fresh or not retry or frame&2 == 0
    # Rust's initial fresh direction is based on whether retries exist.
    newest=not retry or frame&2 == 0
    recent=not fair or frame&2 == 0
    result=[]
    while len(result)<cap and (retry or fresh):
        if retry and (turn or not fresh):
            result.append(retry.pop() if recent else retry.popleft())
            if fair: recent=not recent
        else:
            result.append(fresh.pop() if newest else fresh.popleft()); newest=not newest
        turn=not turn
    return result
for _ in range(20000):
    ids=list(range(rng.randrange(1,400))); rng.shuffle(ids)
    split=rng.randrange(len(ids)+1); retry,fresh=ids[:split],ids[split:]
    cap=rng.randrange(1,80); frame=rng.randrange(100)
    for fair in [True,False]:
        actual=select(retry,fresh,cap,frame,fair,True)
        assert actual==select(retry,fresh,cap,frame,fair,False)
        assert len(actual)==min(cap,len(ids)) and len(set(actual))==len(actual)
assert select(list(range(100)),[],1,6,True,True)==[0]
assert select(list(range(100)),[],1,6,False,True)==[99]
print('40,000 independent bounded queue comparisons passed')
if '--hardware' in sys.argv:
    if os.name != 'nt': raise SystemExit('Windows + NVIDIA required')
    env={k:v for k,v in os.environ.items() if not k.startswith('VICTORY_')}
    env.update(profile, VICTORY_NVIDIA='1', VICTORY_LOW_MEMORY='1', VICTORY_ADAPTIVE_BUDGET='1', VICTORY_REQUIRE_GPU_TEST='1')
    subprocess.run(['cargo','test','--locked','-p','victory-gpu','--features','diagnostics','--','--test-threads=1'], cwd=root, env=env, check=True)
    # Run exact parity test separately and require proof that it executed.
    env['VICTORY_RENDER_XOR_EXTRA']='1'
    cmd=['cargo','test','--locked','-p','victory-gpu','--features','diagnostics','texture::gpu_detile::tests::gpu_detile_readback_matches_cpu_rows','--','--exact','--nocapture']
    completed=subprocess.run(cmd,cwd=root,env=env,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
    print(completed.stdout)
    if completed.returncode or 'STAGE42_GPU_DETILE_PARITY_OK' not in completed.stdout: raise SystemExit('Real NVIDIA parity failed or did not run')
