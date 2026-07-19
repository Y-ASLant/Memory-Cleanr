use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, GetClipboardData, OpenClipboard,
    RemoveClipboardFormatListener,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, MSG,
    RegisterClassW, TranslateMessage, UnregisterClassW, WINDOW_STYLE, WM_CLIPBOARDUPDATE,
    WM_CLOSE, WM_DESTROY, WNDCLASSW, WS_EX_NOACTIVATE,
};
use windows::core::w;

use super::RawClipboardContent;

/// Max text bytes to read from clipboard.
const MAX_CLIPBOARD_TEXT: usize = 1_048_576; // 1 MB
const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;

static PAUSE_UNTIL_MS: AtomicU64 = AtomicU64::new(0);

/// Ignore clipboard updates until the given duration elapses (used when pasting).
pub fn pause_monitor(duration: Duration) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    PAUSE_UNTIL_MS.store(now + duration.as_millis() as u64, Ordering::SeqCst);
}

fn monitor_paused() -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    now < PAUSE_UNTIL_MS.load(Ordering::SeqCst)
}

/// Start clipboard monitoring in a background thread.
/// Returns a receiver for clipboard events and a shutdown handle.
pub fn start_monitor() -> Result<(mpsc::Receiver<RawClipboardContent>, MonitorHandle)> {
    let (tx, rx) = mpsc::sync_channel::<RawClipboardContent>(16);
    let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();

    let join = thread::Builder::new()
        .name("clipboard-monitor".into())
        .spawn(move || {
            if let Err(e) = run_monitor_loop(tx, shutdown_clone) {
                crate::log_msg(&format!("[clipboard] monitor error: {e:#}"));
            }
        })
        .map_err(|e| anyhow::anyhow!("spawn clipboard monitor: {e}"))?;

    Ok((rx, MonitorHandle { join, shutdown }))
}

/// Handle to shut down the monitor thread.
pub struct MonitorHandle {
    join: thread::JoinHandle<()>,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl MonitorHandle {
    /// Signal the monitor thread to stop and wait for it to finish.
    pub fn shutdown(self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::SeqCst);
        // Post WM_CLOSE to break GetMessageW
        // The thread will see shutdown flag and exit
        let _ = self.join.join();
    }
}

fn run_monitor_loop(
    tx: mpsc::SyncSender<RawClipboardContent>,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<()> {
    unsafe {
        let class_name = w!("MemoryCleanrClipMonitor");

        let wnd_class = WNDCLASSW {
            lpfnWndProc: Some(clip_wnd_proc),
            hInstance: windows::Win32::System::LibraryLoader::GetModuleHandleW(None)?.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        let atom = RegisterClassW(&wnd_class);
        if atom == 0 {
            anyhow::bail!("RegisterClassW failed");
        }

        // Store tx in a static for the wnd proc to access.
        // Safety: only one monitor thread exists at a time.
        TX.with(|cell| {
            *cell.borrow_mut() = Some(tx);
        });

        let hwnd = CreateWindowExW(
            WS_EX_NOACTIVATE,
            class_name,
            w!("MemoryCleanrClipMonitor"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
        )?;

        AddClipboardFormatListener(hwnd)?;

        // Message loop — must dispatch to `clip_wnd_proc` (DefWindowProc alone never calls it).
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            if shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }

        RemoveClipboardFormatListener(hwnd)?;
        DestroyWindow(hwnd)?;
        UnregisterClassW(class_name, None)?;
    }
    Ok(())
}

// Thread-local storage for the sender channel.
thread_local! {
    static TX: std::cell::RefCell<Option<mpsc::SyncSender<RawClipboardContent>>> =
        const { std::cell::RefCell::new(None) };
}

unsafe extern "system" fn clip_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            if monitor_paused() {
                return LRESULT(0);
            }
            if let Some(content) = read_clipboard_content() {
                TX.with(|cell| {
                    if let Some(tx) = cell.borrow().as_ref() {
                        let _ = tx.try_send(content);
                    }
                });
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Read current clipboard content (text or files).
fn read_clipboard_content() -> Option<RawClipboardContent> {
    unsafe {
        if OpenClipboard(None).is_err() {
            return None;
        }
        let result = read_clipboard_inner();
        let _ = CloseClipboard();
        result
    }
}

unsafe fn read_clipboard_inner() -> Option<RawClipboardContent> {
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

    if let Some(text) = unsafe { read_unicode_text() } {
        return Some(RawClipboardContent::Text(text));
    }

    // Try files (CF_HDROP)
    let drop_handle = unsafe { GetClipboardData(CF_HDROP) };
    if let Ok(handle) = drop_handle
        && !handle.is_invalid()
    {
        let hdrop = HDROP(handle.0);
        let count = unsafe { DragQueryFileW(hdrop, 0xFFFFFFFF, None) };
        if count > 0 {
            let mut files = Vec::with_capacity(count as usize);
            for i in 0..count {
                let size = unsafe { DragQueryFileW(hdrop, i, None) } as usize;
                if size > 0 {
                    let mut buf = vec![0u16; size + 1];
                    unsafe { DragQueryFileW(hdrop, i, Some(&mut buf)) };
                    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                    if let Ok(path) = String::from_utf16(&buf[..len]) {
                        files.push(path);
                    }
                }
            }
            if !files.is_empty() {
                return Some(RawClipboardContent::Files(files));
            }
        }
    }

    None
}

unsafe fn read_unicode_text() -> Option<String> {
    use windows::Win32::Foundation::HGLOBAL;

    let handle = unsafe { GetClipboardData(CF_UNICODETEXT) };
    let Ok(handle) = handle else {
        return None;
    };
    if handle.is_invalid() {
        return None;
    }

    let hmem = HGLOBAL(handle.0);
    let ptr = unsafe { GlobalLock(hmem) };
    if ptr.is_null() {
        return None;
    }

    let text = read_utf16_from_ptr(ptr as *const u16);
    unsafe {
        let _ = GlobalUnlock(hmem);
    }
    text.filter(|t| !t.is_empty())
}

fn read_utf16_from_ptr(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0usize;
    while unsafe { *ptr.add(len) } != 0 {
        len += 1;
        if len > MAX_CLIPBOARD_TEXT / 2 {
            break;
        }
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    String::from_utf16(slice).ok()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_types_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RawClipboardContent>();
    }

    #[test]
    fn read_utf16_from_ptr_decodes_string() {
        let data: Vec<u16> = "hello\0".encode_utf16().collect();
        let text = read_utf16_from_ptr(data.as_ptr()).expect("text");
        assert_eq!(text, "hello");
    }
}
