#![windows_subsystem = "windows"]

use memory_cleanr::{
    locale, log_msg,
    runtime,
    settings::Settings,
    win32,
};

const ELEVATED_ARG: &str = "--elevated";

fn ensure_elevated() {
    use std::os::windows::ffi::OsStrExt;

    if std::env::args().any(|arg| arg == ELEVATED_ARG) {
        return;
    }

    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_ok() {
            let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let mut ret_len = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                Some((&raw mut elevation).cast()),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut ret_len,
            );
            let _ = CloseHandle(token);
            if ok.is_ok() && elevation.TokenIsElevated != 0 {
                return;
            }
        }

        #[link(name = "shell32")]
        unsafe extern "system" {
            fn ShellExecuteW(
                hwnd: isize,
                lpszverb: *const u16,
                lpszfile: *const u16,
                lpszparams: *const u16,
                lpszdir: *const u16,
                nshowcmd: i32,
            ) -> isize;
        }

        let exe = std::env::current_exe().expect("cannot determine exe path");
        let path: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
        let verb: Vec<u16> = "runas".encode_utf16().chain(Some(0)).collect();
        let param_string = win32::startup::elevation_relaunch_args();
        let params: Vec<u16> = param_string.encode_utf16().chain(Some(0)).collect();

        let h = ShellExecuteW(
            0,
            verb.as_ptr(),
            path.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            1,
        );
        if h as usize > 32
            && win32::process::wait_for_elevated_relaunch(
                std::process::id(),
                concat!(env!("CARGO_BIN_NAME"), ".exe"),
                10_000,
            )
        {
            std::process::exit(0);
        }
    }
}

fn main() {
    ensure_elevated();

    let mut settings = Settings::load();
    if let Err(error) = win32::startup::sync(&settings) {
        log_msg(&format!("[startup] sync failed: {error:#}"));
    }
    locale::apply(&settings);

    if let Err(e) = win32::notification::init() {
        log_msg(&format!("[notification] init failed: {e:#}"));
    }

    if win32::startup::is_startup_launch() {
        if let Err(error) = win32::single_instance::ensure_tray_singleton() {
            log_msg(&error.to_string());
            std::process::exit(0);
        }
        run_tray_session(&mut settings);
        return;
    }

    if let Err(error) = win32::process::ensure_tray_host_running() {
        log_msg(&format!("[gui] failed to start tray host: {error:#}"));
    }

    if let Err(error) = win32::single_instance::ensure_gui_singleton() {
        log_msg(&error.to_string());
        if let Err(activate_error) = win32::process::activate_or_spawn_gui() {
            log_msg(&format!("[gui] activate existing failed: {activate_error:#}"));
        } else {
            log_msg("[gui] activated existing GUI window");
        }
        std::process::exit(0);
    }

    run_gui_session(&mut settings);
}

fn run_tray_session(settings: &mut Settings) {
    *settings = Settings::load();
    locale::apply(settings);

    let (command_tx, command_rx) = std::sync::mpsc::channel();
    let tray_rx = std::sync::Arc::new(std::sync::Mutex::new(command_rx));

    if let Err(error) = runtime::ensure_tray(&command_tx, settings) {
        log_msg(&format!("Failed to install tray icon: {error:#}"));
    }

    if let Err(error) = runtime::run_tray(settings, std::sync::Arc::clone(&tray_rx)) {
        log_msg(&format!("[tray] failed: {error:#}"));
    }
}

fn run_gui_session(settings: &mut Settings) {
    *settings = Settings::load();
    locale::apply(settings);

    let (_command_tx, command_rx) = std::sync::mpsc::channel();
    let tray_rx = std::sync::Arc::new(std::sync::Mutex::new(command_rx));

    if let Err(error) = runtime::run_gui(settings.clone(), std::sync::Arc::clone(&tray_rx)) {
        log_msg(&format!("[gui] failed: {error:#}"));
    }
}
