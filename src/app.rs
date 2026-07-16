use rust_i18n::t;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use anyhow::Result;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::{Root, TitleBar, WindowExt};
use smol::Timer;

use crate::locale;
use crate::memory::{MemorySection, MemoryStatus};
use crate::messages::{build_cleanup_result_message, format_freed_message};
use crate::optimize::{self, MemoryAreas};
use crate::settings::Settings;
use crate::tray::{TrayCommand, dispatch_command};
use crate::ui::layout::SECTION_GAP;
use crate::win32;

const SETTINGS_SAVE_DEBOUNCE: Duration = Duration::from_millis(300);
const OPTIMIZE_RESULT_DISPLAY: Duration = Duration::from_secs(5);
const MEMORY_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

async fn show_toast(title: String, body: String) {
    if let Err(e) = smol::unblock(move || win32::notification::show(&title, &body)).await {
        crate::log_msg(&format!("[notification] failed: {e:#}"));
    }
}

const WINDOW_WIDTH: f32 = 520.;
const WINDOW_MIN_WIDTH: f32 = 520.;
pub const CONTENT_PADDING: f32 = 6.;
const SINGLE_CARD_MAX_WIDTH: f32 = 360.;

pub fn window_size(expanded: bool) -> Size<Pixels> {
    let height = if expanded {
        crate::ui::layout::expanded_window_height(CONTENT_PADDING)
    } else {
        crate::ui::layout::collapsed_window_height(CONTENT_PADDING)
    };
    size(px(WINDOW_WIDTH), px(height))
}

pub fn window_min_size() -> Size<Pixels> {
    size(
        px(WINDOW_MIN_WIDTH),
        px(crate::ui::layout::collapsed_window_height(CONTENT_PADDING)),
    )
}

pub fn window_options(expanded: bool, cx: &App) -> WindowOptions {
    WindowOptions {
        titlebar: Some(TitleBar::title_bar_options()),
        window_bounds: Some(WindowBounds::centered(window_size(expanded), cx)),
        is_resizable: false,
        window_min_size: Some(window_min_size()),
        ..Default::default()
    }
}

pub struct AppEntityHolder(pub Entity<MemoryCleanerApp>);
impl Global for AppEntityHolder {}

pub fn open_main_window(
    cx: &mut AsyncApp,
    settings: Settings,
    tray_rx: std::sync::mpsc::Receiver<TrayCommand>,
) -> Result<()> {
    let options = cx.update(|app| window_options(false, app));
    cx.open_window(options, |window, cx| {
        window.set_window_title(crate::version::APP_NAME);

        let app_entity = cx.new(|cx| MemoryCleanerApp::new(window, cx, settings, tray_rx));
        let _ = win32::window::remove_maximize_button(window);
        crate::ui::theme::init_light_theme(window, cx);
        window.activate_window();
        cx.new(|cx| Root::new(app_entity, window, cx))
    })?;
    Ok(())
}

fn query_sections(show_virtual: bool) -> Result<(MemorySection, Option<MemorySection>)> {
    let status = MemoryStatus::query()?;

    let physical = MemorySection {
        title: t!("memory.physical").to_string(),
        total: status.total_phys,
        used: status.used_phys(),
        avail: status.avail_phys,
        used_percent: status.memory_load as f32,
    };

    let virtual_mem = if show_virtual {
        let virt_used = status
            .total_page_file
            .saturating_sub(status.avail_page_file);
        let virt_percent = if status.total_page_file > 0 {
            (virt_used as f64 / status.total_page_file as f64 * 100.0).round() as u32
        } else {
            0
        };
        Some(MemorySection {
            title: t!("memory.virtual").to_string(),
            total: status.total_page_file,
            used: virt_used,
            avail: status.avail_page_file,
            used_percent: virt_percent as f32,
        })
    } else {
        None
    };

    Ok((physical, virtual_mem))
}

pub struct MemoryCleanerApp {
    pub window: Option<AnyWindowHandle>,
    pub settings: Settings,
    pub physical: MemorySection,
    pub virtual_mem: Option<MemorySection>,
    settings_save_gen: u32,
    memory_refresh_generation: Arc<AtomicU32>,
    window_opening: bool,
    pub is_optimizing: bool,
    pub is_refreshing_icon_cache: bool,
    pub optimize_step: String,
    pub optimize_percent: f32,
    pub optimize_status: String,
    pub optimize_has_errors: bool,
    pub icon_cache_status: String,
    pub settings_expanded: bool,
    window_shown: bool,
    pub cleanup_hotkey_recording: bool,
    pub(crate) hotkey_capture_focus: FocusHandle,
    pub process_exclusion_pick: Option<String>,
}

