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

use crate::app::MemoryCleanerApp;
use crate::optimize::MemoryAreas;

const BOTTOM_COLUMN_GAP: f32 = 10.;
const BOTTOM_INSET: f32 = 10.;

const PANEL_HEADER_PY: f32 = 6.;
/// 设置面板整体额外高度（分摊到主体区上下内边距，各 +2px）。
const SETTINGS_PANEL_EXTRA_V: f32 = 4.;
const PANEL_BODY_PT: f32 = 4. + SETTINGS_PANEL_EXTRA_V / 2.;
const PANEL_BODY_PB: f32 = SETTINGS_PANEL_EXTRA_V / 2.;
const PANEL_TITLE_ROW_H: f32 = 18.;
const CLEANUP_BUTTON_PT: f32 = 4.;
const CLEANUP_BUTTON_PB: f32 = 6.;

fn section_title(icon: IconName, label: &'static str) -> impl IntoElement {
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

fn column_divider(border: Hsla) -> Div {
    div()
        .w(px(1.))
        .flex_shrink_0()
        .bg(border)
}

fn memory_area_checkbox(
    id: &'static str,
    area: MemoryAreas,
    app: &MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let checked = app.settings.memory_areas().contains(area);
    Checkbox::new(id)
        .label(area.label())
        .text_sm()
        .py_1()
        .checked(checked)
        .on_click(cx.listener(move |app, enabled, _, cx| {
            app.set_memory_area(area, *enabled, cx);
        }))
}

fn cleanup_areas_grid(
    app: &MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .gap_4()
        .child(
            v_flex()
                .flex_1()
                .gap_1()
                .child(memory_area_checkbox(
                    "inline-standby-normal",
                    MemoryAreas::STANDBY_LIST,
                    app,
                    cx,
                ))
                .child(memory_area_checkbox(
                    "inline-standby-low",
                    MemoryAreas::STANDBY_LIST_LOW_PRIORITY,
                    app,
                    cx,
                ))
                .child(memory_area_checkbox(
                    "inline-area-working-set",
                    MemoryAreas::WORKING_SET,
                    app,
                    cx,
                ))
                .child(memory_area_checkbox(
                    "inline-area-system-file-cache",
                    MemoryAreas::SYSTEM_FILE_CACHE,
                    app,
                    cx,
                )),
        )
        .child(
            v_flex()
                .flex_1()
                .gap_1()
                .child(memory_area_checkbox(
                    "inline-area-combined",
                    MemoryAreas::COMBINED_PAGE_LIST,
                    app,
                    cx,
                ))
                .child(memory_area_checkbox(
                    "inline-area-modified-file",
                    MemoryAreas::MODIFIED_FILE_CACHE,
                    app,
                    cx,
                ))
                .child(memory_area_checkbox(
                    "inline-area-modified-page",
                    MemoryAreas::MODIFIED_PAGE_LIST,
                    app,
                    cx,
                ))
                .child(memory_area_checkbox(
                    "inline-area-registry-cache",
                    MemoryAreas::REGISTRY_CACHE,
                    app,
                    cx,
                )),
        )
}

fn switch_row(
    id: &'static str,
    icon: IconName,
    title: &'static str,
    description: &'static str,
    checked: bool,
    muted_foreground: Hsla,
    foreground: Hsla,
    cx: &mut Context<MemoryCleanerApp>,
    on_click: impl Fn(&mut MemoryCleanerApp, &bool, &mut Window, &mut Context<MemoryCleanerApp>) + 'static,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .py_1()
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .child(
                    Icon::new(icon)
                        .small()
                        .text_color(muted_foreground),
                )
                .child(
                    Label::new(title)
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(foreground),
                ),
        )
        .child(
            Switch::new(id)
                .checked(checked)
                .tooltip(description)
                .on_click(cx.listener(on_click)),
        )
}

