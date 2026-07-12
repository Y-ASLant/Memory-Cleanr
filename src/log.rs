use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);
static LOG_LOCK: Mutex<()> = Mutex::new(());

pub fn set_debug_enabled(enabled: bool) {
    DEBUG_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn log_file_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("App.log")))
        .unwrap_or_else(|| PathBuf::from("App.log"))
}

fn timestamp() -> String {
    let Ok(duration) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) else {
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

    let line = format!("[{}] {msg}\n", timestamp());
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file_path())
    {
        let _ = file.write_all(line.as_bytes());
    }
}
