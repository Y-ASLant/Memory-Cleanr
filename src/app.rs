use rust_i18n::t;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use anyhow::Result;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::{Root, TitleBar, WindowExt};
use smol::Timer;

use crate::locale;
use crate::memory::MemorySection;
use crate::optimize::{self, MemoryAreas};
use crate::service::{memory, optimize_runner};
use crate::settings::Settings;
use crate::ui::layout::SECTION_GAP;
use crate::win32;
use crate::win32::ipc::IpcMessage;

const SETTINGS_SAVE_DEBOUNCE: Duration = Duration::from_millis(300);
const OPTIMIZE_RESULT_DISPLAY: Duration = Duration::from_secs(5);
const MEMORY_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    ReturnToTray,
    #[allow(dead_code)]
    ExitApp,
}

pub fn resolve_close_action(close_to_notification_area: bool) -> CloseAction {
    if close_to_notification_area {
        CloseAction::ReturnToTray
    } else {
        CloseAction::ExitApp
    }
}

pub fn open_main_window(cx: &mut AsyncApp, settings: Settings) -> Result<()> {
    let options = cx.update(|app| window_options(false, app));
    cx.open_window(options, |window, cx| {
        window.set_window_title(crate::version::APP_NAME);

        let app_entity = cx.new(|cx| MemoryCleanerApp::new(window, cx, settings));
        let _ = win32::window::remove_maximize_button(window);
        crate::ui::theme::init_light_theme(window, cx);

        let root = cx.new(|cx| Root::new(app_entity.clone(), window, cx));
        window.activate_window();
        root
    })?;
    Ok(())
}

pub struct MemoryCleanerApp {
    pub window: Option<AnyWindowHandle>,
    main_hwnd: Option<isize>,
    pub settings: Settings,
    pub physical: MemorySection,
    pub virtual_mem: Option<MemorySection>,
    settings_save_gen: u32,
    memory_refresh_generation: Arc<AtomicU32>,
    pub is_optimizing: bool,
    pub is_refreshing_icon_cache: bool,
    pub optimize_step: String,
    pub optimize_percent: f32,
    pub optimize_status: String,
    pub optimize_has_errors: bool,
    pub icon_cache_status: String,
    pub settings_expanded: bool,
    pub cleanup_hotkey_recording: bool,
    pub(crate) hotkey_capture_focus: FocusHandle,
    is_closing: Arc<AtomicBool>,
}

impl MemoryCleanerApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, settings: Settings) -> Self {
        crate::log::init_from_settings(&settings);

        let show_virtual = settings.show_virtual_memory;
        let (physical, virtual_mem) = memory::initial_sections(show_virtual);
        let is_closing = Arc::new(AtomicBool::new(false));

        let mut app = Self {
            window: None,
            main_hwnd: None,
            settings,
            physical,
            virtual_mem,
            settings_save_gen: 0,
            memory_refresh_generation: Arc::new(AtomicU32::new(0)),
            is_optimizing: false,
            is_refreshing_icon_cache: false,
            optimize_step: String::new(),
            optimize_percent: 0.0,
            optimize_status: String::new(),
            optimize_has_errors: false,
            icon_cache_status: String::new(),
            settings_expanded: false,
            cleanup_hotkey_recording: false,
            hotkey_capture_focus: cx.focus_handle(),
            is_closing: Arc::clone(&is_closing),
        };

        cx.set_global(AppEntityHolder(cx.entity()));
        app.attach_window(window, cx);

