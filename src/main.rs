#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod memory;
mod optimize;
mod privileges;
mod settings;
mod tray;
mod ui;
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

/// Check if running as admin, and if not, re-launch elevated.
#[cfg(target_os = "windows")]
fn ensure_elevated() {
    use privileges::is_elevated;
    if is_elevated().unwrap_or(false) {
        return;
    }

    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_NORMAL;

    let exe = std::env::current_exe().expect("failed to get exe path");
    let wide: Vec<u16> = exe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let verb: Vec<u16> = std::ffi::OsStr::new("runas")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // ShellExecuteW returns an HINSTANCE value; > 32 indicates success.
    let result = unsafe {
        ShellExecuteW(
            None,
            windows::core::PCWSTR(verb.as_ptr()),
            windows::core::PCWSTR(wide.as_ptr()),
            None,
            None,
            SW_NORMAL,
        )
    };
    if result.0 as isize <= 32 {
        eprintln!(
            "Failed to relaunch elevated (ShellExecuteW returned {}); exiting",
            result.0 as isize
        );
        std::process::exit(1);
    }
    std::process::exit(0);
}

#[cfg(not(target_os = "windows"))]
fn ensure_elevated() {}

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
        cx.on_action(|_: &Quit, cx: &mut App| {
            cx.quit();
        });

        let window_options = WindowOptions {
            titlebar: Some(TitleBar::title_bar_options()),
            window_bounds: Some(WindowBounds::centered(app::window_size(), cx)),
            is_resizable: false,
            window_min_size: Some(app::window_size()),
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                // Defer the show/activate decision until the entity is alive, so
                // a user with start_minimized=true never sees the window flash
                // before it gets minimized inside MemoryCleanerApp::new.
                let start_minimized = Settings::load().start_minimized;
                let app_entity = cx.new(|cx| {
                    let view = MemoryCleanerApp::new(window, cx);
                    if start_minimized {
                        let _ = win32::window::hide_to_tray(window);
                    } else {
                        window.activate_window();
                    }
                    view
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
