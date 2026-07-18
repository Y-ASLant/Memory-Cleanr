//! GUI ↔ tray-host IPC over a named pipe (GUI → tray) plus a tray-ready event.

use std::io::ErrorKind;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Result, bail};
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, ERROR_PIPE_CONNECTED};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_WRITE, FILE_SHARE_NONE, OPEN_EXISTING,
    PIPE_ACCESS_INBOUND,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};

use crate::win32::wide::wide_null;

pub const TRAY_READY_EVENT_NAME: &str = "MemoryCleanr_TrayReady_v1";
pub const GUI_TO_TRAY_PIPE_NAME: &str = r"\\.\pipe\MemoryCleanr.GuiToTray.v1";
pub const TRAY_READY_WAIT_MS: u32 = 15_000;
const SEND_RECONNECT_WAIT_MS: u32 = 3_000;

const TAG_REGISTER_GUI: u8 = 1;
const TAG_UNREGISTER_GUI: u8 = 2;
const TAG_SPIN_START: u8 = 3;
const TAG_SPIN_STOP: u8 = 4;
const TAG_SETTINGS_CHANGED: u8 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuiSession {
    pub pid: u32,
    pub hwnd: isize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcMessage {
    RegisterGui { pid: u32, hwnd: isize },
    UnregisterGui,
    SpinStart,
    SpinStop,
    SettingsChanged,
}

static TRAY_GUI_SESSION: Mutex<Option<GuiSession>> = Mutex::new(None);
static GUI_IPC: OnceLock<Mutex<GuiIpcState>> = OnceLock::new();

#[derive(Default)]
struct GuiIpcState {
    writer: Option<GuiIpcWriter>,
    session: Option<GuiSession>,
}

struct GuiIpcWriter {
    pipe: Mutex<isize>,
}

fn gui_ipc() -> &'static Mutex<GuiIpcState> {
    GUI_IPC.get_or_init(|| Mutex::new(GuiIpcState::default()))
}

pub fn encode_message(message: &IpcMessage) -> Vec<u8> {
    match message {
        IpcMessage::RegisterGui { pid, hwnd } => {
            let mut payload = Vec::with_capacity(1 + 4 + 8);
            payload.push(TAG_REGISTER_GUI);
            payload.extend_from_slice(&pid.to_le_bytes());
            payload.extend_from_slice(&hwnd.to_le_bytes());
            payload
        }
        IpcMessage::UnregisterGui => vec![TAG_UNREGISTER_GUI],
        IpcMessage::SpinStart => vec![TAG_SPIN_START],
        IpcMessage::SpinStop => vec![TAG_SPIN_STOP],
        IpcMessage::SettingsChanged => vec![TAG_SETTINGS_CHANGED],
    }
}

pub fn decode_message(payload: &[u8]) -> Result<IpcMessage> {
    let Some(&tag) = payload.first() else {
        bail!("empty IPC payload");
    };
    match tag {
        TAG_REGISTER_GUI => {
            if payload.len() != 1 + 4 + 8 {
                bail!("invalid RegisterGui payload length");
            }
            let pid = u32::from_le_bytes(payload[1..5].try_into().expect("pid bytes"));
            let hwnd = isize::from_le_bytes(payload[5..13].try_into().expect("hwnd bytes"));
            Ok(IpcMessage::RegisterGui { pid, hwnd })
        }
        TAG_UNREGISTER_GUI => Ok(IpcMessage::UnregisterGui),
        TAG_SPIN_START => Ok(IpcMessage::SpinStart),
        TAG_SPIN_STOP => Ok(IpcMessage::SpinStop),
        TAG_SETTINGS_CHANGED => Ok(IpcMessage::SettingsChanged),
        _ => bail!("unknown IPC tag {tag}"),
    }
}

fn encode_frame(payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    frame
}

