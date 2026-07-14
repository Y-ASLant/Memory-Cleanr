use rust_i18n::t;

use gpui::*;
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, TITLE_BAR_HEIGHT,
    button::{Button, ButtonRounded, ButtonVariants},
    h_flex,
    label::Label,
};

use crate::app::MemoryCleanerApp;
use crate::version::APP_NAME;
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

impl TitleBarActionColors {
    fn from_theme(cx: &App, danger: bool) -> Self {
        if danger {
            Self {
                foreground: cx.theme().foreground,
                hover_fg: cx.theme().danger_foreground,
                hover_bg: cx.theme().danger,
                active_bg: cx.theme().danger_active,
            }
        } else {
            Self {
                foreground: cx.theme().foreground,
                hover_fg: cx.theme().secondary_foreground,
                hover_bg: cx.theme().secondary_hover,
                active_bg: cx.theme().secondary_active,
            }
        }
    }
}

fn title_bar_control(
    id: &'static str,
    icon: IconName,
    area: WindowControlArea,
    cx: &App,
    danger: bool,
) -> impl IntoElement {
    let colors = TitleBarActionColors::from_theme(cx, danger);

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
        .window_control_area(area)
        .child(Icon::new(icon).small())
}

fn title_bar_action_control(
    id: &'static str,
    icon: IconName,
    colors: TitleBarActionColors,
    disabled: bool,
    app_cx: &mut Context<MemoryCleanerApp>,
    on_click: impl Fn(&mut MemoryCleanerApp, &mut Window, &mut Context<MemoryCleanerApp>) + 'static,
) -> impl IntoElement {
    let mut control = div()
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
            if disabled {
                return;
            }
            cx.stop_propagation();
            on_click(app, window, cx);
        }))
        .child(Icon::new(icon).small());

    if disabled {
        control = control.opacity(0.45).cursor_not_allowed();
    }

    control
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
        false,
        app_cx,
        |app, window, cx| app.toggle_settings_expanded(window, cx),
    )
}

fn icon_cache_tooltip(app: &MemoryCleanerApp) -> SharedString {
    if app.is_refreshing_icon_cache {
        t!("icon_cache.refreshing").to_string().into()
    } else if app.icon_cache_status.is_empty() {
        t!("icon_cache.tooltip").to_string().into()
    } else {
        app.icon_cache_status.clone().into()
    }
}

fn icon_cache_control(
    app: &MemoryCleanerApp,
    colors: TitleBarActionColors,
    app_cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    Button::new("titlebar-refresh-icon-cache")
        .ghost()
        .rounded(ButtonRounded::None)
        .w(TITLE_BAR_HEIGHT)
        .h(TITLE_BAR_HEIGHT)
        .flex_shrink_0()
        .disabled(app.is_busy())
        .tooltip(icon_cache_tooltip(app))
        .on_click(app_cx.listener(|app, _, window, cx| {
            app.open_icon_cache_confirm_dialog(window, cx);
        }))
        .child(
            div()
                .flex()
                .justify_center()
                .content_center()
                .text_color(colors.foreground)
                .child(Icon::new(IconName::GalleryVerticalEnd).small()),
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
        false,
        app_cx,
        |app, window, cx| app.open_window_behavior_dialog(window, cx),
    )
}

fn title_bar_drag_area(
    window: &mut Window,
    state: &Entity<TitleBarDragState>,
    foreground: Hsla,
) -> impl IntoElement {
    h_flex()
        .id("bar")
        .h_full()
        .flex_shrink_0()
        .flex_1()
        .items_center()
        .gap_2()
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down_out(window.listener_for(state, |state, _, _, _| {
            state.should_move = false;
        }))
        .on_mouse_down(
            MouseButton::Left,
            window.listener_for(state, |state, _, _, _| {
                state.should_move = true;
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            window.listener_for(state, |state, _, _, _| {
                state.should_move = false;
            }),
        )
        .on_mouse_move(window.listener_for(state, |state, _, window, _| {
            if state.should_move {
                state.should_move = false;
                window.start_window_move();
            }
        }))
        .child(Icon::new(IconName::MemoryStick).small())
        .child(
            Label::new(APP_NAME)
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(foreground),
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
    let action_colors = TitleBarActionColors::from_theme(cx, false);

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
            .child(title_bar_drag_area(
                window,
                &state,
                action_colors.foreground,
            ))
            .child({
                let mut actions = h_flex()
                    .items_center()
                    .flex_shrink_0()
                    .h_full()
                    .child(icon_cache_control(app, action_colors, cx));
                if app.settings_expanded {
                    actions = actions.child(window_settings_control(action_colors, cx));
                }
                actions
                    .child(expand_toggle_control(app, action_colors, cx))
                    .child(window_controls(cx))
            }),
    )
}
