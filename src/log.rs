use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);
static LOG_LOCK: Mutex<()> = Mutex::new(());

use crate::settings::Settings;

/// Drop log lines whose `[unix_secs.millis]` timestamp is older than this.
pub const LOG_RETENTION_SECS: u64 = 7 * 24 * 60 * 60;

/// Write a diagnostic message to the Windows debug stream (viewable via DebugView)
/// and, when stderr is attached, also to stderr.
pub fn log_msg(msg: &str) {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OutputDebugStringA(lp_output_string: *const u8);
    }
    let bytes = format!("{msg}\n\0").into_bytes();
    unsafe {
        OutputDebugStringA(bytes.as_ptr());
    }
    eprintln!("{msg}");
    write(msg);
}

pub fn set_debug_enabled(enabled: bool) {
    DEBUG_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Apply `settings.debug_logging` at process/session startup and log the path once.
pub fn init_from_settings(settings: &Settings) {
    apply_debug_settings(settings, true);
}

/// Apply `settings.debug_logging` without emitting the startup log line.
pub fn sync_debug_settings(settings: &Settings) {
    apply_debug_settings(settings, false);
}

fn apply_debug_settings(settings: &Settings, log_startup: bool) {
    set_debug_enabled(settings.debug_logging);
    if settings.debug_logging && log_startup {
        log_msg(&rust_i18n::t!(
            "log.debug_enabled",
            path = log_file_path().display().to_string()
        ));
    }
}

pub fn log_file_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("App.log")))
        .unwrap_or_else(|| PathBuf::from("App.log"))
}

pub(crate) fn parse_line_timestamp(line: &str) -> Option<u64> {
    let inner = line.strip_prefix('[')?.split(']').next()?;
    inner.split('.').next()?.parse().ok()
}

fn purge_stale_entries(path: &Path, now: u64) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    if content.is_empty() {
        return;
    }

    let cutoff = now.saturating_sub(LOG_RETENTION_SECS);

    let mut kept = String::new();
    let mut removed_any = false;
    for line in content.split_inclusive('\n') {
        if line.trim().is_empty() {
            continue;
        }
        match parse_line_timestamp(line) {
            Some(ts) if ts < cutoff => {
                removed_any = true;
            }
            _ => kept.push_str(line),
        }
    }

    if !removed_any {
        return;
    }

    if kept.is_empty() {
        let _ = std::fs::remove_file(path);
        return;
    }

    if let Ok(mut file) = OpenOptions::new().write(true).truncate(true).open(path) {
        let _ = file.write_all(kept.as_bytes());
    }
}

fn timestamp() -> (String, u64) {
    let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return ("unknown".into(), 0);
    };
    let secs = duration.as_secs();
    (format!("{secs}.{:03}", duration.subsec_millis()), secs)
}

/// Append a line to `App.log` when debug logging is enabled.
pub fn write(msg: &str) {
    if !DEBUG_ENABLED.load(Ordering::Relaxed) {
        return;
    }

    let Ok(_guard) = LOG_LOCK.lock() else {
        return;
    };

    let (ts, now) = timestamp();
    let path = log_file_path();
    purge_stale_entries(&path, now);

    let line = format!("[{ts}] {msg}\n");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
        let _ = file.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_timestamp_reads_unix_prefix() {
        assert_eq!(
            parse_line_timestamp("[1700000000.123] hello\n"),
            Some(1_700_000_000)
        );
        assert_eq!(parse_line_timestamp("no timestamp"), None);
    }

    #[test]
    fn retention_window_is_seven_days() {
        assert_eq!(LOG_RETENTION_SECS, 7 * 24 * 60 * 60);
    }
}
