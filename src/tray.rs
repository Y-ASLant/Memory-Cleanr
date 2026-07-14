use rust_i18n::t;

use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::app::MemoryCleanerApp;
use crate::memory::MemorySection;

static TRAY: AtomicPtr<Tray> = AtomicPtr::new(std::ptr::null_mut());

pub struct Tray {
    icon: TrayIcon,
    optimize: MenuItem,
    toggle_window: MenuItem,
    quit: MenuItem,
}

#[derive(Debug, Clone)]
pub enum TrayCommand {
    ActivateWindow,
    RefreshTooltip,
    MenuAction(String),
}

impl Tray {
    pub fn install() -> Result<Receiver<TrayCommand>, Box<dyn std::error::Error>> {
        let (tx, rx) = std::sync::mpsc::channel();
        install_event_handlers(tx);

        let optimize = MenuItem::with_id("optimize", t!("tray.optimize"), true, None);
        let toggle_window = MenuItem::with_id("toggle_window", t!("tray.hide_window"), true, None);
        let quit = MenuItem::with_id("quit", t!("tray.quit"), true, None);
        let menu = Menu::new();
        menu.append(&optimize)?;
        menu.append(&toggle_window)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit)?;

        let icon = load_app_icon().unwrap_or_else(|_| create_fallback_icon());
        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip(t!("tray.tooltip", percent = "—"))
            .with_icon(icon)
            .build()?;

        let tray = Box::new(Self {
            icon: tray_icon,
            optimize,
            toggle_window,
            quit,
        });
        let leaked = Box::leak(tray);
        TRAY.store(leaked, Ordering::Release);

        Ok(rx)
    }
}

fn tray() -> Option<&'static Tray> {
    let ptr = TRAY.load(Ordering::Acquire);
    if ptr.is_null() {
        None
    } else {
        // SAFETY: `TRAY` is set once during install and the value is leaked for process lifetime.
        Some(unsafe { &*ptr })
    }
}

pub fn format_memory_tooltip(
    physical: &MemorySection,
    virtual_mem: Option<&MemorySection>,
) -> String {
    let mut lines = vec![t!("tray.tooltip", percent = physical.percent_label()).to_string()];
    if let Some(virtual_mem) = virtual_mem {
        lines.push(
            t!(
                "tray.tooltip_virtual",
                percent = virtual_mem.percent_label()
            )
            .to_string(),
        );
    }
    lines.join("\n")
}

pub fn sync_display(
    physical: &MemorySection,
    virtual_mem: Option<&MemorySection>,
    window_visible: bool,
) {
    let Some(tray) = tray() else {
        return;
    };

    let _ = tray
        .icon
        .set_tooltip(Some(format_memory_tooltip(physical, virtual_mem)));
    tray.optimize.set_text(t!("tray.optimize"));
    tray.quit.set_text(t!("tray.quit"));
    tray.toggle_window.set_text(if window_visible {
        t!("tray.hide_window")
    } else {
        t!("tray.show_window")
    });
}

fn install_event_handlers(tx: Sender<TrayCommand>) {
    TrayIconEvent::set_event_handler(Some({
        let tx = tx.clone();
        move |event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } => {
                let _ = tx.send(TrayCommand::ActivateWindow);
            }
            TrayIconEvent::Enter { .. } => {
                let _ = tx.send(TrayCommand::RefreshTooltip);
            }
            _ => {}
        }
    }));

    MenuEvent::set_event_handler(Some({
        move |event: MenuEvent| {
            let _ = tx.send(TrayCommand::MenuAction(event.id().0.clone()));
        }
    }));
}

/// Load the application icon from the embedded PNG, resize to 32×32 for
/// the system tray, and convert to raw RGBA for `tray_icon::Icon`.
fn load_app_icon() -> Result<Icon, Box<dyn std::error::Error>> {
    let png_data = include_bytes!("../App.png");
    let img = image::load_from_memory(png_data)?;
    let img = img
        .resize(32, 32, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let (width, height) = img.dimensions();
    Icon::from_rgba(img.into_raw(), width, height).map_err(Into::into)
}

/// Fallback icon used when the embedded PNG cannot be decoded – a simple
/// green circle so the tray is at least visible even if something went
/// wrong with the asset pipeline.
fn create_fallback_icon() -> Icon {
    let width = 16u32;
    let height = 16u32;
    let mut rgba = vec![0u8; (width * height * 4) as usize];

    for y in 0..height {
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            let dx = x as f32 - 7.5;
            let dy = y as f32 - 7.5;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < 7.0 {
                rgba[idx] = 39;
                rgba[idx + 1] = 174;
                rgba[idx + 2] = 96;
                rgba[idx + 3] = 255;
            }
        }
    }

    Icon::from_rgba(rgba, width, height).unwrap_or_else(|_| {
        Icon::from_rgba(vec![0, 0, 0, 0], 1, 1).unwrap_or_else(|_| {
            panic!("tray_icon::Icon::from_rgba rejected a 1x1 transparent buffer")
        })
    })
}

pub fn dispatch_command(
    app: &mut MemoryCleanerApp,
    command: TrayCommand,
    cx: &mut gpui::Context<MemoryCleanerApp>,
) {
    match command {
        TrayCommand::ActivateWindow => app.activate_window(cx),
        TrayCommand::RefreshTooltip => {
            app.refresh_memory();
            app.sync_tray();
        }
        TrayCommand::MenuAction(action) => app.handle_tray_action(&action, cx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::with_locale;
    use crate::memory::MemorySection;

    fn section(title: &str, percent: f32) -> MemorySection {
        MemorySection {
            title: title.into(),
            total: 16 * 1024 * 1024 * 1024,
            used: 8 * 1024 * 1024 * 1024,
            avail: 8 * 1024 * 1024 * 1024,
            used_percent: percent,
        }
    }

    #[test]
    fn format_memory_tooltip_includes_virtual_memory_when_present_zh() {
        with_locale("zh-CN", || {
            let physical = section("物理内存", 46.0);
            let virtual_mem = section("虚拟内存", 86.0);
            let tooltip = format_memory_tooltip(&physical, Some(&virtual_mem));
            assert_eq!(tooltip, "物理内存: 46%\n虚拟内存: 86%");
        });
    }

    #[test]
    fn format_memory_tooltip_includes_virtual_memory_when_present_en() {
        with_locale("en", || {
            let physical = section("Physical Memory", 46.0);
            let virtual_mem = section("Virtual Memory", 86.0);
            let tooltip = format_memory_tooltip(&physical, Some(&virtual_mem));
            assert_eq!(tooltip, "Physical: 46%\nVirtual: 86%");
        });
    }

    #[test]
    fn format_memory_tooltip_omits_virtual_memory_when_absent_zh() {
        with_locale("zh-CN", || {
            let physical = section("物理内存", 46.0);
            assert_eq!(format_memory_tooltip(&physical, None), "物理内存: 46%");
        });
    }

    #[test]
    fn format_memory_tooltip_omits_virtual_memory_when_absent_en() {
        with_locale("en", || {
            let physical = section("Physical Memory", 46.0);
            assert_eq!(format_memory_tooltip(&physical, None), "Physical: 46%");
        });
    }
}
