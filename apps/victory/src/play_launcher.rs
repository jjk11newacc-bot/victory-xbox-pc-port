use anyhow::{Context, bail};
use std::ffi::c_void;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::windows::io::AsRawHandle;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "fsr_preset.rs"]
mod fsr_preset;
use fsr_preset::FsrPreset;

struct LaunchOptions {
    nvidia_low_memory: bool,
    nvidia_performance: bool,
    nvidia_gpu_detile: bool,
    nvidia_profile_log: bool,
    warp: bool,
    ray_tracing: Option<bool>,
    static_bvh: bool,
    console: &'static str,
    local_gpu_memory: bool,
    native_1440p: bool,
    native_1080p: bool,
    render_1080p: bool,
    fsr: Option<FsrPreset>,
    sparse_textures: bool,
    upload_ahead: Option<bool>,
    capture_readbacks: bool,
    rt_write_stats: bool,
    rt_store_trace: bool,
    lighting_trace: bool,
}

fn launcher_options(
    args: &mut std::iter::Peekable<impl Iterator<Item = std::ffi::OsString>>,
) -> anyhow::Result<LaunchOptions> {
    let mut options = LaunchOptions {
        nvidia_low_memory: false,
        nvidia_performance: false,
        nvidia_gpu_detile: false,
        nvidia_profile_log: false,
        warp: false,
        ray_tracing: None,
        static_bvh: false,
        console: "series-x",
        local_gpu_memory: false,
        native_1440p: false,
        native_1080p: false,
        render_1080p: false,
        fsr: None,
        sparse_textures: false,
        upload_ahead: None,
        capture_readbacks: false,
        rt_write_stats: false,
        rt_store_trace: false,
        lighting_trace: false,
    };
    while let Some(option) = args.peek() {
        if option == "--nvidia-low-memory" {
            options.nvidia_low_memory = true;
        } else if option == "--nvidia-performance" {
            options.nvidia_low_memory = true;
            options.nvidia_performance = true;
        } else if option == "--nvidia-gpu-detile" {
            options.nvidia_low_memory = true;
            options.nvidia_gpu_detile = true;
        } else if option == "--nvidia-profile-log" {
            options.nvidia_low_memory = true;
            options.nvidia_profile_log = true;
        } else if option == "--warp" {
            options.warp = true;
        } else if option == "--no-rt" {
            options.ray_tracing = Some(false);
        } else if option == "--rt" {
            options.ray_tracing.get_or_insert(true);
        } else if option == "--rt-static-bvh" {
            options.static_bvh = true;
        } else if option == "--local-gpu-memory" {
            options.local_gpu_memory = true;
        } else if option == "--native-1080p" {
            options.native_1080p = true;
        } else if option == "--native-1440p" || option == "--1080p-to-1440p" {
            options.native_1440p = true;
            options.render_1080p |= option == "--1080p-to-1440p";
        } else if let Some(preset) = option.to_str().and_then(|option| {
            if option == "--fsr-1440p-to-4k" {
                Some(FsrPreset::Quality)
            } else {
                option.strip_prefix("--fsr-").and_then(FsrPreset::from_name)
            }
        }) {
            if options.fsr.replace(preset).is_some_and(|old| old != preset) {
                bail!("Choose one FSR preset");
            }
        } else if option == "--sparse-textures" {
            options.sparse_textures = true;
        } else if option == "--upload-ahead" || option == "--no-upload-ahead" {
            let enable = option == "--upload-ahead";
            if options.upload_ahead.replace(enable).is_some_and(|old| old != enable) {
                bail!("Choose one texture upload mode");
            }
        } else if option == "--capture-readbacks" {
            options.capture_readbacks = true;
        } else if option == "--rt-write-stats" {
            options.rt_write_stats = true;
        } else if option == "--rt-store-trace" {
            options.rt_store_trace = true;
        } else if option == "--lighting-trace" {
            options.lighting_trace = true;
        } else if option == "--console" {
            args.next();
            options.console = match args.next().as_deref().and_then(std::ffi::OsStr::to_str) {
                Some("series-x") => "series-x",
                Some("series-s") => "series-s",
                _ => bail!("--console requires series-x or series-s"),
            };
            continue;
        } else {
            break;
        }
        args.next();
    }
    if options.nvidia_low_memory {
        if options.warp || options.ray_tracing == Some(true) || options.static_bvh
            || options.fsr.is_some() || options.upload_ahead == Some(true)
        {
            bail!("--nvidia-low-memory conflicts with WARP, RT, FSR 4K and upload-ahead");
        }
        options.ray_tracing = Some(false);
        // Display mode is independent of adapter selection. In particular,
        // --nvidia-low-memory alone must not patch an unknown retail image.
        options.local_gpu_memory = true;
        options.sparse_textures = true;
        options.upload_ahead = Some(false);
    }
    if options.static_bvh && options.ray_tracing.is_some() {
        bail!("Choose one mode from --rt, --no-rt, and --rt-static-bvh");
    }
    if options.native_1080p && (options.native_1440p || options.fsr.is_some()) {
        bail!("Choose native 1080p, a 1440p output mode, or FSR 4K");
    }
    if options.fsr.is_some() && options.native_1440p {
        bail!("Choose an FSR preset at 4K or a 1440p output mode");
    }
    if options.fsr.is_some() && options.console != "series-x" {
        bail!("FSR at 4K requires Series X mode");
    }
    Ok(options)
}

