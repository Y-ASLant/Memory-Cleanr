use gpui::*;
use gpui_component::{
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    label::Label,
    progress::ProgressCircle,
    switch::Switch,
    v_flex,
    ActiveTheme, Disableable, Icon, IconName, Sizable,
};

use crate::app::{MemoryCleanerApp, CONTENT_PADDING};
use crate::optimize::MemoryAreas;

const SECTION_GAP: f32 = 4.;
const ROW_GAP: f32 = 4.;

const CLEANUP_BUTTON_H: f32 = 48.;
const BUTTON_STATUS_TRUNCATE_CHARS: usize = 36;

fn panel_section_title(icon: IconName, label: &'static str) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .child(Icon::new(icon).small())
        .child(
            Label::new(label)
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD),
        )
}

fn memory_area_checkbox(
    id: &'static str,
    area: MemoryAreas,
    app: &MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let checked = app.settings.memory_areas().contains(area);
    let mut checkbox = Checkbox::new(id)
        .label(area.label())
        .text_sm()
        .checked(checked)
        .on_click(cx.listener(move |app, enabled, _, cx| {
            app.set_memory_area(area, *enabled, cx);
        }));

    if app.is_optimizing {
        checkbox = checkbox.disabled(true);
    }

    div().flex_1().min_w_0().child(checkbox)
}

fn cleanup_area_row(
    left: (&'static str, MemoryAreas),
    right: (&'static str, MemoryAreas),
    app: &MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .gap_4()
        .child(memory_area_checkbox(left.0, left.1, app, cx))
        .child(memory_area_checkbox(right.0, right.1, app, cx))
}

fn render_cleanup_areas(
    app: &MemoryCleanerApp,
    muted: Hsla,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap(px(ROW_GAP))
        .child(
            div()
                .w_full()
                .rounded(px(4.))
                .px_2()
                .py_1()
                .bg(muted.opacity(0.12))
                .child(
                    Label::new("提示：「待机列表」与「待机列表(低优先级)」只能勾选其一")
                        .text_xs()
                        .text_color(muted),
                ),
        )
        .child(cleanup_area_row(
            ("area-standby", MemoryAreas::STANDBY_LIST),
            ("area-standby-low", MemoryAreas::STANDBY_LIST_LOW_PRIORITY),
            app,
            cx,
        ))
        .child(cleanup_area_row(
            ("area-working-set", MemoryAreas::WORKING_SET),
            ("area-system-cache", MemoryAreas::SYSTEM_FILE_CACHE),
            app,
            cx,
        ))
        .child(cleanup_area_row(
            ("area-modified-page", MemoryAreas::MODIFIED_PAGE_LIST),
            ("area-combined", MemoryAreas::COMBINED_PAGE_LIST),
            app,
            cx,
        ))
        .child(cleanup_area_row(
            ("area-modified-file", MemoryAreas::MODIFIED_FILE_CACHE),
            ("area-registry", MemoryAreas::REGISTRY_CACHE),
            app,
            cx,
        ))
}

struct SwitchRowConfig {
    id: &'static str,
    icon: IconName,
    title: &'static str,
    description: &'static str,
    checked: bool,
}

fn switch_row(
    config: SwitchRowConfig,
    muted: Hsla,
    foreground: Hsla,
    cx: &mut Context<MemoryCleanerApp>,
    on_click: impl Fn(&mut MemoryCleanerApp, &bool, &mut Window, &mut Context<MemoryCleanerApp>) + 'static,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .gap_3()
        .py(px(3.))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .items_start()
                .gap_2()
                .child(
                    div()
                        .flex_shrink_0()
                        .pt(px(1.))
                        .child(Icon::new(config.icon).small().text_color(muted)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(1.))
                        .child(
                            Label::new(config.title)
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(foreground),
                        )
                        .child(
                            Label::new(config.description)
                                .text_xs()
                                .text_color(muted),
                        ),
                ),
        )
        .child(
            div()
                .flex_shrink_0()
                .child(
                    Switch::new(config.id)
                        .checked(config.checked)
                        .on_click(cx.listener(on_click)),
                ),
        )
}

fn render_window_behavior(
    app: &MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let settings = &app.settings;
    let muted = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;

    v_flex()
        .w_full()
        .gap(px(2.))
        .child(switch_row(
            SwitchRowConfig {
                id: "switch-always-on-top",
                icon: IconName::Star,
                title: "窗口置顶",
                description: "窗口始终保持在最前面",
                checked: settings.always_on_top,
            },
            muted,
            foreground,
            cx,
            |app, checked, window, cx| {
                app.set_always_on_top(*checked, window, cx);
            },
        ))
        .child(switch_row(
            SwitchRowConfig {
                id: "switch-close-to-tray",
                icon: IconName::Minimize,
                title: "关闭时隐藏到托盘",
                description: "点击关闭按钮时最小化到系统托盘",
                checked: settings.close_to_notification_area,
            },
            muted,
            foreground,
            cx,
            |app, checked, _, cx| {
                app.set_close_to_tray(*checked, cx);
            },
        ))
        .child(switch_row(
            SwitchRowConfig {
                id: "switch-start-minimized",
                icon: IconName::Settings,
                title: "启动时最小化",
                description: "启动后直接进入托盘，不显示主窗口",
                checked: settings.start_minimized,
            },
            muted,
            foreground,
            cx,
            |app, checked, _, cx| {
                app.set_start_minimized(*checked, cx);
            },
        ))
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max_chars).collect();
    format!("{truncated}…")
}

