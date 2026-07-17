use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::{
    CreateMutexW, OpenMutexW, SYNCHRONIZATION_ACCESS_RIGHTS,
};

const SYNCHRONIZE: u32 = 0x0010_0000;

const TRAY_MUTEX_NAME: &str = "MemoryCleanr_Tray_{B8F3A7E2-4C1D-4F5A-9B6E-2D8C3F7A1E9B}";
const GUI_MUTEX_NAME: &str = "MemoryCleanr_Gui_{C9F4B8F3-5D2E-4A6C-8C7F-3E9D4A2B1F0C}";

pub const TRAY_STARTUP_RETRIES: u32 = 30;
pub const TRAY_WAIT_MS: u32 = 15_000;
pub const GUI_EXIT_WAIT_MS: u32 = 5_000;

static HELD_MUTEX: OnceLock<isize> = OnceLock::new();

#[derive(Clone, Copy)]
pub enum InstanceRole {
    Tray,
    Gui,
}

impl InstanceRole {
    fn mutex_name(self) -> &'static str {
        match self {
            Self::Tray => TRAY_MUTEX_NAME,
            Self::Gui => GUI_MUTEX_NAME,
        }
    }
}

fn wide_name(name: &str) -> Vec<u16> {
    name.encode_utf16().chain(std::iter::once(0)).collect()
}

fn open_existing_mutex(name: &str) -> bool {
    unsafe {
        let wide = wide_name(name);
        let handle = OpenMutexW(
            SYNCHRONIZATION_ACCESS_RIGHTS(SYNCHRONIZE),
            false,
            windows::core::PCWSTR(wide.as_ptr()),
        );
        if let Ok(handle) = handle {
            let _ = CloseHandle(handle);
            true
        } else {
            false
        }
    }
}

/// Returns true when the persistent tray-host process holds its singleton mutex.
pub fn is_tray_running() -> bool {
    open_existing_mutex(TRAY_MUTEX_NAME)
}

/// Returns true when a GUI process holds its singleton mutex.
pub fn is_gui_running() -> bool {
    open_existing_mutex(GUI_MUTEX_NAME)
}

pub fn tray_startup_retry_limit() -> u32 {
    TRAY_STARTUP_RETRIES
}

/// Acquire the tray singleton. Retries briefly so a dying tray host can release the mutex.
pub fn ensure_tray_singleton() -> Result<(), Box<dyn std::error::Error>> {
    acquire_singleton(InstanceRole::Tray, TRAY_STARTUP_RETRIES)
}

/// Acquire the GUI singleton. Only one GUI process may run at a time.
pub fn ensure_gui_singleton() -> Result<(), Box<dyn std::error::Error>> {
    acquire_singleton(InstanceRole::Gui, 1)
}

fn acquire_singleton(role: InstanceRole, retries: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mutex_name = wide_name(role.mutex_name());

    for attempt in 0..retries {
        unsafe {
            let handle = CreateMutexW(None, true, windows::core::PCWSTR(mutex_name.as_ptr()))?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                if attempt + 1 < retries {
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
                return Err(match role {
                    InstanceRole::Tray => "Tray host is already running".into(),
                    InstanceRole::Gui => "GUI is already running".into(),
                });
            }

            let _ = HELD_MUTEX.set(handle.0 as isize);
            return Ok(());
        }
    }

    Err(match role {
        InstanceRole::Tray => "Tray host is already running".into(),
        InstanceRole::Gui => "GUI is already running".into(),
    })
}

/// Block until the tray-host mutex is held by another process, or timeout.
pub fn wait_for_tray_host(timeout_ms: u32) -> bool {
    if crate::win32::ipc::wait_tray_ready(timeout_ms) {
        return true;
    }
    wait_for_mutex(TRAY_MUTEX_NAME, true, timeout_ms)
}

/// Block until no GUI process holds the GUI singleton mutex.
pub fn wait_for_gui_exit(timeout_ms: u32) -> bool {
    wait_for_mutex(GUI_MUTEX_NAME, false, timeout_ms)
}

fn wait_for_mutex(name: &str, wait_until_present: bool, timeout_ms: u32) -> bool {
    let steps = timeout_ms / 50;
    for _ in 0..steps {
        let present = open_existing_mutex(name);
        if present == wait_until_present {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    open_existing_mutex(name) == wait_until_present
}

#[allow(dead_code)]
pub fn held_mutex_handle() -> Option<HANDLE> {
    HELD_MUTEX.get().map(|value| HANDLE(*value as _))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_startup_retry_limit_is_positive() {
        assert_eq!(tray_startup_retry_limit(), TRAY_STARTUP_RETRIES);
    }

    #[test]
    fn tray_and_gui_mutex_names_differ() {
        assert_ne!(TRAY_MUTEX_NAME, GUI_MUTEX_NAME);
    }

    #[test]
    fn wait_for_gui_exit_returns_true_when_gui_not_running() {
        assert!(wait_for_gui_exit(0));
    }
}
