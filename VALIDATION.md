# Validation — 26 November 2026

- PASS: Windows MSVC target Rust type-check for all application binaries and application test targets, using the repository-pinned Rust 1.98.1 toolchain:
  `cargo check --locked --target x86_64-pc-windows-msvc -p victory --features diagnostics,blake3/pure --bins --tests`
- The first attempt with the normal BLAKE3 assembly backend could not run because Linux has no MSVC ml64.exe. The successful check enables BLAKE3's existing pure Rust feature only for validation; Cargo.toml and the Windows setup build flags are unchanged.
- PASS: `python tools/check_nvidia_profile.py`: nine actual launcher/snapshot tests run in an isolated Linux harness; the three snapshot tests also pass with VICTORY_LOW_MEMORY=1. The harness extracts the functions and tests from the production files, rather than reimplementing them. It does not execute Windows adapter selection or graphics calls.
- PASS: `git diff --check`.
- BCPack tables were fetched from the setup script's pinned upstream location and verified against SHA-256 1f1d4760419202bb5672f0c428ce51e456b5d08fbaec8f371d043fd94e01c36b for checking. They remain excluded from the delivered archive, consistent with the original asset policy. Setup downloads them.
- NOT TESTED: linked Windows release binaries, PowerShell setup execution, D3D12 GPU execution, NVIDIA shader correctness, rendered output, total RAM/VRAM usage, and FPS. No game assets or RTX hardware were available.

This is an experimental source patch. There is no claim of playable performance or a verified 16 GB total-memory fit.

## Native 1080p and boundary-scan revision

- PASS: Windows target `cargo check --locked --target x86_64-pc-windows-msvc -p victory --features diagnostics,blake3/pure --bins --tests` after these changes.
- PASS: 12 isolated Rust tests for launch options, snapshot retention, and synthetic display image fields. The display tests exercise native 1080p, native 1440p, 1080p-to-1440p, scanout reservation, scene scale, viewport displacement, wrong hashes and unexpected fields. The three pool tests also pass in a fresh process with VICTORY_LOW_MEMORY=1.
- The Linux harness copies the actual PreparedImage declaration because the full loader currently includes Windows-only crash code on Linux. It uses the real ImageInfo mapping and texture-layout implementation. No game image is substituted or accepted by the runtime's hash check.
- PASS: two additional boundary-search tests covering block edges, empty/tail/unaligned buffers, multiple changes, randomized patterns, and unequal lengths against a scalar oracle.
- Executed the optimized CPU microbenchmark; results and limitations are in PERFORMANCE-FINDINGS.md.
- No linked Windows release, real game, GPU rendering or 30 FPS validation was performed.
