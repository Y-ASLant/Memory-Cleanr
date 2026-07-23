use std::ops::Range;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{ActiveTheme, h_flex, label::Label, v_flex};
use rust_i18n::t;
use smol::Timer;

use crate::app::{ClipboardFilterSlide, ClipboardShiftAnim, MemoryCleanerApp};
use crate::clipboard::ContentType;
use crate::ui::clipboard_item_card::{
    DragClipboardItem, ITEM_HEIGHT, render_clipboard_item,
};

/// Clipboard-only window height (width matches the main 520px window).
pub const CLIPBOARD_WINDOW_HEIGHT: f32 = 600.;
/// Top toolbar: sliding segment track (ElegantClipboard-style).
const TOOLBAR_H: f32 = 44.;
/// Status bar height.
const STATUS_BAR_H: f32 = 28.;
/// Vertical gap between cards.
pub const ITEM_GAP: f32 = 4.;
/// Row height including gap (uniform_list measures one row).
pub const ROW_HEIGHT: f32 = ITEM_HEIGHT + ITEM_GAP;
/// Match ElegantClipboard / dnd-kit sortable transition.
const SHIFT_DURATION: Duration = Duration::from_millis(120);
const SHIFT_TICK: Duration = Duration::from_millis(8);
/// Exit fade + sibling collapse before the row is removed from data.
pub const DELETE_ANIM_MS: u64 = 160;
/// Segment indicator slide (ElegantClipboard `duration-200 ease-out`).
const FILTER_SLIDE_DURATION: Duration = Duration::from_millis(200);
/// Card hover reveal: zone tint, labels, delete affordance (same duration/easing as filter slide).
pub const CLIPBOARD_HOVER_ANIM_MS: u64 = 200;
const FILTER_SEGMENT_COUNT: f32 = 3.;

/// Render the clipboard panel (full window content when clipboard mode is active).
pub fn render_clipboard_panel(
    app: &MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let border = theme.border;
    let total = app.clipboard_items.len();
    let active_filter = app.clipboard_filter;
    let entity = cx.entity().clone();
    let scroll = app.clipboard_list_scroll.clone();
    let is_dragging = app.clipboard_dragging_id.is_some();

    v_flex()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .w_full()
        .h_full()
        .child(render_clipboard_toolbar(app, active_filter, cx))
        .child({
            if total == 0 {
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .child(
                        Label::new("暂无剪贴板记录".to_string())
                            .text_sm()
                            .text_color(muted),
                    )
                    .into_any_element()
            } else {
                div()
                    .id("clipboard-item-list")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .px_2()
                    .py_1()
                    .when(is_dragging, |el| el.cursor_grabbing())
                    .on_drag_move(cx.listener(|app, e: &DragMoveEvent<DragClipboardItem>, window, cx| {
                        update_drag_tearoff(app, e, window, cx);
                        if !app.clipboard_drag_tearoff {
                            update_drop_target_from_pointer(app, e, cx);
                        }
                    }))
                    .on_drop(cx.listener(|app, drag: &DragClipboardItem, _, cx| {
                        if app.clipboard_drag_tearoff {
                            return;
                        }
                        let target = app.clipboard_drop_target_id;
                        if let Some(to) = target
                            && drag.id != to
                        {
                            app.move_clipboard_item(drag.id, to, cx);
                        } else {
                            app.clear_clipboard_drag_preview();
                            cx.notify();
                        }
                    }))
                    .child(
                        // Keep original order while dragging (dnd-kit model). Only commit
                        // arrayMove on drop — never preview-reorder the list.
                        uniform_list("clipboard-virtual-list", total, {
                            let entity = entity.clone();
                            move |range: Range<usize>, _window, cx| {
                                entity.update(cx, |app, cx| render_visible_rows(app, range, cx))
                            }
                        })
                        .track_scroll(&scroll)
                        .with_sizing_behavior(ListSizingBehavior::Auto)
                        .flex_1()
                        .size_full(),
                    )
                    .into_any_element()
            }
        })
        .child(
            h_flex()
                .w_full()
                .h(px(STATUS_BAR_H))
                .flex_shrink_0()
                .px_3()
                .border_t_1()
                .border_color(border)
                .items_center()
                .justify_between()
                .child(
                    Label::new(format!("共 {total} 条"))
                        .text_xs()
                        .text_color(muted),
                )
                .child(
                    Label::new(t!("clipboard.status_hint").to_string())
                        .text_xs()
                        .text_color(muted),
                ),
        )
}

