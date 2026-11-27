# VIctory loading and memory findings

The stock Series X display path renders the loading artwork on the RTX 3060 Ti. Gameplay remains unverified. The initial launch stopped at **1,942 MiB of available physical RAM**, below the launcher's 2,048 MiB guard threshold. The log records a controlled stop. The guard measures RAM available to all of Windows.

## What the runtime changes fixed

- The FSPX compute pipeline handler and shader translator now handle the captured startup shader, five sampler descriptor failures, and the later lane-mask failure. The shaders produced host Direct3D 12 pipelines in the recorded runs.
- The runtime converts supported typeless texture views to concrete Windows formats. The title bar displays dedicated VRAM use and the launcher logs the reason for a controlled low-memory exit.
- The native 1080p path opens after correcting the scanout allocation, but its captured frames are a flat color before Windows presents them. The installed launch profile uses the stock display path.

## What remains

- During stock loading, dedicated VRAM rose to about 2.3 GiB and private process commit to about 9.1 GiB in a diagnostic run. Both the game and Windows use system memory. GPU memory usage alone does not explain the system RAM drop.
- Graphics pipeline method 0x1c8 is now mapped to the graphics-pipeline handler, and the old “method not implemented” warnings are gone. A live pipeline containing vertex, pixel, domain, hull, and geometry shaders is still rejected because the translator only emits vertex, pixel, mesh, and compute stages. The run log also shows multiple compute PSOs rejected by the host with `0x80070057`. The descriptor heap still caps at 1,000,000 while the title requests 1,245,184 views.
- The newer texture staging build can use local GPU upload memory under RAM pressure. The device and texture copy checks passed. This change was included in the separate test build. Its effect on the loading peak was not isolated from the other optimizations.

## Optimization and verification

The current source also reduces the pool of completed GPU command lists when system RAM is low. Active lists and their resources remain owned until the GPU is done with them. Compare the same stock Series X launch with the same guard threshold, and measure the minimum available RAM, peak private commit, dedicated VRAM use, and the last frame reached. Reaching a playable scene is the completion check.

Evidence: runtime logs `run-1795750036314-16796.log` (initial launch) and `run-1795749124133-16804.log` (memory samples).

## Follow-up runs

The optimized executable reached the Series X loading spinner and stopped at 1,970 MiB free. At the spinner the runtime reported 1,878 MiB of VRAM. The launcher now requests an early working-set trim on guarded runs and logs physical memory and system commit separately. The first trim test stopped at 1,922 MiB free. Starting reclamation earlier briefly raised available physical memory to 4,885 MiB, but system commit headroom was only 355 MiB at that sample. The guard subsequently stopped the run at 1,746 MiB free.

Series S held around 6.4 GiB of available system memory for two minutes, but the window stayed black. The test exited normally when closed. Series X progressed farther and exposed more shader translation failures, so reducing RAM pressure and reaching a playable scene still need separate verification.

The new opt-in file-backed path applies to Xbox physical page pools of at least 512 MiB on Windows. It uses a sparse temporary file with shared writable views and automatic cleanup. When backing-file creation fails, the runtime uses the original pagefile-backed path. Disk writes and startup time may increase.

The file-backed Series X test mapped pools of 710 MiB and 6,236 MiB and showed about 3 GiB of system commit headroom early in loading. It progressed into a later burst of compute pipeline requests but still stopped at 2,013 MiB of available RAM. A repeat stopped at 1,891 MiB. Both runs removed the temporary files at exit.

The repeat's virtual address census measured 10,038 MiB of committed private regions just before the spike and 15,267 MiB roughly three seconds later. The large low-address guest reservations held steady and no single new private region accounted for the increase. Virtual address snapshots do not identify allocator call sites, and mapped ranges may be aliases. The next attribution needs allocation stacks or counters around the runtime's heap and GPU resource creation during the later load. The same interval logged unsupported compute shader instructions and resource layouts, so the remaining render failures are independently visible.

Evidence: `latest-test-page-backing/logs/run-1795751142908-3084.log`, `run-1795751264463-18100.log` and its `.regions.txt` snapshot.

## Host command-list allocation

