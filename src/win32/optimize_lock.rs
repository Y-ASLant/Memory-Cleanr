//! Cross-process mutex so GUI and tray host cannot run cleanup concurrently.

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

use crate::win32::wide::wide_null;

const OPTIMIZE_MUTEX_NAME: &str = "MemoryCleanr_Optimize_{A1B2C3D4-E5F6-7890-ABCD-EF1234567890}";

pub struct OptimizeLock {
    handle: HANDLE,
}

impl OptimizeLock {
    /// Returns `None` when another process is already running cleanup.
    pub fn try_acquire() -> Option<Self> {
        unsafe {
            let name = wide_null(OPTIMIZE_MUTEX_NAME);
            let handle = CreateMutexW(
                None,
                false,
                windows::core::PCWSTR(name.as_ptr()),
            )
            .ok()?;
            let wait = WaitForSingleObject(handle, 0);
            if wait.0 != 0 {
                let _ = CloseHandle(handle);
                return None;
            }
            Some(Self { handle })
        }
    }
}

impl Drop for OptimizeLock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optimize_lock_can_be_acquired_and_released() {
        let lock = OptimizeLock::try_acquire().expect("first acquire");
        drop(lock);
        assert!(OptimizeLock::try_acquire().is_some());
    }
}
