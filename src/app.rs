use std::time::Duration;

use anyhow::Result;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::WindowExt;
use smol::Timer;

use crate::memory::{MemorySection, MemoryStatus};
use crate::messages::{build_cleanup_result_message, format_freed_message};
use crate::optimize::{self, MemoryAreas};
use crate::settings::Settings;
use crate::tray::{TrayCommand, dispatch_command};
use crate::ui::layout::SECTION_GAP;
use crate::win32;

const MEMORY_POLL_INTERVAL: Duration = Duration::from_secs(5);
const SETTINGS_SAVE_DEBOUNCE: Duration = Duration::from_millis(300);
const OPTIMIZE_RESULT_DISPLAY: Duration = Duration::from_secs(5);

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

fn query_sections(show_virtual: bool) -> Result<(MemorySection, Option<MemorySection>)> {
    let status = MemoryStatus::query()?;

    let physical = MemorySection {
        title: "物理内存".into(),
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
            title: "虚拟内存".into(),
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
    pub window: AnyWindowHandle,
    pub settings: Settings,
    pub physical: MemorySection,
    pub virtual_mem: Option<MemorySection>,
    settings_save_gen: u32,
    pub is_optimizing: bool,
    pub is_refreshing_icon_cache: bool,
    pub optimize_step: String,
    pub optimize_percent: f32,
    pub optimize_status: String,
    pub icon_cache_status: String,
    pub settings_expanded: bool,
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
            crate::log::write(&format!(
                "调试日志已启用，输出路径: {}",
                crate::log::log_file_path().display()
            ));
        }

        let show_virtual = settings.show_virtual_memory;
        let (physical, virtual_mem) = query_sections(show_virtual).unwrap_or_else(|e| {
            crate::log_msg(&format!("[memory] initial query failed: {e}"));
            (
                MemorySection::unavailable("物理内存"),
                if show_virtual {
                    Some(MemorySection::unavailable("虚拟内存"))
                } else {
                    None
                },
            )
        });
        let window_handle = window.window_handle();

        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, app| {
            weak.update(app, |this, cx| {
                if this.settings.close_to_notification_area {
                    this.settings.save();
                    let _ = win32::window::hide_to_tray(window);
                    this.sync_tray(cx);
                    false
                } else {
                    this.settings.save();
                    true
                }
            })
            .unwrap_or(true)
        });

        if settings.always_on_top {
            let _ = win32::window::set_always_on_top(window, true);
        }

        let app = Self {
            window: window_handle,
            settings,
            physical,
            virtual_mem,
            settings_save_gen: 0,
            is_optimizing: false,
            is_refreshing_icon_cache: false,
            optimize_step: String::new(),
            optimize_percent: 0.0,
            optimize_status: String::new(),
            icon_cache_status: String::new(),
            settings_expanded: false,
        };

        app.start_background_tasks(cx, tray_rx);
        app.sync_tray(cx);

        app
    }

    fn window_visible(&self, cx: &mut Context<Self>) -> bool {
        self.window
            .update(cx, |_, window, _| win32::window::is_visible(window))
            .flatten()
            .unwrap_or(true)
    }

    fn sync_tray(&self, cx: &mut Context<Self>) {
        let virtual_mem = if self.settings.show_virtual_memory {
            self.virtual_mem.as_ref()
        } else {
            None
        };
        crate::tray::sync_display(&self.physical, virtual_mem, self.window_visible(cx));
    }

    fn queue_settings_save(&mut self, cx: &mut Context<Self>) {
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

    pub fn refresh_memory(&mut self, _cx: &mut Context<Self>) -> bool {
        let show_virtual = self.settings.show_virtual_memory;
        let Ok((physical, virtual_mem)) = query_sections(show_virtual) else {
            let degraded = if self.physical.is_unavailable()
                && self.virtual_mem.as_ref().is_none_or(|v| v.is_unavailable())
            {
                false
            } else {
                self.physical = MemorySection::unavailable("物理内存");
                self.virtual_mem = if show_virtual {
                    Some(MemorySection::unavailable("虚拟内存"))
                } else {
                    None
                };
                true
            };
            return degraded;
        };
        let phys_changed = self.physical != physical;
        let virt_changed = self.virtual_mem != virtual_mem;

        if phys_changed {
            self.physical = physical;
        }
        if virt_changed {
            self.virtual_mem = virtual_mem;
        }

        if !(phys_changed || virt_changed) {
            return false;
        }

        true
    }

    pub fn activate_window(&self, cx: &mut Context<Self>) {
        let _ = self.window.update(cx, |_, window, _| {
            let _ = win32::window::show_from_tray(window);
            window.activate_window();
        });
        self.sync_tray(cx);
    }

    pub fn hide_to_tray(&self, cx: &mut Context<Self>) {
        let _ = self.window.update(cx, |_, window, _| {
            let _ = win32::window::hide_to_tray(window);
        });
        self.sync_tray(cx);
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

    pub fn open_window_behavior_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::layout::{
            DIALOG_PADDING_HORIZONTAL, DIALOG_PADDING_TOP, TITLE_BAR_H,
            WINDOW_BEHAVIOR_DIALOG_ESTIMATED_HEIGHT, WINDOW_BEHAVIOR_DIALOG_WIDTH,
            centered_dialog_margin_top,
        };
        use crate::ui::settings_page::render_window_behavior_dialog;

        let weak = cx.weak_entity();
        window.open_dialog(cx, move |dialog, window, _cx| {
            let weak = weak.clone();
            let margin_top = centered_dialog_margin_top(
                window.viewport_size().height,
                WINDOW_BEHAVIOR_DIALOG_ESTIMATED_HEIGHT,
                TITLE_BAR_H,
            );
            dialog
                .title("窗口行为")
                .w(px(WINDOW_BEHAVIOR_DIALOG_WIDTH))
                .margin_top(margin_top)
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

    pub fn set_debug_logging(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.debug_logging = enabled;
        crate::log::set_debug_enabled(enabled);
        if enabled {
            crate::log::write(&format!(
                "调试日志已启用，输出路径: {}",
                crate::log::log_file_path().display()
            ));
        }
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn handle_tray_action(&mut self, action: &str, cx: &mut Context<Self>) {
        match action {
            "optimize" => self.run_optimize(cx),
            "toggle_window" => {
                if self.window_visible(cx) {
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
        cx.spawn(async move |this, cx| {
            loop {
                let (command, rx) = smol::unblock(move || {
                    let result = tray_rx.recv_timeout(MEMORY_POLL_INTERVAL);
                    (result.ok(), tray_rx)
                })
                .await;
                tray_rx = rx;

                if this
                    .update(cx, |this, cx| {
                        let mut changed = false;
                        if let Some(command) = command {
                            dispatch_command(this, command, cx);
                            changed = true;
                        }
                        if this.refresh_memory(cx) {
                            changed = true;
                        }
                        this.sync_tray(cx);
                        if changed {
                            cx.notify();
                        }
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
        name: &'static str,
        run: optimize::OptimizeStepFn,
        step_index: usize,
        total_steps: usize,
    ) -> bool {
        let step_base = step_index as f32 / total_steps as f32;
        let step_span = 1.0 / total_steps as f32;

        let _ = this.update(cx, |app, cx| {
            app.optimize_step = format!("正在清理 {name}...");
            app.optimize_percent = step_base * 100.0;
            cx.notify();
        });

        Timer::after(Duration::from_millis(60)).await;

        let result = smol::unblock(run).await;

        if let Err(e) = &result {
            crate::log::write(&format!("[optimize] {name} 失败: {e:#}"));
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
                    app.optimize_step = format!("正在清理 {name}...");
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
                app.optimize_step = format!(
                    "正在清理 {name} ({drive}:) [{}/{}]...",
                    drive_index + 1,
                    drive_total
                );
                app.optimize_percent = (step_base + sub_base * step_span) * 100.0;
                cx.notify();
            });

            let drive_result = smol::unblock(move || optimize::optimize_drive_cache(drive)).await;
            if let Err(e) = drive_result {
                crate::log::write(&format!(
                    "[optimize] 已修改文件 驱动器 {drive}: 失败: {e:#}"
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
        let Ok(steps) = optimize::step_plan(areas) else {
            self.optimize_status = "请先选择清理区域".into();
            cx.notify();
            return;
        };
        if steps.is_empty() {
            self.optimize_status = "请先选择清理区域".into();
            cx.notify();
            return;
        }

        let avail_before = self.physical.avail;
        let total = steps.len();
        self.is_optimizing = true;
        self.optimize_step = "准备清理...".into();
        self.optimize_percent = 0.0;
        self.optimize_status.clear();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let mut completed = Vec::new();
            let mut errors = Vec::new();

            for (index, (name, run)) in steps.into_iter().enumerate() {
                let ok = if name == MemoryAreas::MODIFIED_FILE_CACHE.label() {
                    Self::run_modified_file_cache_step(this.clone(), cx, index, total).await
                } else {
                    Self::run_optimize_step(this.clone(), cx, name, run, index, total).await
                };

                if ok {
                    completed.push(name);
                    crate::log::write(&format!("[optimize] {name} 成功"));
                } else {
                    errors.push(name);
                }
            }

            let _ = this.update(cx, |app, cx| {
                let _ = app.refresh_memory(cx);
                let avail_after = app.physical.avail;
                let freed_detail = format_freed_message(avail_before, avail_after);
                app.optimize_step.clear();
                app.is_optimizing = false;
                app.optimize_percent = 0.0;
                app.optimize_status =
                    build_cleanup_result_message(&completed, &errors, &freed_detail);
                crate::log::write(&format!("[optimize] 结果: {}", app.optimize_status));
                cx.notify();
            });

            Timer::after(OPTIMIZE_RESULT_DISPLAY).await;

            let _ = this.update(cx, |app, cx| {
                app.optimize_status.clear();
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
                .title("刷新桌面图标缓存")
                .description("将短暂结束并重启资源管理器，桌面与任务栏会闪断。是否继续？")
                .overlay_closable(false)
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("确认")
                        .cancel_text("取消")
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
        self.icon_cache_status = "正在刷新桌面图标缓存…".into();
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

impl Render for MemoryCleanerApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::ui::memory_card::render_memory_card;
        use crate::ui::settings_page::{render_cleanup_footer, render_settings_content};
        use crate::ui::title_bar::render_title_bar;
        use gpui_component::{
            group_box::{GroupBox, GroupBoxVariants},
            h_flex, v_flex,
        };

        let bg = cx.theme().background;
        let show_virtual = self.virtual_mem.is_some();

        let physical_card = GroupBox::new()
            .id("physical-memory-card")
            .outline()
            .w_full()
            .p_0()
            .child(
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
            h_flex()
                .w_full()
                .flex_shrink_0()
                .gap(px(SECTION_GAP))
                .child(div().flex_1().min_w_0().child(physical_card))
                .child(div().flex_1().min_w_0().child({
                    GroupBox::new()
                        .id("virtual-memory-card")
                        .outline()
                        .w_full()
                        .p_0()
                        .child(
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
                        )
                }))
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
                            let mut body = v_flex()
                                .w_full()
                                .flex_shrink_0()
                                .px(px(CONTENT_PADDING))
                                .pt(px(CONTENT_PADDING))
                                .child(memory_row);
                            if self.settings_expanded {
                                body = body
                                    .gap(px(SECTION_GAP))
                                    .child(render_settings_content(self, cx));
                            }
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
