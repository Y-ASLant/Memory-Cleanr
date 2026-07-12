#![windows_subsystem = "windows"]

mod app;
mod log;
mod memory;
mod optimize;
mod privileges;
mod settings;
mod tray;
mod ui;
mod version;
mod win32;

use gpui::{actions, *};
use gpui_component::{Root, Theme, ThemeMode, TitleBar};

use crate::version::APP_NAME;
use app::MemoryCleanerApp;
use settings::Settings;
use tray::Tray;

actions!(wmc_gpui, [Quit]);

/// Passed to the elevated instance so it does not re-trigger UAC.
const ELEVATED_ARG: &str = "--elevated";

/// Write a diagnostic message to the Windows debug stream (viewable via
/// DebugView) and, when stderr is attached, also to stderr. Used instead of
/// bare `eprintln!` because the app is built with `windows_subsystem = "windows"`,
/// which makes stderr invisible in release builds.
pub fn log_msg(msg: &str) {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OutputDebugStringA(lp_output_string: *const u8);
    }
    let mut bytes = format!("{msg}\n").into_bytes();
    bytes.push(0);
    unsafe {
        OutputDebugStringA(bytes.as_ptr());
    }
    eprintln!("{msg}");
    crate::log::write(msg);
}

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
        let params: Vec<u16> = ELEVATED_ARG.encode_utf16().chain(Some(0)).collect();

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

    let tray_rx = match Tray::install() {
        Ok((tray, rx)) => {
            let _ = Box::leak(Box::new(tray));
            rx
        }
        Err(e) => {
            log_msg(&format!("Failed to install tray icon: {e}"));
            let (_tx, rx) = std::sync::mpsc::channel();
            rx
        }
    };

    let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        gpui_component::init(cx);

        cx.bind_keys([KeyBinding::new("alt-f4", Quit, None)]);

        let window_options = WindowOptions {
            titlebar: Some(TitleBar::title_bar_options()),
            window_bounds: Some(WindowBounds::centered(app::window_size(false), cx)),
            is_resizable: false,
            window_min_size: Some(app::window_min_size()),
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                window.set_window_title(APP_NAME);

                let settings = Settings::load();
                let app_entity = cx.new(|cx| {
                    let view = MemoryCleanerApp::new(window, cx, settings, tray_rx);
                    window.activate_window();
                    view
                });
                let weak = app_entity.downgrade();
                cx.on_action(move |_: &Quit, cx: &mut App| {
                    let _ = weak.update(cx, |app, _| app.settings.save());
                    cx.quit();
                });
                let _ = win32::window::remove_maximize_button(window);
                Theme::change(ThemeMode::Light, Some(window), cx);
                cx.new(|cx| Root::new(app_entity, window, cx))
            })
            .expect("Failed to open window");
        })
        .detach();
    });
}
