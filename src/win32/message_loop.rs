//! Hidden Win32 message window for tray-host timer and command dispatch.

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_MESSAGE, KillTimer, MSG,
    PM_REMOVE, PeekMessageW, PostMessageW, PostQuitMessage, RegisterClassW, SetTimer,
    TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_DESTROY, WM_TIMER, WNDCLASSW,
};

pub const WM_APP_TRAY_CMD: u32 = windows::Win32::UI::WindowsAndMessaging::WM_USER + 42;
const TIMER_ID: usize = 1;
const TIMER_INTERVAL_MS: u32 = 500;

static CLASS_REGISTERED: std::sync::OnceLock<()> = std::sync::OnceLock::new();

pub struct MessageLoop {
    hwnd: HWND,
}

impl MessageLoop {
    /// Drop stale messages (especially `WM_QUIT`) left on this thread by GPUI.
    pub fn flush_thread_queue() {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {}
        }
    }

    pub fn new() -> Result<Self> {
        unsafe {
            register_window_class()?;
            let hwnd = create_message_window()?;
            Ok(Self { hwnd })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn start_timer(&self) {
        unsafe {
            let _ = SetTimer(Some(self.hwnd), TIMER_ID, TIMER_INTERVAL_MS, None);
        }
    }

    pub fn post_tray_command(&self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_APP_TRAY_CMD, WPARAM(0), LPARAM(0));
        }
    }

    pub fn request_quit(&self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_DESTROY, WPARAM(0), LPARAM(0));
        }
    }

    /// Runs until `PostQuitMessage` is posted (e.g. via `request_quit`).
    pub fn run<F>(&self, mut dispatch: F)
    where
        F: FnMut(u32),
    {
        unsafe {
            let mut msg = MSG::default();
            loop {
                let result = GetMessageW(&mut msg, None, 0, 0);
                if result.0 == 0 {
                    crate::log_msg("[tray] GetMessageW returned WM_QUIT");
                    break;
                }
                if result.0 == -1 {
                    crate::log_msg("[tray] GetMessageW failed");
                    break;
                }

                match msg.message {
                    WM_TIMER => dispatch(WM_TIMER),
                    WM_APP_TRAY_CMD => dispatch(WM_APP_TRAY_CMD),
                    WM_DESTROY => {
                        let _ = KillTimer(Some(self.hwnd), TIMER_ID);
                        PostQuitMessage(0);
                    }
                    _ => {}
                }

                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

impl Drop for MessageLoop {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_ID);
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.hwnd);
        }
    }
}

unsafe fn register_window_class() -> Result<()> {
    if CLASS_REGISTERED.get().is_some() {
        return Ok(());
    }

    let instance = unsafe { GetModuleHandleW(None).context("GetModuleHandleW failed")? };
    let class_name = windows::core::w!("MemoryCleanrTrayHost");

    let wnd_class = WNDCLASSW {
        lpfnWndProc: Some(tray_host_wnd_proc),
        hInstance: windows::Win32::Foundation::HINSTANCE(instance.0),
        lpszClassName: class_name,
        ..Default::default()
    };

    let atom = unsafe { RegisterClassW(&wnd_class) };
    if atom == 0 {
        bail!("RegisterClassW failed for MemoryCleanrTrayHost");
    }

    let _ = CLASS_REGISTERED.set(());
    Ok(())
}

unsafe extern "system" fn tray_host_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DESTROY => {
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe fn create_message_window() -> Result<HWND> {
    let instance = unsafe { GetModuleHandleW(None).context("GetModuleHandleW failed")? };
    let class_name = windows::core::w!("MemoryCleanrTrayHost");
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            windows::core::w!("MemoryCleanrTrayHost"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
            None,
        )
    }
    .context("CreateWindowExW failed for tray host")?;

    Ok(hwnd)
}
