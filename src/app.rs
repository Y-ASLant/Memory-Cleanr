use std::time::Duration;

use anyhow::Result;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::WindowExt;
use smol::Timer;

use crate::memory::{MemorySection, MemoryStatus};
use crate::optimize::{self, MemoryAreas};
use crate::settings::Settings;
use crate::tray::{poll_menu_events, poll_tray_click};
use crate::ui::layout::SECTION_GAP;
use crate::ui::memory_card::{RING_ANIM_DURATION_MS, RingAnim};
use crate::win32;

pub const TRAY_POLL: Duration = Duration::from_millis(1000);
const SETTINGS_SAVE_DEBOUNCE: Duration = Duration::from_millis(300);
const OPTIMIZE_RESULT_DISPLAY: Duration = Duration::from_secs(5);

const WINDOW_WIDTH: f32 = 560.;
const WINDOW_HEIGHT_COLLAPSED: f32 = 300.;
const WINDOW_MIN_WIDTH: f32 = 560.;
const WINDOW_MIN_HEIGHT: f32 = 300.;
pub const CONTENT_PADDING: f32 = 6.;
const SINGLE_CARD_MAX_WIDTH: f32 = 360.;

pub fn window_size(expanded: bool) -> Size<Pixels> {
    let height = if expanded {
        crate::ui::layout::expanded_window_height(CONTENT_PADDING)
    } else {
        WINDOW_HEIGHT_COLLAPSED
    };
    size(px(WINDOW_WIDTH), px(height))
}

pub fn window_min_size() -> Size<Pixels> {
    size(px(WINDOW_MIN_WIDTH), px(WINDOW_MIN_HEIGHT))
}

fn build_section(
    total: u64,
    used: u64,
    avail: u64,
    used_percent: u32,
    title: &str,
) -> MemorySection {
    MemorySection {
        title: title.into(),
        total,
        used,
        avail,
        used_percent: used_percent as f32,
    }
}

fn query_sections(show_virtual: bool) -> Result<(MemorySection, Option<MemorySection>)> {
    let status = MemoryStatus::query()?;

    let physical = build_section(
        status.total_phys,
        status.used_phys(),
        status.avail_phys,
        status.memory_load,
        "物理内存",
    );

    let virtual_mem = if show_virtual {
        let virt_used = status
            .total_page_file
            .saturating_sub(status.avail_page_file);
        let virt_percent = if status.total_page_file > 0 {
            (virt_used as f64 / status.total_page_file as f64 * 100.0).round() as u32
        } else {
            0
        };
        Some(build_section(
            status.total_page_file,
            virt_used,
            status.avail_page_file,
            virt_percent,
            "虚拟内存",
        ))
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
    pub physical_ring: RingAnim,
    pub virtual_ring: RingAnim,
    physical_ring_gen: u32,
    virtual_ring_gen: u32,
    settings_save_gen: u32,
    pub is_optimizing: bool,
    pub optimize_step: String,
    pub optimize_percent: f32,
    pub optimize_status: String,
    pub settings_expanded: bool,
}

impl MemoryCleanerApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, settings: Settings) -> Self {
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

        let physical_ring = RingAnim::new(physical.used_percent);
        let virtual_ring = virtual_mem
            .as_ref()
            .map(|v| RingAnim::new(v.used_percent))
            .unwrap_or_default();

        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, app| {
            weak.update(app, |this, _| {
                if this.settings.close_to_notification_area {
                    this.settings.save();
                    let _ = win32::window::hide_to_tray(window);
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
            physical_ring,
            virtual_ring,
            physical_ring_gen: 0,
            virtual_ring_gen: 0,
            settings_save_gen: 0,
            is_optimizing: false,
            optimize_step: String::new(),
            optimize_percent: 0.0,
            optimize_status: String::new(),
            settings_expanded: false,
        };

        app.start_background_poll(cx);

        app
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

    fn schedule_ring_sync(
        &mut self,
        which: RingKind,
        generation: u32,
        target: f32,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_millis(RING_ANIM_DURATION_MS)).await;
            let _ = this.update(cx, |app, cx| {
                let (active_gen, ring) = match which {
                    RingKind::Physical => (&mut app.physical_ring_gen, &mut app.physical_ring),
                    RingKind::Virtual => (&mut app.virtual_ring_gen, &mut app.virtual_ring),
                };
                if *active_gen != generation {
                    return;
                }
                ring.from = target;
                cx.notify();
            });
        })
        .detach();
    }

    fn update_ring_target(
        &mut self,
        which: RingKind,
        new_target: f32,
        cx: &mut Context<Self>,
    ) -> bool {
        let target = new_target.clamp(0.0, 100.0);
        let ring = match which {
            RingKind::Physical => &mut self.physical_ring,
            RingKind::Virtual => &mut self.virtual_ring,
        };

        if (ring.to - target).abs() <= 0.01 {
            return false;
        }

        ring.from = ring.to;
        ring.to = target;

        let generation = match which {
            RingKind::Physical => {
                self.physical_ring_gen = self.physical_ring_gen.wrapping_add(1);
                self.physical_ring_gen
            }
            RingKind::Virtual => {
                self.virtual_ring_gen = self.virtual_ring_gen.wrapping_add(1);
                self.virtual_ring_gen
            }
        };

        self.schedule_ring_sync(which, generation, target, cx);
        true
    }

