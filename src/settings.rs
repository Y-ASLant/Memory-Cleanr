use crate::optimize::MemoryAreas;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub always_on_top: bool,
    pub close_to_notification_area: bool,
    pub show_virtual_memory: bool,
    pub memory_areas: u32,
    // 预留字段：自动优化功能（未实现）
    pub auto_optimization_interval: u32,
    pub auto_optimization_memory_usage: u32,
    // 预留字段：优化通知（未实现）
    pub show_optimization_notifications: bool,
    // 预留字段：托盘图标自定义（未实现）
    pub tray_icon_show_memory_usage: bool,
    pub tray_icon_use_transparent_background: bool,
    pub tray_icon_warning_level: u8,
    pub tray_icon_danger_level: u8,
    /// Write debug output to `App.log` next to the executable.
    pub debug_logging: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            always_on_top: false,
            auto_optimization_interval: 0,
            auto_optimization_memory_usage: 0,
            close_to_notification_area: true,
            show_virtual_memory: true,
            show_optimization_notifications: true,
            memory_areas: MemoryAreas::DEFAULT.bits(),
            tray_icon_show_memory_usage: false,
            tray_icon_use_transparent_background: false,
            tray_icon_warning_level: 80,
            tray_icon_danger_level: 90,
            debug_logging: false,
        }
    }
}

impl Settings {
    fn config_dir() -> PathBuf {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("MemoryCleaner")
    }

    fn config_path() -> PathBuf {
        Self::config_dir().join("settings.toml")
    }

    fn ensure_config_dir() {
        let _ = std::fs::create_dir_all(Self::config_dir());
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        let mut settings = match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str(&content) {
                Ok(settings) => settings,
                Err(e) => {
                    crate::log_msg(&format!("Failed to parse {}: {e}", path.display()));
                    Self::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                crate::log_msg(&format!("Failed to read {}: {e}", path.display()));
                Self::default()
            }
        };
        settings.normalize_memory_areas();
        settings
    }

    fn normalize_memory_areas(&mut self) {
        let mut areas = MemoryAreas::from_bits_truncate(self.memory_areas);
        if areas.contains(MemoryAreas::STANDBY_LIST)
            && areas.contains(MemoryAreas::STANDBY_LIST_LOW_PRIORITY)
        {
            areas.remove(MemoryAreas::STANDBY_LIST_LOW_PRIORITY);
            self.memory_areas = areas.bits();
        }
    }

    pub fn save(&self) {
        Self::ensure_config_dir();
        let Ok(content) = toml::to_string_pretty(self) else {
            crate::log_msg("[settings] failed to serialize config");
            return;
        };
        let final_path = Self::config_path();
        let tmp_path = final_path.with_extension("toml.tmp");
        if let Err(e) = std::fs::write(&tmp_path, &content) {
            crate::log_msg(&format!(
                "[settings] failed to write {}: {e}",
                tmp_path.display()
            ));
            return;
        }
        if let Err(e) = std::fs::rename(&tmp_path, &final_path) {
            crate::log_msg(&format!(
                "[settings] failed to rename {} -> {}: {e}",
                tmp_path.display(),
                final_path.display()
            ));
            let _ = std::fs::remove_file(&tmp_path);
        }
    }

    pub fn memory_areas(&self) -> MemoryAreas {
        let mut areas = MemoryAreas::from_bits_truncate(self.memory_areas);
        if areas.contains(MemoryAreas::STANDBY_LIST)
            && areas.contains(MemoryAreas::STANDBY_LIST_LOW_PRIORITY)
        {
            areas.remove(MemoryAreas::STANDBY_LIST_LOW_PRIORITY);
        }
        areas
    }
}