fn status_line_text(app: &MemoryCleanerApp) -> Option<String> {
    if app.is_optimizing {
        return Some(if app.optimize_step.is_empty() {
            "正在准备清理…".into()
        } else {
            app.optimize_step.clone()
        });
    }

    if !app.optimize_status.is_empty() {
        return Some(truncate_chars(
            &app.optimize_status,
            BUTTON_STATUS_TRUNCATE_CHARS,
        ));
    }

    None
}

fn cleanup_button_text_color(app: &MemoryCleanerApp, cx: &App) -> Hsla {
    let theme = cx.theme();
    if app.settings.memory_areas().is_empty() {
        return theme.muted_foreground.opacity(0.5);
    }
    if app.optimize_status.contains("失败") {
        return theme.danger;
    }
    theme.button_primary_foreground
}

fn split_result_status(status: &str) -> (String, Option<String>) {
    if let Some(pos) = status.rfind('。') {
        let after = pos + '。'.len_utf8();
        let summary = status[..after].trim().to_string();
        let detail = status.get(after..).unwrap_or("").trim();
        if detail.is_empty() {
            (summary, None)
        } else {
            (summary, Some(detail.to_string()))
        }
    } else {
        (truncate_chars(status, BUTTON_STATUS_TRUNCATE_CHARS), None)
    }
}

fn render_cleanup_button_content(
    app: &MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let color = cleanup_button_text_color(app, cx);
    let detail_color = cx.theme().button_primary_foreground.opacity(0.75);

    if app.is_optimizing {
        let line = status_line_text(app).unwrap_or_else(|| "正在准备清理…".into());
        return h_flex()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                ProgressCircle::new("inline-optimize-progress").value(app.optimize_percent),
            )
            .child(
                Label::new(line)
                    .text_sm()
                    .text_color(color)
                    .truncate(),
            )
            .into_any_element();
    }

    if !app.optimize_status.is_empty() {
        let (summary, detail) = split_result_status(&app.optimize_status);
        if let Some(detail) = detail {
            return h_flex()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    Label::new(summary)
                        .text_sm()
                        .text_color(color)
                        .truncate(),
                )
                .child(
                    Label::new(detail)
                        .text_xs()
                        .text_color(detail_color)
                        .truncate(),
                )
                .into_any_element();
        }
        return Label::new(summary)
            .text_sm()
            .text_color(color)
            .truncate()
            .into_any_element();
    }

    Label::new("一键清理")
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .into_any_element()
}

fn render_action_footer(
    app: &MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let areas_empty = app.settings.memory_areas().is_empty();
    let mut button = Button::new("inline-optimize")
        .primary()
        .w_full()
        .flex_shrink_0()
        .h(px(CLEANUP_BUTTON_H))
        .disabled(areas_empty || app.is_optimizing)
        .child(render_cleanup_button_content(app, cx))
        .on_click(cx.listener(|app, _, _, cx| {
            app.run_optimize(cx);
        }));

    button = if areas_empty {
        button.tooltip("请先选择清理区域")
    } else if app.is_optimizing {
        let tip = status_line_text(app).unwrap_or_else(|| "正在清理内存".into());
        button.tooltip(tip)
    } else if app.optimize_status.is_empty() {
        button.tooltip("开始清理内存")
    } else {
        button
    };

    div()
        .id("cleanup-footer")
        .w_full()
        .flex_shrink_0()
        .child(button)
}

fn render_settings_details(
    app: &MemoryCleanerApp,
    border: Hsla,
    radius: Pixels,
    muted: Hsla,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    div()
        .id("settings-details-panel")
        .w_full()
        .flex_1()
        .min_h_0()
        .rounded(radius)
        .border_1()
        .border_color(border)
        .overflow_hidden()
        .child(
            div()
                .id("settings-details-scroll")
                .w_full()
                .h_full()
                .overflow_y_scroll()
                .child(
                    v_flex()
                        .w_full()
                        .p(px(CONTENT_PADDING))
                        .gap(px(SECTION_GAP))
                        .child(panel_section_title(IconName::Settings2, "窗口行为"))
                        .child(render_window_behavior(app, cx))
                        .child(div().w_full().h(px(1.)).bg(border))
                        .child(panel_section_title(IconName::Settings, "清理区域"))
                        .child(render_cleanup_areas(app, muted, cx)),
                ),
        )
}

pub fn render_settings_bottom(
    app: &MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let theme = cx.theme();
    let border = theme.border;
    let radius = theme.radius;
    let muted = theme.muted_foreground;

    let mut column = v_flex()
        .id("settings-bottom-column")
        .w_full()
        .flex_1()
        .min_h_0();

    if app.settings_expanded {
        column = column.child(render_settings_details(app, border, radius, muted, cx));
    } else {
        column = column.child(div().flex_1().min_h_0());
    }

    column.child(render_action_footer(app, cx))
}