// Grouped opt-in CPU/memory profile. Keep risky reclamation, shader lowering,
// and extended GPU texture layouts independently testable. Explicit environment
// settings (including "0") override every entry in this table.
const NVIDIA_PERFORMANCE: &[(&str, &str)] = &[
    ("VICTORY_INDEXED_WATCH", "1"),
    ("VICTORY_WATCH_CLEAN_FAST", "1"),
    ("VICTORY_INDEXED_PENDING", "1"),
    ("VICTORY_INDEX_WRITE_GROUPS", "1"),
    ("VICTORY_BATCH_HAZARD_QUERY", "1"),
    ("VICTORY_BATCH_RAW_WORK", "1"),
    ("VICTORY_BATCH_PAGE_RANGES", "1"),
    ("VICTORY_SPARSE_MERGE", "1"),
    ("VICTORY_SETTLED_BLOCK_MERGE", "1"),
    ("VICTORY_SPARSE_REFRESH", "1"),
    ("VICTORY_SHARED_COMPARE_RUNS", "1"),
    ("VICTORY_PACK_SYNC_READBACK", "1"),
    ("VICTORY_REUSE_ROOT_DATA", "1"),
    ("VICTORY_REUSE_GRAPHICS_BINDINGS", "1"),
    ("VICTORY_COMPACT_MIP_UPLOAD", "1"),
    ("VICTORY_STICKY_RAM_PRESSURE", "1"),
    ("VICTORY_TRIM_READBACKS", "1"),
    ("VICTORY_STRICT_ADMISSION", "1"),
    ("VICTORY_STREAMING_FULL_BUDGET", "1"),
    ("VICTORY_STREAMING_SLOW_RECOVERY", "1"),
    ("VICTORY_MATERIAL_TEXTURE_DEMAND", "1"),
    ("VICTORY_CARD_WINDOWS_MB", "256"),
    ("VICTORY_TEXTURE_ROOM_PERCENT", "80"),
    ("VICTORY_TEXTURE_STAGE_MS", "24"),
    ("VICTORY_TEXTURE_STAGE_MB", "96"),
    ("VICTORY_TEXTURE_NO_WAIT", "0"),
    ("VICTORY_PARALLEL_PRELUDES", "0"),
    ("VICTORY_PARALLEL_DETILE", "0"),
    ("VICTORY_WIDE_REFRESH", "0"),
    ("VICTORY_WIDE_MERGE", "0"),
];

