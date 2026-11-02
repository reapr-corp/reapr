//! CPU and memory of an engine's process tree, read around the engine's child process.

use crate::Resources;

/// Cumulative usage of all waited-for children and their waited-for descendants
/// (`getrusage(RUSAGE_CHILDREN)`). `max_rss_kb` is a high-water mark, not a sum. `None` where it cannot
/// be measured (Windows).
#[cfg(unix)]
pub fn children_usage() -> Option<Resources> {
    // SAFETY: `rusage` is plain C data for which all-zero bytes are valid; `getrusage` only writes
    // into the struct we pass, which outlives the call.
    let (rc, ru) = unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        (libc::getrusage(libc::RUSAGE_CHILDREN, &mut ru), ru)
    };
    if rc != 0 {
        return None;
    }
    // time_t / suseconds_t width varies by platform; both are non-negative here.
    let ms = |tv: libc::timeval| (tv.tv_sec as u64) * 1_000 + (tv.tv_usec as u64) / 1_000;
    let max_rss = u64::try_from(ru.ru_maxrss).unwrap_or(0);
    Some(Resources {
        cpu_user_ms: ms(ru.ru_utime),
        cpu_sys_ms: ms(ru.ru_stime),
        // ru_maxrss is KiB on Linux and the BSDs, bytes on macOS.
        max_rss_kb: if cfg!(target_vendor = "apple") {
            max_rss / 1_024
        } else {
            max_rss
        },
    })
}

#[cfg(not(unix))]
pub fn children_usage() -> Option<Resources> {
    None
}

/// CPU time spent between two [`children_usage`] snapshots; peak RSS as of `after`.
pub fn usage_since(before: Resources, after: Resources) -> Resources {
    Resources {
        cpu_user_ms: after.cpu_user_ms.saturating_sub(before.cpu_user_ms),
        cpu_sys_ms: after.cpu_sys_ms.saturating_sub(before.cpu_sys_ms),
        max_rss_kb: after.max_rss_kb,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn children_usage_around_a_child() {
        let before = children_usage().unwrap();
        let status = std::process::Command::new("sh")
            .args(["-c", ":"])
            .status()
            .unwrap();
        assert!(status.success());
        let after = children_usage().unwrap();
        assert!(after.cpu_user_ms >= before.cpu_user_ms);
        assert!(after.cpu_sys_ms >= before.cpu_sys_ms);
        assert!(after.max_rss_kb > 0, "{after:?}");
        let d = usage_since(before, after);
        assert!(d.cpu_user_ms <= after.cpu_user_ms && d.cpu_sys_ms <= after.cpu_sys_ms);
        assert_eq!(d.max_rss_kb, after.max_rss_kb);
    }
}
