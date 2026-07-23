use gpui::*;
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
};
use rust_i18n::t;

use crate::app::AppEntityHolder;
use crate::clipboard::ClipboardItem;
use crate::ui::clipboard_item_card::{DRAG_CARD_WIDTH, ITEM_HEIGHT, render_card_content};

/// Pinned card chrome: card body + padding.
pub const PINNED_WINDOW_WIDTH: f32 = DRAG_CARD_WIDTH + 16.;
pub const PINNED_WINDOW_HEIGHT: f32 = ITEM_HEIGHT + 16.;

pub struct PinnedCardWindow {
    item: ClipboardItem,
}

impl PinnedCardWindow {
    pub fn new(item: ClipboardItem) -> Self {
        Self { item }
    }

    fn paste(&self, cx: &mut Context<Self>) {
        let item_id = self.item.id;
        let app = cx.global::<AppEntityHolder>().0.clone();
        app.update(cx, |app, cx| app.paste_clipboard_item(item_id, cx));
    }

    fn close(&self, window: &mut Window, cx: &mut Context<Self>) {
        let item_id = self.item.id;
        let app = cx.global::<AppEntityHolder>().0.clone();
        window.remove_window();
        app.update(cx, |app, cx| {
            app.pinned_card_handles.remove(&item_id);
            cx.notify();
        });
    }
}

impl Render for PinnedCardWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let item_id = self.item.id;
        let danger = theme.danger;
        let border = theme.border;
        let bg = theme.background;

        div()
            .size_full()
            .bg(bg)
            .p_2()
            .child(
                div()
                    .id(("pinned-card", item_id as u32))
                    .relative()
                    .w_full()
                    .h(px(ITEM_HEIGHT))
                    .overflow_hidden()
                    .bg(bg)
                    .border_1()
                    .border_color(border)
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|style| style.border_color(theme.primary.opacity(0.55)))
                    .on_click(cx.listener(|this, _, _, cx| this.paste(cx)))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .px_2()
                            .py_2()
                            .overflow_hidden()
                            .child(render_card_content(&self.item, cx)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(4.))
                            .right(px(4.))
                            .child(
                                Button::new(("pinned-close", item_id as u32))
                                    .ghost()
                                    .xsmall()
                                    .icon(
                                        Icon::new(IconName::CircleX)
                                            .xsmall()
                                            .text_color(danger),
                                    )
                                    .tooltip(t!("clipboard.unpin_tooltip").to_string())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.close(window, cx);
                                    })),
                            ),
                    ),
            )
    }
}

pub fn pinned_window_options(origin: Point<Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            origin,
            size(px(PINNED_WINDOW_WIDTH), px(PINNED_WINDOW_HEIGHT)),
        ))),
        is_resizable: false,
        focus: false,
        ..Default::default()
    }
}

pub fn pinned_window_origin(screen: Point<Pixels>) -> Point<Pixels> {
    point(
        screen.x - px(PINNED_WINDOW_WIDTH / 2.),
        screen.y - px(PINNED_WINDOW_HEIGHT / 2.),
    )
}

pub fn window_title_for_item(item: &ClipboardItem) -> SharedString {
    let title = item
        .preview
        .lines()
        .next()
        .unwrap_or(item.preview.as_str())
        .trim();
    let mut title: String = title.chars().take(48).collect();
    if item.preview.chars().count() > 48 {
        title.push('…');
    }
    if title.is_empty() {
        title = t!("clipboard.pinned_title").to_string();
    }
    title.into()
}