fn profile_value(name: &str, baseline: &'static str, performance: bool) -> &'static str {
    if performance {
        NVIDIA_PERFORMANCE.iter().find(|&&(key, _)| key == name).map_or(baseline, |&(_, value)| value)
    } else { baseline }
}

pub(super) fn run(warm: bool) -> anyhow::Result<()> {
    let launcher = std::env::current_exe()?;
    let folder = launcher.parent().context("Launcher folder is missing")?;
    let runtime = folder.join("victory.exe");
    let mut args = std::env::args_os().skip(1).peekable();
    let LaunchOptions {
        nvidia_low_memory, nvidia_performance, nvidia_gpu_detile, nvidia_profile_log, warp, ray_tracing, static_bvh, console, local_gpu_memory, native_1440p, native_1080p, render_1080p, fsr,
        sparse_textures, upload_ahead, capture_readbacks, rt_write_stats, rt_store_trace, lighting_trace,
    } = launcher_options(&mut args)?;
    if nvidia_profile_log && !cfg!(feature = "diagnostics") {
        bail!("--nvidia-profile-log requires a diagnostics build. Re-run Setup.cmd -Diagnostics to collect detailed counters.");
    }
    let game = match args.next() {
        Some(path) => PathBuf::from(path),
        None => {
            let config = folder.join("game-path.txt");
            let value = fs::read_to_string(&config)
                .with_context(|| format!("Put the game executable path in {}", config.display()))?;
            folder.join(value.trim())
        }
    };
    if args.next().is_some() {
        bail!(
            "Usage\n{} [--nvidia-low-memory | --nvidia-performance] [--nvidia-gpu-detile] [--nvidia-profile-log] [--rt | --no-rt | --rt-static-bvh] [--warp] [--console series-x|series-s] [--local-gpu-memory] [--native-1080p | --native-1440p | --1080p-to-1440p | --fsr-quality | --fsr-balanced | --fsr-performance] [--sparse-textures] [--upload-ahead | --no-upload-ahead] [--capture-readbacks] [--rt-write-stats] [--rt-store-trace] [--lighting-trace] [game executable]",
            launcher.display()
        );
    }
    if !game.is_file() || !runtime.is_file() {
        bail!(
            "Game or VIctory executable is missing.\n\n{}\n{}",
            game.display(),
            runtime.display()
        );
    }
    let game = std::path::absolute(game)?;
    let logs = folder.join("logs");
    let state = folder.join("state");
    let cache = std::env::var_os("VICTORY_CACHE_DIR")
        .filter(|v| !v.is_empty())
        .map_or_else(|| folder.join("cache"), |value| folder.join(value));
    fs::create_dir_all(&logs)?;
    fs::create_dir_all(&state)?;
    fs::create_dir_all(&cache)?;
    let _session = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(state.join("launcher.lock"))
        .context("Could not lock the launcher state. Check for another DexNom RayTime session.")?;
    let started = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let action = if warm { "warm" } else { "run" };
    let path = logs.join(format!("{action}-{started}-{}.log", std::process::id()));
    let mut log = OpenOptions::new()
        .append(true)
        .create_new(true)
        .open(&path)?;
    writeln!(
        log,
        "Runtime {}\nGame {}",
        runtime.display(),
        game.display()
    )?;
    let latest = if warm {
        "latest-warm.txt"
    } else {
        "latest.txt"
    };
    fs::write(logs.join(latest), path.display().to_string())?;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut child = Command::new(&runtime);
    if nvidia_low_memory {
        child.env("VICTORY_NVIDIA", "1")
            .env("VICTORY_LOW_MEMORY", "1")
            .env_remove("VICTORY_D3D12_WARP")
            .env("VICTORY_FSR_4K", "0")
            .env("VICTORY_NATIVE_1440P", "0")
            .env("VICTORY_NATIVE_1080P", "0")
            .env("VICTORY_RENDER_1080P", "0")
            .env("VICTORY_AMBIENT_FALLBACK", "1");
        // Bound background workers by the host's logical CPUs. On the target
        // i5-12400F, Windows normally reports 12 threads: performance play
        // gets two fill + two pipeline workers, while shader warmup may use
        // four pipeline workers. The six P-cores still run the guest and OS.
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let fill = if nvidia_performance { (cores / 4).clamp(1, 2) } else { cores.min(4) };
        let compile = if warm { (cores / 2).clamp(1, 4) } else { (cores / 4).clamp(1, 2) };
        if std::env::var_os("VICTORY_FILL_WORKERS").is_none() {
            child.env("VICTORY_FILL_WORKERS", fill.to_string());
        }
        writeln!(log, "VICTORY_FILL_WORKERS: {}", std::env::var("VICTORY_FILL_WORKERS").unwrap_or_else(|_| fill.to_string()))?;
        if nvidia_performance {
            if std::env::var_os("VICTORY_PIPELINE_WORKERS").is_none() {
                child.env("VICTORY_PIPELINE_WORKERS", compile.to_string());
            }
            writeln!(log, "VICTORY_PIPELINE_WORKERS: {}", std::env::var("VICTORY_PIPELINE_WORKERS").unwrap_or_else(|_| compile.to_string()))?;
        }
        for name in ["VICTORY_JOIN_COPIES", "VICTORY_WIDE_REFRESH", "VICTORY_TEXTURE_NO_WAIT"] {
            let value = profile_value(name, "1", nvidia_performance);
            if std::env::var_os(name).is_none() { child.env(name, value); }
            writeln!(log, "{name}: {}", std::env::var(name).unwrap_or_else(|_| value.to_owned()))?;
        }
        // Reuse existing dependency-aware parallel paths and budgeted VRAM windows.
        for (name, value) in [
            ("VICTORY_PARALLEL_PRELUDES", "1"),
            ("VICTORY_PARALLEL_DETILE", "1"),
            ("VICTORY_WIDE_MERGE", "1"),
            // Keep the baseline NVIDIA path inside a conservative 8 GiB
            // adapter budget too; the former 512 MiB windows and 85% texture
            // ceiling left less room for the game's render targets and driver.
            ("VICTORY_CARD_WINDOWS_MB", "256"),
            ("VICTORY_TEXTURE_ROOM_PERCENT", "80"),
            ("VICTORY_STRICT_ADMISSION", "1"),
            ("VICTORY_ADAPTIVE_BUDGET", "1"),
            ("VICTORY_TEXTURE_CATCHUP", "0"),
            ("VICTORY_TEXTURE_STAGE_MS", "8"),
            ("VICTORY_TEXTURE_STAGE_MB", "32"),
            ("VICTORY_EARLY_TEXTURES_MS", "3"),
            ("VICTORY_BAKE_TEXTURES_MS", "8"),
            ("VICTORY_LOCAL_GPU_INPUTS", "1"),
            ("VICTORY_RANGE_UNION", "1"),
        ] {
            let value = profile_value(name, value, nvidia_performance);
            if std::env::var_os(name).is_none() {
                child.env(name, value);
            }
            writeln!(log, "{name}: {}", std::env::var(name).unwrap_or_else(|_| value.to_owned()))?;
        }
        if nvidia_performance {
            writeln!(log, "NVIDIA performance profile; compare against baseline fallback if needed")?;
            for &(name, value) in NVIDIA_PERFORMANCE {
                // Baseline knobs already applied through profile_value above.
                if matches!(name, "VICTORY_CARD_WINDOWS_MB" | "VICTORY_TEXTURE_ROOM_PERCENT" | "VICTORY_STRICT_ADMISSION" | "VICTORY_TEXTURE_STAGE_MS" | "VICTORY_TEXTURE_STAGE_MB" | "VICTORY_PARALLEL_PRELUDES" | "VICTORY_PARALLEL_DETILE" | "VICTORY_WIDE_REFRESH" | "VICTORY_WIDE_MERGE") { continue; }
                if std::env::var_os(name).is_none() { child.env(name, value); }
                writeln!(log, "{name}: {}", std::env::var(name).unwrap_or_else(|_| value.to_owned()))?;
            }
        }
        if nvidia_gpu_detile {
            // Base detile only: additional layout and dirty-revision switches
            // require their own parity runs. No shader wave changes here.
            if std::env::var_os("VICTORY_GPU_DETILE").is_none() { child.env("VICTORY_GPU_DETILE", "1"); }
            writeln!(log, "VICTORY_GPU_DETILE: {}", std::env::var("VICTORY_GPU_DETILE").unwrap_or_else(|_| "1".into()))?;
        }
        writeln!(log, "NVIDIA low-memory profile; RT off; native display patch: {}", native_1080p || native_1440p)?;
    }
    if let Some(on) = ray_tracing {
        child.env("VICTORY_RAY_TRACING", if on { "1" } else { "0" });
        writeln!(log, "Ray tracing requested: {}", if on { "on" } else { "off" })?;
    } else if static_bvh {
        child.env("VICTORY_RAY_TRACING", "static-bvh");
        writeln!(log, "Ray tracing requested: static BVH traversal")?;
    }
    if warp {
        child.env("VICTORY_D3D12_WARP", "1");
    }
    if local_gpu_memory {
        child.env("VICTORY_LOCAL_GPU_INPUTS", "1");
    }
    if native_1080p {
        child.env("VICTORY_NATIVE_1080P", "1")
            .env("VICTORY_NATIVE_1440P", "0")
            .env("VICTORY_RENDER_1080P", "0")
            .env("VICTORY_FSR_4K", "0");
        writeln!(log, "Expected scene rendering 1920x1080, output 1920x1080")?;
    }
    if native_1440p {
        child.env("VICTORY_NATIVE_1080P", "0");
        child
            .env("VICTORY_NATIVE_1440P", "1")
            .env("VICTORY_RENDER_1080P", if render_1080p { "1" } else { "0" })
            .env("VICTORY_FSR_4K", "0");
        let expected_render = if render_1080p { "1920x1080" } else { "2560x1440" };
        writeln!(log, "Expected scene rendering {expected_render}, output 2560x1440")?;
    }
    if let Some(preset) = fsr {
        child.env("VICTORY_NATIVE_1080P", "0");
        child.env("VICTORY_FSR_4K", preset.name());
        let provider = folder.join("fidelityfx");
        if std::env::var_os("VICTORY_FIDELITYFX_DIR").is_none()
            && provider.join("amd_fidelityfx_upscaler_dx12.dll").is_file()
        {
            child.env("VICTORY_FIDELITYFX_DIR", provider);
        }
        let [width, height] = preset.render_size();
        writeln!(log, "FSR {} (SDK mode {}) requested at {width}x{height} scene, 3840x2160 output", preset.name(), preset.mode())?;
    }
    if nvidia_profile_log && std::env::var_os("VICTORY_MEMORY_PRESSURE_LOG").is_none() {
        child.env("VICTORY_MEMORY_PRESSURE_LOG", "1");
        writeln!(log, "VICTORY_MEMORY_PRESSURE_LOG: 1")?;
    }
    if nvidia_low_memory && !nvidia_profile_log && std::env::var_os("VICTORY_LOG").is_none() {
        child.env("VICTORY_LOG", std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()));
        writeln!(log, "Quiet NVIDIA run; use --nvidia-profile-log for diagnostic counters")?;
    } else if (fsr.is_some() || upload_ahead == Some(true)) && std::env::var_os("VICTORY_LOG").is_none() {
        let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into());
        child.env("VICTORY_LOG", format!(
            "{filter},victory::run::native_1440p=info,victory_hle::modules::fidelityfx=info,victory_gpu::host=info,victory_gpu::texture::ahead=info"
        ));
    }
    if let Some(enabled) = upload_ahead {
        let value = if enabled { "1" } else { "0" };
        child.env("VICTORY_UPLOAD_AHEAD", value);
        writeln!(log, "Texture copy queue requested: {}", if enabled { "on" } else { "off" })?;
    }
    for (enabled, key, message) in [
        (sparse_textures, "VICTORY_LEVELS_ON_DEMAND", "Sparse texture mips requested"),
        (capture_readbacks, "VICTORY_CAPTURE_READBACKS", "Dirty-page readback capture requested"),
        (rt_write_stats, "VICTORY_RT_WRITE_STATS", "Ray-tracing write statistics requested"),
    ] {
        if enabled {
            child.env(key, "1");
            writeln!(log, "{message}")?;
        }
    }
    if rt_store_trace {
        child.env("VICTORY_CAPTURE_RAW_STORES", victory_shader::emit::RAW_STORE_PROBE_HASH);
        writeln!(log, "InitHeaders store trace requested")?;
    }
    if lighting_trace {
        let trace = folder.join("lighting-trace").join(format!("{started}-{}", std::process::id()));
        fs::create_dir_all(&trace)?;
        let request = trace.join("request.txt");
        // Lighting using the shadow mask, RT resolve, auto-exposure, and the final composite.
        child
            .env("VICTORY_CAPTURE", &trace)
            .env("VICTORY_CAPTURE_REQUEST", &request)
            .env("VICTORY_DUMP_AFTER", "2bf91065944bcff4,7f05ddab771799a9,e622b718b6d9b057,6318f669e4f17d65")
            .env("VICTORY_DUMP_INPUT", "e622b718b6d9b057:s1+0");
        writeln!(log, "Lighting trace request file {}", request.display())?;
    }
    if std::env::var_os("VICTORY_DEBUG_HUD").is_none() {
        child.env("VICTORY_DEBUG_HUD", if nvidia_low_memory && !nvidia_profile_log { "0" } else { "1" });
    }
    // Include externally requested experimental flags too, so A/B runs can be reproduced.
    let mut overrides: Vec<_> = std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("VICTORY_")).collect();
    overrides.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, value) in overrides {
        let value = value.to_string_lossy();
        if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
            writeln!(log, "Inherited {}={value}", key.to_string_lossy())?;
        }
    }
    child
        .current_dir(folder)
        .env("VICTORY_DATA_DIR", &state)
        .env("VICTORY_CACHE_DIR", &cache)
        .stdin(Stdio::null())
        .stderr(log.try_clone()?)
        .creation_flags(CREATE_NO_WINDOW);
    let status = if warm {
        preload(&mut child, &game, &cache, &path, &mut log)?
    } else {
        let mut game_process = child
            .arg("run")
            .arg(&game)
            .args([
                "--execute",
                "--console",
                console,
                "--unimplemented",
                "trap",
            ])
            .stdout(log.try_clone()?)
            .spawn()
            .with_context(|| format!("Could not start {}", runtime.display()))?;
        let status = if std::env::var_os("VICTORY_MEMORY_GUARD").as_deref() == Some(std::ffi::OsStr::new("1")) {
            let Some(status) = wait_with_memory_guard(&mut game_process, &mut log)? else {
                return Ok(());
            };
            status
        } else {
            game_process.wait()?
        };
        writeln!(log, "VIctory exited with {status}")?;
        status
    };
    if !status.success() {
        bail!(
            "VIctory exited with {status}.\n\nLog file\n{}",
            path.display()
        );
    }
    Ok(())
}

