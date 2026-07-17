use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Threading::CreateMutexW;

static INSTANCE_MUTEX: OnceLock<isize> = OnceLock::new();

pub const STARTUP_INSTANCE_RETRIES: u32 = 30;

pub fn single_instance_retry_limit(is_startup_launch: bool) -> u32 {
    if is_startup_launch {
        STARTUP_INSTANCE_RETRIES
    } else {
        1
    }
}

/// Ensure only one instance of the application is running.
pub fn ensure_single_instance() -> Result<(), Box<dyn std::error::Error>> {
    let mutex_name: Vec<u16> = "MemoryCleanr_{B8F3A7E2-4C1D-4F5A-9B6E-2D8C3F7A1E9B}"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let retries = single_instance_retry_limit(crate::win32::startup::is_startup_launch());

    for attempt in 0..retries {
        unsafe {
            let handle = CreateMutexW(None, true, windows::core::PCWSTR(mutex_name.as_ptr()))?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                if attempt + 1 < retries {
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
                return Err("Application is already running".into());
            }

            let _ = INSTANCE_MUTEX.set(handle.0 as isize);
            return Ok(());
        }
    }

    Err("Application is already running".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_instance_retry_limit_waits_for_startup_handoff() {
        assert_eq!(single_instance_retry_limit(true), STARTUP_INSTANCE_RETRIES);
        assert_eq!(single_instance_retry_limit(false), 1);
    }
}
