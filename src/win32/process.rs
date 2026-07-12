use std::mem::MaybeUninit;
use std::time::Duration;

use anyhow::{Context, Result};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};

fn exe_name_matches(entry: &PROCESSENTRY32W, target: &[u16]) -> bool {
    let name = entry.szExeFile;
    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    name[..len] == target[..]
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
