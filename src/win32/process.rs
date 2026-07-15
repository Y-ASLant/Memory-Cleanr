use std::mem::MaybeUninit;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, GetLastError};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::K32EmptyWorkingSet;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE, TerminateProcess,
};

/// Normalize a process name for exclusion matching: lowercase, no whitespace, no `.exe`.
pub fn normalize_process_name(name: &str) -> String {
    let trimmed: String = name.chars().filter(|c| !c.is_whitespace()).collect();
    let lower = trimmed.to_ascii_lowercase();
    lower
        .strip_suffix(".exe")
        .unwrap_or(lower.as_str())
        .to_string()
}

fn exe_name_matches(entry: &PROCESSENTRY32W, target: &[u16]) -> bool {
    let name = entry.szExeFile;
    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    name[..len] == target[..]
}

fn exe_base_name_from_entry(entry: &PROCESSENTRY32W) -> String {
    let name = entry.szExeFile;
    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    let utf16 = &name[..len];
    normalize_process_name(&String::from_utf16_lossy(utf16))
}

fn with_process_snapshot<F>(mut f: F) -> Result<()>
where
    F: FnMut(&PROCESSENTRY32W) -> bool,
{
    unsafe {
        let snapshot =
            CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).context("CreateToolhelp32Snapshot")?;
        let mut entry = MaybeUninit::<PROCESSENTRY32W>::zeroed();
        (*entry.as_mut_ptr()).dwSize = size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, entry.as_mut_ptr()).is_ok() {
            loop {
                if f(entry.assume_init_ref()) {
                    break;
                }
                if Process32NextW(snapshot, entry.as_mut_ptr()).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
    }
    Ok(())
}

pub fn is_process_excluded(process_name: &str, excluded: &[String]) -> bool {
    let normalized = normalize_process_name(process_name);
    excluded.iter().any(|name| name == &normalized)
}

/// Distinct running process base names, excluding this app and already-excluded entries.
pub fn list_running_process_names(self_base: &str, excluded: &[String]) -> Vec<String> {
    let self_normalized = normalize_process_name(self_base);
    let mut names = Vec::new();

    let _ = with_process_snapshot(|entry| {
        let name = exe_base_name_from_entry(entry);
        if name == self_normalized
            || excluded.iter().any(|excluded| excluded == &name)
            || names.iter().any(|existing| existing == &name)
        {
            return false;
        }
        names.push(name);
        false
    });

    names.sort();
    names
}

/// Empty working sets for every running process except those in `excluded`.
pub fn empty_working_sets_except(excluded: &[String]) -> Result<()> {
    let mut errors = Vec::new();

    with_process_snapshot(|entry| {
        let name = exe_base_name_from_entry(entry);
        if is_process_excluded(&name, excluded) {
            return false;
        }

        let pid = entry.th32ProcessID;
        let handle = match unsafe {
            OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_SET_QUOTA, false, pid)
        } {
            Ok(handle) => handle,
            Err(_) => return false,
        };

        let result = unsafe { K32EmptyWorkingSet(handle) };
        if !result.as_bool() {
            let last_error = unsafe { GetLastError() };
            if last_error != ERROR_ACCESS_DENIED {
                errors.push(format!("{name} (pid {pid}): {last_error:?}"));
            }
        }
        let _ = unsafe { CloseHandle(handle) };
        false
    })?;

    if errors.is_empty() {
        Ok(())
    } else {
        bail!("Working Set per-process errors: {}", errors.join(", "));
    }
}

/// Return true if another process with the same executable name is running.
pub fn has_sibling_process(current_pid: u32, exe_name: &str) -> bool {
    let target: Vec<u16> = exe_name.encode_utf16().collect();
    let mut found = false;
    let _ = with_process_snapshot(|entry| {
        if entry.th32ProcessID != current_pid && exe_name_matches(entry, &target) {
            found = true;
            return true;
        }
        false
    });
    found
}

/// Return true if any process with the given executable name is running.
pub fn is_process_running(exe_name: &str) -> bool {
    let target: Vec<u16> = exe_name.encode_utf16().collect();
    let mut found = false;
    let _ = with_process_snapshot(|entry| {
        if exe_name_matches(entry, &target) {
            found = true;
            return true;
        }
        false
    });
    found
}

/// Terminate every running process whose executable name matches `exe_name`.
pub fn kill_process_by_name(exe_name: &str) -> Result<u32> {
    let target: Vec<u16> = exe_name.encode_utf16().collect();
    let mut killed = 0u32;

    with_process_snapshot(|entry| {
        if !exe_name_matches(entry, &target) {
            return false;
        }
        let pid = entry.th32ProcessID;
        if let Ok(handle) = unsafe { OpenProcess(PROCESS_TERMINATE, false, pid) } {
            if unsafe { TerminateProcess(handle, 1) }.is_ok() {
                killed += 1;
            }
            let _ = unsafe { CloseHandle(handle) };
        }
        false
    })?;

    Ok(killed)
}

/// Wait until no process with the given name is running, or timeout.
pub fn wait_for_process_exit(exe_name: &str, timeout_ms: u32) -> bool {
    let steps = timeout_ms / 100;
    for _ in 0..steps {
        if !is_process_running(exe_name) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    !is_process_running(exe_name)
}

/// Best-effort wait until an elevated relaunch is observed, or timeout.
pub fn wait_for_elevated_relaunch(current_pid: u32, exe_name: &str, timeout_ms: u32) -> bool {
    let steps = timeout_ms / 100;
    for _ in 0..steps {
        if has_sibling_process(current_pid, exe_name) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_process_name_strips_exe_and_whitespace() {
        assert_eq!(normalize_process_name(" Chrome.EXE "), "chrome");
        assert_eq!(normalize_process_name("firefox"), "firefox");
    }

    #[test]
    fn is_process_excluded_matches_case_insensitive_base_names() {
        let excluded = vec!["chrome".to_string()];
        assert!(is_process_excluded("Chrome.exe", &excluded));
        assert!(!is_process_excluded("firefox", &excluded));
    }
}