unsafe fn write_all(handle: HANDLE, bytes: &[u8]) -> Result<()> {
    let mut offset = 0usize;
    while offset < bytes.len() {
        let mut written = 0u32;
        let ok = unsafe {
            windows::Win32::Storage::FileSystem::WriteFile(
                handle,
                Some(&bytes[offset..]),
                Some(&mut written),
                None,
            )
        };
        if ok.is_err() || written == 0 {
            bail!("WriteFile failed while sending IPC frame");
        }
        offset += written as usize;
    }
    Ok(())
}

unsafe fn read_exact(handle: HANDLE, buffer: &mut [u8]) -> Result<()> {
    let mut offset = 0usize;
    while offset < buffer.len() {
        let mut read = 0u32;
        let ok = unsafe {
            windows::Win32::Storage::FileSystem::ReadFile(
                handle,
                Some(&mut buffer[offset..]),
                Some(&mut read),
                None,
            )
        };
        if ok.is_err() || read == 0 {
            bail!("ReadFile failed while receiving IPC frame");
        }
        offset += read as usize;
    }
    Ok(())
}

unsafe fn read_frame(handle: HANDLE) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    unsafe { read_exact(handle, &mut len_buf)? };
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 4096 {
        bail!("IPC frame too large ({len} bytes)");
    }
    let mut payload = vec![0u8; len];
    unsafe { read_exact(handle, &mut payload)? };
    Ok(payload)
}

pub fn set_gui_session(session: Option<GuiSession>) {
    if let Ok(mut guard) = TRAY_GUI_SESSION.lock() {
        *guard = session;
    }
}

pub fn gui_session() -> Option<GuiSession> {
    TRAY_GUI_SESSION.lock().ok().and_then(|guard| *guard)
}

pub fn signal_tray_ready() {
    unsafe {
        let name = wide_null(TRAY_READY_EVENT_NAME);
        if let Ok(event) = CreateEventW(None, true, true, windows::core::PCWSTR(name.as_ptr())) {
            let _ = SetEvent(event);
            let _ = CloseHandle(event);
        }
    }
}

pub fn wait_tray_ready(timeout_ms: u32) -> bool {
    unsafe {
        let name = wide_null(TRAY_READY_EVENT_NAME);
        let Ok(event) = CreateEventW(
            None,
            true,
            false,
            windows::core::PCWSTR(name.as_ptr()),
        ) else {
            return false;
        };
        let wait_ms = timeout_ms.min(TRAY_READY_WAIT_MS);
        let result = WaitForSingleObject(event, wait_ms);
        let _ = CloseHandle(event);
        result.0 == 0 || result.0 == 0x0000_0080 // WAIT_OBJECT_0 or WAIT_ABANDONED
    }
}

