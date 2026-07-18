use anyhow::{Context, Result};
use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GWL_STYLE, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowTextW, HWND_NOTOPMOST, HWND_TOPMOST, IsIconic, IsWindow, IsWindowVisible,
    PostMessageW, SetForegroundWindow, SHOW_WINDOW_CMD, SW_HIDE, SW_RESTORE, SW_SHOW,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, WM_CLOSE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX,
};

use crate::version::APP_NAME;

fn show_window(hwnd: HWND, cmd: SHOW_WINDOW_CMD) -> Result<()> {
    unsafe {
        // ShowWindow returns the previous visibility state, not success/failure.
        let _ = ShowWindow(hwnd, cmd);
    }
    Ok(())
}

fn apply_extended_style(hwnd: HWND, update: impl FnOnce(u32) -> u32) -> Result<()> {
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, update(style) as _);
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
    Ok(())
}

pub(crate) fn hwnd_from_window(window: &Window) -> Result<HWND> {
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|e| anyhow::anyhow!("window handle unavailable: {e}"))?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        anyhow::bail!("unsupported platform window handle");
    };

    Ok(HWND(win32.hwnd.get() as _))
}

pub fn hide_hwnd_raw(hwnd: isize) -> Result<()> {
    unsafe {
        let hwnd = HWND(hwnd as _);
        if IsWindow(Some(hwnd)).as_bool() {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
    Ok(())
}

/// Restore the window from tray-only hidden state.
pub fn show_from_tray(window: &Window) -> Result<()> {
    let hwnd = hwnd_from_window(window)?;
    apply_extended_style(hwnd, |style| {
        (style & !WS_EX_TOOLWINDOW.0) | WS_EX_APPWINDOW.0
    })?;
    let cmd = unsafe {
        if IsIconic(hwnd).as_bool() {
            SW_RESTORE
        } else {
            SW_SHOW
        }
    };
    show_window(hwnd, cmd)?;
    Ok(())
}

pub fn set_always_on_top(window: &Window, on_top: bool) -> Result<()> {
    let hwnd = hwnd_from_window(window)?;
    let insert_after = if on_top {
        Some(HWND_TOPMOST)
    } else {
        Some(HWND_NOTOPMOST)
    };

    unsafe {
        SetWindowPos(
            hwnd,
            insert_after,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
        .context("SetWindowPos failed")?;
    }

    Ok(())
}

/// Returns true when the given window title belongs to the main GUI window.
pub fn gui_window_title_matches(title: &str) -> bool {
    title == APP_NAME
}

struct FindGuiWindow {
    found: Option<HWND>,
}

unsafe extern "system" fn enum_gui_windows(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    // SAFETY: `lparam` points to `FindGuiWindow` for the duration of `EnumWindows`.
    let state = unsafe { &mut *(lparam.0 as *mut FindGuiWindow) };
    if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
        return windows::core::BOOL::from(true);
    }

    let title_len = unsafe { GetWindowTextLengthW(hwnd) };
    if title_len <= 0 {
        return windows::core::BOOL::from(true);
    }

    let mut buffer = vec![0u16; title_len as usize + 1];
    let read = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    if read <= 0 {
        return windows::core::BOOL::from(true);
    }

    let title = String::from_utf16_lossy(&buffer[..read as usize]);
    if gui_window_title_matches(&title) {
        state.found = Some(hwnd);
        return windows::core::BOOL::from(false);
    }

    windows::core::BOOL::from(true)
}

/// IPC-registered GUI window handle (does not enumerate windows).
pub fn registered_gui_hwnd() -> Option<HWND> {
    let session = crate::win32::ipc::gui_session()?;
    let hwnd = HWND(session.hwnd as _);
    if unsafe { IsWindow(Some(hwnd)).as_bool() } {
        Some(hwnd)
    } else {
        None
    }
}

/// Registered GUI window, or enumerate by title when IPC has not registered yet.
pub fn resolve_gui_hwnd() -> Option<HWND> {
    registered_gui_hwnd().or_else(find_gui_hwnd)
}

/// Locate the top-level GUI window owned by another process.
pub fn find_gui_hwnd() -> Option<HWND> {
    let mut state = FindGuiWindow { found: None };
    unsafe {
        let _ = EnumWindows(
            Some(enum_gui_windows),
            LPARAM(std::ptr::from_mut(&mut state) as isize),
        );
    }
    state.found
}

pub fn is_gui_window_visible() -> bool {
    resolve_gui_hwnd().is_some_and(|hwnd| unsafe { IsWindowVisible(hwnd).as_bool() })
}

pub fn is_hwnd_visible(hwnd: isize) -> bool {
    unsafe {
        let hwnd = HWND(hwnd as _);
        IsWindow(Some(hwnd)).as_bool() && IsWindowVisible(hwnd).as_bool()
    }
}

pub fn activate_hwnd(hwnd: isize) -> bool {
    unsafe {
        let hwnd = HWND(hwnd as _);
        if !IsWindow(Some(hwnd)).as_bool() {
            return false;
        }
        let cmd = if IsIconic(hwnd).as_bool() {
            SW_RESTORE
        } else {
            SW_SHOW
        };
        let _ = ShowWindow(hwnd, cmd);
        let _ = SetForegroundWindow(hwnd);
    }
    true
}

pub fn request_hwnd_close(hwnd: isize) -> bool {
    unsafe {
        let hwnd = HWND(hwnd as _);
        if !IsWindow(Some(hwnd)).as_bool() {
            return false;
        }
        PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok()
    }
}

/// Bring an existing GUI window to the foreground.
pub fn activate_gui_window() -> bool {
    if let Some(hwnd) = registered_gui_hwnd() {
        return activate_hwnd(hwnd.0 as isize);
    }
    find_gui_hwnd()
        .map(|hwnd| activate_hwnd(hwnd.0 as isize))
        .unwrap_or(false)
}

/// Ask the GUI process to close via `WM_CLOSE` (respects close-to-tray settings).
pub fn request_gui_close() -> bool {
    if let Some(hwnd) = registered_gui_hwnd() {
        return request_hwnd_close(hwnd.0 as isize);
    }
    find_gui_hwnd()
        .map(|hwnd| request_hwnd_close(hwnd.0 as isize))
        .unwrap_or(false)
}

/// Remove the maximize/restore button from the window title bar.
pub fn remove_maximize_button(window: &Window) -> Result<()> {
    let hwnd = hwnd_from_window(window)?;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let new_style = style & !WS_MAXIMIZEBOX.0;
        SetWindowLongPtrW(hwnd, GWL_STYLE, new_style as _);
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_window_title_matches_app_name_only() {
        assert!(gui_window_title_matches(APP_NAME));
        assert!(!gui_window_title_matches("Memory Cleanr"));
        assert!(!gui_window_title_matches(""));
    }
}
