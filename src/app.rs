use std::time::{Duration, Instant};

use anyhow::Result;
use gpui::*;
use gpui_component::ActiveTheme;
use smol::Timer;

use crate::memory::{MemorySection, MemoryStatus};
use crate::optimize::{self, MemoryAreas};
use crate::settings::Settings;
use crate::tray::{poll_menu_events, poll_tray_click};
use crate::win32;

pub const TRAY_POLL: Duration = Duration::from_millis(200);

const WINDOW_WIDTH: f32 = 640.;
const WINDOW_HEIGHT: f32 = 460.;
const CONTENT_PADDING: f32 = 10.;
const CONTENT_PADDING_BOTTOM: f32 = 10.;
const SECTION_GAP: f32 = 8.;

pub fn window_size() -> Size<Pixels> {
    size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT))
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
    pub is_optimizing: bool,
    pub optimize_step: String,
    pub optimize_percent: f32,
    pub last_optimize_time: Option<Instant>,
}

impl MemoryCleanerApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = Settings::load();
        let (physical, virtual_mem) =
            query_sections(settings.show_virtual_memory).unwrap_or_else(|e| {
                crate::log_msg(&format!("[memory] initial query failed: {e}"));
                (MemorySection::unavailable("物理内存"), None)
            });
        let window_handle = window.window_handle();

        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, app| {
            weak.update(app, |this, _| {
                if this.settings.close_to_notification_area {
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
            is_optimizing: false,
            optimize_step: String::new(),
            optimize_percent: 0.0,
            last_optimize_time: None,
        };

        app.start_background_poll(cx);

        app
    }

    pub fn refresh_memory(&mut self) -> bool {
        let Ok((physical, virtual_mem)) = query_sections(self.settings.show_virtual_memory) else {
            return false;
        };
        let phys_changed = self.physical != physical;
        let virt_changed = self.virtual_mem != virtual_mem;

        if phys_changed {
            self.physical = physical;
        }
        if virt_changed {
            self.virtual_mem = virtual_mem;
        }

        phys_changed || virt_changed
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
        self.settings.save();
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
        self.settings.save();
        cx.notify();
    }

    pub fn set_close_to_tray(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.close_to_notification_area = enabled;
        self.settings.save();
        cx.notify();
    }

    pub fn set_start_minimized(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.start_minimized = enabled;
        self.settings.save();
        cx.notify();
    }

    pub fn handle_tray_action(&mut self, action: &str, cx: &mut Context<Self>) {
        match action {
            "optimize" => self.run_optimize(cx),
            "show" => self.activate_window(cx),
            "hide" => self.hide_to_tray(cx),
            "quit" => cx.quit(),
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
        const MEMORY_POLL_TICKS: u32 = 15;

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
                            if this.refresh_memory() {
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

    pub fn finish_optimize(&mut self, message: String, cx: &mut Context<Self>) {
        self.optimize_step = message;
        self.optimize_percent = 100.0;
        let _ = self.refresh_memory();
        cx.notify();
    }

    pub fn run_optimize(&mut self, cx: &mut Context<Self>) {
        if self.is_optimizing {
            return;
        }

        let areas = self.settings.memory_areas();
        let Ok(steps) = optimize::step_plan(areas) else {
            return;
        };
        if steps.is_empty() {
            return;
        }

        let total = steps.len() as i32;
        self.is_optimizing = true;
        self.optimize_step = "准备清理...".into();
        self.optimize_percent = 0.0;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let mut completed = Vec::new();
            let mut errors = Vec::new();
            for (index, (name, run)) in steps.into_iter().enumerate() {
                let _ = this.update(cx, |this, cx| {
                    this.optimize_step = format!("正在清理 {name}...");
                    this.optimize_percent = (index as f32 / total as f32) * 100.0;
                    cx.notify();
                });

                Timer::after(Duration::from_millis(60)).await;

                match smol::unblock(run).await {
                    Ok(()) => {
                        completed.push(name);
                    }
                    Err(e) => {
                        crate::log_msg(&format!("[optimize] {name}: {e}"));
                        errors.push(name);
                    }
                }

                let _ = this.update(cx, |this, cx| {
                    this.optimize_percent = ((index + 1) as f32 / total as f32) * 100.0;
                    let _ = this.refresh_memory();
                    cx.notify();
                });

                Timer::after(Duration::from_millis(100)).await;
            }

            let message = match (completed.is_empty(), errors.is_empty()) {
                (true, true) => "未清理任何区域".into(),
                (true, false) => format!("失败：{}（需要管理员权限）", errors.join("、")),
                (false, true) => format!("完成：{}", completed.join("、")),
                (false, false) => format!(
                    "完成：{}，失败：{}",
                    completed.join("、"),
                    errors.join("、")
                ),
            };

            let _ = this.update(cx, |this, cx| {
                this.finish_optimize(message, cx);
            });

            Timer::after(Duration::from_secs(3)).await;

            let _ = this.update(cx, |this, cx| {
                this.is_optimizing = false;
                this.optimize_step.clear();
                this.optimize_percent = 0.0;
                this.last_optimize_time = Some(Instant::now());
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for MemoryCleanerApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::ui::memory_card::render_memory_card;
        use crate::ui::settings_page::render_settings_bottom;
        use crate::ui::title_bar::render_title_bar;
        use gpui_component::{
            group_box::{GroupBox, GroupBoxVariants},
            h_flex, v_flex,
        };

        // Extract theme colors before mutable borrows
        let bg = {
            let theme = cx.theme();
            theme.background
        };

        let physical = self.physical.clone();
        let virtual_mem = self.virtual_mem.clone();

        // 根布局：纵向堆叠；窗口固定为 WINDOW_WIDTH × WINDOW_HEIGHT。
        div().relative().w_full().h_full().overflow_hidden().child(
            v_flex()
                .w_full()
                .h_full()
                .justify_start()
                .bg(bg)
                .child(render_title_bar(self, window, cx))
                .child(
                    v_flex()
                        .w_full()
                        .flex_1()
                        .min_h_0()
                        .justify_start()
                        .items_start()
                        .px(px(CONTENT_PADDING))
                        .pt(px(CONTENT_PADDING))
                        .pb(px(CONTENT_PADDING_BOTTOM))
                        .gap(px(SECTION_GAP))
                        .child(
                            h_flex()
                                .w_full()
                                .flex_shrink_0()
                                .gap(px(SECTION_GAP))
                                .child(
                                    div().flex_1().min_w_0().child(
                                        GroupBox::new()
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
                                                        &physical,
                                                        "physical-memory",
                                                        true,
                                                        window,
                                                        cx,
                                                    )),
                                            ),
                                    ),
                                )
                                .child(div().flex_1().min_w_0().child({
                                    let right = GroupBox::new()
                                        .id("virtual-memory-card")
                                        .outline()
                                        .w_full()
                                        .p_0();
                                    if let Some(virt) = virtual_mem.as_ref() {
                                        right.child(
                                            v_flex()
                                                .w_full()
                                                .items_center()
                                                .py(px(crate::ui::memory_card::MEMORY_CARD_PY))
                                                .child(render_memory_card(
                                                    virt,
                                                    "virtual-memory",
                                                    false,
                                                    window,
                                                    cx,
                                                )),
                                        )
                                    } else {
                                        right
                                    }
                                })),
                        )
                        .child(render_settings_bottom(self, cx)),
                ),
        )
    }
}