impl MemoryCleanerApp {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        settings: Settings,
        tray_rx: std::sync::mpsc::Receiver<TrayCommand>,
    ) -> Self {
        crate::log::set_debug_enabled(settings.debug_logging);
        if settings.debug_logging {
            crate::log::write(&t!(
                "log.debug_enabled",
                path = crate::log::log_file_path().display().to_string()
            ));
        }

        let show_virtual = settings.show_virtual_memory;
        let (physical, virtual_mem) = query_sections(show_virtual).unwrap_or_else(|e| {
            crate::log_msg(&format!("[memory] initial query failed: {e}"));
            (
                MemorySection::unavailable(&t!("memory.physical")),
                if show_virtual {
                    Some(MemorySection::unavailable(&t!("memory.virtual")))
                } else {
                    None
                },
            )
        });

        let mut app = Self {
            window: None,
            settings,
            physical,
            virtual_mem,
            settings_save_gen: 0,
            memory_refresh_generation: Arc::new(AtomicU32::new(0)),
            window_opening: false,
            is_optimizing: false,
            is_refreshing_icon_cache: false,
            optimize_step: String::new(),
            optimize_percent: 0.0,
            optimize_status: String::new(),
            optimize_has_errors: false,
            icon_cache_status: String::new(),
            settings_expanded: false,
            window_shown: true,
            cleanup_hotkey_recording: false,
            hotkey_capture_focus: cx.focus_handle(),
            process_exclusion_pick: None,
        };

        cx.set_global(AppEntityHolder(cx.entity()));
        app.attach_window(window, cx);
        app.start_background_tasks(cx, tray_rx);
        app.sync_tray();

        app
    }

    fn attach_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.window = Some(window.window_handle());
        self.window_shown = true;

        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, gpui_app| {
            crate::log_msg("[close] on_window_should_close");
            let should_close = weak
                .update(gpui_app, |this, _| {
                    this.request_close("should_close", window)
                })
                .unwrap_or(true);

            if should_close {
                gpui_app.quit();
            }
            should_close
        });

        if self.settings.always_on_top {
            let _ = win32::window::set_always_on_top(window, true);
        }

        self.start_memory_refresh(cx);
    }

    fn pause_memory_refresh(&self) {
        self.memory_refresh_generation
            .fetch_add(1, Ordering::Relaxed);
    }

    fn start_memory_refresh(&self, cx: &mut Context<Self>) {
        if self.window.is_none() {
            return;
        }

        let generation = self.memory_refresh_generation.load(Ordering::Relaxed);
        let gen_arc = Arc::clone(&self.memory_refresh_generation);
        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(MEMORY_REFRESH_INTERVAL).await;
                if gen_arc.load(Ordering::Relaxed) != generation {
                    break;
                }
                let Ok(()) = this.update(cx, |app, cx| {
                    if app.window.is_none() || !app.window_shown {
                        return;
                    }
                    if app.refresh_memory() {
                        cx.notify();
                        app.sync_tray();
                    }
                }) else {
                    break;
                };
            }
        })
        .detach();
    }

    fn open_window(&mut self, cx: &mut Context<Self>) {
        if self.window.is_some() || self.window_opening {
            return;
        }

        self.window_opening = true;
        let expanded = self.settings_expanded;
        cx.spawn(async move |this, cx| {
            let entity = match this.upgrade() {
                Some(entity) => entity,
                None => return,
            };

            let options = cx.update(|app| window_options(expanded, app));
            let opened = cx.open_window(options, |window, cx| {
                entity.update(cx, |app, cx| {
                    app.attach_window(window, cx);
                    app.window_opening = false;
                });
                window.set_window_title(crate::version::APP_NAME);
                let _ = win32::window::remove_maximize_button(window);
                crate::ui::theme::init_light_theme(window, cx);
                window.activate_window();
                cx.new(|cx| Root::new(entity.clone(), window, cx))
            });

            if opened.is_err() {
                entity.update(cx, |app, _| app.window_opening = false);
            } else {
                entity.update(cx, |app, _| app.sync_tray());
            }
        })
        .detach();
    }

    fn window_visible(&self) -> bool {
        self.window.is_some() && self.window_shown
    }

    pub(crate) fn sync_tray(&self) {
        let virtual_mem = if self.settings.show_virtual_memory {
            self.virtual_mem.as_ref()
        } else {
            None
        };
        crate::tray::sync_display(&self.physical, virtual_mem, self.window_visible());
    }

    fn set_unavailable_sections(&mut self, show_virtual: bool) {
        self.physical = MemorySection::unavailable(&t!("memory.physical"));
        self.virtual_mem = if show_virtual {
            Some(MemorySection::unavailable(&t!("memory.virtual")))
        } else {
            None
        };
    }

    pub(crate) fn queue_settings_save(&mut self, cx: &mut Context<Self>) {
        self.settings_save_gen = self.settings_save_gen.wrapping_add(1);
        let generation = self.settings_save_gen;

        cx.spawn(async move |this, cx| {
            Timer::after(SETTINGS_SAVE_DEBOUNCE).await;
            let _ = this.update(cx, |app, _| {
                if app.settings_save_gen == generation {
                    app.settings.save();
                }
            });
        })
        .detach();
    }

    pub fn refresh_memory(&mut self) -> bool {
        let show_virtual = self.settings.show_virtual_memory;
        let Ok((physical, virtual_mem)) = query_sections(show_virtual) else {
            if self.physical.is_unavailable()
                && self.virtual_mem.as_ref().is_none_or(|v| v.is_unavailable())
            {
                return false;
            }
            self.set_unavailable_sections(show_virtual);
            return true;
        };

        let changed = self.physical != physical || self.virtual_mem != virtual_mem;
        if changed {
            self.physical = physical;
            self.virtual_mem = virtual_mem;
        }
        changed
    }

    pub fn activate_window(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.window {
            match handle.update(cx, |_, window, _| -> Result<()> {
                crate::log_msg("[window] activate_window");
                win32::window::show_from_tray(window)?;
                window.activate_window();
                Ok(())
            }) {
                Ok(Ok(())) => {
                    self.window_shown = true;
                    self.start_memory_refresh(cx);
                    self.sync_tray();
                    return;
                }
                Ok(Err(e)) => crate::log_msg(&format!("[window] show_from_tray failed: {e:#}")),
                Err(_) => crate::log_msg("[window] activate_window handle update failed"),
            }
            self.window = None;
        }
        self.open_window(cx);
    }

    /// Remove the GPUI window and drop our handle. `activate_window` recreates it via
    /// `open_window()`.
    fn destroy_window_to_tray(&mut self, window: &mut Window, source: &str) {
        window.remove_window();
        self.window = None;
        self.window_shown = false;
        self.pause_memory_refresh();
        crate::log_msg(&format!("[close] hide_to_tray destroy ok source={source}"));
    }

    /// Handle a close request. Returns `true` when the app should quit entirely.
    pub fn request_close(&mut self, source: &str, window: &mut Window) -> bool {
        crate::log_msg(&format!(
            "[close] request_close source={source} close_to_tray={}",
            self.settings.close_to_notification_area
        ));
        self.settings.save();
        if self.settings.close_to_notification_area {
            self.destroy_window_to_tray(window, source);
            self.sync_tray();
            false
        } else {
            true
        }
    }

    pub fn hide_to_tray(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.window {
            match handle.update(cx, |_, window, _| window.remove_window()) {
                Ok(()) => {
                    crate::log_msg("[close] hide_to_tray destroy ok source=tray_menu");
                }
                Err(_) => crate::log_msg("[close] hide_to_tray handle update failed"),
            }
        } else {
            crate::log_msg("[close] hide_to_tray no window handle");
        }
        self.window = None;
        self.window_shown = false;
        self.pause_memory_refresh();
        self.sync_tray();
    }

    pub fn apply_locale(&mut self, cx: &mut Context<Self>) {
        locale::apply(&self.settings);
        let show_virtual = self.settings.show_virtual_memory;
        if let Ok((physical, virtual_mem)) = query_sections(show_virtual) {
            self.physical = physical;
            self.virtual_mem = virtual_mem;
        } else {
            self.set_unavailable_sections(show_virtual);
        }
        if !self.is_optimizing {
            self.optimize_status.clear();
            self.optimize_has_errors = false;
            self.optimize_step.clear();
        }
        if !self.is_refreshing_icon_cache {
            self.icon_cache_status.clear();
        }
        self.sync_tray();
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn set_memory_area(&mut self, area: MemoryAreas, enabled: bool, cx: &mut Context<Self>) {
        if self.is_optimizing {
            return;
        }

        let mut areas = self.settings.memory_areas();
        if enabled {
            if area == MemoryAreas::STANDBY_LIST {
                areas.remove(MemoryAreas::STANDBY_LIST_LOW_PRIORITY);
            } else if area == MemoryAreas::STANDBY_LIST_LOW_PRIORITY {
                areas.remove(MemoryAreas::STANDBY_LIST);
            }
            areas.insert(area);
        } else {
            areas.remove(area);
        }
        self.settings.memory_areas = areas.bits();
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn set_process_exclusion_pick(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        if self.is_optimizing {
            return;
        }
        self.process_exclusion_pick = name;
        cx.notify();
    }

    pub fn add_excluded_process(&mut self, cx: &mut Context<Self>) {
        if self.is_optimizing {
            return;
        }
        let Some(name) = self.process_exclusion_pick.clone() else {
            return;
        };
        let normalized = win32::process::normalize_process_name(&name);
        if normalized.is_empty() {
            return;
        }
        if self
            .settings
            .excluded_processes
            .iter()
            .any(|existing| existing == &normalized)
        {
            return;
        }
        self.settings.excluded_processes.push(normalized);
        self.settings.excluded_processes.sort();
        self.process_exclusion_pick = None;
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn remove_excluded_process(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.is_optimizing {
            return;
        }
        let normalized = win32::process::normalize_process_name(name);
        self.settings
            .excluded_processes
            .retain(|existing| existing != &normalized);
        if self.process_exclusion_pick.as_deref() == Some(normalized.as_str()) {
            self.process_exclusion_pick = None;
        }
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn available_processes_for_exclusion(&self) -> Vec<String> {
        win32::process::list_running_process_names(
            crate::version::PROCESS_BASE_NAME,
            &self.settings.excluded_processes,
        )
    }

    pub fn open_window_behavior_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::layout::{
            DIALOG_PADDING_HORIZONTAL, DIALOG_PADDING_TOP, WINDOW_BEHAVIOR_DIALOG_WIDTH,
        };
        use crate::ui::settings_page::render_window_behavior_dialog;

        self.cancel_cleanup_hotkey_recording(cx);

        let weak = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let weak = weak.clone();
            dialog
                .title(t!("dialog.window_behavior"))
                .w(px(WINDOW_BEHAVIOR_DIALOG_WIDTH))
                .pt(px(DIALOG_PADDING_TOP))
                .pb(px(CONTENT_PADDING))
                .pl(px(DIALOG_PADDING_HORIZONTAL))
                .pr(px(DIALOG_PADDING_HORIZONTAL))
                .overlay_closable(false)
                .content({
                    let weak = weak.clone();
                    move |content, _window, cx| {
                        content.child(render_window_behavior_dialog(weak.clone(), cx))
                    }
                })
        });
    }

    pub fn toggle_settings_expanded(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_expanded = !self.settings_expanded;
        window.resize(window_size(self.settings_expanded));
        cx.notify();
    }

    pub fn set_always_on_top(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.always_on_top = enabled;
        let _ = win32::window::set_always_on_top(window, enabled);
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn set_close_to_tray(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.close_to_notification_area = enabled;
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn set_show_optimization_notifications(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.show_optimization_notifications = enabled;
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn set_cleanup_hotkey_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.cleanup_hotkey_enabled = enabled;
        if !enabled {
            self.cleanup_hotkey_recording = false;
        }
        win32::hotkey::sync(&self.settings);
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn start_cleanup_hotkey_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings.cleanup_hotkey_enabled {
            return;
        }
        self.cleanup_hotkey_recording = true;
        window.focus(&self.hotkey_capture_focus, cx);
        cx.notify();
    }

    pub fn handle_cleanup_hotkey_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if !self.cleanup_hotkey_recording {
            return;
        }

        if event.keystroke.key.eq_ignore_ascii_case("escape") {
            self.cleanup_hotkey_recording = false;
            cx.notify();
            return;
        }

        let keystroke = &event.keystroke;
        let Some(chord) = win32::hotkey::HotkeyBinding::format_chord(
            keystroke.modifiers.control,
            keystroke.modifiers.alt,
            keystroke.modifiers.shift,
            keystroke.modifiers.platform,
            &keystroke.key,
        ) else {
            return;
        };

        self.settings.cleanup_hotkey = chord;
        self.cleanup_hotkey_recording = false;
        win32::hotkey::sync(&self.settings);
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn cancel_cleanup_hotkey_recording(&mut self, cx: &mut Context<Self>) {
        if self.cleanup_hotkey_recording {
            self.cleanup_hotkey_recording = false;
            cx.notify();
        }
    }

    pub fn set_debug_logging(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.debug_logging = enabled;
        crate::log::set_debug_enabled(enabled);
        if enabled {
            crate::log::write(&t!(
                "log.debug_enabled",
                path = crate::log::log_file_path().display().to_string()
            ));
        }
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn handle_tray_action(&mut self, action: &str, cx: &mut Context<Self>) {
        match action {
            "optimize" => self.run_optimize(cx),
            "toggle_window" => {
                if self.window_visible() {
                    self.hide_to_tray(cx);
                } else {
                    self.activate_window(cx);
                }
            }
            "quit" => {
                self.settings.save();
                cx.quit();
            }
            _ => {}
        }
    }

    pub fn start_background_tasks(
        &self,
        cx: &mut Context<Self>,
        mut tray_rx: std::sync::mpsc::Receiver<TrayCommand>,
    ) {
        // 托盘命令监听
        cx.spawn(async move |this, cx| {
            loop {
                let (command, rx) = smol::unblock(move || {
                    let result = tray_rx.recv();
                    (result, tray_rx)
                })
                .await;
                tray_rx = rx;

                let Ok(command) = command else {
                    break;
                };

                if this
                    .update(cx, |this, cx| {
                        dispatch_command(this, command, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.is_refreshing_icon_cache || self.is_optimizing
    }

    async fn run_optimize_step(
        this: WeakEntity<Self>,
        cx: &mut AsyncApp,
        name: String,
        run: optimize::OptimizeStepFn,
        step_index: usize,
        total_steps: usize,
    ) -> bool {
        let step_base = step_index as f32 / total_steps as f32;
        let step_span = 1.0 / total_steps as f32;

        let _ = this.update(cx, |app, cx| {
            app.optimize_step = t!("optimize.step", name = name.clone()).to_string();
            app.optimize_percent = step_base * 100.0;
            cx.notify();
        });

        Timer::after(Duration::from_millis(60)).await;

        let result = smol::unblock(run).await;

        if let Err(e) = &result {
            crate::log::write(&format!("[optimize] {name} failed: {e:#}"));
        }

        let _ = this.update(cx, |app, cx| {
            app.optimize_percent = (step_base + step_span) * 100.0;
            cx.notify();
        });

        Timer::after(Duration::from_millis(100)).await;
        result.is_ok()
    }

    async fn run_modified_file_cache_step(
        this: WeakEntity<Self>,
        cx: &mut AsyncApp,
        step_index: usize,
        total_steps: usize,
    ) -> bool {
        let step_base = step_index as f32 / total_steps as f32;
        let step_span = 1.0 / total_steps as f32;
        let name = MemoryAreas::MODIFIED_FILE_CACHE.label();

        let drives = match smol::unblock(optimize::fixed_drives).await {
            drives if drives.is_empty() => {
                let _ = this.update(cx, |app, cx| {
                    app.optimize_step = t!("optimize.step", name = name.clone()).to_string();
                    app.optimize_percent = (step_base + step_span) * 100.0;
                    cx.notify();
                });
                return true;
            }
            drives => drives,
        };

        let drive_total = drives.len();
        let mut failed = Vec::new();

        for (drive_index, drive) in drives.into_iter().enumerate() {
            let sub_base = drive_index as f32 / drive_total as f32;

            let _ = this.update(cx, |app, cx| {
                app.optimize_step = t!(
                    "optimize.step_with_progress",
                    name = name.clone(),
                    drive = drive.to_string(),
                    current = (drive_index + 1).to_string(),
                    total = drive_total.to_string()
                )
                .to_string();
                app.optimize_percent = (step_base + sub_base * step_span) * 100.0;
                cx.notify();
            });

            let drive_result = smol::unblock(move || optimize::optimize_drive_cache(drive)).await;
            if let Err(e) = drive_result {
                crate::log::write(&format!(
                    "[optimize] modified file cache drive {drive}: failed: {e:#}"
                ));
                failed.push(drive);
            }

            let _ = this.update(cx, |app, cx| {
                app.optimize_percent =
                    (step_base + (drive_index + 1) as f32 / drive_total as f32 * step_span) * 100.0;
                cx.notify();
            });
        }

        failed.is_empty()
    }

    pub fn run_optimize(&mut self, cx: &mut Context<Self>) {
        if self.is_optimizing {
            return;
        }

        let areas = self.settings.memory_areas();
        let excluded = self.settings.excluded_processes.clone();
        let steps = match optimize::step_plan(areas, &excluded) {
            Ok(s) if !s.is_empty() => s,
            _ => {
                self.optimize_status = t!("tooltip.select_areas").to_string();
                cx.notify();
                return;
            }
        };

        let avail_before = self.physical.avail;
        let total = steps.len();
        let notify = self.settings.show_optimization_notifications;
        self.is_optimizing = true;
        self.optimize_step = t!("button.cleanup_preparing").to_string();
        self.optimize_percent = 0.0;
        self.optimize_status.clear();
        self.optimize_has_errors = false;
        crate::tray::start_spin();
        cx.notify();

        cx.spawn(async move |this, cx| {
            if notify {
                show_toast(
                    t!("notification.optimize_start_title").to_string(),
                    t!("notification.optimize_start_body").to_string(),
                )
                .await;
            }

            let mut completed: Vec<String> = Vec::new();
            let mut errors: Vec<String> = Vec::new();

            for (index, (name, run)) in steps.into_iter().enumerate() {
                let ok = if name == MemoryAreas::MODIFIED_FILE_CACHE.label() {
                    Self::run_modified_file_cache_step(this.clone(), cx, index, total).await
                } else {
                    Self::run_optimize_step(this.clone(), cx, name.clone(), run, index, total).await
                };

                if ok {
                    completed.push(name.clone());
                    crate::log::write(&format!("[optimize] {name} succeeded"));
                } else {
                    errors.push(name);
                }
            }

            let notification = this
                .update(cx, |app, cx| {
                    let _ = app.refresh_memory();
                    let avail_after = app.physical.avail;
                    let freed_detail = format_freed_message(avail_before, avail_after);
                    app.optimize_step.clear();
                    app.is_optimizing = false;
                    app.optimize_percent = 0.0;
                    crate::tray::stop_spin();
                    let completed_refs: Vec<&str> = completed.iter().map(|s| s.as_str()).collect();
                    let errors_refs: Vec<&str> = errors.iter().map(|s| s.as_str()).collect();
                    app.optimize_has_errors = !errors.is_empty();
                    app.optimize_status =
                        build_cleanup_result_message(&completed_refs, &errors_refs, &freed_detail);
                    crate::log::write(&format!("[optimize] result: {}", app.optimize_status));
                    cx.notify();
                    if app.settings.show_optimization_notifications {
                        Some((
                            t!("notification.optimize_title").to_string(),
                            app.optimize_status.clone(),
                        ))
                    } else {
                        None
                    }
                })
                .ok()
                .flatten();

            if let Some((title, body)) = notification {
                show_toast(title, body).await;
            }

            Timer::after(OPTIMIZE_RESULT_DISPLAY).await;

            let _ = this.update(cx, |app, cx| {
                app.optimize_status.clear();
                app.optimize_has_errors = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn open_icon_cache_confirm_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy() {
            return;
        }

        use gpui_component::WindowExt;
        use gpui_component::dialog::DialogButtonProps;

        let weak = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            alert
                .title(t!("icon_cache.confirm_title"))
                .description(t!("icon_cache.confirm_desc"))
                .overlay_closable(false)
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("dialog.confirm"))
                        .cancel_text(t!("dialog.cancel"))
                        .show_cancel(true),
                )
                .on_ok({
                    let weak = weak.clone();
                    move |_, _window, cx| {
                        let _ = weak.update(cx, |app, cx| app.run_icon_cache_refresh(cx));
                        true
                    }
                })
        });
    }

    pub fn run_icon_cache_refresh(&mut self, cx: &mut Context<Self>) {
        if self.is_busy() {
            return;
        }

        self.is_refreshing_icon_cache = true;
        self.icon_cache_status = t!("icon_cache.refreshing").to_string();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let outcome = smol::unblock(crate::icon_cache::refresh).await;
            let message = outcome.user_message();
            crate::log_msg(&format!("[icon_cache] {message}"));
            for failure in &outcome.failures {
                crate::log::write(&format!("[icon_cache] {failure}"));
            }

            let _ = this.update(cx, |app, cx| {
                app.is_refreshing_icon_cache = false;
                app.icon_cache_status = message;
                cx.notify();
            });

            Timer::after(OPTIMIZE_RESULT_DISPLAY).await;

            let _ = this.update(cx, |app, cx| {
                app.icon_cache_status.clear();
                cx.notify();
            });
        })
        .detach();
    }
}

/// 创建内存卡片的 GroupBox 容器
fn memory_group_box(
    id: &'static str,
    child: impl IntoElement,
) -> gpui_component::group_box::GroupBox {
    use gpui_component::group_box::{GroupBox, GroupBoxVariants};

    GroupBox::new()
        .id(id)
        .outline()
        .w_full()
        .p_0()
        .content_style(StyleRefinement::default().p_2())
        .child(child)
}

impl Render for MemoryCleanerApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::ui::memory_card::render_memory_card;
        use crate::ui::settings_page::{render_cleanup_footer, render_settings_content};
        use crate::ui::title_bar::render_title_bar;
        use gpui::prelude::FluentBuilder;
        use gpui_component::{h_flex, v_flex};

        let bg = cx.theme().background;
        let show_virtual = self.virtual_mem.is_some();

        let physical_card = memory_group_box(
            "physical-memory-card",
            v_flex()
                .w_full()
                .items_center()
                .py(px(crate::ui::memory_card::MEMORY_CARD_PY))
                .child(render_memory_card(
                    &self.physical,
                    "physical-memory",
                    true,
                    cx,
                )),
        );

        let memory_row = if show_virtual {
            let virtual_card = memory_group_box(
                "virtual-memory-card",
                v_flex()
                    .w_full()
                    .items_center()
                    .py(px(crate::ui::memory_card::MEMORY_CARD_PY))
                    .child(render_memory_card(
                        self.virtual_mem
                            .as_ref()
                            .expect("virtual card requires data"),
                        "virtual-memory",
                        false,
                        cx,
                    )),
            );

            h_flex()
                .w_full()
                .flex_shrink_0()
                .gap(px(SECTION_GAP))
                .child(div().flex_1().min_w_0().child(physical_card))
                .child(div().flex_1().min_w_0().child(virtual_card))
                .into_any_element()
        } else {
            h_flex()
                .w_full()
                .flex_shrink_0()
                .justify_center()
                .child(
                    div()
                        .w_full()
                        .max_w(px(SINGLE_CARD_MAX_WIDTH))
                        .child(physical_card),
                )
                .into_any_element()
        };

        div()
            .relative()
            .w_full()
            .h_full()
            .child(
                div().w_full().h_full().overflow_hidden().child(
                    v_flex()
                        .w_full()
                        .h_full()
                        .overflow_hidden()
                        .bg(bg)
                        .child(render_title_bar(self, window, cx))
                        .child({
                            let body = v_flex()
                                .w_full()
                                .flex_shrink_0()
                                .px(px(CONTENT_PADDING))
                                .pt(px(CONTENT_PADDING))
                                .child(memory_row)
                                .when(self.settings_expanded, |body| {
                                    body.gap(px(SECTION_GAP))
                                        .child(render_settings_content(self, cx))
                                });

                            v_flex()
                                .w_full()
                                .flex_shrink_0()
                                .min_h_0()
                                .overflow_hidden()
                                .gap(px(SECTION_GAP))
                                .child(body)
                                .child(
                                    div()
                                        .w_full()
                                        .flex_shrink_0()
                                        .px(px(CONTENT_PADDING))
                                        .pb(px(CONTENT_PADDING))
                                        .child(render_cleanup_footer(self, cx)),
                                )
                        }),
                ),
            )
            .children(gpui_component::Root::render_dialog_layer(window, cx))
    }
}
