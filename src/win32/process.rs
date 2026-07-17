use std::collections::HashMap;
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, GetLastError};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, K32EmptyWorkingSet, PROCESS_MEMORY_COUNTERS,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
    PROCESS_TERMINATE, TerminateProcess,
};

use crate::memory::MemoryStatus;

/// Running process entry for the exclusion picker dropdown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessPickerEntry {
    pub name: String,
    pub instance_count: u32,
    pub working_set_bytes: u64,
    /// How many instances returned a readable working-set size.
    pub memory_readable_count: u32,
}

impl ProcessPickerEntry {
    pub fn memory_display(&self) -> Option<String> {
        if self.memory_readable_count == 0 {
            None
        } else {
            Some(MemoryStatus::format_bytes(self.working_set_bytes))
        }
    }
}

/// Processes that should not appear in the exclusion picker.
fn is_picker_hidden_process(name: &str) -> bool {
    matches!(name, "[systemprocess]" | "systemidleprocess")
}

fn query_process_working_set_bytes(pid: u32) -> Option<u64> {
    unsafe {
        let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_QUERY_INFORMATION;
        let handle = match OpenProcess(access, false, pid) {
            Ok(handle) => handle,
            Err(_) => return None,
        };

        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        let ok = GetProcessMemoryInfo(
            handle,
            &mut counters,
            size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
        .is_ok();
        let _ = CloseHandle(handle);

        if ok {
            Some(counters.WorkingSetSize as u64)
        } else {
            None
        }
    }
}

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

/// Distinct running processes for the exclusion picker, excluding system/hidden entries.
pub fn list_processes_for_exclusion_picker(
    self_base: &str,
    excluded: &[String],
) -> Vec<ProcessPickerEntry> {
    let self_normalized = normalize_process_name(self_base);
    let mut by_name: HashMap<String, ProcessPickerEntry> = HashMap::new();

    let _ = with_process_snapshot(|entry| {
        let name = exe_base_name_from_entry(entry);
        if name.is_empty()
            || name == self_normalized
            || is_picker_hidden_process(&name)
            || excluded.iter().any(|ex| ex == &name)
        {
            return false;
        }

        let working_set = query_process_working_set_bytes(entry.th32ProcessID);
        by_name
            .entry(name.clone())
            .and_modify(|item| {
                item.instance_count += 1;
                if let Some(bytes) = working_set {
                    item.memory_readable_count += 1;
                    item.working_set_bytes = item.working_set_bytes.saturating_add(bytes);
                }
            })
            .or_insert(ProcessPickerEntry {
                name,
                instance_count: 1,
                working_set_bytes: working_set.unwrap_or(0),
                memory_readable_count: u32::from(working_set.is_some()),
            });
        false
    });

    let mut entries: Vec<_> = by_name.into_values().collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
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
        let handle =
            match unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_SET_QUOTA, false, pid) }
            {
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

/// Ensure the persistent tray-host process is running.
pub fn ensure_tray_host_running() -> Result<()> {
    if crate::win32::single_instance::is_tray_running() {
        return Ok(());
    }

    spawn_tray_instance()?;
    if crate::win32::ipc::wait_tray_ready(crate::win32::ipc::TRAY_READY_WAIT_MS) {
        return Ok(());
    }
    if !crate::win32::single_instance::wait_for_tray_host(
        crate::win32::single_instance::TRAY_WAIT_MS,
    ) {
        bail!("tray host failed to start within timeout");
    }
    Ok(())
}

/// Activate an existing GUI window or spawn a new GUI process.
pub fn activate_or_spawn_gui() -> Result<()> {
    if let Some(session) = crate::win32::ipc::gui_session() {
        if !crate::win32::window::activate_hwnd(session.hwnd) {
            crate::log_msg("[tray] registered GUI hwnd is no longer valid");
        }
        return Ok(());
    }

    if crate::win32::single_instance::is_gui_running() {
        if !crate::win32::window::activate_gui_window() {
            crate::log_msg("[tray] GUI running but window not registered yet");
        }
        return Ok(());
    }

    spawn_gui_instance()
}

/// Toggle the GUI window: spawn, hide via close, or activate.
pub fn toggle_gui_window() -> Result<()> {
    if let Some(session) = crate::win32::ipc::gui_session() {
        if crate::win32::window::is_hwnd_visible(session.hwnd) {
            if !crate::win32::window::request_hwnd_close(session.hwnd) {
                crate::log_msg("[tray] failed to request GUI close");
            }
        } else if !crate::win32::window::activate_hwnd(session.hwnd) {
            crate::log_msg("[tray] failed to activate hidden GUI window");
        }
        return Ok(());
    }

    if crate::win32::single_instance::is_gui_running() {
        if crate::win32::window::is_gui_window_visible() {
            if !crate::win32::window::request_gui_close() {
                crate::log_msg("[tray] failed to request GUI close");
            }
        } else if !crate::win32::window::activate_gui_window() {
            crate::log_msg("[tray] failed to activate hidden GUI window");
        }
        return Ok(());
    }

    spawn_gui_instance()
}

fn is_process_alive(pid: u32) -> bool {
    unsafe {
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(handle) => {
                let _ = CloseHandle(handle);
                true
            }
            Err(_) => false,
        }
    }
}

