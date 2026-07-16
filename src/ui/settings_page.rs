use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    group_box::{GroupBox, GroupBoxVariants},
    h_flex,
    kbd::Kbd,
    label::Label,
    menu::{DropdownMenu, PopupMenuItem},
    progress::ProgressCircle,
    scroll::ScrollableElement as _,
    switch::Switch,
    tag::Tag,
    v_flex,
};
use rust_i18n::t;

use crate::app::MemoryCleanerApp;
use crate::memory::MemoryStatus;
use crate::optimize::MemoryAreas;
use crate::ui::layout::{
    CLEANUP_BUTTON_H, EXCLUSION_LIST_PADDING, EXCLUSION_SELECTOR_H, EXCLUSION_TAG_GAP,
    MAIN_CONTENT_PADDING, MAIN_WINDOW_WIDTH, PROCESS_PICKER_MENU_MAX_H, SECTION_GAP,
    process_exclusion_list_max_height, process_exclusion_selector_width,
};
use crate::win32::hotkey::HotkeyBinding;
use crate::win32::process::ProcessPickerEntry;

const ROW_GAP: f32 = 6.;
const BUTTON_STATUS_TRUNCATE_CHARS: usize = 24;

fn process_picker_detail(entry: &ProcessPickerEntry) -> String {
    let memory = MemoryStatus::format_bytes(entry.working_set_bytes);
    if entry.instance_count > 1 {
        t!(
            "settings.process_exclusion_picker_detail",
            count = entry.instance_count,
            memory = memory
        )
        .to_string()
    } else {
        memory
    }
}

fn language_options() -> [(&'static str, String); 3] {
    [
        ("auto", t!("settings.language_auto").to_string()),
        ("zh-CN", t!("settings.language_zh").to_string()),
        ("en", t!("settings.language_en").to_string()),
    ]
}

fn panel_section_title(icon: IconName, label: String) -> impl IntoElement {
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
                .rounded(cx.theme().radius)
                .px_2()
                .py_1()
                .bg(muted.opacity(0.12))
                .child(
                    Label::new(t!("settings.cleanup_areas_hint").to_string())
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

fn render_process_exclusion_tag(
    index: usize,
    name: &str,
    app: &MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let name = name.to_string();
    let remove_id = SharedString::from(format!("process-exclusion-remove-{index}"));
    let muted = cx.theme().muted_foreground;

    Tag::secondary()
        .outline()
        .small()
        .rounded(cx.theme().radius)
        .child(h_flex().items_center().gap_1().child(name.clone()).child({
            let name = name.clone();
            let mut button = Button::new(remove_id)
                .ghost()
                .xsmall()
                .flex_shrink_0()
                .tooltip(t!("settings.process_exclusion_remove").to_string())
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.remove_excluded_process(&name, cx);
                }))
                .icon(Icon::new(IconName::CircleX).xsmall().text_color(muted));
            if app.is_optimizing {
                button = button.disabled(true);
            }
            button
        }))
}

fn render_process_exclusion_list(
    app: &MemoryCleanerApp,
    excluded: &[String],
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let list_height = process_exclusion_list_max_height();
    let border = cx.theme().border;
    let muted_fg = cx.theme().muted_foreground;
    let empty_fg = cx.theme().foreground.opacity(0.55);

    let content = if excluded.is_empty() {
        div()
            .w_full()
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                Label::new(t!("settings.process_exclusion_empty").to_string())
                    .text_sm()
                    .text_color(empty_fg),
            )
            .into_any_element()
    } else {
        excluded
            .iter()
            .enumerate()
            .fold(
                h_flex().w_full().flex_wrap().gap(px(EXCLUSION_TAG_GAP)),
                |tags, (index, name)| {
                    tags.child(render_process_exclusion_tag(index, name, app, cx))
                },
            )
            .into_any_element()
    };

    div()
        .id("process-exclusion-list")
        .w_full()
        .h(px(list_height))
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(border)
        .bg(muted_fg.opacity(0.06))
        .p(px(EXCLUSION_LIST_PADDING))
        .overflow_y_scrollbar()
        .child(content)
}

