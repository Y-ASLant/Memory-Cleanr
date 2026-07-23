mod clipboard_ops;
mod memory;
mod optimize_impl;
mod pinned_card;
mod window;

use rust_i18n::t;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::{Duration, Instant};

use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::{Root, TitleBar};
use smol::Timer;

use crate::anim::AnimatedValue;
use crate::locale;
use crate::memory::{MemorySection, MemoryStatus};
use crate::optimize::MemoryAreas;
use crate::settings::Settings;
use crate::tray::{TrayCommand, dispatch_command};
use crate::ui::layout::SECTION_GAP;
use crate::win32;

pub(crate) const SETTINGS_SAVE_DEBOUNCE: Duration = Duration::from_millis(300);
pub(crate) const OPTIMIZE_RESULT_DISPLAY: Duration = Duration::from_secs(5);
pub(crate) const MEMORY_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) async fn show_toast(title: String, body: String) {
    if let Err(e) = smol::unblock(move || win32::notification::show(&title, &body)).await {
        crate::log_msg(&format!("[notification] failed: {e:#}"));
    }
}

pub const WINDOW_WIDTH: f32 = 520.;
pub const WINDOW_MIN_WIDTH: f32 = 520.;
pub const CONTENT_PADDING: f32 = 6.;

pub fn window_size(expanded: bool, clipboard_visible: bool) -> Size<Pixels> {
    let height = if clipboard_visible {
        crate::ui::clipboard_panel::CLIPBOARD_WINDOW_HEIGHT
    } else if expanded {
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

pub fn window_options(expanded: bool, clipboard_visible: bool, cx: &App) -> WindowOptions {
    WindowOptions {
        titlebar: Some(TitleBar::title_bar_options()),
        window_bounds: Some(WindowBounds::centered(
            window_size(expanded, clipboard_visible),
            cx,
        )),
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
    launch_hidden: bool,
) -> Result<()> {
    let options = cx.update(|app| window_options(false, false, app));
    cx.open_window(options, |window, cx| {
        window.set_window_title(crate::version::APP_NAME);

        let app_entity =
            cx.new(|cx| MemoryCleanerApp::new(window, cx, settings, tray_rx, launch_hidden));
        let _ = win32::window::remove_maximize_button(window);
        crate::ui::theme::init_light_theme(window, cx);

        let root = cx.new(|cx| Root::new(app_entity.clone(), window, cx));

        if launch_hidden {
            app_entity.update(cx, |app, _| {
                app.destroy_window_to_tray(window, "startup");
                app.sync_tray();
            });
        } else {
            window.activate_window();
        }

        root
    })?;
    Ok(())
}

fn query_sections() -> Result<(MemorySection, MemorySection)> {
    let status = MemoryStatus::query()?;

    let physical = MemorySection {
        title: t!("memory.physical").to_string(),
        total: status.total_phys,
        used: status.used_phys(),
        avail: status.avail_phys,
        used_percent: status.memory_load as f32,
    };

    let virt_used = status
        .total_page_file
        .saturating_sub(status.avail_page_file);
    let virt_percent = if status.total_page_file > 0 {
        (virt_used as f64 / status.total_page_file as f64 * 100.0).round() as u32
    } else {
        0
    };
    let virtual_mem = MemorySection {
        title: t!("memory.virtual").to_string(),
        total: status.total_page_file,
        used: virt_used,
        avail: status.avail_page_file,
        used_percent: virt_percent as f32,
    };

    Ok((physical, virtual_mem))
}

pub struct MemoryCleanerApp {
    pub window: Option<AnyWindowHandle>,
    pub settings: Settings,
    pub physical: MemorySection,
    pub virtual_mem: MemorySection,
    pub(crate) settings_save_gen: u32,
    pub(crate) memory_refresh_generation: Arc<AtomicU32>,
    pub(crate) anim_generation: Arc<AtomicU32>,
    pub(crate) window_opening: bool,
    pub is_optimizing: bool,
    pub is_refreshing_icon_cache: bool,
    pub optimize_step: String,
    pub optimize_percent: f32,
    pub optimize_status: String,
    pub optimize_has_errors: bool,
    pub icon_cache_status: String,
    pub settings_expanded: bool,
    pub(crate) window_shown: bool,
    pub cleanup_hotkey_recording: bool,
    pub(crate) hotkey_capture_focus: FocusHandle,
    /// Clipboard history storage (None until clipboard is enabled).
    pub clipboard_storage: Option<crate::clipboard::storage::ClipboardStorage>,
    /// Clipboard monitor shutdown handle.
    pub clipboard_monitor: Option<crate::clipboard::monitor::MonitorHandle>,
    /// Clipboard panel visible state.
    pub clipboard_visible: bool,
    /// Cached clipboard items for the panel.
    pub clipboard_items: Vec<crate::clipboard::ClipboardItem>,
    /// Filter clipboard list by content type (`None` = all).
    pub clipboard_filter: Option<crate::clipboard::ContentType>,
    /// Sliding filter pill tween (ElegantClipboard-style segment indicator).
    pub clipboard_filter_slide: Option<ClipboardFilterSlide>,
    /// Bumps to cancel the filter-slide ticker.
    pub clipboard_filter_tick_gen: u32,
    /// Selected index in clipboard list (for keyboard nav).
    pub clipboard_selected: Option<usize>,
    /// Drop target while dragging to reorder.
    pub clipboard_drop_target_id: Option<i64>,
    /// Item currently being dragged (dims the source card).
    pub clipboard_dragging_id: Option<i64>,
    /// Drag pointer left the main window — release should pin a desktop card.
    pub clipboard_drag_tearoff: bool,
    /// Grab offset within the card (matches GPUI active_drag cursor_offset).
    pub clipboard_drag_cursor_offset: Option<Point<Pixels>>,
    /// Borderless follower window while dragging outside the main window.
    pub clipboard_tearoff_preview_handle: Option<AnyWindowHandle>,
    /// Prevents duplicate async open while the follower window is being created.
    pub clipboard_tearoff_preview_opening: bool,
    /// Bumps to cancel the global drag-position tracker.
    pub clipboard_drag_track_tick_gen: u32,
    /// Floating desktop card windows keyed by clipboard item id.
    pub pinned_card_handles: HashMap<i64, AnyWindowHandle>,
    /// Card under the pointer (reveals row actions).
    pub clipboard_hovered_id: Option<i64>,
    /// Item playing delete exit animation before removal.
    pub clipboard_deleting_id: Option<i64>,
    /// Per-item translateY tween while reordering (dnd-kit style make-way).
    pub clipboard_shift_anims: HashMap<i64, ClipboardShiftAnim>,
    /// Per-card hover reveal opacity tween.
    pub clipboard_hover_fades: HashMap<i64, ClipboardHoverFade>,
    /// Bumps to cancel the in-flight shift ticker.
    pub clipboard_shift_tick_gen: u32,
    /// Bumps to cancel the hover-fade ticker.
    pub clipboard_hover_fade_tick_gen: u32,
    /// Scroll handle for the clipboard virtual list.
    pub clipboard_list_scroll: UniformListScrollHandle,
    pub(crate) anim_physical: AnimatedValue,
    pub(crate) anim_virtual: AnimatedValue,
    pub(crate) anim_optimize: AnimatedValue,
    pub(crate) anim_used_phys: AnimatedValue,
    pub(crate) anim_avail_phys: AnimatedValue,
    pub(crate) anim_used_virt: AnimatedValue,
    pub(crate) anim_avail_virt: AnimatedValue,
    pub(crate) anim_dirty: bool,
}

/// One card's translateY animation during drag reorder.
#[derive(Clone, Debug)]
pub struct ClipboardShiftAnim {
    pub from: f32,
    pub to: f32,
    pub start: Instant,
}

/// Filter segment indicator position (0 = 全部, 1 = 文本, 2 = 文件).
#[derive(Clone, Debug)]
pub struct ClipboardFilterSlide {
    pub from: f32,
    pub to: f32,
    pub start: Instant,
}

/// Per-card hover reveal opacity (zone tint, labels, delete button).
#[derive(Clone, Debug)]
pub struct ClipboardHoverFade {
    pub from: f32,
    pub to: f32,
    pub start: Instant,
}

impl MemoryCleanerApp {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        settings: Settings,
        tray_rx: std::sync::mpsc::Receiver<TrayCommand>,
        launch_hidden: bool,
    ) -> Self {
        crate::log::set_debug_enabled(settings.debug_logging);
        if settings.debug_logging {
            crate::log::write(&t!(
                "log.debug_enabled",
                path = crate::log::log_file_path().display().to_string()
            ));
        }

        let (physical, virtual_mem) = query_sections().unwrap_or_else(|e| {
            crate::log_msg(&format!("[memory] initial query failed: {e}"));
            (
                MemorySection::unavailable(&t!("memory.physical")),
                MemorySection::unavailable(&t!("memory.virtual")),
            )
        });

        let clipboard_storage = if settings.clipboard_enabled {
            match crate::clipboard::storage::ClipboardStorage::open() {
                Ok(s) => Some(s),
                Err(e) => {
                    crate::log_msg(&format!("[clipboard] storage open failed: {e:#}"));
                    None
                }
            }
        } else {
            None
        };

        let phys_percent = physical.used_percent;
        let phys_used = physical.used as f32;
        let phys_avail = physical.avail as f32;
        let virt_percent = virtual_mem.used_percent;
        let virt_used = virtual_mem.used as f32;
        let virt_avail = virtual_mem.avail as f32;

        let mut app = Self {
            window: None,
            settings,
            physical,
            virtual_mem,
            settings_save_gen: 0,
            memory_refresh_generation: Arc::new(AtomicU32::new(0)),
            anim_generation: Arc::new(AtomicU32::new(0)),
            window_opening: false,
            is_optimizing: false,
            is_refreshing_icon_cache: false,
            optimize_step: String::new(),
            optimize_percent: 0.0,
            optimize_status: String::new(),
            optimize_has_errors: false,
            icon_cache_status: String::new(),
            settings_expanded: false,
            window_shown: !launch_hidden,
            cleanup_hotkey_recording: false,
            hotkey_capture_focus: cx.focus_handle(),
            clipboard_storage,
            clipboard_monitor: None,
            clipboard_visible: false,
            clipboard_items: Vec::new(),
            clipboard_filter: None,
            clipboard_filter_slide: None,
            clipboard_filter_tick_gen: 0,
            clipboard_selected: None,
            clipboard_drop_target_id: None,
            clipboard_dragging_id: None,
            clipboard_drag_tearoff: false,
            clipboard_drag_cursor_offset: None,
            clipboard_tearoff_preview_handle: None,
            clipboard_tearoff_preview_opening: false,
            clipboard_drag_track_tick_gen: 0,
            pinned_card_handles: HashMap::new(),
            clipboard_hovered_id: None,
            clipboard_deleting_id: None,
            clipboard_shift_anims: HashMap::new(),
            clipboard_hover_fades: HashMap::new(),
            clipboard_shift_tick_gen: 0,
            clipboard_hover_fade_tick_gen: 0,
            clipboard_list_scroll: UniformListScrollHandle::new(),
            anim_physical: AnimatedValue::new(phys_percent),
            anim_virtual: AnimatedValue::new(virt_percent),
            anim_optimize: AnimatedValue::new(0.0),
            anim_used_phys: AnimatedValue::new(phys_used),
            anim_avail_phys: AnimatedValue::new(phys_avail),
            anim_used_virt: AnimatedValue::new(virt_used),
            anim_avail_virt: AnimatedValue::new(virt_avail),
            anim_dirty: false,
        };

        cx.set_global(AppEntityHolder(cx.entity()));
        app.attach_window(window, cx, launch_hidden);
        app.start_background_tasks(cx, tray_rx);
        app.sync_tray();

        app
    }

    pub(crate) fn attach_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        launch_hidden: bool,
    ) {
        self.window = Some(window.window_handle());
        self.window_shown = !launch_hidden;
        if let Ok(hwnd) = win32::window::hwnd_from_window(window) {
            win32::focus::save_current_focus();
            win32::focus::set_our_hwnd(hwnd);
        }

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

        if self.settings.always_on_top
            && let Err(error) = win32::window::set_always_on_top(window, true)
        {
            crate::log_msg(&format!(
                "[window] set_always_on_top(true) failed: {error:#}"
            ));
        }

        if !launch_hidden {
            self.start_memory_refresh(cx);
            self.start_anim(cx);
        }

        if self.clipboard_visible {
            self.refresh_clipboard_items();
            window.resize(window_size(self.settings_expanded, true));
        }
    }

    pub(crate) fn sync_tray(&self) {
        crate::tray::sync_display(&self.physical, &self.virtual_mem, self.window_visible());
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

    pub(crate) fn is_busy(&self) -> bool {
        self.is_refreshing_icon_cache || self.is_optimizing
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
            "clipboard" => self.show_clipboard_window(cx),
            "quit" => {
                self.settings.save();
                cx.quit();
            }
            _ => {}
        }
    }

    pub fn start_background_tasks(
        &mut self,
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

        // 剪贴板监听
        if self.settings.clipboard_enabled {
            match crate::clipboard::monitor::start_monitor() {
                Ok((clip_rx, handle)) => {
                    self.clipboard_monitor = Some(handle);
                    crate::log_msg("[clipboard] monitor started");

                    cx.spawn(async move |this, cx| {
                        let mut rx = clip_rx;
                        loop {
                            let (result, returned_rx) = smol::unblock(move || {
                                let r = rx.recv();
                                (r, rx)
                            })
                            .await;
                            rx = returned_rx;

                            let Ok(content) = result else {
                                break;
                            };

                            if this
                                .update(cx, |app, cx| {
                                    app.handle_clipboard_content(content, cx);
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .detach();
                }
                Err(e) => {
                    crate::log_msg(&format!("[clipboard] monitor start failed: {e:#}"));
                }
            }
        }
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

    pub fn apply_locale(&mut self, cx: &mut Context<Self>) {
        locale::apply(&self.settings);
        if let Ok((physical, virtual_mem)) = query_sections() {
            self.physical = physical;
            self.virtual_mem = virtual_mem;
        } else {
            self.physical = MemorySection::unavailable(&t!("memory.physical"));
            self.virtual_mem = MemorySection::unavailable(&t!("memory.virtual"));
        }
        self.sync_anim_targets_from_sections();
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
        // Drop / Esc / release-outside clears GPUI drag; sync reorder or pin preview state.
        if self.clipboard_dragging_id.is_some() && !cx.has_active_drag() {
            let tearoff = self.clipboard_drag_tearoff;
            let item_id = self.clipboard_dragging_id;
            self.clipboard_drag_tearoff = false;
            self.clear_clipboard_drag_preview(cx);
            if tearoff && let Some(id) = item_id {
                self.open_pinned_card_from_tearoff(id, cx);
            }
        }
        use crate::ui::memory_card::render_memory_card;
        use crate::ui::settings_page::{render_cleanup_footer, render_settings_content};
        use crate::ui::title_bar::render_title_bar;
        use gpui::prelude::FluentBuilder;
        use gpui_component::{h_flex, v_flex};

        let bg = cx.theme().background;

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
                    self.anim_physical.current,
                    self.animated_used_phys(),
                    self.animated_avail_phys(),
                    cx,
                )),
        );

        let virtual_card = memory_group_box(
            "virtual-memory-card",
            v_flex()
                .w_full()
                .items_center()
                .py(px(crate::ui::memory_card::MEMORY_CARD_PY))
                .child(render_memory_card(
                    &self.virtual_mem,
                    "virtual-memory",
                    false,
                    self.anim_virtual.current,
                    self.animated_used_virt(),
                    self.animated_avail_virt(),
                    cx,
                )),
        );

        let memory_row = h_flex()
            .w_full()
            .flex_shrink_0()
            .gap(px(SECTION_GAP))
            .child(div().flex_1().min_w_0().child(physical_card))
            .child(div().flex_1().min_w_0().child(virtual_card))
            .into_any_element();

        // Left panel content (memory management)
        let left_panel = {
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
        };

        let main_content = if self.clipboard_visible {
            use crate::ui::clipboard_panel::render_clipboard_panel;

            render_clipboard_panel(self, cx).into_any_element()
        } else {
            left_panel.into_any_element()
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
                        .child(main_content),
                ),
            )
            .children(gpui_component::Root::render_dialog_layer(window, cx))
    }
}
