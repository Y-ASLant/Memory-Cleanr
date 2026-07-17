use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::sync::{Mutex, mpsc};

use anyhow::Result;

use crate::settings::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Gui,
    Quit,
}

pub mod gui_app;
pub mod tray_host;

pub type TrayReceiver = Arc<Mutex<Receiver<crate::tray::TrayCommand>>>;

pub fn ensure_tray(
    command_tx: &mpsc::Sender<crate::tray::TrayCommand>,
    settings: &crate::settings::Settings,
) -> Result<()> {
    crate::tray::install(command_tx.clone()).map_err(|error| anyhow::anyhow!("{error}"))?;
    crate::win32::hotkey::bind_command_sender(command_tx.clone());
    crate::win32::hotkey::sync(settings);
    Ok(())
}

pub fn run_tray(settings: &Settings, tray_rx: TrayReceiver) -> Result<Phase> {
    crate::log_msg("[tray] session start");
    let phase = tray_host::run(settings, tray_rx)?;
    crate::log_msg(&format!("[tray] session finished -> {phase:?}"));
    Ok(phase)
}

pub fn run_gui(settings: crate::settings::Settings, tray_rx: TrayReceiver) -> Result<()> {
    gui_app::run(settings, tray_rx)
}