fn render_process_exclusion(
    app: &MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let weak = cx.weak_entity();
    let excluded = app.settings.excluded_processes.clone();
    let available = app.available_processes_for_exclusion();
    let pick = app.process_exclusion_pick.clone();
    let pick_label = pick
        .clone()
        .unwrap_or_else(|| t!("settings.process_exclusion_select").to_string());
    let can_add = pick.is_some() && !app.is_optimizing;
    let selector_h = px(EXCLUSION_SELECTOR_H);
    let selector_w = px(process_exclusion_selector_width(
        MAIN_WINDOW_WIDTH,
        MAIN_CONTENT_PADDING,
    ));

    v_flex()
        .w_full()
        .gap(px(crate::ui::layout::EXCLUSION_FOOTER_GAP))
        .child(render_process_exclusion_list(app, &excluded, cx))
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_3()
                .child(div().flex_1().min_w_0().w_full().child({
                    let weak = weak.clone();
                    let available = available.clone();
                    let pick = pick.clone();
                    Button::new("process-exclusion-select")
                        .outline()
                        .small()
                        .w_full()
                        .h(selector_h)
                        .when(app.is_optimizing, |this| this.disabled(true))
                        .label(pick_label)
                        .dropdown_caret(true)
                        .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _, _| {
                            let menu = available.iter().fold(menu, |menu, entry| {
                                let name = entry.name.clone();
                                let detail = process_picker_detail(entry);
                                let checked = pick.as_deref() == Some(name.as_str());
                                let weak = weak.clone();
                                let label = name.clone();
                                let detail_label = detail.clone();
                                menu.item(
                                    PopupMenuItem::element(move |_, cx| {
                                        h_flex()
                                            .w_full()
                                            .items_center()
                                            .justify_between()
                                            .gap_2()
                                            .child(Label::new(label.clone()).text_sm().truncate())
                                            .child(
                                                Label::new(detail_label.clone())
                                                    .text_sm()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .flex_shrink_0(),
                                            )
                                    })
                                    .checked(checked)
                                    .on_click(
                                        move |_, _, cx| {
                                            let _ = weak.update(cx, |app, cx| {
                                                app.set_process_exclusion_pick(
                                                    Some(name.clone()),
                                                    cx,
                                                );
                                            });
                                        },
                                    ),
                                )
                            });
                            menu.scrollable(true)
                                .max_h(px(PROCESS_PICKER_MENU_MAX_H))
                                .min_w(selector_w)
                                .max_w(selector_w)
                        })
                }))
                .child({
                    let mut button = Button::new("process-exclusion-add")
                        .outline()
                        .small()
                        .flex_shrink_0()
                        .w(selector_h)
                        .h(selector_h)
                        .tooltip(t!("settings.process_exclusion_add").to_string())
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.add_excluded_process(cx);
                        }))
                        .child(Label::new("+").text_sm().font_weight(FontWeight::SEMIBOLD));
                    if !can_add {
                        button = button.disabled(true);
                    }
                    button
                }),
        )
}

struct SwitchRowConfig {
    id: &'static str,
    icon: IconName,
    title: String,
    description: String,
    checked: bool,
}

fn switch_row_app(
    config: SwitchRowConfig,
    muted: Hsla,
    foreground: Hsla,
    on_click: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let icon = config.icon;

    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .gap_3()
        .py(px(3.))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.))
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .child(Icon::new(icon.clone()).small().text_color(muted)),
                        )
                        .child(
                            Label::new(config.title)
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(foreground),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .items_start()
                        .gap_2()
                        .child(
                            div()
                                .flex_shrink_0()
                                .invisible()
                                .flex()
                                .items_center()
                                .child(Icon::new(icon).small()),
                        )
                        .child(
                            Label::new(config.description)
                                .text_xs()
                                .text_color(muted)
                                .flex_1()
                                .min_w_0(),
                        ),
                ),
        )
        .child(
            div().flex_shrink_0().child(
                Switch::new(config.id)
                    .checked(config.checked)
                    .on_click(on_click),
            ),
        )
}

fn render_version_row(cx: &App) -> impl IntoElement {
    let link_color = cx.theme().primary;
    let version = format!("v{}", crate::version::VERSION);

    h_flex().w_full().justify_center().items_center().child(
        div()
            .id("version-link")
            .cursor_pointer()
            .on_click(|_, _, cx| cx.open_url(crate::version::REPO_URL))
            .child(
                Label::new(version)
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(link_color),
            ),
    )
}

