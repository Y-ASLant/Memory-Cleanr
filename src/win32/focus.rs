//! Track the last external foreground window so paste can restore focus.
//!
//! Memory Cleanr runs elevated; `SendInput` to medium-IL apps is blocked by UIPI.
//! Restoring the previous HWND and posting `WM_PASTE` is the reliable path.

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsWindow, SetForegroundWindow,
};

static PREV_FOREGROUND_HWND: AtomicIsize = AtomicIsize::new(0);
static OUR_HWND: AtomicIsize = AtomicIsize::new(0);

/// Remember our main window HWND (excluded when saving previous focus).
pub fn set_our_hwnd(hwnd: HWND) {
    OUR_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
}

pub fn clear_our_hwnd() {
    OUR_HWND.store(0, Ordering::Relaxed);
}

/// Current main window HWND, if known and still valid.
pub fn our_hwnd() -> Option<HWND> {
    let raw = OUR_HWND.load(Ordering::Relaxed);
    if raw == 0 {
        return None;
    }
    let hwnd = HWND(raw as *mut _);
    if unsafe { IsWindow(Some(hwnd)).as_bool() } {
        Some(hwnd)
    } else {
        None
    }
}

/// Save the current foreground window if it is not our own.
pub fn save_current_focus() {
    let hwnd = unsafe { GetForegroundWindow() };
    let val = hwnd.0 as isize;
    if val == 0 {
        return;
    }
    let our = OUR_HWND.load(Ordering::Relaxed);
    if our != 0 && val == our {
        return;
    }
    PREV_FOREGROUND_HWND.store(val, Ordering::Relaxed);
}

/// After we hide ourselves, remember whoever Windows activated (fallback target).
pub fn capture_foreground_after_hide() {
    let hwnd = unsafe { GetForegroundWindow() };
    let val = hwnd.0 as isize;
    let our = OUR_HWND.load(Ordering::Relaxed);
    if val != 0 && val != our {
        PREV_FOREGROUND_HWND.store(val, Ordering::Relaxed);
    }
}

/// Force a window to the foreground (AttachThreadInput unlocks SetForegroundWindow).
pub fn force_foreground(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return false;
        }
        let fg = GetForegroundWindow();
        let cur = GetCurrentThreadId();
        let fg_tid = GetWindowThreadProcessId(fg, None);
        let target_tid = GetWindowThreadProcessId(hwnd, None);

        if fg_tid != 0 && fg_tid != cur {
            let _ = AttachThreadInput(cur, fg_tid, true);
        }
        if target_tid != 0 && target_tid != cur && target_tid != fg_tid {
            let _ = AttachThreadInput(cur, target_tid, true);
        }

        let _ = BringWindowToTop(hwnd);
        let ok = SetForegroundWindow(hwnd).as_bool();

        if fg_tid != 0 && fg_tid != cur {
            let _ = AttachThreadInput(cur, fg_tid, false);
        }
        if target_tid != 0 && target_tid != cur && target_tid != fg_tid {
            let _ = AttachThreadInput(cur, target_tid, false);
        }
        ok
    }
}

/// Restore the previously saved foreground window (best effort).
pub fn restore_previous_foreground() -> bool {
    let prev = PREV_FOREGROUND_HWND.load(Ordering::Relaxed);
    if prev == 0 {
        crate::log_msg("[focus] no previous foreground hwnd saved");
        return false;
    }
    force_foreground(HWND(prev as *mut _))
}

/// Bring our main window back to the foreground after paste.
pub fn restore_our_foreground() -> bool {
    match our_hwnd() {
        Some(hwnd) => force_foreground(hwnd),
        None => false,
    }
}

/// HWND that should receive paste (saved previous, else current foreground).
pub fn paste_target_hwnd() -> Option<HWND> {
    let prev = PREV_FOREGROUND_HWND.load(Ordering::Relaxed);
    if prev != 0 {
        let hwnd = HWND(prev as *mut _);
        if unsafe { IsWindow(Some(hwnd)).as_bool() } {
            return Some(hwnd);
        }
    }
    let fg = unsafe { GetForegroundWindow() };
    let our = OUR_HWND.load(Ordering::Relaxed);
    if fg.0 as isize != 0 && fg.0 as isize != our {
        Some(fg)
    } else {
        None
    }
}