fn render_visible_rows(
    app: &mut MemoryCleanerApp,
    range: Range<usize>,
    cx: &mut Context<MemoryCleanerApp>,
) -> Vec<AnyElement> {
    let now = Instant::now();

    range
        .filter_map(|idx| {
            let item = app.clipboard_items.get(idx)?;
            let id = item.id;
            let selected = app.clipboard_selected == Some(idx);
            // Sample the in-flight 120ms ease-out tween (not the raw target).
            let shift_y = sample_shift_y(app, id, now);

            let card = render_clipboard_item(item, idx, selected, app, cx);

            Some(
                // No overflow clip: siblings must paint into neighbors while translating.
                div()
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .relative()
                    .child(
                        div()
                            .absolute()
                            .w_full()
                            .h(px(ITEM_HEIGHT))
                            .top(px(shift_y))
                            .child(card),
                    )
                    .into_any_element(),
            )
        })
        .collect()
}

/// Same geometry as `@dnd-kit` `verticalListSortingStrategy` for equal-height rows.
pub(crate) fn sortable_shift_y(index: usize, active: usize, over: usize) -> f32 {
    if active == over {
        return 0.;
    }
    if active < over {
        // Moving down: items (active, over] shift up to open a hole at `over`.
        if index > active && index <= over {
            return -ROW_HEIGHT;
        }
    } else if index >= over && index < active {
        // Moving up: items [over, active) shift down.
        return ROW_HEIGHT;
    }
    0.
}

fn ease_out_quad(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    1.0 - (1.0 - t) * (1.0 - t)
}

/// Sample hover-reveal opacity for zone tint, labels, and delete affordance.
pub fn sample_clipboard_hover_opacity(app: &MemoryCleanerApp, id: i64, now: Instant) -> f32 {
    if let Some(anim) = app.clipboard_hover_fades.get(&id) {
        let elapsed = now.saturating_duration_since(anim.start);
        let t = elapsed.as_secs_f32() / (CLIPBOARD_HOVER_ANIM_MS as f32 / 1000.);
        if t >= 1. {
            return anim.to;
        }
        return anim.from + (anim.to - anim.from) * ease_out_quad(t);
    }
    if app.clipboard_hovered_id == Some(id) {
        1.0
    } else {
        0.0
    }
}

/// Retarget a card's hover-reveal tween when pointer enters/leaves.
pub fn begin_clipboard_hover_fade(
    app: &mut MemoryCleanerApp,
    id: i64,
    cx: &mut Context<MemoryCleanerApp>,
) {
    let now = Instant::now();
    let to = if app.clipboard_hovered_id == Some(id) {
        1.0
    } else {
        0.0
    };
    let from = sample_clipboard_hover_opacity(app, id, now);
    if (from - to).abs() < 0.001 {
        app.clipboard_hover_fades.remove(&id);
        return;
    }
    app.clipboard_hover_fades.insert(
        id,
        crate::app::ClipboardHoverFade {
            from,
            to,
            start: now,
        },
    );
    start_clipboard_hover_fade_ticker(app, cx);
    cx.notify();
}

