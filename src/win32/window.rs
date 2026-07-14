use anyhow::{Context, Result};
use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetWindowLongPtrW, HWND_NOTOPMOST, HWND_TOPMOST, IsIconic,
    SHOW_WINDOW_CMD, SW_HIDE, SW_RESTORE, SW_SHOW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SetWindowLongPtrW, SetWindowPos, ShowWindow, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    WS_MAXIMIZEBOX,
};

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

/// Hide the window and remove it from the taskbar (tray-only visibility).
pub fn hide_to_tray(window: &Window) -> Result<()> {
    let hwnd = hwnd_from_window(window)?;
    apply_extended_style(hwnd, |style| {
        (style | WS_EX_TOOLWINDOW.0) & !WS_EX_APPWINDOW.0
    })?;
    show_window(hwnd, SW_HIDE)?;
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