fn render_language_selector(
    weak: &WeakEntity<MemoryCleanerApp>,
    muted: Hsla,
    foreground: Hsla,
    cx: &App,
) -> impl IntoElement {
    let current = {
        let app = weak.upgrade();
        app.as_ref()
            .map(|a| a.read(cx).settings.language.clone())
            .unwrap_or_else(|| "auto".into())
    };

    let options = language_options();
    let current_label = options
        .iter()
        .find(|(k, _)| *k == current.as_str())
        .map(|(_, l)| l.clone())
        .unwrap_or_else(|| t!("settings.language_auto").to_string());

    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .gap_3()
        .py(px(3.))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.))
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .child(Icon::new(IconName::Globe).small().text_color(muted)),
                        )
                        .child(
                            Label::new(t!("settings.language").to_string())
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(foreground),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .items_start()
                        .gap_2()
                        .child(
                            div()
                                .flex_shrink_0()
                                .invisible()
                                .flex()
                                .items_center()
                                .child(Icon::new(IconName::Globe).small()),
                        )
                        .child(
                            Label::new(t!("settings.language_desc").to_string())
                                .text_xs()
                                .text_color(muted)
                                .flex_1()
                                .min_w_0(),
                        ),
                ),
        )
        .child({
            let weak = weak.clone();
            Button::new("language-select")
                .ghost()
                .small()
                .min_w(px(128.))
                .label(current_label)
                .dropdown_caret(true)
                .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                    let weak = weak.clone();
                    let current = current.clone();
                    options.iter().fold(menu, |menu, (value, label)| {
                        let value = (*value).to_string();
                        let label = label.clone();
                        let checked = current == value;
                        let weak = weak.clone();
                        menu.item(PopupMenuItem::new(label).checked(checked).on_click(
                            move |_, _, cx| {
                                let _ = weak.update(cx, |app, cx| {
                                    if app.settings.language != value {
                                        app.settings.language = value.clone();
                                        app.apply_locale(cx);
                                    }
                                });
                            },
                        ))
                    })
                })
        })
}

fn cleanup_hotkey_display(
    recording: bool,
    chord: &str,
    border: Hsla,
    background: Hsla,
    foreground: Hsla,
    primary: Hsla,
    muted: Hsla,
) -> Div {
    if recording {
        div().child(
            Label::new(t!("settings.cleanup_hotkey_recording").to_string())
                .text_sm()
                .text_color(primary),
        )
    } else if let Some(keystroke) = HotkeyBinding::chord_to_keystroke(chord) {
        div().child(
            Kbd::new(keystroke)
                .bg(background)
                .border_color(border)
                .text_color(foreground),
        )
    } else {
        div().child(Label::new(chord.to_string()).text_sm().text_color(muted))
    }
}

fn render_cleanup_hotkey_row(
    weak: &WeakEntity<MemoryCleanerApp>,
    muted: Hsla,
    foreground: Hsla,
    cx: &App,
) -> impl IntoElement {
    let Some(app) = weak.upgrade() else {
        return div();
    };

    let app = app.read(cx);
    let enabled = app.settings.cleanup_hotkey_enabled;
    let recording = app.cleanup_hotkey_recording;
    let chord = app.settings.cleanup_hotkey.clone();
    let focus = app.hotkey_capture_focus.clone();
    let border = cx.theme().border;
    let background = cx.theme().background;
    let primary = cx.theme().primary;
    let radius = cx.theme().radius;

    let weak_switch = weak.clone();
    let weak_capture = weak.clone();
    let focus_capture = focus.clone();

    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .gap_3()
        .py(px(3.))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.))
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .child(Icon::new(IconName::ALargeSmall).small().text_color(muted)),
                        )
                        .child(
                            Label::new(t!("settings.cleanup_hotkey").to_string())
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(foreground),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .items_start()
                        .gap_2()
                        .child(
                            div()
                                .flex_shrink_0()
                                .invisible()
                                .flex()
                                .items_center()
                                .child(Icon::new(IconName::ALargeSmall).small()),
                        )
                        .child(
                            Label::new(t!("settings.cleanup_hotkey_desc").to_string())
                                .text_xs()
                                .text_color(muted)
                                .flex_1()
                                .min_w_0(),
                        ),
                ),
        )
        .child(
            h_flex()
                .flex_shrink_0()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .id("cleanup-hotkey-capture")
                        .track_focus(&focus)
                        .min_w(px(128.))
                        .h(px(28.))
                        .px_2()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(radius)
                        .border_1()
                        .border_color(if recording { primary } else { border })
                        .bg(background)
                        .when(enabled, |this| this.cursor_pointer())
                        .when(!enabled, |this| this.opacity(0.5))
                        .on_key_down({
                            let weak = weak_capture.clone();
                            move |event, _, cx| {
                                let _ = weak.update(cx, |app, cx| {
                                    app.handle_cleanup_hotkey_key(event, cx);
                                });
                            }
                        })
                        .on_click({
                            let weak = weak_capture;
                            move |_, window, cx| {
                                if !enabled {
                                    return;
                                }
                                let _ = weak.update(cx, |app, cx| {
                                    app.start_cleanup_hotkey_recording(window, cx);
                                });
                                window.focus(&focus_capture, cx);
                            }
                        })
                        .child(cleanup_hotkey_display(
                            recording, &chord, border, background, foreground, primary, muted,
                        )),
                )
                .child(
                    Switch::new("dialog-switch-cleanup-hotkey")
                        .checked(enabled)
                        .on_click({
                            let weak = weak_switch;
                            move |checked, _, cx| {
                                let _ = weak.update(cx, |app, cx| {
                                    app.set_cleanup_hotkey_enabled(*checked, cx);
                                });
                            }
                        }),
                ),
        )
}