/// Keep a test run from exhausting physical RAM and locking up the desktop.
/// This is deliberately opt-in because it can stop a title during loading.
/// Leave room to react before the hard stop without paging out the game
/// while Windows still has ample physical RAM.
fn working_set_trim_range(min_free_mib: u64) -> std::ops::Range<u64> {
    const TRIM_HEADROOM_MIB: u64 = 768;
    min_free_mib.saturating_sub(128)..min_free_mib + TRIM_HEADROOM_MIB
}

fn wait_with_memory_guard(child: &mut Child, log: &mut File) -> anyhow::Result<Option<ExitStatus>> {
    use windows_sys::Win32::System::ProcessStatus::{
        K32EmptyWorkingSet, K32GetPerformanceInfo, K32GetProcessMemoryInfo, PERFORMANCE_INFORMATION,
        PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    const TRIM_COOLDOWN: Duration = Duration::from_secs(8);
    const CRITICAL_TRIM_COOLDOWN: Duration = Duration::from_millis(750);
    let min_free_mib = std::env::var("VICTORY_MEMORY_GUARD_MIB")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|&value| (1024..=4096).contains(&value))
        .unwrap_or(2048);
    let trim_range = working_set_trim_range(min_free_mib);
    writeln!(log, "Memory guard threshold: {min_free_mib} MiB available")?;
    writeln!(log, "Working set trim threshold: {} MiB physical RAM available", trim_range.end)?;
    let process = child.as_raw_handle();
    let mut last_trim: Option<Instant> = None;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        // SAFETY: Windows fills this correctly sized plain data structure.
        let mut memory: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        memory.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        // SAFETY: `memory` is writable and has its size set above.
        let mut measured = unsafe { GlobalMemoryStatusEx(&mut memory) } != 0;
        let mut free_mib = (memory.ullAvailPhys >> 20) as u64;
        let mut perf = PERFORMANCE_INFORMATION {
            cb: size_of::<PERFORMANCE_INFORMATION>() as u32,
            ..Default::default()
        };
        // Commit headroom can be exhausted while physical pages remain
        // available (notably on machines with a small page file). Trimming a
        // working set does not release committed memory, so guard both pools.
        let mut commit_free_mib = (unsafe { K32GetPerformanceInfo(&mut perf, perf.cb) } != 0)
            .then(|| ((perf.CommitLimit.saturating_sub(perf.CommitTotal) * perf.PageSize) >> 20) as u64);
        let mut effective_free_mib = commit_free_mib.map_or(free_mib, |commit| free_mib.min(commit));
        let trim_cooldown = if free_mib < min_free_mib + 256 {
            CRITICAL_TRIM_COOLDOWN
        } else {
            TRIM_COOLDOWN
        };
        if measured
            && trim_range.contains(&free_mib)
            && last_trim.is_none_or(|at| at.elapsed() >= trim_cooldown)
        {
            last_trim = Some(Instant::now());
            let mut counters = PROCESS_MEMORY_COUNTERS {
                cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                ..Default::default()
            };
            // SAFETY: the child owns the process handle and both outputs are sized.
            let sampled = unsafe { K32GetProcessMemoryInfo(process, &mut counters, counters.cb) } != 0;
            // Give Windows a chance to reclaim resident pages before the guard trips.
            // This preserves the guest mappings and leaves the hard stop in place.
            // SAFETY: the process handle remains owned by the child.
            let trimmed = unsafe { K32EmptyWorkingSet(process) } != 0;
            let before_trim = free_mib;
            let before_commit = commit_free_mib;
            // SAFETY: reuse the same correctly sized writable output.
            measured = unsafe { GlobalMemoryStatusEx(&mut memory) } != 0;
            free_mib = (memory.ullAvailPhys >> 20) as u64;
            // SAFETY: the output buffer is writable and stores the supplied size.
            commit_free_mib = (unsafe { K32GetPerformanceInfo(&mut perf, perf.cb) } != 0)
                .then(|| ((perf.CommitLimit.saturating_sub(perf.CommitTotal) * perf.PageSize) >> 20) as u64);
            effective_free_mib = commit_free_mib.map_or(free_mib, |commit| free_mib.min(commit));
            // Keep guarding the live process even if the log becomes unwritable.
            let _ = writeln!(
                log,
                "Working set trim: success={trimmed}, physical available={before_trim} MiB, after={:?} MiB, commit headroom before={before_commit:?} MiB after={commit_free_mib:?} MiB, resident={:?} MiB, process commit={:?} MiB",
                measured.then_some(free_mib),
                sampled.then_some(counters.WorkingSetSize >> 20),
                sampled.then_some(counters.PagefileUsage >> 20)
            );
        }
        let physical_low = measured && memory.ullAvailPhys < (min_free_mib << 20);
        let commit_low = commit_free_mib.is_some_and(|free| free < min_free_mib);
        if !measured || physical_low || commit_low {
            let reason = if !measured {
                "Stopped the game because available RAM could not be measured".to_owned()
            } else if commit_low {
                format!("Stopped the game before Windows exhausted commit ({:?} MiB headroom, {free_mib} MiB physical RAM free)", commit_free_mib)
            } else {
                format!("Stopped the game before Windows ran out of RAM ({free_mib} MiB free)")
            };
            // Stop the child first. A logging error must not leave it running.
            if let Err(error) = child.kill() {
                if let Some(status) = child.try_wait()? {
                    return Ok(Some(status));
                }
                return Err(error).context("Could not stop the game after the memory guard triggered");
            }
            child.wait()?;
            writeln!(log, "{reason}")?;
            if !measured {
                anyhow::bail!("{reason}");
            }
            return Ok(None);
        }
        let poll_ms = if effective_free_mib < min_free_mib + 1536 { 100 } else { 250 };
        std::thread::sleep(Duration::from_millis(poll_ms));
    }
}

