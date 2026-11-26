# Stage 44: audit of Stage 43 against “victory stage41 possible issues.txt”

Target: RTX 3060 Ti 8 GB / 16 GB DDR4 / native 1080p / RT off. This is source, not a runnable game or a compiled Windows build. No FPS estimate is a benchmark. The attached report is a static review of Stage 41; its observations do not automatically apply to Stage 43. No Windows, Cargo, GPU or game execution here.

## Changes

1. `apps/victory/src/play_launcher.rs`: remove the implicit native-1080p choice for the NVIDIA low-memory flag. Existing main shortcut explicitly requests 1080p, stock shortcut now genuinely bypasses native-resolution patch (and may cost more memory). Game executable hash/patch checks remain unchanged. Parser tests cover stock, 1080p and independent profile flags. Add `--nvidia-performance`, `--nvidia-gpu-detile`, `--nvidia-profile-log`. The performance profile opts into 28 existing/new knobs (see `NVIDIA_PERFORMANCE` in the source), including indexed watch/pending, batched raw/page work, sparse merging and refresh, shared comparisons, packed readback, root/binding reuse, compact mips, sticky RAM pressure, readback trimming, strict admission, full streaming accounting and slow recovery. Explicit environment values override the table. Riskier raw-slot reclamation, wave changes and expanded GPU texture modes remain separate; no blanket enable-all. The separate GPU flag opts into only base detile and preserves CPU fallback. Quiet profile defaults to warn (unless RUST_LOG/VICTORY_LOG override) and HUD off; diagnostic flag restores info and memory diagnostics. Numeric inherited runtime settings and profile settings are logged. The experimental bundle is not a verified combination; compare one switch at a time against original baseline if it fails or regresses.
2. `budget.rs`: optional bounded recovery when frame average >100 ms and backlog >2000. Up to 96 MiB/24 ms instead of 32 MiB/8 ms, no 1.5 s catch-up, fast-frame allowance unchanged. These are admission/time-loop budgets, not hard limits on any single texture or frame. This can improve texture arrival but may increase work in a slow frame. New retry cap uses presented-frame average rather than intervals between staging passes (roughly two seconds, max 120 frames; not a hard wall-clock deadline). Original policy retained when flag off.
3. `draw.rs`, `descriptor/host.rs`: use new retry cap; optionally alternate oldest/newest retries, including when only one candidate fits, preserving fresh/retry alternation and bounded candidate selection. Do not discard pending descriptors or remove waits. Still does not guarantee that unstreamed game data becomes available. Adds direct Rust queue regression test.
4. Performance profile uses 256 MiB card windows, 80% texture budget, limited fill and compile workers, disables wide refresh/merge and parallel preludes/detile to test less CPU contention. Existing adaptive budget already derives sparse pool limits from live adapter budget; strict admission is now available through opt-in profile. These are heuristics, not a global VRAM hard cap. Fewer workers can reduce throughput; smaller card windows and budgets may cause texture pop-in or more transfers. Override individually if needed. Material demand is opt-in and may itself add CPU/GPU work; turn off if slower.
5. `Setup-DexNomRuntime.ps1`: install four additional shortcuts (performance, diagnostic, base GPU detile, performance warmup), default to two build jobs, configurable `-BuildJobs`. Does not reduce live guest memory. `tools/Validate-Stage44.ps1` runs Stage 43 actual tests then static/model checks and actual GPU tests with the new profile; marker required for NVIDIA parity. This was not run in this environment. Existing baseline shortcuts and fallback preserved.

## Report item-by-item disposition

