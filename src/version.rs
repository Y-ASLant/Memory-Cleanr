/// User-visible application name (window title, task manager, title bar).
pub const APP_NAME: &str = "Memory Cleaner";

/// Executable base name without `.exe`, used for process exclusion matching.
pub const PROCESS_BASE_NAME: &str = "MemoryCleanr";

/// Application version from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Source repository opened from the version link in settings.
pub const REPO_URL: &str = "https://github.com/Y-ASLant/MemoryCleanr";
