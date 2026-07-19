use std::ops::Range;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{
    ActiveTheme, Disableable, button::Button, h_flex, label::Label, v_flex,
};

use crate::app::MemoryCleanerApp;
use crate::clipboard::ContentType;
use crate::ui::clipboard_item_card::{
    DragClipboardItem, ITEM_HEIGHT, render_clipboard_item,
};

/// Clipboard-only window height (width matches the main 520px window).
pub const CLIPBOARD_WINDOW_HEIGHT: f32 = 600.;
/// Filter bar height.
const FILTER_BAR_H: f32 = 34.;
/// Status bar height.
const STATUS_BAR_H: f32 = 28.;
/// Vertical gap between cards.
pub const ITEM_GAP: f32 = 4.;
/// Row height including gap (uniform_list measures one row).
pub const ROW_HEIGHT: f32 = ITEM_HEIGHT + ITEM_GAP;

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
        .child(
            h_flex()
                .w_full()
                .h(px(FILTER_BAR_H))
                .flex_shrink_0()
                .px_3()
                .gap_2()
                .items_center()
                .child(filter_button(
                    cx,
                    "clipboard-filter-all",
                    "全部",
                    active_filter.is_none(),
                    |app, cx| app.set_clipboard_filter(None, cx),
                ))
                .child(filter_button(
                    cx,
                    "clipboard-filter-text",
                    "文本",
                    active_filter == Some(ContentType::Text),
                    |app, cx| app.set_clipboard_filter(Some(ContentType::Text), cx),
                ))
                .child(filter_button(
                    cx,
                    "clipboard-filter-file",
                    "文件",
                    active_filter == Some(ContentType::File),
                    |app, cx| app.set_clipboard_filter(Some(ContentType::File), cx),
                ))
                .child(div().flex_1())
                .child({
                    let unpinned = app
                        .clipboard_items
                        .iter()
                        .filter(|item| !item.is_pinned)
                        .count();
                    Button::new("clipboard-clear")
                        .label("清空")
                        .text_xs()
                        .disabled(unpinned == 0)
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.open_clipboard_clear_confirm(window, cx);
                        }))
                }),
        )
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
                    .on_drag_move(cx.listener(|app, e: &DragMoveEvent<DragClipboardItem>, _, cx| {
                        update_drop_target_from_pointer(app, e, cx);
                    }))
                    .on_drop(cx.listener(|app, drag: &DragClipboardItem, _, cx| {
                        let target = app.clipboard_drop_target_id;
                        app.clipboard_drop_target_id = None;
                        app.clipboard_dragging_id = None;
                        if let Some(to) = target
                            && drag.id != to
                        {
                            app.move_clipboard_item(drag.id, to, cx);
                        } else {
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
                    Label::new("点击粘贴 · 删除需确认 · 拖拽左侧调整顺序".to_string())
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
    let dragging_id = app.clipboard_dragging_id;
    let drop_target_id = app.clipboard_drop_target_id;
    let active_idx = dragging_id.and_then(|id| {
        app.clipboard_items
            .iter()
            .position(|item| item.id == id)
    });
    let over_idx = drop_target_id.and_then(|id| {
        app.clipboard_items
            .iter()
            .position(|item| item.id == id)
    });

    range
        .filter_map(|idx| {
            let item = app.clipboard_items.get(idx)?;
            let id = item.id;
            let selected = app.clipboard_selected == Some(idx);
            let is_source = dragging_id == Some(id);
            // dnd-kit verticalListSortingStrategy: translateY only — never reorder rows.
            // Set the target offset directly (no with_animation keyed on over): GPUI
            // oneshot animations restart on each notify and cause the bounce.
            let target_y = match (active_idx, over_idx) {
                (Some(active), Some(over)) if !is_source => {
                    sortable_shift_y(idx, active, over)
                }
                _ => 0.,
            };

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
                            .top(px(target_y))
                            .child(card),
                    )
                    .into_any_element(),
            )
        })
        .collect()
}

/// Same geometry as `@dnd-kit` `verticalListSortingStrategy` for equal-height rows.
fn sortable_shift_y(index: usize, active: usize, over: usize) -> f32 {
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
        cx.notify();
    }
}

fn filter_button(
    cx: &mut Context<MemoryCleanerApp>,
    id: &'static str,
    label: &'static str,
    active: bool,
    action: fn(&mut MemoryCleanerApp, &mut Context<MemoryCleanerApp>),
) -> impl IntoElement {
    let mut button = Button::new(id).label(label).text_xs();
    if active {
        button = button.outline();
    }
    button.on_click(cx.listener(move |app, _, _, cx| action(app, cx)))
}