fn render_settings_options_content(
    app: &MemoryCleanerApp,
    muted_foreground: Hsla,
    foreground: Hsla,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let settings = &app.settings;

    v_flex()
        .w_full()
        .h_full()
        .justify_between()
        .child(switch_row(
            "inline-always-on-top",
            IconName::Star,
            "窗口置顶",
            "窗口始终保持在最前面",
            settings.always_on_top,
            muted_foreground,
            foreground,
            cx,
            |app, checked, window, cx| {
                app.set_always_on_top(*checked, window, cx);
            },
        ))
        .child(switch_row(
            "inline-close-to-tray",
            IconName::Minimize,
            "关闭时隐藏到托盘",
            "关闭窗口时最小化到系统托盘",
            settings.close_to_notification_area,
            muted_foreground,
            foreground,
            cx,
            |app, checked, _, cx| {
                app.set_close_to_tray(*checked, cx);
            },
        ))
        .child(switch_row(
            "inline-start-minimized",
            IconName::Settings,
            "启动时最小化",
            "程序启动时自动最小化到托盘",
            settings.start_minimized,
            muted_foreground,
            foreground,
            cx,
            |app, checked, _, cx| {
                app.set_start_minimized(*checked, cx);
            },
        ))
}

fn cleanup_button_label(app: &MemoryCleanerApp) -> String {
    if app.is_optimizing {
        if app.optimize_step.is_empty() {
            "清理中...".into()
        } else {
            app.optimize_step.clone()
        }
    } else if app.optimize_percent >= 100.0 && !app.optimize_step.is_empty() {
        app.optimize_step.clone()
    } else {
        "一键清理".into()
    }
}

fn render_cleanup_button(
    app: &MemoryCleanerApp,
    border: Hsla,
    inset: Pixels,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let areas_empty = app.settings.memory_areas().is_empty();
    let show_progress = app.is_optimizing
        || (app.optimize_percent >= 100.0 && !app.optimize_step.is_empty());

    let mut button = Button::new("inline-optimize")
        .label(cleanup_button_label(app))
        .primary()
        .w_full()
        .px_8()
        .disabled(areas_empty || app.is_optimizing)
        .tooltip(if areas_empty {
            "请先选择清理区域"
        } else if app.is_optimizing {
            "正在清理内存"
        } else {
            "开始清理内存"
        });

    if show_progress {
        button = button.icon(
            ProgressCircle::new("inline-optimize-progress").value(app.optimize_percent),
        );
    }

    div()
        .w_full()
        .flex_shrink_0()
        .px(inset)
        .pb(px(CLEANUP_BUTTON_PB))
        .pt(px(CLEANUP_BUTTON_PT))
        .border_t_1()
        .border_color(border)
        .child(button.on_click(cx.listener(|app, _, _, cx| {
            app.run_optimize(cx);
        })))
}

pub fn render_settings_bottom(
    app: &MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let theme = cx.theme();
    let border = theme.border;
    let radius = theme.radius;
    let muted_foreground = theme.muted_foreground;
    let foreground = theme.foreground;
    let gap = px(BOTTOM_COLUMN_GAP);
    let inset = px(BOTTOM_INSET);

    // 不用 GroupBox：outline 内层固定 p_4 + gap_4，且易被父级 flex 撑高。
    div()
        .id("settings-bottom-panel")
        .w_full()
        .flex_1()
        .min_h_0()
        .rounded(radius)
        .border_1()
        .border_color(border)
        .overflow_hidden()
        .child(
            v_flex()
                .w_full()
                .h_full()
                .justify_start()
                .child(
                    h_flex()
                        .w_full()
                        .flex_shrink_0()
                        .items_center()
                        .gap(gap)
                        .px(inset)
                        .py(px(PANEL_HEADER_PY))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(section_title(IconName::Settings2, "设置选项")),
                        )
                        .child(column_divider(border).h(px(PANEL_TITLE_ROW_H)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(section_title(IconName::Settings, "清理区域")),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .flex_1()
                        .min_h_0()
                        .items_stretch()
                        .gap(gap)
                        .px(inset)
                        .pb(px(PANEL_BODY_PB))
                        .pt(px(PANEL_BODY_PT))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .child(render_settings_options_content(
                                    app,
                                    muted_foreground,
                                    foreground,
                                    cx,
                                )),
                        )
                        .child(column_divider(border).self_stretch())
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(cleanup_areas_grid(app, cx)),
                        ),
                )
                .child(render_cleanup_button(app, border, inset, cx)),
        )
}