        app
    }

    fn attach_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.window = Some(window.window_handle());
        if let Ok(hwnd) = win32::window::hwnd_from_window(window) {
            self.main_hwnd = Some(hwnd.0 as isize);
            let session = win32::ipc::GuiSession {
                pid: std::process::id(),
                hwnd: hwnd.0 as isize,
            };
            std::thread::Builder::new()
                .name("gui-ipc-init".into())
                .spawn(move || {
                    if let Err(error) = win32::ipc::register_gui_session(session) {
                        crate::log_msg(&format!("[ipc] GUI register failed: {error:#}"));
                    }
                })
                .ok();
        }

        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |_window, gpui_app| {
            crate::log_msg("[close] on_window_should_close");
            let _ = weak.update(gpui_app, |this, cx| this.request_close("should_close", cx));
            false
        });

        if self.settings.always_on_top {
            let _ = win32::window::set_always_on_top(window, true);
        }

        self.start_memory_refresh(cx);
    }

    fn start_memory_refresh(&self, cx: &mut Context<Self>) {
        let generation = self.memory_refresh_generation.load(Ordering::Relaxed);
        let gen_arc = Arc::clone(&self.memory_refresh_generation);
        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(MEMORY_REFRESH_INTERVAL).await;
                if gen_arc.load(Ordering::Relaxed) != generation {
                    break;
                }
                let Ok(()) = this.update(cx, |app, cx| {
                    if app.refresh_memory() {
                        cx.notify();
                    }
                }) else {
                    break;
                };
            }
        })
        .detach();
    }

    pub(crate) fn queue_settings_save(&mut self, cx: &mut Context<Self>) {
        self.settings_save_gen = self.settings_save_gen.wrapping_add(1);
        let generation = self.settings_save_gen;

        cx.spawn(async move |this, cx| {
            Timer::after(SETTINGS_SAVE_DEBOUNCE).await;
            let saved = this
                .update(cx, |app, _| {
                    if app.settings_save_gen == generation {
                        app.settings.save();
                        true
                    } else {
                        false
                    }
                })
                .ok();
            if saved == Some(true) {
                win32::ipc::send_to_tray_logged(
                    IpcMessage::SettingsChanged,
                    "settings notify",
                );
            }
        })
        .detach();
    }

    pub fn refresh_memory(&mut self) -> bool {
        memory::refresh_sections(
            &mut self.physical,
            &mut self.virtual_mem,
            self.settings.show_virtual_memory,
        )
    }

    pub fn close_action(&mut self, source: &str) -> CloseAction {
        crate::log_msg(&format!(
            "[close] close_action source={source} close_to_tray={}",
            self.settings.close_to_notification_area
        ));
        self.settings.save();
        crate::log_msg("[close] settings saved");
        resolve_close_action(self.settings.close_to_notification_area)
    }

    pub fn handle_window_close(&mut self, cx: &mut Context<Self>) {
        self.request_close("titlebar", cx);
    }

    fn request_close(&mut self, source: &str, cx: &mut Context<Self>) {
        if self
            .is_closing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            crate::log_msg(&format!("[close] request_close ignored source={source}"));
            return;
        }

        let action = self.close_action(source);
        self.stop_memory_refresh();
        self.hide_main_window();
        std::thread::Builder::new()
            .name("gui-ipc-unregister".into())
            .spawn(|| {
                win32::ipc::send_to_tray_logged(IpcMessage::UnregisterGui, "unregister GUI");
            })
            .ok();
        match action {
            CloseAction::ReturnToTray => {
                crate::log_msg("[close] hide_to_tray");
            }
            CloseAction::ExitApp => {
                crate::log_msg("[close] quit_app");
            }
        }
        cx.defer(|cx| {
            crate::log_msg("[close] schedule_app_quit");
            cx.quit();
        });
    }

    fn hide_main_window(&self) {
        let Some(hwnd) = self.main_hwnd else {
            crate::log_msg("[close] hide_window skipped (no hwnd)");
            return;
        };
        match win32::window::hide_hwnd_raw(hwnd) {
            Ok(()) => crate::log_msg("[close] hide_window"),
            Err(error) => {
                crate::log_msg(&format!("[close] hide_window failed: {error:#}"));
            }
        }
    }

    fn stop_memory_refresh(&self) {
        self.memory_refresh_generation
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn activate_window(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.window {
            let _ = handle.update(cx, |_, window, _| -> Result<()> {
                crate::log_msg("[window] activate_window");
                win32::window::show_from_tray(window)?;
                window.activate_window();
                Ok(())
            });
        }
    }

    pub fn apply_locale(&mut self, cx: &mut Context<Self>) {
        locale::apply(&self.settings);
        let show_virtual = self.settings.show_virtual_memory;
        let (physical, virtual_mem) = memory::query_sections(show_virtual).unwrap_or_else(|_| {
            memory::unavailable_sections(show_virtual)
        });
        self.physical = physical;
        self.virtual_mem = virtual_mem;
        if !self.is_optimizing {
            self.optimize_status.clear();
            self.optimize_has_errors = false;
            self.optimize_step.clear();
        }
        if !self.is_refreshing_icon_cache {
            self.icon_cache_status.clear();
        }
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

    pub fn add_excluded_process_by_name(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.is_optimizing {
            return;
        }
        let normalized = win32::process::normalize_process_name(name);
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
        self.queue_settings_save(cx);
        cx.notify();
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

    pub fn set_always_on_top(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.always_on_top = enabled;
        if let Some(handle) = self.window {
            let _ = handle.update(cx, |_, window, _| {
                let _ = win32::window::set_always_on_top(window, enabled);
            });
        }
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn set_close_to_tray(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.close_to_notification_area = enabled;
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn set_run_at_startup(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if let Err(error) = win32::startup::set_enabled(enabled) {
            crate::log_msg(&format!(
                "[startup] set_enabled({enabled}) failed: {error:#}"
            ));
            cx.notify();
            return;
        }
        self.settings.run_at_startup = enabled;
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
        crate::log::sync_debug_settings(&self.settings);
        if enabled {
            crate::log::write(&t!(
                "log.debug_enabled",
                path = crate::log::log_file_path().display().to_string()
            ));
        }
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.is_refreshing_icon_cache || self.is_optimizing
    }

    pub fn run_optimize(&mut self, cx: &mut Context<Self>) {
        if self.is_optimizing {
            return;
        }

        let areas = self.settings.memory_areas();
        let excluded = self.settings.excluded_processes.clone();
        if !matches!(optimize::step_plan(areas, &excluded), Ok(steps) if !steps.is_empty()) {
            self.optimize_status = t!("tooltip.select_areas").to_string();
            cx.notify();
            return;
        }

        let avail_before = self.physical.avail;
        let settings = self.settings.clone();
        let notify = self.settings.show_optimization_notifications;
        self.is_optimizing = true;
        self.optimize_step = t!("button.cleanup_preparing").to_string();
        self.optimize_percent = 0.0;
        self.optimize_status.clear();
        self.optimize_has_errors = false;
        win32::ipc::send_to_tray_logged(IpcMessage::SpinStart, "spin start");
        cx.notify();

        cx.spawn(async move |this, cx| {
            if notify {
                smol::unblock({
                    let settings = settings.clone();
                    move || optimize_runner::notify_optimize_start(&settings)
                })
                .await;
            }

            let (progress_tx, progress_rx) = std::sync::mpsc::channel();
            let settings_for_thread = settings.clone();
            let worker = std::thread::Builder::new()
                .name("gui-optimize".into())
                .spawn(move || {
                    optimize_runner::run(&settings_for_thread, avail_before, |update| {
                        let _ = progress_tx.send(update);
                    })
                })
                .expect("spawn optimize worker");

            loop {
                match progress_rx.try_recv() {
                    Ok(update) => {
                        let _ = this.update(cx, |app, cx| {
                            app.optimize_step = update.step_label;
                            app.optimize_percent = update.percent;
                            cx.notify();
                        });
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        if worker.is_finished() {
                            break;
                        }
                        Timer::after(Duration::from_millis(16)).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
            }

            let result = worker.join().expect("optimize worker panicked");

            this
                .update(cx, |app, cx| {
                    let _ = app.refresh_memory();
                    app.optimize_step.clear();
                    app.is_optimizing = false;
                    app.optimize_percent = 0.0;
                    win32::ipc::send_to_tray_logged(IpcMessage::SpinStop, "spin stop");
                    app.optimize_has_errors = result.has_errors;
                    app.optimize_status = result.status_message.clone();
                    crate::log::write(&format!("[optimize] result: {}", app.optimize_status));
                    cx.notify();
                })
                .ok();

            if notify {
                let settings = settings.clone();
                let result = result.clone();
                smol::unblock(move || optimize_runner::notify_optimize_complete(&settings, &result))
                    .await;
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
