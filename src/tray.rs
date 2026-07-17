use rust_i18n::t;

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use image::{RgbaImage, imageops};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::memory::MemorySection;

/// Delay between 90° rotation steps while optimizing.
const SPIN_STEP_MS: u64 = 120;

static TRAY: AtomicPtr<Tray> = AtomicPtr::new(std::ptr::null_mut());
static ICON_FRAMES: OnceLock<[RgbaImage; 4]> = OnceLock::new();
static CMD_TX: OnceLock<Sender<TrayCommand>> = OnceLock::new();
static TRAY_INSTALLED: AtomicBool = AtomicBool::new(false);
static SPIN_GENERATION: AtomicU32 = AtomicU32::new(0);
static SPIN_ACTIVE: AtomicBool = AtomicBool::new(false);

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
    /// Global hotkey (`RegisterHotKey`) triggered memory cleanup.
    Optimize,
    MenuAction(String),
    /// Tray icon spin animation frame (0–3). Applied on the tray-host or GPUI thread.
    SetSpinFrame(u32),
}

impl Tray {
    pub fn install(tx: Sender<TrayCommand>) -> Result<(), Box<dyn std::error::Error>> {
        let _ = CMD_TX.get_or_init(|| tx.clone());

        if TRAY_INSTALLED.swap(true, Ordering::AcqRel) {
            return Ok(());
        }

        install_event_handlers(tx);

        let optimize = MenuItem::with_id("optimize", t!("tray.optimize"), true, None);
        let toggle_window = MenuItem::with_id("toggle_window", t!("tray.hide_window"), true, None);
        let quit = MenuItem::with_id("quit", t!("tray.quit"), true, None);
        let menu = Menu::new();
        menu.append(&optimize)?;
        menu.append(&toggle_window)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit)?;

        let source = load_icon_source();
        let (width, height) = source.dimensions();
        let _ = ICON_FRAMES.set(build_icon_frames(&source));

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip(t!("tray.tooltip", percent = "—"))
            .with_icon(icon_from_rgba(source.into_raw(), width, height))
            .build()?;

        let tray = Box::new(Self {
            icon: tray_icon,
            optimize,
            toggle_window,
            quit,
        });
        let leaked = Box::leak(tray);
        TRAY.store(leaked, Ordering::Release);

        Ok(())
    }
}

pub fn install(tx: Sender<TrayCommand>) -> Result<(), Box<dyn std::error::Error>> {
    Tray::install(tx)
}

pub fn is_installed() -> bool {
    TRAY_INSTALLED.load(Ordering::Acquire)
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

fn icon_from_rgba(rgba: Vec<u8>, width: u32, height: u32) -> Icon {
    Icon::from_rgba(rgba, width, height).unwrap_or_else(|_| {
        Icon::from_rgba(vec![0, 0, 0, 0], 1, 1).unwrap_or_else(|_| {
            panic!("tray_icon::Icon::from_rgba rejected a 1x1 transparent buffer")
        })
    })
}

/// Rotate a square tray icon by quarter turns (0–3). Avoids alpha tricks on Windows.
pub(crate) fn rotate_quarters(source: &RgbaImage, quarters: u32) -> RgbaImage {
    match quarters % 4 {
        0 => source.clone(),
        1 => imageops::rotate90(source),
        2 => imageops::rotate180(source),
        3 => imageops::rotate270(source),
        _ => unreachable!(),
    }
}

/// Build the four 90° rotation frames once at tray install.
fn build_icon_frames(source: &RgbaImage) -> [RgbaImage; 4] {
    [
        rotate_quarters(source, 0),
        rotate_quarters(source, 1),
        rotate_quarters(source, 2),
        rotate_quarters(source, 3),
    ]
}

fn icon_at_rotation(quarters: u32) -> Icon {
    let Some(frames) = ICON_FRAMES.get() else {
        return create_fallback_icon();
    };
    let frame = &frames[(quarters % 4) as usize];
    let (width, height) = frame.dimensions();
    icon_from_rgba(frame.clone().into_raw(), width, height)
}

fn set_tray_icon_rotation(quarters: u32) {
    let Some(tray) = tray() else {
        return;
    };
    let _ = tray.icon.set_icon(Some(icon_at_rotation(quarters)));
}

pub fn stop_spin() {
    SPIN_GENERATION.fetch_add(1, Ordering::Relaxed);
    SPIN_ACTIVE.store(false, Ordering::Relaxed);
    if let Some(tx) = CMD_TX.get() {
        let _ = tx.send(TrayCommand::SetSpinFrame(0));
    }
}

pub fn start_spin() {
    if SPIN_ACTIVE.swap(true, Ordering::Relaxed) {
        return;
    }

    let generation = SPIN_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;

    thread::Builder::new()
        .name("tray-icon-spin".into())
        .spawn(move || {
            let mut quarters = 0u32;
            loop {
                if SPIN_GENERATION.load(Ordering::Relaxed) != generation {
                    break;
                }

                if let Some(tx) = CMD_TX.get() {
                    let _ = tx.send(TrayCommand::SetSpinFrame(quarters));
                }
                quarters = quarters.wrapping_add(1);

                thread::sleep(Duration::from_millis(SPIN_STEP_MS));
            }
        })
        .ok();
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

fn load_icon_source() -> RgbaImage {
    load_app_icon_rgba().unwrap_or_else(|_| {
        let (rgba, width, height) = fallback_icon_rgba();
        RgbaImage::from_raw(width, height, rgba).expect("fallback icon rgba")
    })
}

fn load_app_icon_rgba() -> Result<RgbaImage, Box<dyn std::error::Error>> {
    let png_data = include_bytes!("../App.png");
    let img = image::load_from_memory(png_data)?;
    Ok(img
        .resize(32, 32, image::imageops::FilterType::Lanczos3)
        .to_rgba8())
}

fn fallback_icon_rgba() -> (Vec<u8>, u32, u32) {
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

    (rgba, width, height)
}

fn create_fallback_icon() -> Icon {
    let source = load_icon_source();
    let (width, height) = source.dimensions();
    icon_from_rgba(source.into_raw(), width, height)
}

pub fn apply_spin_frame(quarters: u32) {
    if quarters == 0 || SPIN_ACTIVE.load(Ordering::Relaxed) {
        set_tray_icon_rotation(quarters);
    }
}

pub fn dispatch_gui_command(
    app: &mut crate::app::MemoryCleanerApp,
    command: TrayCommand,
    cx: &mut gpui::Context<crate::app::MemoryCleanerApp>,
) {
    match command {
        TrayCommand::ActivateWindow => app.activate_window(cx),
        TrayCommand::RefreshTooltip => {
            app.refresh_memory();
            app.sync_tray();
        }
        TrayCommand::Optimize => app.run_optimize(cx),
        TrayCommand::MenuAction(action) => app.handle_tray_action(&action, cx),
        TrayCommand::SetSpinFrame(quarters) => apply_spin_frame(quarters),
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

    fn sample_icon() -> RgbaImage {
        RgbaImage::from_raw(
            2,
            2,
            vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 0, 0, 255],
        )
        .expect("sample icon")
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

    #[test]
    fn rotate_quarters_cycles_square_icon_dimensions() {
        let source = sample_icon();
        for quarters in 0..4 {
            let rotated = rotate_quarters(&source, quarters);
            assert_eq!(rotated.dimensions(), (2, 2));
        }
        assert_eq!(rotate_quarters(&source, 4), rotate_quarters(&source, 0));
    }
}