| ID | Stage 44 disposition |
|---|---|
| A1 | Confirmed; original profile kept for A/B; explicit performance shortcut turns on selected improvements. Individual gates still default off without the profile. High-risk gates intentionally separate. |
| A2 | CPU bottleneck plausible; no new measurement. Batched/indexed/binding flags now usable together but no 29x claim. |
| A3 | Unresolved: no measured gameplay speedup. |
| A4 | Selective paths enabled in experimental profile; unwatched/uncertain memory still needs comparison. |
| A5 | Experimental profile opts into root/binding reuse and batching. |
| A6 | Packed/shared readback selected; true dependencies still require fences. |
| A7 | Stage 43 added paired exchange and fixes; divergent cross-wave and device removal unresolved; no forced wide-wave setting. |
| A8 | Necessary barriers retained. Removing them without proof risks GPU corruption. |
| A9 | Opt-in profile reduces concurrency and disables several competing wide paths; actual CPU topology and effect unmeasured. |
| A10 | Quiet profile default, diagnostic mode available. Baseline keeps original logging. |
| A11 | Window allocation reduced to 256 MiB in experimental profile; still not zero-copy. |
| A12 | Diagnostics still used for setup; overhead unmeasured. |
| A13 | Hardware-dependent; no changes to BIOS/CPU/RAM. |
| B1 | RT disabled and ambient fallback retained; not an exact lighting replacement. |
| B2 | Upstream black materials unresolved; fair queue and material-demand hints may improve streaming-related cases but not shader/material bugs. |
| B3 | Bounded slow-scene recovery and shared full streaming budget opt-in. |
| B4 | Remap no-wait still trades latency for delayed refresh; use VICTORY_TEXTURE_NO_WAIT=0 for parity. |
| B5 | Shorter retry cap at low FPS and fair retry scheduling; partial data protection and existing 500 ms partial poll preserved. |
| B6 | Report's 2 GiB figure is a maximum; Stage 43 already uses adaptive live budget. Experimental profile adds strict admission, 80% room, 256 MiB windows. No guaranteed total cap. |
| B7 | Separate base GPU detile shortcut, CPU fallback. |
| B8 | Actual shader/driver parity still unverified; extended flags not enabled indiscriminately. |
| B9 | Exact executable hash check deliberately retained. |
| B10 | Confirmed and fixed parser behavior; stock shortcut no longer patches display. |
| B11 | UI/viewport and TAA compatibility still need actual gameplay. |
| B12 | Risky reclamation gates not bundled. Retired readback trim and sticky RAM controls opt in; actual lifetime tests pending. |
| B13 | Indexed watch/pending opt in via profile, not validated in the game. |
| B14 | Verified-table setup already exists, hash checks preserved; actual download and build pending. |
| C1 | Profile caps compile workers; same warmup entry, no guarantee every graphics state precompiles. No unsafe compute-draw skipping. |
| C2 | Improved optional reclaim and fewer workers; working set target still unverified. |
| C3 | Sticky RAM pressure available through profile. |
| C4 | No forced wide-wave or larger GPU detile profile; TDR risk remains. No OS timeout modifications. |
| C5 | Earlier reclamation retained; leak-free operation not proven. Need repeated scene/long soak. |
| C6 | Bounded recovery and present-average retry policy; smoothing and clamping remain. |
| C7 | Reduced workers and wide paths in profile; measure against original. |
| C8 | Quiet logging; SSD availability external. |
| C9 | Windows settings/driver/Resizable BAR external. |
| D1 | Cargo missing; compile still blocked. |
| D2 | Direct Rust tests added and Windows runner updated; not executed here. |
| D3 | Stage 43 model rerun passed; Stage 40 timeout not resolved by these tests. |
| D4 | Earlier unconditional synchronization fixes retained; hardware correctness not verified. |
| D5 | Setup limits build jobs; pinned Rust/MSVC/SDK prerequisites remain. |
| E1 | Hardware capability requirement unchanged. |
| E2 | NVIDIA compatibility still unproven. |
| E3 | Historical reference machine not equivalent; no promised FPS. |
| F | Baseline/experimental/GPU/diagnostic shortcuts and actual-test runner available; real A/B remains mandatory. |

## Suggested validation

On Windows install pinned Rust/MSVC/SDK and verified format tables; execute `powershell -File tools/Validate-Stage44.ps1` before using the experimental profile. Run original NVIDIA warmup and baseline at 1080p, save log and scene; compare diagnostic performance profile with same camera and cache state, not quiet vs verbose. Compare individual overrides if needed. Base GPU detile is a separate test; do not enable all extension flags at once. Use original profile to roll back; set explicit environment value 0 to disable a single gate. Verify textures, UI, device errors, dedicated/shared VRAM, private bytes/working set, frame-time distribution, and long-session memory growth. No test of actual FPS, textures, leak-free operation or RAM consumption was possible here. Do not use the experimental profile as a certified fix.
