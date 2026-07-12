use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);
static LOG_LOCK: Mutex<()> = Mutex::new(());

/// Drop log lines whose `[unix_secs.millis]` timestamp is older than this.
const LOG_RETENTION_SECS: u64 = 7 * 24 * 60 * 60;

pub fn set_debug_enabled(enabled: bool) {
    DEBUG_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn log_file_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("App.log")))
        .unwrap_or_else(|| PathBuf::from("App.log"))
}

fn now_secs() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

fn parse_line_timestamp(line: &str) -> Option<u64> {
    let inner = line.strip_prefix('[')?.split(']').next()?;
    inner.split('.').next()?.parse().ok()
}

fn purge_stale_entries(path: &Path) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    if content.is_empty() {
        return;
    }

    let Some(now) = now_secs() else {
        return;
    };
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

fn timestamp() -> String {
    let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return "unknown".into();
    };
    format!("{}.{:03}", duration.as_secs(), duration.subsec_millis())
}

/// Append a line to `App.log` when debug logging is enabled.
pub fn write(msg: &str) {
    if !DEBUG_ENABLED.load(Ordering::Relaxed) {
        return;
    }

    let Ok(_guard) = LOG_LOCK.lock() else {
        return;
    };

    let path = log_file_path();
    purge_stale_entries(&path);

    let line = format!("[{}] {msg}\n", timestamp());
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
}
