use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{
    ActiveTheme, Icon, IconName, Root, Sizable,
    button::{Button, ButtonVariants},
};
use rust_i18n::t;
use smol::Timer;

use crate::app::AppEntityHolder;
use crate::clipboard::ClipboardItem;
use crate::ui::clipboard_item_card::{
    DRAG_CARD_WIDTH, ITEM_HEIGHT, render_card_content, render_split_card, render_zone_overlay,
};
use crate::ui::clipboard_panel::CLIPBOARD_HOVER_ANIM_MS;

pub const PINNED_WINDOW_WIDTH: f32 = DRAG_CARD_WIDTH;

const PINNED_WINDOW_HEIGHT: f32 = ITEM_HEIGHT;
const HOVER_TICK: Duration = Duration::from_millis(8);

#[derive(Clone, Debug)]
struct ZoneFade {
    from: f32,
    to: f32,
    start: Instant,
}

pub struct PinnedCardWindow {
    item: ClipboardItem,
    hovered: bool,
    zone_fade: Option<ZoneFade>,
    zone_fade_tick_gen: u32,
}

impl PinnedCardWindow {
    pub fn new(item: ClipboardItem) -> Self {
        Self {
            item,
            hovered: false,
            zone_fade: None,
            zone_fade_tick_gen: 0,
        }
    }

    fn paste(&self, window: &mut Window, cx: &mut Context<Self>) {
        let item_id = self.item.id;
        let pinned = window.window_handle();
        let app = cx.global::<AppEntityHolder>().0.clone();
        app.update(cx, |app, cx| {
            app.paste_clipboard_item_from_pinned(item_id, pinned, cx)
        });
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

    fn set_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if self.hovered == hovered {
            return;
        }
        self.hovered = hovered;
        self.begin_zone_fade(cx);
    }

    fn sample_zone_opacity(&self, now: Instant) -> f32 {
        if let Some(anim) = &self.zone_fade {
            let elapsed = now.saturating_duration_since(anim.start);
            let duration = Duration::from_millis(CLIPBOARD_HOVER_ANIM_MS);
            if elapsed >= duration {
                return anim.to;
            }
            let t = elapsed.as_secs_f32() / duration.as_secs_f32();
            let eased = 1.0 - (1.0 - t) * (1.0 - t);
            return anim.from + (anim.to - anim.from) * eased;
        }
        if self.hovered { 1.0 } else { 0.0 }
    }

    fn begin_zone_fade(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let to = if self.hovered { 1.0 } else { 0.0 };
        let from = self.sample_zone_opacity(now);
        if (from - to).abs() < 0.001 {
            self.zone_fade = None;
            cx.notify();
            return;
        }
        self.zone_fade = Some(ZoneFade {
            from,
            to,
            start: now,
        });
        self.zone_fade_tick_gen = self.zone_fade_tick_gen.wrapping_add(1);
        let tick_gen = self.zone_fade_tick_gen;
        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(HOVER_TICK).await;
                let keep = this
                    .update(cx, |this, cx| {
                        if this.zone_fade_tick_gen != tick_gen {
                            return false;
                        }
                        let Some(anim) = &this.zone_fade else {
                            return false;
                        };
                        let elapsed = Instant::now().saturating_duration_since(anim.start);
                        if elapsed >= Duration::from_millis(CLIPBOARD_HOVER_ANIM_MS) {
                            this.zone_fade = None;
                            cx.notify();
                            return false;
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }
}

impl Render for PinnedCardWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let item_id = self.item.id;
        let danger = theme.danger;
        let zone_opacity = self.sample_zone_opacity(Instant::now());
        let show_close = zone_opacity > 0.01;
        let hover_border = theme.primary.opacity(0.55);

        div()
            .relative()
            .w_full()
            .h_full()
            .child(
                div()
                    .id(("pinned-card", item_id as u32))
                    .relative()
                    .w(px(PINNED_WINDOW_WIDTH))
                    .h(px(PINNED_WINDOW_HEIGHT))
                    .bg(theme.background)
                    .border_1()
                    .border_color(theme.border)
                    .rounded_md()
                    .overflow_hidden()
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        this.set_hovered(*hovered, cx);
                    }))
                    .hover(move |style| style.border_color(hover_border))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .opacity(zone_opacity)
                            .child(render_zone_overlay()),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .px_2()
                            .py_2()
                            .overflow_hidden()
                            .child(render_card_content(&self.item, cx)),
                    )
                    .child(render_split_card(
                        div()
                            .id(("pinned-drag", item_id as u32))
                            .size_full()
                            .cursor_grab()
                            .window_control_area(WindowControlArea::Drag),
                        div()
                            .id(("pinned-paste", item_id as u32))
                            .size_full()
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                this.paste(window, cx);
                            })),
                    ))
                    .when(show_close, |el| {
                        el.child(
                            div()
                                .id(("pinned-close-wrap", item_id as u32))
                                .absolute()
                                .top(px(4.))
                                .right(px(4.))
                                .opacity(zone_opacity)
                                .on_click(|_, _, cx| cx.stop_propagation())
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
                        )
                    }),
            )
            .children(Root::render_dialog_layer(window, cx))
    }
}

pub fn pinned_window_options(origin: Point<Pixels>) -> WindowOptions {
    WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            origin,
            size(px(PINNED_WINDOW_WIDTH), px(PINNED_WINDOW_HEIGHT)),
        ))),
        kind: WindowKind::PopUp,
        focus: false,
        is_resizable: false,
        is_movable: true,
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
