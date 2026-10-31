//! The machine PhotoCraft runs on: how much memory it has, for budgets that scale with it (the
//! GPU canvas budget, a `.pcraft` load's decoded-bytes budget, #8595).

/// Physical memory of this machine in bytes, if known. `PHOTOCRAFT_RAM_MB` overrides it (to
/// simulate a smaller machine).
pub fn physical_memory() -> Option<u64> {
    static RAM: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *RAM.get_or_init(|| {
        if let Some(mb) = std::env::var("PHOTOCRAFT_RAM_MB").ok().and_then(|v| v.trim().parse::<u64>().ok()) {
            return mb.checked_mul(1 << 20);
        }
        detect_physical_memory()
    })
}

#[cfg(target_os = "macos")]
fn detect_physical_memory() -> Option<u64> {
    let out = std::process::Command::new("/usr/sbin/sysctl").args(["-n", "hw.memsize"]).output().ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

#[cfg(target_os = "linux")]
fn detect_physical_memory() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = info.lines().find_map(|l| l.strip_prefix("MemTotal:"))?.trim().trim_end_matches("kB").trim().parse().ok()?;
    kb.checked_mul(1024)
}

#[cfg(target_os = "freebsd")]
fn detect_physical_memory() -> Option<u64> {
    let out = std::process::Command::new("/sbin/sysctl").args(["-n", "hw.physmem"]).output().ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

#[cfg(windows)]
fn detect_physical_memory() -> Option<u64> {
    winsafe::GlobalMemoryStatusEx().map(|m| m.ullTotalPhys).ok().filter(|b| *b > 0)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd", windows)))]
fn detect_physical_memory() -> Option<u64> {
    None
}