/// Terminate a single process by PID.
pub fn terminate_process_pid(pid: u32) -> Result<()> {
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, false, pid).context("OpenProcess")?;
        TerminateProcess(handle, 1).context("TerminateProcess")?;
        let _ = CloseHandle(handle);
    }
    Ok(())
}

/// Ask a running GUI process to exit before the tray host shuts down.
pub fn request_gui_shutdown() {
    if let Some(session) = crate::win32::ipc::gui_session() {
        let _ = crate::win32::window::request_hwnd_close(session.hwnd);
        if crate::win32::single_instance::wait_for_gui_exit(
            crate::win32::single_instance::GUI_EXIT_WAIT_MS,
        ) {
            crate::win32::ipc::set_gui_session(None);
            return;
        }
        if (crate::win32::single_instance::is_gui_running() || is_process_alive(session.pid))
            && let Err(error) = terminate_process_pid(session.pid)
        {
            crate::log_msg(&format!("[tray] terminate GUI failed: {error:#}"));
        }
        crate::win32::ipc::set_gui_session(None);
        return;
    }

    if !crate::win32::single_instance::is_gui_running() {
        return;
    }

    let _ = crate::win32::window::request_gui_close();
    if !crate::win32::single_instance::wait_for_gui_exit(
        crate::win32::single_instance::GUI_EXIT_WAIT_MS,
    ) {
        crate::log_msg("[tray] GUI did not exit before tray shutdown");
    }
}

pub fn spawn_tray_instance() -> Result<()> {
    let spec = tray_instance_launch_spec()?;
    std::process::Command::new(&spec.exe)
        .args(&spec.args)
        .spawn()
        .context("failed to spawn tray instance")?;
    Ok(())
}

pub fn spawn_gui_instance() -> Result<()> {
    let spec = gui_instance_launch_spec()?;
    std::process::Command::new(&spec.exe)
        .args(&spec.args)
        .spawn()
        .context("failed to spawn GUI instance")?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceLaunchSpec {
    pub exe: PathBuf,
    pub args: Vec<String>,
}

pub fn tray_instance_launch_spec() -> Result<InstanceLaunchSpec> {
    Ok(InstanceLaunchSpec {
        exe: std::env::current_exe().context("current_exe unavailable")?,
        args: vec![crate::win32::startup::STARTUP_ARG.to_string()],
    })
}

pub fn gui_instance_launch_spec() -> Result<InstanceLaunchSpec> {
    Ok(InstanceLaunchSpec {
        exe: std::env::current_exe().context("current_exe unavailable")?,
        args: Vec::new(),
    })
}

pub fn launch_spec_for_path(exe: &Path, args: &[&str]) -> InstanceLaunchSpec {
    InstanceLaunchSpec {
        exe: exe.to_path_buf(),
        args: args.iter().map(|arg| (*arg).to_string()).collect(),
    }
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

    #[test]
    fn is_picker_hidden_process_matches_system_entries() {
        assert!(is_picker_hidden_process("[systemprocess]"));
        assert!(is_picker_hidden_process("systemidleprocess"));
        assert!(!is_picker_hidden_process("chrome"));
    }

    #[test]
    fn process_picker_entry_memory_display() {
        let unknown = ProcessPickerEntry {
            name: "lsass".to_string(),
            instance_count: 1,
            working_set_bytes: 0,
            memory_readable_count: 0,
        };
        assert_eq!(unknown.memory_display(), None);

        let readable = ProcessPickerEntry {
            name: "chrome".to_string(),
            instance_count: 2,
            working_set_bytes: 512 * 1024 * 1024,
            memory_readable_count: 2,
        };
        assert_eq!(
            readable.memory_display(),
            Some(MemoryStatus::format_bytes(readable.working_set_bytes))
        );
    }

    #[test]
    fn tray_instance_launch_spec_includes_startup_flag() {
        let spec = tray_instance_launch_spec().expect("current_exe");
        assert_eq!(
            spec.args,
            vec![crate::win32::startup::STARTUP_ARG.to_string()]
        );
    }

    #[test]
    fn gui_instance_launch_spec_has_no_extra_args() {
        let spec = gui_instance_launch_spec().expect("current_exe");
        assert!(spec.args.is_empty());
    }

    #[test]
    fn launch_spec_for_path_builds_expected_command() {
        let spec = launch_spec_for_path(
            Path::new(r"C:\Tools\MemoryCleanr.exe"),
            &[crate::win32::startup::STARTUP_ARG],
        );
        assert_eq!(
            spec,
            InstanceLaunchSpec {
                exe: PathBuf::from(r"C:\Tools\MemoryCleanr.exe"),
                args: vec![crate::win32::startup::STARTUP_ARG.to_string()],
            }
        );
    }
}