    fn sync_ring_targets(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        changed |= self.update_ring_target(RingKind::Physical, self.physical.used_percent, cx);
        if let Some(virt) = self.virtual_mem.as_ref() {
            changed |= self.update_ring_target(RingKind::Virtual, virt.used_percent, cx);
        }
        changed
    }

    pub fn refresh_memory(&mut self, cx: &mut Context<Self>) -> bool {
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
                self.sync_ring_targets(cx);
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

        self.sync_ring_targets(cx);
        true
    }

    pub fn activate_window(&self, cx: &mut Context<Self>) {
        let _ = self.window.update(cx, |_, window, _| {
            let _ = win32::window::show_from_tray(window);
            window.activate_window();
        });
    }

    pub fn hide_to_tray(&self, cx: &mut Context<Self>) {
        let _ = self.window.update(cx, |_, window, _| {
            let _ = win32::window::hide_to_tray(window);
        });
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
        use crate::ui::layout::{DIALOG_PADDING_HORIZONTAL, DIALOG_PADDING_TOP};
        use crate::ui::settings_page::render_window_behavior_dialog;

        let weak = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let weak = weak.clone();
            dialog
                .title("窗口行为")
                .w(px(400.))
                .pt(px(DIALOG_PADDING_TOP))
                .pb(px(CONTENT_PADDING))
                .pl(px(DIALOG_PADDING_HORIZONTAL))
                .pr(px(DIALOG_PADDING_HORIZONTAL))
                .overlay_closable(true)
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
            "show" => self.activate_window(cx),
            "hide" => self.hide_to_tray(cx),
            "quit" => {
                self.settings.save();
                cx.quit();
            }
            _ => {}
        }
    }

    pub fn poll_tray(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;

        if poll_tray_click() {
            self.activate_window(cx);
            changed = true;
        }

        while let Some(action) = poll_menu_events() {
            self.handle_tray_action(&action, cx);
            changed = true;
        }

        changed
    }

    pub fn start_background_poll(&self, cx: &mut Context<Self>) {
        const MEMORY_POLL_TICKS: u32 = 5; // 5 × TRAY_POLL ≈ 5 s memory refresh

        cx.spawn(async move |this, cx| {
            let mut ticks = 0u32;
            loop {
                Timer::after(TRAY_POLL).await;
                ticks += 1;

                if this
                    .update(cx, |this, cx| {
                        let mut changed = this.poll_tray(cx);
                        if ticks >= MEMORY_POLL_TICKS {
                            ticks = 0;
                            if this.refresh_memory(cx) {
                                changed = true;
                            }
                        }
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

    fn format_freed_message(avail_before: u64, avail_after: u64) -> String {
        if avail_after > avail_before {
            format!(
                "+{}",
                MemoryStatus::format_bytes(avail_after - avail_before)
            )
        } else {
            String::new()
        }
    }

    fn build_result_message(completed: &[&str], errors: &[&str], freed_detail: &str) -> String {
        match (completed.is_empty(), errors.is_empty()) {
            (true, true) => "未执行清理".into(),
            (true, false) => format!("清理失败：{}", errors.join("、")),
            (false, true) => {
                if freed_detail.is_empty() {
                    format!("清理完成（{} 项）", completed.len())
                } else {
                    format!("清理完成 · {freed_detail}")
                }
            }
            (false, false) => format!("完成 {} 项，失败：{}", completed.len(), errors.join("、")),
        }
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
                let freed_detail = Self::format_freed_message(avail_before, avail_after);
                app.optimize_step.clear();
                app.is_optimizing = false;
                app.optimize_percent = 0.0;
                app.optimize_status =
                    Self::build_result_message(&completed, &errors, &freed_detail);
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
}

#[derive(Clone, Copy)]
enum RingKind {
    Physical,
    Virtual,
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
        let physical_ring = self.physical_ring;
        let virtual_ring = self.virtual_ring;
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
                        physical_ring,
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
                                    virtual_ring,
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
            .overflow_hidden()
            .child(
                v_flex()
                    .w_full()
                    .h_full()
                    .overflow_hidden()
                    .bg(bg)
                    .child(render_title_bar(self, window, cx))
                    .child(
                        v_flex()
                            .w_full()
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            .child({
                                let mut body = v_flex()
                                    .w_full()
                                    .px(px(CONTENT_PADDING))
                                    .pt(px(CONTENT_PADDING))
                                    .gap(px(SECTION_GAP))
                                    .child(memory_row)
                                    .child(render_settings_content(self, cx));
                                if self.settings_expanded {
                                    body = body.flex_shrink_0();
                                } else {
                                    body = body.flex_1().min_h_0();
                                }
                                body
                            })
                            .child(
                                div()
                                    .w_full()
                                    .flex_shrink_0()
                                    .px(px(CONTENT_PADDING))
                                    .pb(px(CONTENT_PADDING))
                                    .pt(px(SECTION_GAP))
                                    .child(render_cleanup_footer(self, cx)),
                            ),
                    ),
            )
            .children(gpui_component::Root::render_dialog_layer(window, cx))
    }
}
