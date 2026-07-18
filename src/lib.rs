rust_i18n::i18n!("locales", fallback = "zh-CN");

pub mod app;
pub mod icon_cache;
pub mod locale;
pub mod log;
pub mod memory;
pub mod messages;
pub mod optimize;
pub mod privileges;
pub mod runtime;
pub mod service;
pub mod settings;
pub mod tray;
pub mod ui;
pub mod version;
pub mod win32;

pub use log::log_msg;
pub use version::APP_NAME;

/// Show a localized fatal error when the tray host cannot start before GUI launch.
pub fn report_tray_startup_failure(error: &impl std::fmt::Display) {
    win32::dialog::show_error(
        &rust_i18n::t!("error.tray_startup_title"),
        &rust_i18n::t!("error.tray_startup_failed", detail = error.to_string()),
    );
}
