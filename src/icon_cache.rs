use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_SYSTEM,
    FILE_FLAGS_AND_ATTRIBUTES, GetFileAttributesW, INVALID_FILE_ATTRIBUTES, SetFileAttributesW,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, KEY_SET_VALUE, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
};
use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
use windows::core::PCWSTR;

const TRAY_SUBKEY: &str =
    "Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\TrayNotify";

pub struct RefreshOutcome {
    pub explorer_restarted: bool,
    pub failures: Vec<String>,
}

impl RefreshOutcome {
    pub fn user_message(&self) -> String {
        if !self.explorer_restarted {
            "刷新失败：无法重启资源管理器".into()
        } else if self.failures.is_empty() {
            "桌面图标缓存已刷新".into()
        } else {
            "已刷新，部分缓存未能清理".into()
        }
    }
}

pub fn refresh() -> RefreshOutcome {
    let mut failures = Vec::new();
    let Some(()) = stop_explorer(&mut failures) else {
        return RefreshOutcome {
            explorer_restarted: false,
            failures,
        };
    };

    clean_files(&mut failures);
    let explorer_restarted = restart_explorer(&mut failures);
    RefreshOutcome {
        explorer_restarted,
        failures,
    }
}

fn stop_explorer(failures: &mut Vec<String>) -> Option<()> {
    match crate::win32::process::kill_process_by_name("explorer.exe") {
        Err(e) => {
            failures.push(format!("结束 explorer 失败: {e:#}"));
            None
        }
        Ok(0) => Some(()),
        Ok(_) => {
            if !crate::win32::process::wait_for_process_exit("explorer.exe", 5000) {
                failures.push("explorer 退出超时".into());
            }
            Some(())
        }
    }
}

fn clean_files(failures: &mut Vec<String>) {
    let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
        failures.push("LOCALAPPDATA 未设置".into());
        return;
    };

    delete_file(&local.join("IconCache.db"), failures);
    let explorer = local.join("Microsoft").join("Windows").join("Explorer");
    for prefix in ["iconcache_", "thumbcache_"] {
        delete_db_prefix(&explorer, prefix, failures);
    }
    for value in ["IconStreams", "PastIconsStream"] {
        delete_reg_value(TRAY_SUBKEY, value, failures);
    }
}

fn restart_explorer(failures: &mut Vec<String>) -> bool {
    match std::process::Command::new("explorer.exe").spawn() {
        Ok(_) => {
            notify_shell();
            true
        }
        Err(e) => {
            failures.push(format!("重启 explorer 失败: {e}"));
            false
        }
    }
}

fn delete_db_prefix(dir: &Path, prefix: &str, failures: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(prefix) && name.ends_with(".db") {
            delete_file(&entry.path(), failures);
        }
    }
}

fn delete_file(path: &Path, failures: &mut Vec<String>) {
    if !path.is_file() {
        return;
    }
    clear_attrs(path);
    if std::fs::remove_file(path).is_err() {
        failures.push(format!("删除失败: {}", path.display()));
    }
}

fn clear_attrs(path: &Path) {
    let wide = to_wide(&path.to_string_lossy());
    unsafe {
        let attrs = GetFileAttributesW(PCWSTR(wide.as_ptr()));
        if attrs == INVALID_FILE_ATTRIBUTES {
            return;
        }
        let mask = FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0 | FILE_ATTRIBUTE_READONLY.0;
        let _ = SetFileAttributesW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(attrs & !mask),
        );
    }
}

fn delete_reg_value(subkey: &str, value: &str, failures: &mut Vec<String>) {
    unsafe {
        let subkey_wide = to_wide(subkey);
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CLASSES_ROOT,
            PCWSTR(subkey_wide.as_ptr()),
            Some(0),
            KEY_SET_VALUE,
            &mut key,
        )
        .is_err()
        {
            return;
        }
        let value_wide = to_wide(value);
        if RegDeleteValueW(key, PCWSTR(value_wide.as_ptr())).is_err() {
            failures.push(format!("删除注册表值 {value} 失败"));
        }
        let _ = RegCloseKey(key);
    }
}

fn notify_shell() {
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
}

fn to_wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}