pub fn render_window_behavior_dialog(
    weak: WeakEntity<MemoryCleanerApp>,
    cx: &App,
) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;

    let Some(app) = weak.upgrade() else {
        return v_flex()
            .w_full()
            .child(div().w_full().pt(px(4.)).child(render_version_row(cx)));
    };

    let settings = app.read(cx).settings.clone();

    v_flex()
        .w_full()
        .gap(px(2.))
        .child(render_language_selector(&weak, muted, foreground, cx))
        .child(render_cleanup_hotkey_row(&weak, muted, foreground, cx))
        .child(switch_row_app(
            SwitchRowConfig {
                id: "dialog-switch-always-on-top",
                icon: IconName::Star,
                title: t!("settings.always_on_top").to_string(),
                description: t!("settings.always_on_top_desc").to_string(),
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
                title: t!("settings.close_to_tray").to_string(),
                description: t!("settings.close_to_tray_desc").to_string(),
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
                id: "dialog-switch-optimization-notifications",
                icon: IconName::Bell,
                title: t!("settings.optimization_notifications").to_string(),
                description: t!("settings.optimization_notifications_desc").to_string(),
                checked: settings.show_optimization_notifications,
            },
            muted,
            foreground,
            {
                let weak = weak.clone();
                move |checked, _window, cx| {
                    let _ = weak.update(cx, |app, cx| {
                        app.set_show_optimization_notifications(*checked, cx);
                    });
                }
            },
        ))
        .child(switch_row_app(
            SwitchRowConfig {
                id: "dialog-switch-debug-logging",
                icon: IconName::Settings2,
                title: t!("settings.debug_logging").to_string(),
                description: t!("settings.debug_logging_desc").to_string(),
                checked: settings.debug_logging,
            },
            muted,
            foreground,
            {
                let weak = weak.clone();
                move |checked, _window, cx| {
                    let _ = weak.update(cx, |app, cx| {
                        app.set_debug_logging(*checked, cx);
                    });
                }
            },
        ))
        .child(
            div()
                .w_full()
                .mt(px(SECTION_GAP))
                .pt(px(SECTION_GAP))
                .border_t_1()
                .border_color(muted.opacity(0.25))
                .child(render_version_row(cx)),
        )
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
        t!("button.cleanup_preparing").to_string()
    } else {
        app.optimize_step.clone()
    }
}

fn cleanup_result_text(app: &MemoryCleanerApp) -> String {
    truncate_chars(&app.optimize_status, BUTTON_STATUS_TRUNCATE_CHARS)
}

fn cleanup_button_is_danger(app: &MemoryCleanerApp) -> bool {
    !app.optimize_status.is_empty() && app.optimize_has_errors
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
                ProgressCircle::new("inline-optimize-progress")
                    .color(color)
                    .small()
                    .value(app.optimize_percent),
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

    Label::new(t!("button.cleanup").to_string())
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .into_any_element()
}

pub fn render_cleanup_footer(
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

    if areas_empty {
        button.tooltip(t!("tooltip.select_areas").to_string())
    } else if app.is_optimizing {
        button.tooltip(cleanup_step_text(app))
    } else if app.optimize_status.is_empty() {
        button.tooltip(t!("tooltip.start_cleanup").to_string())
    } else {
        button.tooltip(app.optimize_status.clone())
    }
}

pub fn render_settings_content(
    app: &MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let theme = cx.theme();
    render_settings_details(app, theme.muted_foreground, cx)
}

fn compact_group_box_style() -> StyleRefinement {
    StyleRefinement::default().p_2().gap_2()
}

fn render_settings_card(
    id: &'static str,
    title_icon: IconName,
    title: String,
    content: impl IntoElement,
) -> impl IntoElement {
    GroupBox::new()
        .id(id)
        .outline()
        .w_full()
        .p_0()
        .content_style(compact_group_box_style())
        .child(
            v_flex()
                .w_full()
                .gap(px(4.))
                .child(panel_section_title(title_icon, title))
                .child(content),
        )
}

fn render_settings_details(
    app: &MemoryCleanerApp,
    muted: Hsla,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    v_flex()
        .id("settings-details-panel")
        .w_full()
        .flex_shrink_0()
        .gap(px(SECTION_GAP))
        .child(render_settings_card(
            "process-exclusion-card",
            IconName::CircleX,
            t!("settings.process_exclusion").to_string(),
            render_process_exclusion(app, cx),
        ))
        .child(render_settings_card(
            "cleanup-areas-card",
            IconName::Settings,
            t!("settings.cleanup_areas").to_string(),
            render_cleanup_areas(app, muted, cx),
        ))
}
