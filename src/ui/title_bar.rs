use gpui::*;
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, TITLE_BAR_HEIGHT, h_flex, label::Label,
};

use crate::app::MemoryCleanerApp;

const APP_NAME: &str = "Memory Cleaner";
const TITLE_BAR_LEFT_PADDING: Pixels = px(12.);

struct TitleBarDragState {
    should_move: bool,
}

#[derive(Clone, Copy)]
struct TitleBarActionColors {
    foreground: Hsla,
    hover_fg: Hsla,
    hover_bg: Hsla,
    active_bg: Hsla,
}

fn title_bar_control(
    id: &'static str,
    icon: IconName,
    area: WindowControlArea,
    cx: &App,
    is_close: bool,
) -> impl IntoElement {
    let hover_fg = if is_close {
        cx.theme().danger_foreground
    } else {
        cx.theme().secondary_foreground
    };
    let hover_bg = if is_close {
        cx.theme().danger
    } else {
        cx.theme().secondary_hover
    };
    let active_bg = if is_close {
        cx.theme().danger_active
    } else {
        cx.theme().secondary_active
    };

    div()
        .id(id)
        .flex()
        .w(TITLE_BAR_HEIGHT)
        .h_full()
        .flex_shrink_0()
        .justify_center()
        .content_center()
        .items_center()
        .text_color(cx.theme().foreground)
        .hover(|style| style.bg(hover_bg).text_color(hover_fg))
        .active(|style| style.bg(active_bg).text_color(hover_fg))
        .window_control_area(area)
        .child(Icon::new(icon).small())
}

fn title_bar_action_control(
    id: &'static str,
    icon: IconName,
    colors: TitleBarActionColors,
    app_cx: &mut Context<MemoryCleanerApp>,
    on_click: impl Fn(&mut MemoryCleanerApp, &mut Window, &mut Context<MemoryCleanerApp>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .w(TITLE_BAR_HEIGHT)
        .h_full()
        .flex_shrink_0()
        .justify_center()
        .content_center()
        .items_center()
        .text_color(colors.foreground)
        .hover(|style| style.bg(colors.hover_bg).text_color(colors.hover_fg))
        .active(|style| style.bg(colors.active_bg).text_color(colors.hover_fg))
        .on_click(app_cx.listener(move |app, _, window, cx| {
            cx.stop_propagation();
            on_click(app, window, cx);
        }))
        .child(Icon::new(icon).small())
}

fn expand_toggle_control(
    app: &MemoryCleanerApp,
    colors: TitleBarActionColors,
    app_cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let icon = if app.settings_expanded {
        IconName::ChevronUp
    } else {
        IconName::ChevronDown
    };

    title_bar_action_control(
        "titlebar-expand-toggle",
        icon,
        colors,
        app_cx,
        |app, window, cx| app.toggle_settings_expanded(window, cx),
    )
}

fn window_settings_control(
    colors: TitleBarActionColors,
    app_cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    title_bar_action_control(
        "titlebar-window-settings",
        IconName::Settings2,
        colors,
        app_cx,
        |app, window, cx| app.open_window_behavior_dialog(window, cx),
    )
}

fn window_controls(cx: &App) -> impl IntoElement {
    h_flex()
        .id("window-controls")
        .items_center()
        .flex_shrink_0()
        .h_full()
        .child(title_bar_control(
            "minimize",
            IconName::WindowMinimize,
            WindowControlArea::Min,
            cx,
            false,
        ))
        .child(title_bar_control(
            "close",
            IconName::WindowClose,
            WindowControlArea::Close,
            cx,
            true,
        ))
}

pub fn render_title_bar(
    app: &MemoryCleanerApp,
    window: &mut Window,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let state = window.use_state(cx, |_, _| TitleBarDragState { should_move: false });
    let title_bar_border = cx.theme().title_bar_border;
    let title_bar_bg = cx.theme().title_bar;
    let foreground = cx.theme().foreground;
    let hover_fg = cx.theme().secondary_foreground;
    let hover_bg = cx.theme().secondary_hover;
    let active_bg = cx.theme().secondary_active;
    let action_colors = TitleBarActionColors {
        foreground,
        hover_fg,
        hover_bg,
        active_bg,
    };

    div().flex_shrink_0().child(
        div()
            .id("title-bar")
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(TITLE_BAR_HEIGHT)
            .pl(TITLE_BAR_LEFT_PADDING)
            .border_b_1()
            .border_color(title_bar_border)
            .bg(title_bar_bg)
            .on_mouse_down_out(window.listener_for(&state, |state, _, _, _| {
                state.should_move = false;
            }))
            .on_mouse_down(
                MouseButton::Left,
                window.listener_for(&state, |state, _, _, _| {
                    state.should_move = true;
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                window.listener_for(&state, |state, _, _, _| {
                    state.should_move = false;
                }),
            )
            .on_mouse_move(window.listener_for(&state, |state, _, window, _| {
                if state.should_move {
                    state.should_move = false;
                    window.start_window_move();
                }
            }))
            .child(
                h_flex()
                    .id("bar")
                    .h_full()
                    .justify_between()
                    .flex_shrink_0()
                    .flex_1()
                    .window_control_area(WindowControlArea::Drag)
                    .child(
                        h_flex()
                            .h_full()
                            .items_center()
                            .gap_2()
                            .child(Icon::new(IconName::MemoryStick).small())
                            .child(
                                Label::new(APP_NAME)
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(foreground),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .flex_shrink_0()
                    .h_full()
                    .child(window_settings_control(action_colors, cx))
                    .child(expand_toggle_control(app, action_colors, cx))
                    .child(window_controls(cx)),
            ),
    )
}