The low-memory profile now acquires host D3D12 command lists when recording starts. An empty list can be closed and reset without creating a driver allocation. When a submitted list is detached, its host list stays with the submission until completion. Its title list waits until the next recording to acquire a replacement.

With the same Series X profile, cache, file backing and RAM guard, the baseline reported 9,437 MiB of private commit and 5,442 MiB resident at the first working-set trim. The rebuilt runtime reported 8,891 MiB of private commit and 5,070 MiB resident. These are single-run measurements taken at comparable available-RAM thresholds. Later loading still drove private commit to at least 14,920 MiB, and the guard closed the rebuilt runtime at 1,934 MiB available.

Evidence: `run-1795751763709-10728.log` (baseline) and `run-1795752022981-6428.log` (rebuilt runtime), with its `.memory.jsonl` samples.

Limiting pipeline compilation to one worker still triggered the guard at 1,855 MiB available. The worker setting was applied only for that test.

The diagnostic build recorded 13 live Windows command lists, 980 MiB of live Rust heap allocations, and zero MiB of list scratch and list upload buffers at the last frame sample before the spike. Private commit then went from approximately 9,260 MiB to 15,277 MiB in about 3.5 seconds. Frame samples stopped during that interval, so the ownership of the extra allocations is still unknown. Windows Performance Recorder denied access to its VirtualAllocation profile (0x80070005).

Evidence: `latest-test-memory-diagnostics/logs/run-1795752654354-13920.log` and its `.memory.txt` samples.

## Dedicated GPU upload memory

The RTX 3060 Ti has 8,192 MiB of VRAM. The command-list test reported 3,421 MiB of dedicated usage before the late memory spike, so 2,400 MiB is not the adapter's cap. D3D12's local and non-local budget queries distinguish dedicated GPU memory from system RAM.

The host now uses the existing budgeted local GPU upload allocator for the shader-visible guest-address table and for buffer-to-texture command-list staging under RAM pressure. The CPU only writes those mappings and the GPU reads them. Normal upload heaps remain the fallback when the hardware, allocation size or budget precludes local upload memory. Diagnostic builds report cumulative bytes allocated through this path. This count measures allocation traffic rather than residency.

The live Series X test confirmed a local GPU upload allocation. At about 66 seconds, it had 6,708 MiB of available system RAM, 8,643 MiB of private process commit, and 2,047 MiB of dedicated VRAM use. Private commit reached 13,808 MiB less than four seconds later. The guard stopped the run at 1,994 MiB of available system RAM, before gameplay. The extra private commit has not been attributed to any of the GPU upload paths.

Evidence: `latest-test-local-upload/logs/run-1795753020984-14296.log` and its `.memory.jsonl` samples.

The first 1 GiB guard run stopped at 1,002 MiB free. Lowering the guard had also moved the trim trigger from 4,352 to 3,328 MiB available. The rebuilt launcher now starts reclamation below 4,352 MiB with either guard setting.

The RTX VRAM test profile preserves the 1 GiB hard stop and raises the texture admission ceiling from 80% to 85% of the DXGI budget. The combined test passed the previous stop point and peaked at over 6,000 MiB of dedicated VRAM. The test window was black at 2:10, so increased residency did not establish that the scene rendered correctly. The test changed both reclamation timing and the texture budget, so their effects were not isolated.

Evidence: `run-1795753477896-12044.log` (first 1 GiB run), `run-1795753627494-17708.log` and its `.vram.jsonl` samples (RTX VRAM test).

## Longer-run optimization

The prior 85% VRAM run lasted about 6 minutes 40 seconds and stopped at 1,011 MiB free. A late sample measured 198.7 ms per frame in CPU-side copy preparation and 4,611 indexed texture uploads deferred by the VRAM budget. The user reported that this run reached gameplay.

The next profile enables sparse buffer refresh coverage and idle readback and pipeline registry reclamation. It typically excluded 650–840 MiB of address coverage per sampled frame in the heavy scene. This is scheduling coverage, not actual transferred bytes. That run reached roughly 6 GiB of dedicated VRAM and stopped after about 4 minutes, at 1,021 MiB free. The scene was blank when inspected and there was no proven frame-rate improvement.

