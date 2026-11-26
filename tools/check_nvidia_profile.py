"""Run the actual platform-independent launcher/pool tests without Windows.
Requires Python 3 and Cargo. Generated harnesses live in a temporary directory.
"""
from pathlib import Path
import os
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
launcher = (root / 'apps/victory/src/play_launcher.rs').read_text()
shadow = (root / 'crates/victory-gpu/src/shadow.rs').read_text()
with tempfile.TemporaryDirectory(prefix='victory-profile-') as tmp:
    project = Path(tmp)
    (project / 'src').mkdir()
    (project / 'Cargo.toml').write_text('''[package]
name = "victory-profile-checks"
version = "0.0.0"
edition = "2024"
[dependencies]
anyhow = "=1.0.104"
''')
    manifest = project / 'Cargo.toml'
    with manifest.open('a') as f:
        for name in ['victory-pe', 'victory-formats']:
            f.write(f'{name} = {{ path = "{(root / "crates" / name).as_posix()}" }}\n')
        f.write('blake3 = "=1.8.7"\ntracing = "=0.1.44"\n')
    display = (root / 'apps/victory/src/native_1440p.rs').read_text()
    # Exclude the existing optional retail-image test; no game image is supplied.
    display = display[:display.index('    #[test]\n    fn fsr_presets_leave')] + '}\n'
    (project / 'src/native_1440p.rs').write_text(display)
    (project / 'src/fsr_preset.rs').write_text((root / 'apps/victory/src/fsr_preset.rs').read_text())
    options = launcher[launcher.index('struct LaunchOptions {'):launcher.index('pub(super) fn run(')]
    summary = launcher[launcher.index('fn check_warm_summary('):launcher.index('#[allow(dead_code)]')]
    launcher_tests = launcher[launcher.index('#[cfg(test)]\nmod tests {'):]
    pool = shadow[shadow.index('#[derive(Default)]\nstruct SnapshotPool'):shadow.index('static POOL:')]
    pool_tests = shadow[shadow.index('#[cfg(test)]\nmod snapshot_pool_tests'):shadow.index('const NOT_WATCHED:')]
    loader = (root / 'crates/victory-loader/src/prepare.rs').read_text()
    image_type = loader[loader.index('#[derive(Debug, Clone)]'):loader.index('impl PreparedImage')]
    (project / 'src/lib.rs').write_text('extern crate self as victory_loader;\nuse victory_pe::ImageInfo;\n' + image_type + 'mod native_1440p;\nuse anyhow::bail;\nuse std::collections::BTreeMap;\nmod fsr_preset;\nuse fsr_preset::FsrPreset;\n' + options + summary + launcher_tests + pool + pool_tests)
    subprocess.run(['cargo', 'test', '--manifest-path', str(project / 'Cargo.toml')], check=True)
    env = dict(os.environ, VICTORY_LOW_MEMORY='1')
    subprocess.run(['cargo', 'test', '--manifest-path', str(project / 'Cargo.toml'), 'snapshot_pool_tests'], env=env, check=True)