fn start_clipboard_hover_fade_ticker(
    app: &mut MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) {
    app.clipboard_hover_fade_tick_gen = app.clipboard_hover_fade_tick_gen.wrapping_add(1);
    let tick_gen = app.clipboard_hover_fade_tick_gen;
    cx.spawn(async move |this, cx| {
        loop {
            Timer::after(SHIFT_TICK).await;
            let keep = this
                .update(cx, |app, cx| {
                    if app.clipboard_hover_fade_tick_gen != tick_gen {
                        return false;
                    }
                    if app.clipboard_hover_fades.is_empty() {
                        return false;
                    }
                    let now = Instant::now();
                    let duration = Duration::from_millis(CLIPBOARD_HOVER_ANIM_MS);
                    let animating = app.clipboard_hover_fades.values().any(|anim| {
                        now.saturating_duration_since(anim.start) < duration
                    });
                    app.clipboard_hover_fades.retain(|_, anim| {
                        now.saturating_duration_since(anim.start) < duration
                    });
                    if animating {
                        cx.notify();
                    }
                    animating
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        }
    })
    .detach();
}

fn sample_shift_y(app: &MemoryCleanerApp, id: i64, now: Instant) -> f32 {
    let Some(anim) = app.clipboard_shift_anims.get(&id) else {
        return 0.;
    };
    let elapsed = now.saturating_duration_since(anim.start);
    let t = elapsed.as_secs_f32() / SHIFT_DURATION.as_secs_f32();
    if t >= 1. {
        return anim.to;
    }
    anim.from + (anim.to - anim.from) * ease_out_quad(t)
}

/// Retarget every card's translateY tween from its *current* visual position.
/// Call whenever `clipboard_drop_target_id` changes (or drag starts).
pub fn sync_clipboard_shift_anims(app: &mut MemoryCleanerApp, cx: &mut Context<MemoryCleanerApp>) {
    let dragging_id = app.clipboard_dragging_id;
    let Some(active) = dragging_id.and_then(|id| {
        app.clipboard_items
            .iter()
            .position(|item| item.id == id)
    }) else {
        app.clipboard_shift_anims.clear();
        return;
    };
    let over = app
        .clipboard_drop_target_id
        .and_then(|id| {
            app.clipboard_items
                .iter()
                .position(|item| item.id == id)
        })
        .unwrap_or(active);

    let now = Instant::now();
    let mut changed = false;
    let ids: Vec<(usize, i64)> = app
        .clipboard_items
        .iter()
        .enumerate()
        .map(|(idx, item)| (idx, item.id))
        .collect();

    for (idx, id) in ids {
        let target = if Some(id) == dragging_id {
            0.
        } else {
            sortable_shift_y(idx, active, over)
        };
        let current = sample_shift_y(app, id, now);
        let prev_to = app
            .clipboard_shift_anims
            .get(&id)
            .map(|a| a.to)
            .unwrap_or(0.);
        if (prev_to - target).abs() > 0.5 {
            app.clipboard_shift_anims.insert(
                id,
                ClipboardShiftAnim {
                    from: current,
                    to: target,
                    start: now,
                },
            );
            changed = true;
        } else if !app.clipboard_shift_anims.contains_key(&id) && target.abs() > 0.5 {
            app.clipboard_shift_anims.insert(
                id,
                ClipboardShiftAnim {
                    from: 0.,
                    to: target,
                    start: now,
                },
            );
            changed = true;
        }
    }

    // Drop anims for items no longer in the list.
    app.clipboard_shift_anims
        .retain(|id, _| app.clipboard_items.iter().any(|item| item.id == *id));

    if changed {
        start_clipboard_shift_ticker(app, cx);
        cx.notify();
    }
}

/// Slide cards below a deleting row up into its slot (FLIP), while the row fades.
pub fn begin_delete_collapse(
    app: &mut MemoryCleanerApp,
    deleted_index: usize,
    cx: &mut Context<MemoryCleanerApp>,
) {
    let now = Instant::now();
    let deleting_id = app.clipboard_deleting_id;
    let ids: Vec<(usize, i64)> = app
        .clipboard_items
        .iter()
        .enumerate()
        .map(|(idx, item)| (idx, item.id))
        .collect();

    for (idx, id) in ids {
        if Some(id) == deleting_id {
            // Subtle lift while fading out.
            app.clipboard_shift_anims.insert(
                id,
                ClipboardShiftAnim {
                    from: sample_shift_y(app, id, now),
                    to: -12.,
                    start: now,
                },
            );
            continue;
        }
        if idx <= deleted_index {
            continue;
        }
        let current = sample_shift_y(app, id, now);
        app.clipboard_shift_anims.insert(
            id,
            ClipboardShiftAnim {
                from: current,
                to: -ROW_HEIGHT,
                start: now,
            },
        );
    }

    start_clipboard_shift_ticker(app, cx);
}

fn start_clipboard_shift_ticker(
    app: &mut MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) {
    app.clipboard_shift_tick_gen = app.clipboard_shift_tick_gen.wrapping_add(1);
    let tick_gen = app.clipboard_shift_tick_gen;
    cx.spawn(async move |this, cx| {
        loop {
            Timer::after(SHIFT_TICK).await;
            let keep = this
                .update(cx, |app, cx| {
                    if app.clipboard_shift_tick_gen != tick_gen {
                        return false;
                    }
                    let live = app.clipboard_dragging_id.is_some()
                        || app.clipboard_deleting_id.is_some();
                    if !live {
                        return false;
                    }
                    let now = Instant::now();
                    let animating = app.clipboard_shift_anims.values().any(|anim| {
                        now.saturating_duration_since(anim.start) < SHIFT_DURATION
                    });
                    if animating {
                        cx.notify();
                    }
                    // Keep ticking while drag/delete is active so retargets stay smooth.
                    true
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        }
    })
    .detach();
}

/// Resolve `over` from pointer Y — closest row center (dnd-kit `closestCenter` for
/// uniform rows) with light hysteresis so the boundary doesn't chatter.
fn update_drag_tearoff(
    app: &mut MemoryCleanerApp,
    e: &DragMoveEvent<DragClipboardItem>,
    window: &mut Window,
    cx: &mut Context<MemoryCleanerApp>,
) {
    if app.clipboard_drag_tearoff {
        return;
    }
    let pos = e.event.position;
    let size = window.viewport_size();
    let outside = pos.x < px(0.)
        || pos.y < px(0.)
        || pos.x >= size.width
        || pos.y >= size.height;
    if !outside {
        return;
    }
    app.clipboard_drag_tearoff = true;
    app.clipboard_drop_target_id = None;
    app.clipboard_shift_anims.clear();
    app.clipboard_shift_tick_gen = app.clipboard_shift_tick_gen.wrapping_add(1);
    cx.notify();
}

/// Resolve `over` from pointer Y — closest row center (dnd-kit `closestCenter` for
/// uniform rows) with light hysteresis so the boundary doesn't chatter.
fn update_drop_target_from_pointer(
    app: &mut MemoryCleanerApp,
    e: &DragMoveEvent<DragClipboardItem>,
    cx: &mut Context<MemoryCleanerApp>,
) {
    let items = &app.clipboard_items;
    let n = items.len();
    if n == 0 {
        return;
    }

    let row = ROW_HEIGHT;
    let scroll_y = f32::from(
        app.clipboard_list_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .y,
    );
    // offset.y is ≤ 0 when scrolled down; convert viewport Y → content Y.
    let y = f32::from(e.event.position.y - e.bounds.origin.y) - scroll_y;

    // Closest row center: centers sit at i*row + ITEM_HEIGHT/2.
    let mut best_idx = if y <= 0. {
        0
    } else {
        let approx = ((y - ITEM_HEIGHT * 0.5) / row).round() as isize;
        approx.clamp(0, (n as isize) - 1) as usize
    };

    if let Some(current_id) = app.clipboard_drop_target_id
        && let Some(current_idx) = items.iter().position(|item| item.id == current_id)
        && current_idx != best_idx
    {
        let current_center = current_idx as f32 * row + ITEM_HEIGHT * 0.5;
        // Stick to current over until pointer crosses ~35% toward the neighbor center.
        let stick = row * 0.35;
        if (y - current_center).abs() < stick {
            best_idx = current_idx;
        }
    }

    let best_id = items[best_idx].id;
    if app.clipboard_drop_target_id != Some(best_id) {
        app.clipboard_drop_target_id = Some(best_id);
        sync_clipboard_shift_anims(app, cx);
    }
}

/// ElegantClipboard-style segment track with a sliding white indicator.
fn render_clipboard_toolbar(
    app: &MemoryCleanerApp,
    active_filter: Option<ContentType>,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let theme = cx.theme();
    let radius = theme.radius;
    let muted_bg = theme.muted;
    let background = theme.background;
    let slide = sample_filter_slide(app, Instant::now());
    let indicator_left = slide / FILTER_SEGMENT_COUNT;

    let seg_all = filter_segment(
        cx,
        "clipboard-filter-all",
        t!("clipboard.filter_all").to_string(),
        active_filter.is_none(),
        |app, cx| app.set_clipboard_filter(None, cx),
    );
    let seg_text = filter_segment(
        cx,
        "clipboard-filter-text",
        t!("clipboard.filter_text").to_string(),
        active_filter == Some(ContentType::Text),
        |app, cx| app.set_clipboard_filter(Some(ContentType::Text), cx),
    );
    let seg_file = filter_segment(
        cx,
        "clipboard-filter-file",
        t!("clipboard.filter_file").to_string(),
        active_filter == Some(ContentType::File),
        |app, cx| app.set_clipboard_filter(Some(ContentType::File), cx),
    );

    h_flex()
        .w_full()
        .h(px(TOOLBAR_H))
        .flex_shrink_0()
        .px_2()
        .pt_2()
        .pb_1()
        .items_center()
        .child(
            div()
                .id("clipboard-filter-track")
                .w_full()
                .h(px(32.))
                .p(px(2.))
                .rounded(radius)
                .bg(muted_bg)
                .child(
                    div()
                        .relative()
                        .size_full()
                        .child(
                            div()
                                .absolute()
                                .top(px(0.))
                                .bottom(px(0.))
                                .left(relative(indicator_left))
                                .w(relative(1. / FILTER_SEGMENT_COUNT))
                                .rounded(radius)
                                .bg(background)
                                .shadow(vec![BoxShadow {
                                    color: hsla(0., 0., 0., 0.08),
                                    offset: point(px(0.), px(1.)),
                                    blur_radius: px(2.),
                                    spread_radius: px(0.),
                                    inset: false,
                                }]),
                        )
                        .child(
                            h_flex()
                                .relative()
                                .size_full()
                                .child(seg_all)
                                .child(seg_text)
                                .child(seg_file),
                        ),
                ),
        )
}

fn filter_index(filter: Option<ContentType>) -> f32 {
    match filter {
        None => 0.,
        Some(ContentType::Text) => 1.,
        Some(ContentType::File) => 2.,
    }
}

fn sample_filter_slide(app: &MemoryCleanerApp, now: Instant) -> f32 {
    let Some(anim) = &app.clipboard_filter_slide else {
        return filter_index(app.clipboard_filter);
    };
    let t = now
        .saturating_duration_since(anim.start)
        .as_secs_f32()
        / FILTER_SLIDE_DURATION.as_secs_f32();
    if t >= 1. {
        return anim.to;
    }
    let e = 1.0 - (1.0 - t.clamp(0., 1.)) * (1.0 - t.clamp(0., 1.));
    anim.from + (anim.to - anim.from) * e
}

/// Start the segment indicator sliding toward `filter`.
pub fn begin_filter_slide(
    app: &mut MemoryCleanerApp,
    filter: Option<ContentType>,
    cx: &mut Context<MemoryCleanerApp>,
) {
    let now = Instant::now();
    let from = sample_filter_slide(app, now);
    let to = filter_index(filter);
    if (from - to).abs() < 0.001 {
        app.clipboard_filter_slide = None;
        return;
    }
    app.clipboard_filter_slide = Some(ClipboardFilterSlide {
        from,
        to,
        start: now,
    });
    start_filter_slide_ticker(app, cx);
}

fn start_filter_slide_ticker(
    app: &mut MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) {
    app.clipboard_filter_tick_gen = app.clipboard_filter_tick_gen.wrapping_add(1);
    let tick_gen = app.clipboard_filter_tick_gen;
    cx.spawn(async move |this, cx| {
        loop {
            Timer::after(SHIFT_TICK).await;
            let keep = this
                .update(cx, |app, cx| {
                    if app.clipboard_filter_tick_gen != tick_gen {
                        return false;
                    }
                    let Some(anim) = &app.clipboard_filter_slide else {
                        return false;
                    };
                    let elapsed = Instant::now().saturating_duration_since(anim.start);
                    if elapsed >= FILTER_SLIDE_DURATION {
                        app.clipboard_filter_slide = None;
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
}

fn filter_segment(
    cx: &mut Context<MemoryCleanerApp>,
    id: &'static str,
    label: String,
    active: bool,
    action: fn(&mut MemoryCleanerApp, &mut Context<MemoryCleanerApp>),
) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let radius = theme.radius;
    let fg = if active {
        theme.foreground
    } else {
        theme.muted_foreground
    };
    let active_fg = theme.foreground;

    div()
        .id(id)
        .flex_1()
        .h_full()
        .min_w_0()
        .px_2()
        .rounded(radius)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |style| style.text_color(active_fg))
        .on_click(cx.listener(move |app, _, _, cx| action(app, cx)))
        .child(Label::new(label).text_xs().text_color(fg))
}
