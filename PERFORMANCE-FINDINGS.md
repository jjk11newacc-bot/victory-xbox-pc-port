# Why the archive reports 1–4 FPS

## Evidence in the supplied archive

BENCHMARK.md reports a City run on an RX 7900 XTX with translated RT enabled:

| Metric | Earlier implementation | Implementation after an upstream write-tracking fix |
| --- | ---: | ---: |
| Mean frame time | 3,469 ms | 969 ms |
| Guest memory outside registered buffers compared each frame | 12,793 MB | 2,655 MB |
| Wait for guest memory reads each frame | 2,036 ms | 95 ms |
| Draws each frame | 12,583 | 12,520 |

These are the author's measurements, not measurements made here. The documented write-tracking fix is already present in this ZIP. It must not be presented as a new fix from this patch. Other entries report 391 MB/frame texture uploads during a streaming period and pipeline compilation on cold starts. Those are different captures and should not be added together into a synthetic frame-time breakdown.

The main architectural costs are broad guest-memory write declarations, CPU/GPU synchronization and copies, translated shader work (including compute-based ray traversal), and streaming/compilation. See host.rs submission/readback paths, shadow.rs snapshots/merge paths, texture.rs staging and pipeline/ray_tracing.rs. This is a compatibility runtime, not a conventional native PC port with freely relocatable game allocations.

30 FPS requires 33.3 ms/frame. The reported 969 ms average would need about a 29-fold reduction. Lower resolution alone cannot remove CPU memory scanning, submission overhead or dependency waits. NVIDIA selection provides no speed benefit on a PC whose only GPU is already the 3060 Ti.

## Changes in this revision

1. Native 1920x1080 rendering AND output via `--native-1080p`, selected by the NVIDIA Low Memory shortcut. The previous shortcut rendered 1080p into 1440p output. Scanout allocation, display reporting, UI correction and patched-image cache are updated together. Existing game-build hash and field checks are retained.
2. Faster searches for exact first/last changed bytes in snapshot merge paths. Equal blocks are compared 64 bytes at a time; endpoint changes return immediately. This does not remove comparisons or change the bytes published back to guest memory.
3. Enable the existing bounded parallel upload-fill path for the NVIDIA profile (up to four workers). Dependent operations and GPU queue ordering remain unchanged.
4. Retain RT off, ambient fallback, sparse textures, supported/budget-checked GPU upload heaps and the lower idle snapshot-cache cap from the previous revision.

## Local CPU microbenchmark

The included `tools/bench-memory.rs` compares the original boundary search with the new one. Each case uses two 64 KiB CPU buffers and 100,000 calls in an optimized Linux Rust binary. This is a small hot-buffer microbenchmark, not a GPU, streaming, full-frame or Windows benchmark. It does not measure the entire memory-comparison workload.

```text
equal first: before=392863us after=360344us ratio=1.09x
equal last: before=360907us after=314421us ratio=1.15x
middle first: before=242310us after=177441us ratio=1.37x
middle last: before=196550us after=158396us ratio=1.24x
last first: before=355461us after=296017us ratio=1.20x
last last: before=216us after=162us ratio=1.33x
first first: before=243us after=162us ratio=1.50x
first last: before=306450us after=295965us ratio=1.04x

```

In this run the helper improved approximately 1.04–1.50x across these cases. This is NOT a 1.04–1.50x gameplay FPS claim. Real gains may be small, absent, or different on the user's CPU. The benchmark is included for reproduction, not as proof of 30 FPS.

## What remains unknown

There is no game image, game data, Windows execution environment, or RTX 3060 Ti here. Runtime shader compatibility, actual 1080p rendering, memory fit, lighting and FPS remain untested. A confirmed 30 FPS fix cannot be claimed. The new memory helper and launcher/display-field tests do pass; Windows target type-checking passes.

The NVIDIA Low Memory shortcut now runs the optimized profile with quiet logging and the debug HUD disabled by default. For counters, first install a diagnostics-enabled build with `Setup.cmd -Diagnostics`; Setup then creates the `NVIDIA Performance - diagnostic` shortcut (`--nvidia-profile-log`). Use the log named by `%LOCALAPPDATA%\DexNom Runtime\logs\latest.txt`; it enables per-frame CPU preparation, copies, texture work, GPU waits, copy-back, dropped draws, process RAM and memory pressure reporting. Capture a settled scene after initial compilation and streaming. Compare against the NVIDIA Baseline fallback with the same save and camera. This evidence guides further optimization without disabling necessary synchronization or corrupting game state.
