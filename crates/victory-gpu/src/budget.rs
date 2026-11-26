//! Experimental coordinated admission and streaming pacing. No live resource eviction.
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;
const RAM_PRESSURE: u8 = 1;
const RAM_CRITICAL: u8 = 2;

fn ram_pressure_for(available: Option<u64>, low_memory: bool) -> u8 {
    let Some(available) = available else { return 0 };
    let (pressure, critical) = if low_memory { (6 * GIB, 3 * GIB) } else { (4 * GIB, 2 * GIB) };
    if available < critical { RAM_CRITICAL }
    else if available < pressure { RAM_PRESSURE }
    else { 0 }
}

/// Host RAM headroom tier. The PC profile starts trimming caches earlier
/// because guest, host and Windows allocations compete for system DDR4.
pub(crate) fn ram_pressure(available: Option<u64>) -> u8 {
    static LOW_MEMORY: OnceLock<bool> = OnceLock::new();
    let low_memory = *LOW_MEMORY.get_or_init(|| std::env::var_os("VICTORY_LOW_MEMORY").is_some_and(|v| v == "1"));
    ram_pressure_for(available, low_memory)
}

pub(crate) fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_ADAPTIVE_BUDGET").is_some_and(|v| v == "1"))
}
/// Experimental additional admission checks; off unless explicitly requested.
pub(crate) fn strict_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_STRICT_ADMISSION").is_some_and(|v| v == "1"))
}
/// Keep retirement cache limits in force between pressure sweeps.
pub(crate) fn sticky_pressure() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_STICKY_RAM_PRESSURE").is_some_and(|v| v == "1"))
}
pub(crate) fn ceiling(budget: u64, percent: u64) -> u64 {
    (budget / 100 * percent).min(budget.saturating_sub(1024 * MIB))
}
pub(crate) fn pool_cap(budget: u64) -> u64 { (budget / 5).min(2048 * MIB) }
pub(crate) fn window_cap(budget: u64) -> u64 { budget / 16 }
struct Pacing { last: Option<Instant>, average_ms: u64, frame: u64, spent_us: u64 }
static PACING: Mutex<Pacing> = Mutex::new(Pacing { last: None, average_ms: 0, frame: u64::MAX, spent_us: 0 });
pub(crate) fn present() {
    if !enabled() { return; }
    let mut p = PACING.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    if let Some(last) = p.last {
        let ms = now.duration_since(last).as_millis().clamp(1, 2000) as u64;
        p.average_ms = if p.average_ms == 0 { ms } else { (p.average_ms * 7 + ms) / 8 };
    }
    p.last = Some(now);
}
fn recovery_allowance(ms: u64, backlog: u64) -> (u64, u64) {
    // Allow limited recovery in a badly starved streaming scene, but never
    // spend the old 1.5 s catch-up budget. Fast frames keep the original cap.
    if ms > 100 && backlog > 2000 {
        ((ms / 4).clamp(32, 96), (ms / 12).clamp(8, 24))
    } else { allowance(ms, backlog) }
}
fn allowance(ms: u64, backlog: u64) -> (u64,u64) {
    if ms == 0 { (32, 8) } else if ms <= 40 { (16, 3) } else if ms <= 100 { (24, 5) } else if backlog > 2000 { (32, 8) } else { (24, 5) }
}
pub(crate) fn slow_recovery_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_STREAMING_SLOW_RECOVERY").is_some_and(|v| v == "1"))
}
fn retry_frames_for(ms: u64) -> u64 { (2000 / ms.max(1)).clamp(1, 120) }
/// Queue retries use presented frames rather than the number of staging passes.
pub(crate) fn retry_frames(fallback: u64) -> u64 {
    if !enabled() || !slow_recovery_enabled() { return fallback; }
    let p = PACING.lock().unwrap_or_else(|e| e.into_inner());
    if p.average_ms == 0 { fallback.min(16) } else { retry_frames_for(p.average_ms) }
}
pub(crate) fn remaining(frame: u64, backlog: u64) -> (u64,u64) {
    let mut p = PACING.lock().unwrap_or_else(|e| e.into_inner());
    if p.frame != frame { p.frame = frame; p.spent_us = 0; }
    let recover = slow_recovery_enabled();
    let (mb, ms) = if recover { recovery_allowance(p.average_ms, backlog) } else { allowance(p.average_ms, backlog) };
    (mb * MIB, (ms * 1000).saturating_sub(p.spent_us))
}
pub(crate) fn charge(frame: u64, us: u64) {
    let mut p = PACING.lock().unwrap_or_else(|e| e.into_inner());
    if p.frame == frame { p.spent_us = p.spent_us.saturating_add(us); }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn limits() { assert_eq!(ceiling(8<<30, 90), 7<<30); assert_eq!(ceiling(512*MIB,90),0); assert_eq!(window_cap(8<<30),512*MIB); assert!(pool_cap(8<<30)<2<<30); }
    #[test] fn slow_recovery_is_bounded() {
        assert_eq!(recovery_allowance(33, 10000), (16, 3));
        assert_eq!(recovery_allowance(1000, 100), (24, 5));
        assert_eq!(recovery_allowance(1000, 10000), (96, 24));
        assert_eq!(recovery_allowance(120, 10000), (32, 10));
        assert_eq!(retry_frames_for(1000), 2);
        assert_eq!(retry_frames_for(250), 8);
        assert_eq!(retry_frames_for(33), 60);
        assert_eq!(retry_frames_for(1), 120);
    }
    #[test] fn pacing() { assert_eq!(allowance(33,3000),(16,3)); assert_eq!(allowance(200,3000),(32,8)); }
    #[test] fn pc_profile_trims_caches_with_more_headroom() {
        assert_eq!(ram_pressure_for(None, true), 0);
        assert_eq!(ram_pressure_for(Some(7 * GIB), true), 0);
        assert_eq!(ram_pressure_for(Some(5 * GIB), true), RAM_PRESSURE);
        assert_eq!(ram_pressure_for(Some(3 * GIB), true), RAM_PRESSURE);
        assert_eq!(ram_pressure_for(Some(2 * GIB), true), RAM_CRITICAL);
        assert_eq!(ram_pressure_for(Some(5 * GIB), false), 0);
        assert_eq!(ram_pressure_for(Some(3 * GIB), false), RAM_PRESSURE);
        assert_eq!(ram_pressure_for(Some(1 * GIB), false), RAM_CRITICAL);
    }
}

/// Include queue maintenance, rechecks, selection and early-return work in
/// the existing adaptive time budget. Scope-based accounting also handles
/// failed staging attempts. No draw or mandatory remap is skipped here.
pub(crate) struct StreamingCharge { frame: u64, started: Option<Instant> }
impl StreamingCharge {
    pub(crate) fn begin(frame: u64, backlog: u64) -> Self {
        static ON: OnceLock<bool> = OnceLock::new();
        let active = enabled() && *ON.get_or_init(|| std::env::var_os("VICTORY_STREAMING_FULL_BUDGET").is_some_and(|v| v == "1"));
        let started = active.then(Instant::now);
        if active { let _ = remaining(frame, backlog); }
        Self { frame, started }
    }
    pub(crate) fn active(&self) -> bool { self.started.is_some() }
    pub(crate) fn elapsed_us(&self) -> u64 {
        self.started.map_or(0, |at| at.elapsed().as_micros() as u64)
    }
    pub(crate) fn remaining_us(&self, backlog: u64) -> u64 {
        remaining(self.frame, backlog).1.saturating_sub(self.elapsed_us())
    }
}
impl Drop for StreamingCharge {
    fn drop(&mut self) {
        if self.active() { charge(self.frame, self.elapsed_us()); }
    }
}
