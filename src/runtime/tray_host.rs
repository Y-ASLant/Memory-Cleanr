use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use rust_i18n::t;

use crate::locale;
use crate::memory::MemorySection;
use crate::runtime::TrayReceiver;
use crate::service::{memory, optimize_runner};
use crate::settings::Settings;
use crate::tray::{self, TrayCommand};
use crate::win32::ipc::{self, IpcMessage};
use crate::win32::message_loop::MessageLoop;

const FORWARDER_POLL_MS: u64 = 100;

struct TrayHostState {
    settings: Settings,
    physical: MemorySection,
    virtual_mem: Option<MemorySection>,
    is_optimizing: Arc<AtomicBool>,
    command_pending: Arc<std::sync::Mutex<Vec<TrayCommand>>>,
    notify_hwnd: isize,
}

impl TrayHostState {
    fn new(
        settings: Settings,
        command_pending: Arc<std::sync::Mutex<Vec<TrayCommand>>>,
        notify_hwnd: isize,
    ) -> Self {
        locale::apply(&settings);
        crate::log::set_debug_enabled(settings.debug_logging);
        if settings.debug_logging {
            crate::log_msg(&format!(
                "[log] debug enabled path={}",
                crate::log::log_file_path().display()
            ));
        }

        let show_virtual = settings.show_virtual_memory;
        let (physical, virtual_mem) = memory::initial_sections(show_virtual);

        Self {
            settings,
            physical,
            virtual_mem,
            is_optimizing: Arc::new(AtomicBool::new(false)),
            command_pending,
            notify_hwnd,
        }
    }

    fn sync_tray(&self) {
        let virtual_mem =
            memory::virtual_for_display(&self.virtual_mem, self.settings.show_virtual_memory);
        let window_visible = ipc::gui_session()
            .is_some_and(|session| crate::win32::window::is_hwnd_visible(session.hwnd))
            || crate::win32::window::is_gui_window_visible();
        tray::sync_display(&self.physical, virtual_mem, window_visible);
    }

    fn refresh_memory(&mut self) -> bool {
        memory::refresh_sections(
            &mut self.physical,
            &mut self.virtual_mem,
            self.settings.show_virtual_memory,
        )
    }

    fn spawn_optimize(&mut self) {
        if self
            .is_optimizing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return;
        }

        tray::start_spin();

        let settings = self.settings.clone();
        let avail_before = self.physical.avail;
        let is_optimizing = Arc::clone(&self.is_optimizing);
        let pending = Arc::clone(&self.command_pending);
        let notify_hwnd = self.notify_hwnd;

        thread::Builder::new()
            .name("tray-optimize".into())
            .spawn(move || {
                if settings.show_optimization_notifications
                    && let Err(e) = crate::win32::notification::show(
                        &t!("notification.optimize_start_title"),
                        &t!("notification.optimize_start_body"),
                    )
                {
                    crate::log_msg(&format!("[notification] failed: {e:#}"));
                }

                let result = optimize_runner::run(&settings, avail_before, |_update| {});

                tray::stop_spin();
                is_optimizing.store(false, Ordering::Release);

                if settings.show_optimization_notifications
                    && !result.status_message.is_empty()
                    && result.status_message != t!("tooltip.select_areas")
                    && let Err(e) = crate::win32::notification::show(
                        &t!("notification.optimize_title"),
                        &result.status_message,
                    )
                {
                    crate::log_msg(&format!("[notification] failed: {e:#}"));
                }

                if let Ok(mut queue) = pending.lock() {
                    queue.push(TrayCommand::RefreshTooltip);
                }
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        Some(windows::Win32::Foundation::HWND(notify_hwnd as *mut _)),
                        crate::win32::message_loop::WM_APP_TRAY_CMD,
                        windows::Win32::Foundation::WPARAM(0),
                        windows::Win32::Foundation::LPARAM(0),
                    );
                }
            })
            .ok();
    }
}

