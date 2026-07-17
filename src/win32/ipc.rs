//! GUI ↔ tray-host IPC over a named pipe (GUI → tray) plus a tray-ready event.

use std::io::ErrorKind;
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

pub const TRAY_READY_EVENT_NAME: &str = "MemoryCleanr_TrayReady_v1";
pub const GUI_TO_TRAY_PIPE_NAME: &str = r"\\.\pipe\MemoryCleanr.GuiToTray.v1";
pub const TRAY_READY_WAIT_MS: u32 = 15_000;

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
pub enum IpcIncoming {
    RegisterGui { pid: u32, hwnd: isize },
    UnregisterGui,
    SpinStart,
    SpinStop,
    SettingsChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcOutgoing {
    RegisterGui { pid: u32, hwnd: isize },
    UnregisterGui,
    SpinStart,
    SpinStop,
    SettingsChanged,
}

static GUI_SESSION: Mutex<Option<GuiSession>> = Mutex::new(None);
static GUI_WRITER: OnceLock<Arc<GuiIpcWriter>> = OnceLock::new();

struct GuiIpcWriter {
    pipe: Mutex<isize>,
}

fn wide_name(name: &str) -> Vec<u16> {
    name.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn encode_outgoing(message: &IpcOutgoing) -> Vec<u8> {
    match message {
        IpcOutgoing::RegisterGui { pid, hwnd } => {
            let mut payload = Vec::with_capacity(1 + 4 + 8);
            payload.push(TAG_REGISTER_GUI);
            payload.extend_from_slice(&pid.to_le_bytes());
            payload.extend_from_slice(&hwnd.to_le_bytes());
            payload
        }
        IpcOutgoing::UnregisterGui => vec![TAG_UNREGISTER_GUI],
        IpcOutgoing::SpinStart => vec![TAG_SPIN_START],
        IpcOutgoing::SpinStop => vec![TAG_SPIN_STOP],
        IpcOutgoing::SettingsChanged => vec![TAG_SETTINGS_CHANGED],
    }
}

pub fn decode_incoming(payload: &[u8]) -> Result<IpcIncoming> {
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
            Ok(IpcIncoming::RegisterGui { pid, hwnd })
        }
        TAG_UNREGISTER_GUI => Ok(IpcIncoming::UnregisterGui),
        TAG_SPIN_START => Ok(IpcIncoming::SpinStart),
        TAG_SPIN_STOP => Ok(IpcIncoming::SpinStop),
        TAG_SETTINGS_CHANGED => Ok(IpcIncoming::SettingsChanged),
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
    if let Ok(mut guard) = GUI_SESSION.lock() {
        *guard = session;
    }
}

pub fn gui_session() -> Option<GuiSession> {
    GUI_SESSION.lock().ok().and_then(|guard| *guard)
}

pub fn signal_tray_ready() {
    unsafe {
        let name = wide_name(TRAY_READY_EVENT_NAME);
        if let Ok(event) = CreateEventW(None, true, true, windows::core::PCWSTR(name.as_ptr())) {
            let _ = SetEvent(event);
            let _ = CloseHandle(event);
        }
    }
}

pub fn wait_tray_ready(timeout_ms: u32) -> bool {
    unsafe {
        let name = wide_name(TRAY_READY_EVENT_NAME);
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
            let name = wide_name(GUI_TO_TRAY_PIPE_NAME);
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

    pub fn send(&self, message: IpcOutgoing) -> Result<()> {
        let payload = encode_outgoing(&message);
        let frame = encode_frame(&payload);
        let guard = self
            .pipe
            .lock()
            .map_err(|_| anyhow::anyhow!("IPC pipe mutex poisoned"))?;
        unsafe { write_all(HANDLE(*guard as _), &frame) }
    }
}

pub fn init_gui_writer(session: GuiSession) -> Result<()> {
    let writer = Arc::new(GuiIpcWriter::connect(TRAY_READY_WAIT_MS)?);
    writer.send(IpcOutgoing::RegisterGui {
        pid: session.pid,
        hwnd: session.hwnd,
    })?;
    let _ = GUI_WRITER.set(Arc::clone(&writer));
    set_gui_session(Some(session));
    Ok(())
}

pub fn send_gui_message(message: IpcOutgoing) -> Result<()> {
    let Some(writer) = GUI_WRITER.get() else {
        bail!("GUI IPC writer is not initialized");
    };
    writer.send(message)
}

pub fn notify_settings_changed() {
    let _ = send_gui_message(IpcOutgoing::SettingsChanged).map_err(|error| {
        crate::log_msg(&format!("[ipc] settings notify failed: {error:#}"));
    });
}

pub fn notify_spin_start() {
    let _ = send_gui_message(IpcOutgoing::SpinStart).map_err(|error| {
        crate::log_msg(&format!("[ipc] spin start failed: {error:#}"));
    });
}

pub fn notify_spin_stop() {
    let _ = send_gui_message(IpcOutgoing::SpinStop).map_err(|error| {
        crate::log_msg(&format!("[ipc] spin stop failed: {error:#}"));
    });
}

pub fn unregister_gui() {
    let _ = send_gui_message(IpcOutgoing::UnregisterGui).map_err(|error| {
        crate::log_msg(&format!("[ipc] unregister failed: {error:#}"));
    });
    set_gui_session(None);
}

pub fn spawn_tray_server(
    notify_hwnd: isize,
    pending: Arc<Mutex<Vec<IpcIncoming>>>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("tray-ipc-server".into())
        .spawn(move || {
            if let Err(error) = run_tray_server_loop(notify_hwnd, pending) {
                crate::log_msg(&format!("[ipc] tray server exited: {error:#}"));
            }
        })
        .expect("spawn tray IPC server")
}

fn run_tray_server_loop(notify_hwnd: isize, pending: Arc<Mutex<Vec<IpcIncoming>>>) -> Result<()> {
    signal_tray_ready();
    loop {
        let pipe = unsafe { create_server_pipe()? };
        let connected = unsafe { ConnectNamedPipe(pipe, None) };
        if connected.is_err() && unsafe { windows::Win32::Foundation::GetLastError() }
            != ERROR_PIPE_CONNECTED
        {
            let _ = unsafe { CloseHandle(pipe) };
            thread::sleep(Duration::from_millis(50));
            continue;
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
}

unsafe fn create_server_pipe() -> Result<HANDLE> {
    let name = wide_name(GUI_TO_TRAY_PIPE_NAME);
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
    pending: &Arc<Mutex<Vec<IpcIncoming>>>,
    notify_hwnd: isize,
) -> Result<()> {
    loop {
        let payload = match unsafe { read_frame(pipe) } {
            Ok(payload) => payload,
            Err(_) => break,
        };
        let message = decode_incoming(&payload)?;
        if let IpcIncoming::RegisterGui { pid, hwnd } = message {
            set_gui_session(Some(GuiSession { pid, hwnd }));
        }
        if matches!(message, IpcIncoming::UnregisterGui) {
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
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            Some(windows::Win32::Foundation::HWND(notify_hwnd as *mut _)),
            crate::win32::message_loop::WM_APP_TRAY_CMD,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_gui_payload_roundtrip() {
        let payload = encode_outgoing(&IpcOutgoing::RegisterGui {
            pid: 4242,
            hwnd: 0x1234,
        });
        let decoded = decode_incoming(&payload).expect("decode");
        assert_eq!(
            decoded,
            IpcIncoming::RegisterGui {
                pid: 4242,
                hwnd: 0x1234
            }
        );
    }

    #[test]
    fn simple_ipc_tags_roundtrip() {
        for message in [
            IpcOutgoing::UnregisterGui,
            IpcOutgoing::SpinStart,
            IpcOutgoing::SpinStop,
            IpcOutgoing::SettingsChanged,
        ] {
            let payload = encode_outgoing(&message);
            let decoded = decode_incoming(&payload).expect("decode");
            let expected = match message {
                IpcOutgoing::UnregisterGui => IpcIncoming::UnregisterGui,
                IpcOutgoing::SpinStart => IpcIncoming::SpinStart,
                IpcOutgoing::SpinStop => IpcIncoming::SpinStop,
                IpcOutgoing::SettingsChanged => IpcIncoming::SettingsChanged,
                IpcOutgoing::RegisterGui { .. } => unreachable!(),
            };
            assert_eq!(decoded, expected);
        }
    }

    #[test]
    fn frame_length_prefix_is_little_endian() {
        let frame = encode_frame(&[TAG_SPIN_START]);
        assert_eq!(frame, vec![1, 0, 0, 0, TAG_SPIN_START]);
    }
}
