#![windows_subsystem = "windows"]

use gpui::{actions, *};

use memory_cleanr::{
    app::{self, AppEntityHolder},
    locale, log_msg,
    settings::Settings,
    tray::Tray,
    win32,
};

actions!(wmc_gpui, [Quit]);

/// Passed to the elevated instance so it does not re-trigger UAC.
const ELEVATED_ARG: &str = "--elevated";

/// If the current process is not running as administrator, re-launch
/// itself with `ShellExecuteW("runas")` and exit. This avoids embedding
/// a `requireAdministrator` manifest (which conflicts with GPUI's own
/// manifest via Cargo feature unification).
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
        // ShellExecute may return > 32 even when the user later cancels UAC.
        // Wait for the elevated child before exiting; otherwise continue unelevated.
        if h as usize > 32
            && win32::process::wait_for_elevated_relaunch(
                std::process::id(),
                concat!(env!("CARGO_BIN_NAME"), ".exe"),
                10_000,
            )
        {
            std::process::exit(0);
        }
        // User cancelled UAC — continue without admin; some cleanup areas will fail.
    }
}

fn main() {
    ensure_elevated();
    if let Err(e) = win32::single_instance::ensure_single_instance() {
        log_msg(&e.to_string());
        std::process::exit(0);
    }

    let settings = Settings::load();
    if let Err(error) = win32::startup::sync(&settings) {
        log_msg(&format!("[startup] sync failed: {error:#}"));
    }
    let launch_hidden = win32::startup::is_startup_launch();
    locale::apply(&settings);

    if let Err(e) = win32::notification::init() {
        log_msg(&format!("[notification] init failed: {e:#}"));
    }

    let (command_tx, command_rx) = std::sync::mpsc::channel();
    win32::hotkey::bind_command_sender(command_tx.clone());

    Tray::install(command_tx.clone()).unwrap_or_else(|e| {
        log_msg(&format!("Failed to install tray icon: {e}"));
    });
    win32::hotkey::sync(&settings);

    let app = gpui_platform::application()
        .with_assets(gpui_component_assets::Assets)
        .with_quit_mode(QuitMode::Explicit);

    app.run(move |cx| {
        gpui_component::init(cx);

        cx.bind_keys([KeyBinding::new("alt-f4", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| {
            let entity = cx
                .try_global::<AppEntityHolder>()
                .map(|holder| holder.0.clone());
            if let Some(entity) = entity {
                entity.update(cx, |app, _| app.settings.save());
            }
            cx.quit();
        });

        cx.spawn(async move |cx| {
            app::open_main_window(cx, settings, command_rx, launch_hidden)
                .expect("Failed to open window");
        })
        .detach();
    });
}