pub fn run(settings: &Settings, tray_rx: TrayReceiver) -> Result<()> {
    MessageLoop::flush_thread_queue();

    let message_loop = MessageLoop::new()?;
    message_loop.start_timer();

    let pending = Arc::new(std::sync::Mutex::new(Vec::<TrayCommand>::new()));
    let ipc_pending = Arc::new(std::sync::Mutex::new(Vec::<IpcMessage>::new()));
    let mut state = TrayHostState::new(
        settings.clone(),
        Arc::clone(&pending),
        message_loop.hwnd().0 as isize,
    );
    state.sync_tray();

    let shutdown = Arc::new(AtomicBool::new(false));
    let ipc_server = ipc::spawn_tray_server(
        message_loop.hwnd().0 as isize,
        Arc::clone(&ipc_pending),
        Arc::clone(&shutdown),
    );

    let pending_for_thread = Arc::clone(&pending);
    let hwnd = message_loop.hwnd();
    let hwnd_token = hwnd.0 as isize;
    let shutdown_for_thread = Arc::clone(&shutdown);

    let forwarder = thread::Builder::new()
        .name("tray-cmd-forward".into())
        .spawn(move || {
            while !shutdown_for_thread.load(Ordering::Acquire) {
                let command = tray_rx
                    .lock()
                    .expect("tray receiver mutex poisoned")
                    .recv_timeout(Duration::from_millis(FORWARDER_POLL_MS));

                match command {
                    Ok(command) => {
                        if let Ok(mut queue) = pending_for_thread.lock() {
                            queue.push(command);
                        }
                        unsafe {
                            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                                Some(windows::Win32::Foundation::HWND(hwnd_token as *mut _)),
                                crate::win32::message_loop::WM_APP_TRAY_CMD,
                                windows::Win32::Foundation::WPARAM(0),
                                windows::Win32::Foundation::LPARAM(0),
                            );
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })?;

    message_loop.run(|msg| match msg {
        WM_TIMER => {
            state.refresh_memory();
            state.sync_tray();
        }
        WM_APP_TRAY_CMD => {
            let commands: Vec<TrayCommand> = pending
                .lock()
                .map(|mut queue| queue.drain(..).collect())
                .unwrap_or_default();
            let ipc_messages: Vec<IpcMessage> = ipc_pending
                .lock()
                .map(|mut queue| queue.drain(..).collect())
                .unwrap_or_default();

            for message in ipc_messages {
                dispatch_ipc(&mut state, message);
            }

            for command in commands {
                if dispatch_command(&mut state, command, &shutdown) {
                    message_loop.request_quit();
                    return;
                }
            }
        }
        _ => {}
    });

    drop(message_loop);
    ipc::wake_tray_server(&shutdown);
    let _ = forwarder.join();
    let _ = ipc_server.join();

    crate::log_msg("[tray] message loop exited");
    Ok(())
}

const WM_TIMER: u32 = windows::Win32::UI::WindowsAndMessaging::WM_TIMER;
const WM_APP_TRAY_CMD: u32 = crate::win32::message_loop::WM_APP_TRAY_CMD;

fn reload_settings(state: &mut TrayHostState) {
    state.settings = Settings::load();
    locale::apply(&state.settings);
    crate::log::set_debug_enabled(state.settings.debug_logging);
    crate::win32::hotkey::sync(&state.settings);
    state.refresh_memory();
    state.sync_tray();
}

fn dispatch_ipc(state: &mut TrayHostState, message: IpcMessage) {
    match message {
        IpcMessage::RegisterGui { .. } | IpcMessage::UnregisterGui => {
            state.sync_tray();
        }
        IpcMessage::SpinStart => {
            tray::start_spin();
        }
        IpcMessage::SpinStop => {
            tray::stop_spin();
        }
        IpcMessage::SettingsChanged => {
            reload_settings(state);
        }
    }
}

/// Returns `true` when the tray host should exit.
fn dispatch_command(
    state: &mut TrayHostState,
    command: TrayCommand,
    shutdown: &AtomicBool,
) -> bool {
    match command {
        TrayCommand::ActivateWindow => {
            if let Err(error) = crate::win32::process::activate_or_spawn_gui() {
                crate::log_msg(&format!("[tray] activate GUI failed: {error:#}"));
            }
            false
        }
        TrayCommand::RefreshTooltip => {
            state.refresh_memory();
            state.sync_tray();
            false
        }
        TrayCommand::Optimize => {
            state.spawn_optimize();
            false
        }
        TrayCommand::MenuAction(action) => match action.as_str() {
            "optimize" => {
                state.spawn_optimize();
                false
            }
            "toggle_window" => {
                if let Err(error) = crate::win32::process::toggle_gui_window() {
                    crate::log_msg(&format!("[tray] toggle GUI failed: {error:#}"));
                }
                false
            }
            "quit" => {
                ipc::wake_tray_server(shutdown);
                crate::win32::process::request_gui_shutdown();
                true
            }
            _ => false,
        },
        TrayCommand::SetSpinFrame(quarters) => {
            tray::apply_spin_frame(quarters);
            false
        }
    }
}

impl Drop for TrayHostState {
    fn drop(&mut self) {
        while self.is_optimizing.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.refresh_memory();
        self.sync_tray();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn test_host() -> TrayHostState {
        TrayHostState::new(Settings::default(), Arc::new(Mutex::new(Vec::new())), 0)
    }

    #[test]
    fn dispatch_command_refreshes_tooltip_without_quitting() {
        let mut host = test_host();
        let shutdown = AtomicBool::new(false);
        assert!(!dispatch_command(
            &mut host,
            TrayCommand::RefreshTooltip,
            &shutdown,
        ));
    }

    #[test]
    fn dispatch_command_applies_spin_frame_without_quitting() {
        let mut host = test_host();
        let shutdown = AtomicBool::new(false);
        assert!(!dispatch_command(
            &mut host,
            TrayCommand::SetSpinFrame(1),
            &shutdown,
        ));
    }

    #[test]
    fn dispatch_command_quits_on_menu_quit() {
        let mut host = test_host();
        let shutdown = AtomicBool::new(false);
        assert!(dispatch_command(
            &mut host,
            TrayCommand::MenuAction("quit".into()),
            &shutdown,
        ));
        assert!(shutdown.load(Ordering::Acquire));
    }
}
