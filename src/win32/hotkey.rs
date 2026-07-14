use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG,
    PostQuitMessage, PostThreadMessageW, RegisterClassW, TranslateMessage, WM_DESTROY, WM_HOTKEY,
    WNDCLASSW, WINDOW_EX_STYLE, WINDOW_STYLE,
};

use crate::settings::Settings;
use crate::tray::TrayCommand;

const HOTKEY_ID_OPTIMIZE: i32 = 1;
const WM_APP_SHUTDOWN: u32 = windows::Win32::UI::WindowsAndMessaging::WM_USER + 1;

static COMMAND_TX: OnceLock<Sender<TrayCommand>> = OnceLock::new();
static SERVICE: OnceLock<Mutex<HotkeyService>> = OnceLock::new();
static CLASS_REGISTERED: OnceLock<()> = OnceLock::new();

/// Parsed global hotkey chord for `RegisterHotKey`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyBinding {
    pub modifiers: HOT_KEY_MODIFIERS,
    pub virtual_key: VIRTUAL_KEY,
}

impl HotkeyBinding {
    pub const DEFAULT_CLEANUP: &'static str = "Alt+Shift+C";

    pub fn default_cleanup() -> Self {
        Self::parse(Self::DEFAULT_CLEANUP).expect("default cleanup hotkey is valid")
    }

    pub fn parse(chord: &str) -> Option<Self> {
        let chord = chord.trim();
        if chord.is_empty() {
            return None;
        }

        let parts: Vec<&str> = chord.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
        if parts.len() < 2 {
            return None;
        }

        let (modifiers, key) = parts.split_at(parts.len() - 1);
        let mut flags = HOT_KEY_MODIFIERS(0);
        for modifier in modifiers {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => flags |= MOD_CONTROL,
                "alt" => flags |= MOD_ALT,
                "shift" => flags |= MOD_SHIFT,
                "win" | "windows" => flags |= MOD_WIN,
                _ => return None,
            }
        }

        if flags == HOT_KEY_MODIFIERS(0) {
            return None;
        }

        let virtual_key = parse_virtual_key(key[0])?;
        Some(Self {
            modifiers: flags | MOD_NOREPEAT,
            virtual_key,
        })
    }
}

fn parse_virtual_key(key: &str) -> Option<VIRTUAL_KEY> {
    let key = key.trim();
    if key.len() == 1 {
        let ch = key.chars().next()?;
        if ch.is_ascii_alphabetic() {
            let vk = ch.to_ascii_uppercase() as u32;
            return Some(VIRTUAL_KEY(vk as u16));
        }
        if ch.is_ascii_digit() {
            let vk = ch as u32;
            return Some(VIRTUAL_KEY(vk as u16));
        }
    }
    None
}

struct HotkeyWorker {
    thread_id: u32,
    join_handle: Option<JoinHandle<()>>,
}

impl Drop for HotkeyWorker {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(
                self.thread_id,
                WM_APP_SHUTDOWN,
                WPARAM(0),
                LPARAM(0),
            );
        }
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

struct HotkeyService {
    worker: Option<HotkeyWorker>,
}

impl HotkeyService {
    fn new() -> Self {
        Self { worker: None }
    }

    fn apply(&mut self, settings: &Settings) {
        self.worker = None;

        if !settings.cleanup_hotkey_enabled {
            crate::log_msg("[hotkey] disabled");
            return;
        }

        let Some(binding) = HotkeyBinding::parse(&settings.cleanup_hotkey) else {
            crate::log_msg("[hotkey] invalid chord; hotkey not registered");
            return;
        };

        let Some(tx) = COMMAND_TX.get() else {
            crate::log_msg("[hotkey] command channel unavailable");
            return;
        };

        match spawn_hotkey_worker(tx.clone(), binding) {
            Ok(worker) => {
                crate::log_msg(&format!(
                    "[hotkey] registered {}",
                    settings.cleanup_hotkey
                ));
                self.worker = Some(worker);
            }
            Err(e) => crate::log_msg(&format!("[hotkey] register failed: {e:#}")),
        }
    }
}

fn spawn_hotkey_worker(_tx: Sender<TrayCommand>, binding: HotkeyBinding) -> Result<HotkeyWorker> {
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<u32>>(1);

    let join_handle = std::thread::Builder::new()
        .name("hotkey-listener".into())
        .spawn(move || {
            let thread_id = unsafe { GetCurrentThreadId() };
            let setup = run_hotkey_setup(binding);
            let _ = ready_tx.send(setup.as_ref().map(|_| thread_id).map_err(|e| {
                anyhow::anyhow!("{e:#}")
            }));

            let Ok(hwnd) = setup else {
                return;
            };

            unsafe {
                message_loop(hwnd);
                let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID_OPTIMIZE);
                let _ = DestroyWindow(hwnd);
            }
        })
        .context("failed to spawn hotkey listener thread")?;

    let thread_id = ready_rx
        .recv()
        .context("hotkey listener exited before registration completed")??;

    Ok(HotkeyWorker {
        thread_id,
        join_handle: Some(join_handle),
    })
}

