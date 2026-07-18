//! Native Win32 message boxes for fatal startup errors (no GPUI yet).

use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::PCWSTR;

use crate::win32::wide::wide_null;

pub fn show_error(title: &str, message: &str) {
    let title = wide_null(title);
    let message = wide_null(message);
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}