fn preload(
    child: &mut Command,
    game: &Path,
    cache: &Path,
    path: &Path,
    log: &mut File,
) -> anyhow::Result<ExitStatus> {
    let root = game.parent().context("Game install folder is missing")?;
    writeln!(
        log,
        "Scan {}\nCache {}\nThe offline reader recognizes SGD5 records and raw DXBC in files up to 2 GiB. Graphics pipelines are created when the game supplies their draw state.",
        root.display(),
        cache.display()
    )?;
    println!(
        "Preloading shaders and compute pipelines from {}\nLive log: {}",
        root.display(),
        path.display()
    );
    let mut worker = child
        .arg("warm")
        .arg(root)
        .stdout(Stdio::piped())
        .spawn()
        .context("Could not start DexNom RayTime shader preload")?;
    let stdout = worker
        .stdout
        .take()
        .context("Warmup output pipe is missing")?;
    let last =
        BufReader::new(stdout)
            .lines()
            .try_fold(None, |summary, line| -> anyhow::Result<_> {
                let line = line?;
                writeln!(log, "{line}")?;
                let _ = writeln!(std::io::stdout().lock(), "{line}");
                Ok(if line.starts_with("warm ") {
                    Some(line)
                } else {
                    summary
                })
            });
    let status = worker.wait()?;
    writeln!(log, "VIctory exited with {status}")?;
    if status.success()
        && let Err(message) = check_warm_summary(last?.as_deref())
    {
        writeln!(log, "{message}")?;
        bail!("{message}\n\nLog file\n{}", path.display());
    }
    Ok(status)
}