The next executable avoids allocating a gap vector when there are no outstanding GPU writes that affect a buffer refresh. Its launcher can repeat a trim every two seconds while within 768 MiB of the hard stop. Away from that range the existing eight-second cooldown applies.

Evidence: `run-1795753627494-17708.log` and `run-1795754113219-11216.log`.

The rebuilt runtime and launcher were tested together with these settings. Dedicated VRAM peaked at 6,291 MiB. A late sample showed about 5 FPS and roughly 18 GiB of private commit. The run stopped around four minutes in at 1,007 MiB of available RAM. The grouped profile remains available as `Play - RTX Optimized Test.cmd`, and the previous executables were backed up in the test folder.

## Stage 52 shader and commit findings

The latest full diagnostic run (`run-1795783007758-14168.log`) recorded 378 compute shader translation failures, one graphics pipeline refused because its hull/domain/geometry stages are unsupported, and several compute pipelines refused by D3D12 with `0x80070057`. The most frequent compute blockers are descriptors that were not traced to a load/build, descriptors changed after loading, descriptor words read as data, and wave/lane operations the current lowering does not support. These failures drop the corresponding dispatches or draws and remain rendering blockers.

That run later hit a 35,651,600-byte allocation failure with about 203 MiB of system commit headroom. The opt-in memory guard previously watched available physical RAM and could miss this condition after trimming. It now uses the lower of physical RAM and system commit headroom for its trim/hard-stop decision. A guarded repeat stopped at 549 MiB commit headroom with 9,885 MiB physical RAM free (`run-1795783663329-14236.log`), before the allocator failure. This protects Windows; it does not reduce the title's memory demand or make the game playable.

The stage-dump hook is now at pipeline bytecode intake so unsupported stages can be captured before host-pipeline validation. The guarded run stopped before reaching the five-stage pipeline, so its dump contains compute, vertex, and pixel containers only; it did not yet provide the missing hull/domain/geometry payloads. The current game-rendering issue remains open.

Evidence: `run-1795754458319-14700.log` and its `.vram.jsonl` samples.

## Scratch and texture staging reclamation

The idle command-list scratch pool now shrinks from 128 MiB to 32 MiB under moderate RAM pressure and 8 MiB under critical pressure. Leased scratch stays owned by its command list. The memory-and-FPS test profile also enables reclamation of completed texture staging and the indexed write journal. It keeps the 85% texture budget and 1 GiB RAM guard.

The release build ran about 6 minutes 8 seconds, up from about 4 minutes in the immediately preceding test. Dedicated VRAM peaked at 6,810 MiB versus 6,296 MiB in the preceding run's full trace. About 997 MiB of staging leases were released cumulatively. This is allocator traffic, not a simultaneous RAM saving. A guard sample reached 18,810 MiB of private commit and the guard stopped the game at 1,011 MiB available. The window was black when inspected.

For log samples with at least 1,000 draws per frame, median CPU copy preparation was 62.1 ms versus 93.6 ms previously. Median FPS was 6.3 versus 6.0. These are different sets of frames, so the comparison is indicative rather than a controlled gameplay benchmark.

Evidence: `run-1795754919113-7476.log` and its `.vram.jsonl` samples.

## Imported-save memory recovery test

The guard samples every 100 ms near the configured stop. A trim is followed by a fresh memory measurement and may recover a reading within 128 MiB below the stop before the process is closed. Deferred texture admission waits for up to 128 MiB of extra local headroom before restarting, while allocations within a running admission pass use the ordinary ceiling. The title bar reports dedicated usage, the DXGI local budget and shared GPU memory separately.

The updated Company Cars profile reached in-game interaction prompts and committed a new SARC0015 autosave. The world was dark behind the prompts. By 4:52 the runtime reported 6,683 MiB dedicated and about 4,737 MiB shared. Shared usage peaked at 5,053 MiB in the trace, and private commit remained around 18–19 GiB in the heavy scene. These measurements show continued memory pressure despite surviving the previous loading stop.

Evidence: `ps5-story-save-20261127-103729/logs/run-1795756393059-1892.log` and its `.memory.jsonl` trace.
