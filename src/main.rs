#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
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

use app::MemoryCleanerApp;
use settings::Settings;
use tray::Tray;

actions!(wmc_gpui, [Quit]);

/// Write a diagnostic message to the Windows debug stream (viewable via
/// DebugView) and, when stderr is attached, also to stderr. Used instead of
/// bare `eprintln!` because the app is built with `windows_subsystem = "windows"`,
/// which makes stderr invisible in release builds.
#[cfg(target_os = "windows")]
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
}

#[cfg(not(target_os = "windows"))]
pub fn log_msg(msg: &str) {
    eprintln!("{msg}");
}


/// If the current process is not running as administrator, re-launch
/// itself with `ShellExecuteW("runas")` and exit. This avoids embedding
/// a `requireAdministrator` manifest (which conflicts with GPUI's own
/// manifest via Cargo feature unification).
#[cfg(target_os = "windows")]
fn ensure_elevated() {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        // Check current elevation status.
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
            if ok.is_ok() && elevation.TokenIsElevated != 0 {
                return; // Already admin.
            }
        }

        // Not admin — re-launch elevated.
        // We call ShellExecuteW directly via FFI to avoid adding the
        // Win32_UI_Shell cargo feature (and its transitive deps).
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
        let verb: Vec<u16> = "runas\0".encode_utf16().collect();

        let h = ShellExecuteW(0, verb.as_ptr(), path.as_ptr(), std::ptr::null(), std::ptr::null(), 1);
        if h > 32 {
            std::process::exit(0);
        }
        // If elevation failed (user cancelled UAC), fall through to run
        // without admin.  Some cleanup areas will fail but the app works.
    }
}
fn main() {
    ensure_elevated();
    if let Err(e) = win32::single_instance::ensure_single_instance() {
        log_msg(&e.to_string());
        std::process::exit(0);
    }

    let _tray = match Tray::install() {
        Ok(tray) => Some(tray),
        Err(e) => {
            log_msg(&format!("Failed to install tray icon: {e}"));
            None
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
                let settings = Settings::load();
                let start_minimized = settings.start_minimized;
                let app_entity = cx.new(|cx| {
                    let view = MemoryCleanerApp::new(window, cx, settings);
                    if start_minimized {
                        let _ = win32::window::hide_to_tray(window);
                    } else {
                        window.activate_window();
                    }
                    view
                });
                let weak = app_entity.downgrade();
                cx.on_action(move |_: &Quit, cx: &mut App| {
                    let _ = weak.update(cx, |app, _| app.settings.save());
                    cx.quit();
                });
                window.set_window_title("Memory Cleaner");
                let _ = win32::window::remove_maximize_button(window);
                Theme::change(ThemeMode::Light, Some(window), cx);
                cx.new(|cx| Root::new(app_entity, window, cx))
            })
            .expect("Failed to open window");
        })
        .detach();
    });
}