/// The runtime prints this after saving its pipeline library. Preserve its exact
/// counters in the log, including partial refusal; an empty scan is not a preload.
fn check_warm_summary(summary: Option<&str>) -> Result<(), &'static str> {
    let Some(summary) = summary else {
        return Err("VIctory did not report a shader warmup result");
    };
    let mut words = summary.split_whitespace();
    if words.next() != Some("warm")
        || words
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .is_none_or(|n| n == 0)
    {
        return Err("No shaders successfully translated. See the runtime totals in the log");
    }
    Ok(())
}

#[allow(dead_code)] // the console warmer reports errors to stderr
pub(super) fn show_error(text: &str) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(owner: *mut c_void, text: *const u16, title: *const u16, flags: u32) -> i32;
    }
    let text: Vec<_> = text.encode_utf16().chain([0]).collect();
    let title: Vec<_> = "DexNom RayTime".encode_utf16().chain([0]).collect();
    // SAFETY: the two strings are NUL terminated and live for the call.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x10) };
}

#[cfg(test)]
mod tests {
    use super::{check_warm_summary, launcher_options, working_set_trim_range};

    #[test]
    fn a_one_gib_guard_trims_only_near_the_physical_limit() {
        let trim = working_set_trim_range(1024);
        assert!(!trim.contains(&3007));
        assert!(trim.contains(&1500));
        assert!(trim.contains(&1024));
        assert!(!trim.contains(&800));
    }

