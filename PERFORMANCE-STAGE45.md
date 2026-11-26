# Stage 45: NVIDIA low-memory defaults and runtime overhead

Target: RTX 3060 Ti (8 GB VRAM), 16 GB system RAM, native 1080p, ray tracing off.

This stage follows a static audit of the Stage 44 source against `victory stage41 possible issues.txt`. The issue list is project context; it does not replace the user's request to improve the Xbox-to-PC port.

## Changes

1. The ordinary NVIDIA low-memory launch now limits card-backed windows to 256 MiB (previously 512 MiB), uses an 80% texture admission threshold (previously 85%), and enables strict admission. This leaves more adapter budget available to render targets and the driver on an 8 GB card. Explicit environment values still take precedence.
2. Every NVIDIA low-memory launch now defaults to `warn` logging and disables the debug HUD. `--nvidia-profile-log` restores the diagnostic HUD and memory-pressure samples in a diagnostics-enabled build (`Setup.cmd -Diagnostics`); Setup only creates the diagnostic shortcut for that build. An explicit `VICTORY_LOG`, `RUST_LOG`, or `VICTORY_DEBUG_HUD` still overrides the defaults.
3. The Stage 44 static integration checker asserts these memory and quiet-mode defaults.
4. Setup's main NVIDIA Low Memory shortcut now activates the optimized profile; the NVIDIA shader warmup uses the same profile. A separate baseline fallback shortcut retains the previous smaller switch set for comparison and rollback. The stock-display shortcut also uses the optimized profile without the native-resolution patch.
5. Base GPU texture detiling is available through the explicit `--nvidia-gpu-detile` flag, with CPU fallback for ineligible textures. It is no longer enabled by the default NVIDIA profile until GPU-to-CPU parity passes on the target RTX 3060 Ti. Extended, partial, dirty-page, volume, and linear special modes remain gated.
6. The optimized profile sets `VICTORY_TEXTURE_NO_WAIT=0`, preventing refresh deferrals during guest-memory remaps. The baseline fallback preserves the prior no-wait behavior for A/B comparison; the optimized choice may wait longer but aims to avoid stale texture revisions.
7. The low-memory profile now uses shared RAM-pressure tiers across cache maintenance. On the 16 GB PC profile, cache trimming starts below 6 GiB available RAM and critical reclaim starts below 3 GiB; normal launches retain 4/2 GiB thresholds. Shader/snapshot caches and idle readbacks trim earlier, while reusable upload chunks are preserved until critical pressure to avoid needless recreation or extra system-memory staging.

## Remaining target-specific work

These changes address concrete launch, memory-pressure and texture-refresh policies. They do not establish an FPS gain, a hard total-memory cap, correct rendering for every game build, or leak-free long sessions. The runtime still duplicates guest resources across system and local GPU memory where PC access requires it. Performance and image correctness need validation with the actual game executable, Windows toolchain, and RTX 3060 Ti.

Run `powershell -File tools/Validate-Stage44.ps1` on the target Windows machine after initializing the verified format tables. Then compare the same settled scene and camera using NVIDIA Low Memory and NVIDIA Baseline fallback. Record median/p95 frame time, CPU preparation, GPU waits, texture work, dropped draws, process RAM, dedicated VRAM, and shared GPU memory. Keep the optimized profile only if the capture improves without texture, UI, lighting, or stability regressions.
