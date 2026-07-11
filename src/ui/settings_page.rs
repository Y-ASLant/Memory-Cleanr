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

const SECTION_GAP: f32 = 6.;
const ROW_GAP: f32 = 6.;

const CLEANUP_BUTTON_H: f32 = 48.;
const BUTTON_STATUS_TRUNCATE_CHARS: usize = 24;

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

fn switch_row_app(
    config: SwitchRowConfig,
    muted: Hsla,
    foreground: Hsla,
    on_click: impl Fn(&bool, &mut Window, &mut App) + 'static,
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
                        .on_click(on_click),
                ),
        )
}

pub fn render_window_behavior_dialog(
    weak: WeakEntity<MemoryCleanerApp>,
    cx: &App,
) -> impl IntoElement {
    let app = weak.upgrade().expect("MemoryCleanerApp entity should exist");
    let settings = app.read(cx).settings.clone();
    let muted = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;

    v_flex()
        .w_full()
        .gap(px(2.))
        .child(switch_row_app(
            SwitchRowConfig {
                id: "dialog-switch-always-on-top",
                icon: IconName::Star,
                title: "窗口置顶",
                description: "窗口始终保持在最前面",
                checked: settings.always_on_top,
            },
            muted,
            foreground,
            {
                let weak = weak.clone();
                move |checked, window, cx| {
                    let _ = weak.update(cx, |app, cx| {
                        app.set_always_on_top(*checked, window, cx);
                    });
                }
            },
        ))
        .child(switch_row_app(
            SwitchRowConfig {
                id: "dialog-switch-close-to-tray",
                icon: IconName::Minimize,
                title: "关闭时隐藏到托盘",
                description: "点击关闭按钮时最小化到系统托盘",
                checked: settings.close_to_notification_area,
            },
            muted,
            foreground,
            {
                let weak = weak.clone();
                move |checked, _window, cx| {
                    let _ = weak.update(cx, |app, cx| {
                        app.set_close_to_tray(*checked, cx);
                    });
                }
            },
        ))
        .child(switch_row_app(
            SwitchRowConfig {
                id: "dialog-switch-start-minimized",
                icon: IconName::Settings,
                title: "启动时最小化",
                description: "启动后直接进入托盘，不显示主窗口",
                checked: settings.start_minimized,
            },
            muted,
            foreground,
            {
                let weak = weak.clone();
                move |checked, _window, cx| {
                    let _ = weak.update(cx, |app, cx| {
                        app.set_start_minimized(*checked, cx);
                    });
                }
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

fn cleanup_step_text(app: &MemoryCleanerApp) -> String {
    if app.optimize_step.is_empty() {
        "准备清理…".into()
    } else {
        app.optimize_step.clone()
    }
}

fn cleanup_result_text(app: &MemoryCleanerApp) -> String {
    truncate_chars(&app.optimize_status, BUTTON_STATUS_TRUNCATE_CHARS)
}

fn cleanup_button_is_danger(app: &MemoryCleanerApp) -> bool {
    !app.optimize_status.is_empty() && app.optimize_status.starts_with("清理失败")
}

fn cleanup_button_text_color(app: &MemoryCleanerApp, cx: &App) -> Hsla {
    let theme = cx.theme();
    if app.settings.memory_areas().is_empty() {
        return theme.muted_foreground.opacity(0.5);
    }
    if cleanup_button_is_danger(app) {
        return theme.danger_foreground;
    }
    theme.button_primary_foreground
}

fn render_cleanup_button_content(
    app: &MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let color = cleanup_button_text_color(app, cx);

    if app.is_optimizing {
        let line = truncate_chars(&cleanup_step_text(app), BUTTON_STATUS_TRUNCATE_CHARS);
        return h_flex()
            .w_full()
            .px_3()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                ProgressCircle::new("inline-optimize-progress").value(app.optimize_percent),
            )
            .child(
                Label::new(line)
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(color)
                    .truncate(),
            )
            .into_any_element();
    }

    if !app.optimize_status.is_empty() {
        return Label::new(cleanup_result_text(app))
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
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
        .w_full()
        .flex_shrink_0()
        .h(px(CLEANUP_BUTTON_H))
        .disabled(areas_empty)
        .child(render_cleanup_button_content(app, cx))
        .on_click(cx.listener(|app, _, _, cx| {
            app.run_optimize(cx);
        }));

    button = if cleanup_button_is_danger(app) {
        button.danger()
    } else {
        button.primary()
    };

    button = if areas_empty {
        button.tooltip("请先选择清理区域")
    } else if app.is_optimizing {
        button.tooltip(cleanup_step_text(app))
    } else if app.optimize_status.is_empty() {
        button.tooltip("开始清理内存")
    } else {
        button.tooltip(app.optimize_status.clone())
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
        .flex_shrink_0()
        .rounded(radius)
        .border_1()
        .border_color(border)
        .child(
            v_flex()
                .w_full()
                .p(px(CONTENT_PADDING))
                .gap(px(SECTION_GAP))
                .child(panel_section_title(IconName::Settings, "清理区域"))
                .child(render_cleanup_areas(app, muted, cx)),
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
        .min_h_0()
        .gap(px(SECTION_GAP));

    if app.settings_expanded {
        column = column.child(render_settings_details(app, border, radius, muted, cx));
    } else {
        column = column.child(div().flex_1().min_h_0());
    }

    column.child(render_action_footer(app, cx))
}