pub fn bind_command_sender(tx: Sender<TrayCommand>) {
    let _ = COMMAND_TX.set(tx);
}

pub fn init(settings: &Settings) {
    let service = SERVICE.get_or_init(|| Mutex::new(HotkeyService::new()));
    service
        .lock()
        .expect("hotkey service mutex poisoned")
        .apply(settings);
}

pub fn sync(settings: &Settings) {
    let Some(service) = SERVICE.get() else {
        return;
    };
    service
        .lock()
        .expect("hotkey service mutex poisoned")
        .apply(settings);
}

fn run_hotkey_setup(binding: HotkeyBinding) -> Result<HWND> {
    unsafe {
        register_hotkey_window_class()?;

        let hwnd = create_message_window()?;
        RegisterHotKey(
            Some(hwnd),
            HOTKEY_ID_OPTIMIZE,
            binding.modifiers,
            binding.virtual_key.0 as u32,
        )
        .context("RegisterHotKey failed")?;

        Ok(hwnd)
    }
}

unsafe fn register_hotkey_window_class() -> Result<()> {
    if CLASS_REGISTERED.get().is_some() {
        return Ok(());
    }

    let instance = unsafe { GetModuleHandleW(None).context("GetModuleHandleW failed")? };
    let class_name = windows::core::w!("MemoryCleanrHotkey");

    let wnd_class = WNDCLASSW {
        lpfnWndProc: Some(hotkey_wnd_proc),
        hInstance: HINSTANCE(instance.0),
        lpszClassName: class_name,
        ..Default::default()
    };

    let atom = unsafe { RegisterClassW(&wnd_class) };
    if atom == 0 {
        bail!("RegisterClassW failed");
    }

    let _ = CLASS_REGISTERED.set(());
    Ok(())
}

unsafe fn create_message_window() -> Result<HWND> {
    let instance = unsafe { GetModuleHandleW(None).context("GetModuleHandleW failed")? };
    let class_name = windows::core::w!("MemoryCleanrHotkey");
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            windows::core::w!("MemoryCleanrHotkey"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(HINSTANCE(instance.0)),
            None,
        )
    }
    .context("CreateWindowExW failed")?;

    Ok(hwnd)
}

unsafe extern "system" fn hotkey_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_HOTKEY if wparam.0 == HOTKEY_ID_OPTIMIZE as usize => {
            if let Some(tx) = COMMAND_TX.get() {
                let _ = tx.send(TrayCommand::Optimize);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe fn message_loop(hwnd: HWND) {
    let mut msg = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if result.0 == 0 || result.0 == -1 {
            break;
        }

        if msg.message == WM_APP_SHUTDOWN {
            let _ = unsafe { DestroyWindow(hwnd) };
            continue;
        }

        let _ = unsafe { TranslateMessage(&msg) };
        unsafe { DispatchMessageW(&msg) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_default_cleanup_hotkey() {
        let binding = HotkeyBinding::parse("Alt+Shift+C").expect("valid chord");
        assert_eq!(binding.modifiers, MOD_ALT | MOD_SHIFT | MOD_NOREPEAT);
        assert_eq!(binding.virtual_key, VIRTUAL_KEY(b'C' as u16));
    }

    #[test]
    fn parse_rejects_empty_and_modifier_only_chords() {
        assert!(HotkeyBinding::parse("").is_none());
        assert!(HotkeyBinding::parse("Ctrl+Shift").is_none());
        assert!(HotkeyBinding::parse("M").is_none());
    }

    #[test]
    fn parse_supports_alt_and_win_modifiers() {
        let binding = HotkeyBinding::parse("Ctrl+Alt+O").expect("valid chord");
        assert_eq!(
            binding.modifiers,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT
        );
        assert_eq!(binding.virtual_key, VIRTUAL_KEY(b'O' as u16));

        let binding = HotkeyBinding::parse("Win+Shift+C").expect("valid chord");
        assert_eq!(
            binding.modifiers,
            MOD_WIN | MOD_SHIFT | MOD_NOREPEAT
        );
        assert_eq!(binding.virtual_key, VIRTUAL_KEY(b'C' as u16));
    }
}