    #[test]
    fn explicit_off_wins_in_either_order_and_keeps_the_game_path() {
        for (switches, expected) in [
            (vec![], None), (vec!["--rt"], Some(true)), (vec!["--no-rt"], Some(false)),
            (vec!["--rt", "--no-rt"], Some(false)), (vec!["--no-rt", "--rt"], Some(false)),
        ] {
            let mut args = switches.into_iter().chain(["--warp", "game path.exe"]).map(std::ffi::OsString::from).peekable();
            let options = launcher_options(&mut args).unwrap();
            assert!(options.warp);
            assert_eq!(options.ray_tracing, expected);
            assert_eq!(options.console, "series-x");
            assert!(!options.local_gpu_memory);
            assert!(!options.lighting_trace);
            assert!(!options.rt_store_trace);
            assert_eq!(options.upload_ahead, None);
            assert_eq!(args.next().as_deref(), Some(std::ffi::OsStr::new("game path.exe")));
        }
    }

    #[test]
    fn console_profile_is_explicit_and_preserves_the_game_path() {
        let mut args = ["--no-rt", "--console", "series-s", "--local-gpu-memory",
            "--sparse-textures", "--capture-readbacks", "--rt-write-stats", "--rt-store-trace", "--lighting-trace", "game path.exe"]
            .into_iter()
            .map(std::ffi::OsString::from)
            .peekable();
        let options = launcher_options(&mut args).unwrap();
        assert!(!options.warp);
        assert_eq!(options.ray_tracing, Some(false));
        assert_eq!(options.console, "series-s");
        assert!(options.local_gpu_memory);
        assert!(options.sparse_textures && options.capture_readbacks && options.rt_write_stats);
        assert!(options.lighting_trace);
        assert!(options.rt_store_trace);
        assert_eq!(args.next().as_deref(), Some(std::ffi::OsStr::new("game path.exe")));

        let mut invalid = ["--console", "one-x"]
            .into_iter()
            .map(std::ffi::OsString::from)
            .peekable();
        assert!(launcher_options(&mut invalid).is_err());
    }

    #[test]
    fn scene_upscale_uses_1440p_output_and_preserves_the_game_path() {
        for (switches, expected) in [
            (vec!["--native-1440p"], false),
            (vec!["--1080p-to-1440p"], true),
            (vec!["--native-1440p", "--1080p-to-1440p"], true),
            (vec!["--1080p-to-1440p", "--native-1440p"], true),
        ] {
            let mut args = switches
                .into_iter()
                .chain(["game path.exe"])
                .map(std::ffi::OsString::from)
                .peekable();
            let options = launcher_options(&mut args).unwrap();
            assert!(options.native_1440p);
            assert_eq!(options.render_1080p, expected);
            assert_eq!(args.next().as_deref(), Some(std::ffi::OsStr::new("game path.exe")));
        }
    }

