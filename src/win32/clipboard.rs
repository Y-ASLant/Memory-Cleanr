//! Write to the system clipboard and paste into the previous foreground window.
//!
//! Elevated processes cannot `SendInput` into medium-IL apps (UIPI). After writing
//! the clipboard we hide ourselves, restore the target HWND, and post `WM_PASTE`.

use std::mem::size_of;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, POINT, WPARAM};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_CONTROL,
    VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_V, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SendMessageTimeoutW, SMTO_ABORTIFHUNG};

use crate::win32::focus;

const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const WM_PASTE: u32 = 0x0302;

#[repr(C)]
struct DropFiles {
    p_files: u32,
    pt: POINT,
    f_nc: i32,
    f_wide: i32,
}

fn open_clipboard_retry() -> Result<()> {
    for attempt in 0..8 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(15 + attempt * 10));
    }
    anyhow::bail!("OpenClipboard failed after retries")
}

/// Set Unicode text on the system clipboard.
pub fn set_text(text: &str) -> Result<()> {
    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    utf16.push(0);
    let byte_len = utf16.len() * 2;

    open_clipboard_retry()?;
    let result = (|| -> Result<()> {
        unsafe {
            EmptyClipboard().context("EmptyClipboard failed")?;

            let hmem = GlobalAlloc(GMEM_MOVEABLE, byte_len).context("GlobalAlloc failed")?;
            let ptr = GlobalLock(hmem);
            if ptr.is_null() {
                anyhow::bail!("GlobalLock failed");
            }
            std::ptr::copy_nonoverlapping(utf16.as_ptr(), ptr as *mut u16, utf16.len());
            let _ = GlobalUnlock(hmem);

            SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hmem.0)))
                .context("SetClipboardData failed")?;
            Ok(())
        }
    })();
    unsafe {
        let _ = CloseClipboard();
    }
    result
}

/// Set file paths on the system clipboard (CF_HDROP).
pub fn set_files(paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        anyhow::bail!("no file paths to set");
    }

    let header_size = size_of::<DropFiles>();
    let mut path_data: Vec<u16> = Vec::new();
    for path in paths {
        path_data.extend(path.encode_utf16());
        path_data.push(0);
    }
    path_data.push(0);
    let total_size = header_size + path_data.len() * 2;

    open_clipboard_retry()?;
    let result = (|| -> Result<()> {
        unsafe {
            EmptyClipboard().context("EmptyClipboard failed")?;

            let hmem = GlobalAlloc(GMEM_MOVEABLE, total_size).context("GlobalAlloc failed")?;
            let base = GlobalLock(hmem);
            if base.is_null() {
                anyhow::bail!("GlobalLock failed");
            }
            let base = base as *mut u8;

            let drop = base as *mut DropFiles;
            (*drop).p_files = header_size as u32;
            (*drop).pt = POINT { x: 0, y: 0 };
            (*drop).f_nc = 0;
            (*drop).f_wide = 1;

            std::ptr::copy_nonoverlapping(
                path_data.as_ptr() as *const u8,
                base.add(header_size),
                path_data.len() * 2,
            );

            let _ = GlobalUnlock(hmem);
            SetClipboardData(CF_HDROP, Some(HANDLE(hmem.0))).context("SetClipboardData failed")?;
            Ok(())
        }
    })();
    unsafe {
        let _ = CloseClipboard();
    }
    result
}

/// Paste into the previously focused window.
///
/// Caller should already have hidden our window (so the OS can activate the target).
/// This does **not** show our window again — leave that to the UI thread.
pub fn paste_into_target() -> Result<()> {
    // Whoever Windows activated after our hide is the best fallback target.
    focus::capture_foreground_after_hide();

    let target = focus::paste_target_hwnd();
    let Some(hwnd) = target else {
        crate::log_msg("[clipboard] no paste target hwnd");
        anyhow::bail!("no paste target");
    };

    if !focus::force_foreground(hwnd) {
        crate::log_msg("[clipboard] force_foreground failed, continuing with WM_PASTE");
    }
    std::thread::sleep(std::time::Duration::from_millis(80));

    if post_paste(hwnd) {
        crate::log_msg("[clipboard] WM_PASTE delivered");
    } else {
        crate::log_msg("[clipboard] WM_PASTE failed, trying SendInput");
    }
    std::thread::sleep(std::time::Duration::from_millis(20));

    // Best-effort for elevated targets / apps that ignore WM_PASTE.
    simulate_paste()?;
    // Give the target time to apply the paste before we steal focus back.
    std::thread::sleep(std::time::Duration::from_millis(150));
    Ok(())
}

fn post_paste(hwnd: HWND) -> bool {
    unsafe {
        let mut result: usize = 0;
        let sent = SendMessageTimeoutW(
            hwnd,
            WM_PASTE,
            WPARAM(0),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            500,
            Some(&mut result),
        );
        sent.0 != 0 || PostMessageW(Some(hwnd), WM_PASTE, WPARAM(0), LPARAM(0)).is_ok()
    }
}

/// Simulate Ctrl+V (may be blocked by UIPI when we are elevated).
pub fn simulate_paste() -> Result<()> {
    release_if_held(VK_MENU.0);
    release_if_held(VK_SHIFT.0);
    release_if_held(VK_LWIN.0);
    release_if_held(VK_RWIN.0);

    let user_ctrl = is_key_pressed(VK_CONTROL.0);
    if !user_ctrl {
        send_key(VK_CONTROL.0, false);
    }
    send_key(VK_V.0, false);
    std::thread::sleep(std::time::Duration::from_millis(8));
    send_key(VK_V.0, true);
    if !user_ctrl {
        send_key(VK_CONTROL.0, true);
    }
    Ok(())
}

fn is_key_pressed(vk: u16) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    unsafe { GetAsyncKeyState(i32::from(vk)) < 0 }
}

fn send_key(vk: u16, up: bool) {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        let sent = SendInput(&[input], size_of::<INPUT>() as i32);
        if sent == 0 {
            crate::log_msg("[clipboard] SendInput returned 0 (UIPI may block elevated→medium)");
        }
    }
}

fn release_if_held(vk: u16) {
    for _ in 0..20 {
        if !is_key_pressed(vk) {
            return;
        }
        send_key(vk, true);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