impl GuiIpcWriter {
    fn connect(timeout_ms: u32) -> Result<Self> {
        let steps = (timeout_ms / 50).max(1);
        for _ in 0..steps {
            if wait_tray_ready(50) {
                match Self::try_open_pipe() {
                    Ok(writer) => return Ok(writer),
                    Err(error) => {
                        if error.downcast_ref::<std::io::Error>().is_some_and(|e| {
                            e.kind() == ErrorKind::NotFound || e.kind() == ErrorKind::WouldBlock
                        }) {
                            thread::sleep(Duration::from_millis(50));
                            continue;
                        }
                        return Err(error);
                    }
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
        bail!("tray IPC pipe unavailable")
    }

    fn try_open_pipe() -> Result<Self> {
        unsafe {
            let name = wide_null(GUI_TO_TRAY_PIPE_NAME);
            let handle = CreateFileW(
                windows::core::PCWSTR(name.as_ptr()),
                FILE_GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(HANDLE::default()),
            );
            if handle.is_err() {
                bail!(std::io::Error::new(
                    ErrorKind::NotFound,
                    "IPC pipe not ready"
                ));
            }
            Ok(Self {
                pipe: Mutex::new(handle?.0 as isize),
            })
        }
    }

    fn send(&self, message: &IpcMessage) -> Result<()> {
        let payload = encode_message(message);
        let frame = encode_frame(&payload);
        let guard = self
            .pipe
            .lock()
            .map_err(|_| anyhow::anyhow!("IPC pipe mutex poisoned"))?;
        if *guard == 0 {
            bail!("IPC pipe handle is closed");
        }
        unsafe { write_all(HANDLE(*guard as _), &frame) }
    }

    fn close(&self) {
        if let Ok(mut guard) = self.pipe.lock()
            && *guard != 0
        {
            unsafe {
                let _ = CloseHandle(HANDLE(*guard as _));
            }
            *guard = 0;
        }
    }
}

fn connect_gui_writer(state: &mut GuiIpcState, timeout_ms: u32) -> Result<()> {
    if let Some(writer) = state.writer.as_ref() {
        writer.close();
    }
    state.writer = None;

    let writer = GuiIpcWriter::connect(timeout_ms)?;
    if let Some(session) = state.session {
        writer.send(&IpcMessage::RegisterGui {
            pid: session.pid,
            hwnd: session.hwnd,
        })?;
        set_gui_session(Some(session));
    }
    state.writer = Some(writer);
    Ok(())
}

fn clear_gui_writer(state: &mut GuiIpcState) {
    if let Some(writer) = state.writer.as_ref() {
        writer.close();
    }
    state.writer = None;
    state.session = None;
    set_gui_session(None);
}

/// Register the GUI window with the tray host and establish the IPC pipe.
pub fn register_gui_session(session: GuiSession) -> Result<()> {
    let mut state = gui_ipc()
        .lock()
        .map_err(|_| anyhow::anyhow!("GUI IPC state mutex poisoned"))?;
    state.session = Some(session);
    connect_gui_writer(&mut state, TRAY_READY_WAIT_MS)
}

pub fn send_to_tray(message: IpcMessage) -> Result<()> {
    let is_unregister = matches!(message, IpcMessage::UnregisterGui);
    let mut state = gui_ipc()
        .lock()
        .map_err(|_| anyhow::anyhow!("GUI IPC state mutex poisoned"))?;

    if state.writer.is_none() {
        if is_unregister {
            clear_gui_writer(&mut state);
            return Ok(());
        }
        if state.session.is_some() {
            connect_gui_writer(&mut state, SEND_RECONNECT_WAIT_MS)?;
        } else {
            bail!("GUI IPC writer is not initialized");
        }
    }

    let writer = state
        .writer
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("GUI IPC writer is not connected"))?;

    match writer.send(&message) {
        Ok(()) => {
            if is_unregister {
                clear_gui_writer(&mut state);
            }
            Ok(())
        }
        Err(error) => {
            crate::log_msg(&format!("[ipc] send failed, reconnecting: {error:#}"));
            if is_unregister {
                clear_gui_writer(&mut state);
                return Ok(());
            }
            if state.session.is_none() {
                return Err(error);
            }
            connect_gui_writer(&mut state, SEND_RECONNECT_WAIT_MS)?;
            state
                .writer
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("GUI IPC reconnect failed"))?
                .send(&message)
        }
    }
}

pub fn send_to_tray_logged(message: IpcMessage, context: &str) {
    if let Err(error) = send_to_tray(message) {
        crate::log_msg(&format!("[ipc] {context} failed: {error:#}"));
    }
}

pub fn spawn_tray_server(
    notify_hwnd: isize,
    pending: Arc<Mutex<Vec<IpcMessage>>>,
    shutdown: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("tray-ipc-server".into())
        .spawn(move || {
            if let Err(error) = run_tray_server_loop(notify_hwnd, pending, shutdown) {
                crate::log_msg(&format!("[ipc] tray server exited: {error:#}"));
            }
        })
        .expect("spawn tray IPC server")
}

/// Unblock `ConnectNamedPipe` / `ReadFile` so the tray IPC thread can exit.
pub fn wake_tray_server(shutdown: &AtomicBool) {
    shutdown.store(true, Ordering::Release);
    poke_tray_pipe_client();
}

fn poke_tray_pipe_client() {
    unsafe {
        let name = wide_null(GUI_TO_TRAY_PIPE_NAME);
        let handle = CreateFileW(
            windows::core::PCWSTR(name.as_ptr()),
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(HANDLE::default()),
        );
        if let Ok(handle) = handle {
            let _ = CloseHandle(handle);
        }
    }
}

fn run_tray_server_loop(
    notify_hwnd: isize,
    pending: Arc<Mutex<Vec<IpcMessage>>>,
    shutdown: Arc<AtomicBool>,
) -> Result<()> {
    signal_tray_ready();
    while !shutdown.load(Ordering::Acquire) {
        let pipe = unsafe { create_server_pipe()? };
        let connected = unsafe { ConnectNamedPipe(pipe, None) };
        if connected.is_err() && unsafe { windows::Win32::Foundation::GetLastError() }
            != ERROR_PIPE_CONNECTED
        {
            let _ = unsafe { CloseHandle(pipe) };
            if shutdown.load(Ordering::Acquire) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        if shutdown.load(Ordering::Acquire) {
            unsafe {
                let _ = CloseHandle(pipe);
            }
            break;
        }

        let read_result = read_messages_from_pipe(pipe, &pending, notify_hwnd);
        set_gui_session(None);
        unsafe {
            let _ = DisconnectNamedPipe(pipe);
            let _ = CloseHandle(pipe);
        }
        if read_result.is_err() {
            crate::log_msg("[ipc] GUI disconnected");
        }
    }
    Ok(())
}

unsafe fn create_server_pipe() -> Result<HANDLE> {
    let name = wide_null(GUI_TO_TRAY_PIPE_NAME);
    let handle = unsafe {
        CreateNamedPipeW(
            windows::core::PCWSTR(name.as_ptr()),
            PIPE_ACCESS_INBOUND,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            4096,
            4096,
            0,
            None,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        bail!("CreateNamedPipeW failed");
    }
    Ok(handle)
}

fn read_messages_from_pipe(
    pipe: HANDLE,
    pending: &Arc<Mutex<Vec<IpcMessage>>>,
    notify_hwnd: isize,
) -> Result<()> {
    loop {
        let payload = match unsafe { read_frame(pipe) } {
            Ok(payload) => payload,
            Err(_) => break,
        };
        let message = decode_message(&payload)?;
        if let IpcMessage::RegisterGui { pid, hwnd } = message {
            set_gui_session(Some(GuiSession { pid, hwnd }));
        }
        if matches!(message, IpcMessage::UnregisterGui) {
            set_gui_session(None);
        }
        if let Ok(mut queue) = pending.lock() {
            queue.push(message);
        }
        post_ipc_notify(notify_hwnd);
    }
    Ok(())
}

fn post_ipc_notify(notify_hwnd: isize) {
    crate::win32::message_loop::post_tray_cmd_to_hwnd(notify_hwnd);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_gui_payload_roundtrip() {
        let payload = encode_message(&IpcMessage::RegisterGui {
            pid: 4242,
            hwnd: 0x1234,
        });
        let decoded = decode_message(&payload).expect("decode");
        assert_eq!(
            decoded,
            IpcMessage::RegisterGui {
                pid: 4242,
                hwnd: 0x1234
            }
        );
    }

    #[test]
    fn simple_ipc_tags_roundtrip() {
        for message in [
            IpcMessage::UnregisterGui,
            IpcMessage::SpinStart,
            IpcMessage::SpinStop,
            IpcMessage::SettingsChanged,
        ] {
            let payload = encode_message(&message);
            assert_eq!(decode_message(&payload).expect("decode"), message);
        }
    }

    #[test]
    fn frame_length_prefix_is_little_endian() {
        let frame = encode_frame(&[TAG_SPIN_START]);
        assert_eq!(frame, vec![1, 0, 0, 0, TAG_SPIN_START]);
    }

    #[test]
    fn unregister_without_writer_clears_state() {
        let session = GuiSession {
            pid: 42,
            hwnd: 0x100,
        };
        {
            let mut state = gui_ipc().lock().expect("ipc lock");
            state.session = Some(session);
        }
        send_to_tray(IpcMessage::UnregisterGui).expect("unregister");
        let state = gui_ipc().lock().expect("ipc lock");
        assert!(state.writer.is_none());
        assert!(state.session.is_none());
    }
}