    #[test]
    fn fsr_presets_use_the_series_x_4k_output() {
        use super::FsrPreset;
        for (switch, preset, queue, enabled) in [
            ("--fsr-1440p-to-4k", FsrPreset::Quality, "--upload-ahead", true),
            ("--fsr-quality", FsrPreset::Quality, "--upload-ahead", true),
            ("--fsr-balanced", FsrPreset::Balanced, "--upload-ahead", true),
            ("--fsr-performance", FsrPreset::Performance, "--upload-ahead", true),
            ("--fsr-performance", FsrPreset::Performance, "--no-upload-ahead", false),
        ] {
            let mut args = [switch, "--rt", queue, "--rt-store-trace", "game path.exe"]
                .into_iter()
                .map(std::ffi::OsString::from)
                .peekable();
            let options = launcher_options(&mut args).unwrap();
            assert_eq!(options.fsr, Some(preset));
            assert_eq!(options.upload_ahead, Some(enabled));
            assert!(options.rt_store_trace);
            assert!(!options.native_1440p && !options.render_1080p);
            assert_eq!(options.console, "series-x");
            assert_eq!(options.ray_tracing, Some(true));
            assert_eq!(args.next().as_deref(), Some(std::ffi::OsStr::new("game path.exe")));
        }

        for switches in [
            &["--fsr-1440p-to-4k", "--native-1440p"][..],
            &["--1080p-to-1440p", "--fsr-1440p-to-4k"],
            &["--console", "series-s", "--fsr-1440p-to-4k"],
            &["--fsr-performance", "--fsr-balanced"],
            &["--upload-ahead", "--no-upload-ahead"],
        ] {
            let mut invalid = switches.iter().copied().map(std::ffi::OsString::from).peekable();
            assert!(launcher_options(&mut invalid).is_err());
        }
    }

    #[test]
    fn nvidia_profile_sets_memory_policy_and_rejects_conflicts() {
        let mut args = ["--nvidia-low-memory", "game.exe"].into_iter().map(std::ffi::OsString::from).peekable();
        let options = launcher_options(&mut args).unwrap();
        assert!(options.nvidia_low_memory && options.local_gpu_memory && options.sparse_textures);
        assert_eq!(options.ray_tracing, Some(false));
        assert_eq!(options.upload_ahead, Some(false));
        assert!(!options.native_1080p && !options.native_1440p);
        assert_eq!(args.next().unwrap(), "game.exe");
        for flag in ["--warp", "--rt", "--rt-static-bvh", "--fsr-quality", "--upload-ahead"] {
            for switches in [["--nvidia-low-memory", flag], [flag, "--nvidia-low-memory"]] {
                let mut args = switches.into_iter().map(std::ffi::OsString::from).peekable();
                assert!(launcher_options(&mut args).is_err());
            }
        }
    }

    #[test]
    fn performance_and_gpu_profiles_are_independent() {
        for (flags, perf, gpu, logging) in [
            (vec!["--nvidia-performance"], true, false, false),
            (vec!["--nvidia-gpu-detile"], false, true, false),
            (vec!["--nvidia-performance", "--nvidia-gpu-detile", "--nvidia-profile-log"], true, true, true),
        ] {
            let mut args = flags.into_iter().chain(["--native-1080p", "game.exe"]).map(std::ffi::OsString::from).peekable();
            let options = launcher_options(&mut args).unwrap();
            assert!(options.nvidia_low_memory && options.native_1080p);
            assert_eq!((options.nvidia_performance, options.nvidia_gpu_detile, options.nvidia_profile_log), (perf, gpu, logging));
            assert_eq!(args.next().unwrap(), "game.exe");
        }
        assert_eq!(super::profile_value("VICTORY_CARD_WINDOWS_MB", "256", true), "256");
        assert_eq!(super::profile_value("VICTORY_CARD_WINDOWS_MB", "256", false), "256");
        assert_eq!(super::profile_value("VICTORY_GPU_DETILE", "0", true), "0");
        assert_eq!(super::profile_value("VICTORY_GPU_DETILE", "0", false), "0");
        assert_eq!(super::profile_value("VICTORY_TEXTURE_NO_WAIT", "1", true), "0");
        assert_eq!(super::profile_value("VICTORY_TEXTURE_NO_WAIT", "1", false), "1");
    }

    #[test]
    fn native_1080p_is_a_distinct_output_mode() {
        let mut args = ["--nvidia-low-memory", "--native-1080p", "game.exe"].into_iter().map(std::ffi::OsString::from).peekable();
        let options = launcher_options(&mut args).unwrap();
        assert!(options.native_1080p);
        assert!(!options.native_1440p && !options.render_1080p);
        assert_eq!(args.next().unwrap(), "game.exe");
        for other in ["--native-1440p", "--1080p-to-1440p", "--fsr-quality"] {
            for switches in [["--native-1080p", other], [other, "--native-1080p"]] {
                let mut args = switches.into_iter().map(std::ffi::OsString::from).peekable();
                assert!(launcher_options(&mut args).is_err());
            }
        }
    }

    #[test]
    fn empty_and_incomplete_scans_are_reported() {
        assert!(check_warm_summary(None).is_err());
        assert!(
            check_warm_summary(Some(
                "warm  0 translated, 0 refused; 0 compute pipelines, 0 refused; 0 s"
            ))
            .is_err()
        );
        assert!(
            check_warm_summary(Some(
                "warm  0 translated, 2 refused; 0 compute pipelines, 0 refused; 1 s"
            ))
            .is_err()
        );
        assert!(
            check_warm_summary(Some(
                "warm  1 translated, 2 refused; 3 compute pipelines, 4 refused; 5 s"
            ))
            .is_ok()
        );
    }
}
