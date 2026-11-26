# Stage 48: remaining FPS, texture, and render audit

This pass continued the Stage 41 checklist review against the current launcher, texture upload and detile paths, write-watch refresh, RAM/VRAM budgeting, setup script, and the Xbox-to-PC memory notes. The checklist is treated as hypotheses; hardware-dependent entries are not represented as confirmed bugs.

## Changes

- Removed GPU texture detiling from the default NVIDIA performance profile. Base GPU detile now requires the explicit `--nvidia-gpu-detile` experimental option. Extended layout modes remain separately gated, and ineligible inputs retain the CPU fallback. This favors known texture output and reduces unvalidated GPU dispatch/TDR exposure; it may increase CPU texture preparation time until parity and timing are verified on the RTX target.
- Enabled the conservative clean-page write-watch fast path in the NVIDIA profile. It skips page refresh work only when every covered page has an initialized copied generation at least as new as the newest journaled write. Any uninitialized/stale page falls through to the prior refresh path. A regression test covers current, stale, uninitialized, empty and no-write ranges.
- Made the default Setup build omit `diagnostics` instrumentation. Detailed HUD/counters require `Setup.cmd -Diagnostics`. Setup now creates the diagnostic shortcut only for that build and removes the stale shortcut when returning to a normal release build. The launcher also rejects profile logging in a non-diagnostics binary with an actionable message.
- Updated Stage 45 and performance notes to state the diagnostic build requirement accurately. The Stage 44 static checker now asserts the detile opt-in and diagnostic-shortcut behavior.

## Checklist disposition

| Area | Disposition |
| --- | --- |
| Profile switches disabled by default (A1) | Addressed in the NVIDIA profile for the existing, source-audited optimizations. GPU detile remains opt-in because its GPU parity has not been run on the target card. |
| CPU cost and guest-memory comparisons (A2-A4) | Reduced by existing indexed/write-journal paths and the new provable clean-page fast path. Broad comparisons for unwatched or uncertain memory remain necessary for correctness; no source-only fix can establish frame-time gain. |
| Draw, readback and barriers (A5-A8) | Existing binding reuse/readback optimizations remain active where configured. Required readback dependencies and conservative UAV ordering remain. Removing them without a scene-specific proof risks stale data or corrupted output. NVIDIA wave behavior remains unverified; risky wide-wave overrides remain off. |
| Worker contention and diagnostic overhead (A9-A10, A12) | NVIDIA profile avoids parallel prelude/detile oversubscription; default release omits diagnostics; worker counts are bounded. Detailed counters remain available in a separate diagnostics build. |
| Discrete PC memory path (A11, B6, C2-C3) | Stages 46-47 separate host RAM, local VRAM and shared/non-local budgets; optional local mirrors fail closed without a DXGI budget. This adapts guest memory semantics but cannot make separate PC pools unified or guarantee a total 16 GB fit. |
| Lighting/material appearance (B1-B2) | RT-off fallback can change lighting; black materials are an upstream/game-specific issue with no reproducer/assets here. No generic source fix established. |
| Streaming and texture freshness (B3-B5) | Existing bounded recovery increases the per-frame allowance under severe backlog; performance profile waits for stable remapped guest memory. Partial-map retries retain the old image until data is ready. Late mip detail remains possible while CPU/GPU work is overloaded. |
| Detiling, display patches, write tracking (B7-B14) | CPU detile is now the default. GPU detile is opt-in until parity. Native display patches remain exact-build gated. Sparse/indexed/reclamation paths remain guarded by their current correctness checks; no new reclamation was enabled. BCPack download hash verification remains. |
| Compilation, long sessions, pacing (C1, C4-C7) | Shader warmup and bounded pacing remain. Removing default GPU detile reduces one unverified dispatch source. Long-session leak behavior and real TDR risk need target soak testing; no blanket synchronization or device timeout changes were made. |
| Build, GPU, system and hardware claims (D1-E3, F) | Not verifiable here without the Rust/MSVC toolchain, Windows D3D12 runtime, game assets and RTX 3060 Ti. SSD, driver, power plan, CPU/RAM speed and target game build are machine-specific. |

## Validation

- PASS: PowerShell parser reports no syntax errors for `Setup-DexNomRuntime.ps1`.
- PASS: PowerShell source assertions confirm the 30-entry NVIDIA profile, no automatic GPU detile, the clean-write switch, the launcher diagnostics guard, and conditional diagnostic shortcut handling.
- PASS: Manual source review confirms GPU detile still has eligibility and RAM/VRAM checks and CPU fallback; optional local mirrors still require a successful local-budget query.
- NOT RUN: Python `tools/check_stage44.py`, Rust compilation/tests, Windows linking, D3D12 parity, game rendering, FPS/frame-time capture, target memory pressure, or long-session soak. This environment has no usable Python, Cargo/Rust, MSVC compiler, game executable or target GPU. A partial rustup download attempt was stopped without installing global tools.

## Target-PC checks still required

Build normally with `Setup.cmd`; use `Setup.cmd -Diagnostics` for measurement. Warm pipelines, then compare the same scene/save/camera and resolution in the NVIDIA Performance and Baseline fallback profiles. Record median and p95 frame time, CPU preparation/guest-memory refresh time, GPU waits and copy-back, texture residency/streaming, private RAM, host available RAM, local VRAM budget/use and shared GPU memory. Check UI, materials, texture edges/mips, device-removal messages, and at least a long gameplay session. Run GPU detile parity separately before enabling `--nvidia-gpu-detile`.

## Primary memory references

- [Xbox Wire: Series X memory capacity and bandwidth regions](https://news.xbox.com/en-us/2020/03/16/xbox-series-x-tech/)
- [Microsoft GDK: Xbox resource guarantees and memory model](https://learn.microsoft.com/en-us/gaming/gdk/docs/gdk-dev/intro/introduction?view=gdk-2604)
- [Microsoft Learn: D3D12 architecture and UMA properties](https://learn.microsoft.com/en-us/windows/desktop/api/d3d12/ns-d3d12-feature-data-architecture)
- [Microsoft Learn: D3D12 residency budgets](https://learn.microsoft.com/en-us/windows/win32/direct3d12/residency)
- [Microsoft DirectX specification: D3D12 resource heaps and aliases](https://microsoft.github.io/DirectX-Specs/d3d/ResourceHeaps.html)
