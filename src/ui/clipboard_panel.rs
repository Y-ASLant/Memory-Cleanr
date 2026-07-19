use std::ops::Range;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{ActiveTheme, button::Button, h_flex, label::Label, v_flex};

use crate::app::MemoryCleanerApp;
use crate::clipboard::{ClipboardItem, ContentType};
use crate::ui::clipboard_item_card::{
    DragClipboardItem, ITEM_HEIGHT, render_clipboard_item,
};

/// Clipboard-only window height (width matches the main 520px window).
pub const CLIPBOARD_WINDOW_HEIGHT: f32 = 600.;
/// Filter bar height.
const FILTER_BAR_H: f32 = 34.;
/// Status bar height.
const STATUS_BAR_H: f32 = 28.;
/// Vertical gap between cards (`mb_1`).
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

    // Display count uses the same preview order as drag (arrayMove while dragging).
    let display_count = preview_ordered_count(
        &app.clipboard_items,
        app.clipboard_dragging_id,
        app.clipboard_drop_target_id,
    );

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
                .child(
                    Button::new("clipboard-clear")
                        .label("清空")
                        .text_xs()
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.clear_clipboard_history(cx);
                        })),
                ),
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
                        uniform_list("clipboard-virtual-list", display_count, {
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
                    Label::new("点击粘贴 · 双击删除 · 拖拽左侧调整顺序".to_string())
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
    let display_ids: Vec<(usize, i64)> = {
        let display = preview_ordered_items(
            &app.clipboard_items,
            app.clipboard_dragging_id,
            app.clipboard_drop_target_id,
        );
        range
            .filter_map(|idx| display.get(idx).map(|item| (idx, item.id)))
            .collect()
    };
    let is_dragging = app.clipboard_dragging_id.is_some();

    display_ids
        .into_iter()
        .filter_map(|(idx, id)| {
            let item = app.clipboard_items.iter().find(|item| item.id == id)?;
            let selected = app.clipboard_selected == Some(idx);
            let dimmed = is_dragging && app.clipboard_dragging_id != Some(item.id);
            Some(
                div()
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .when(dimmed, |el| el.opacity(0.88))
                    .child(
                        div()
                            .w_full()
                            .h(px(ITEM_HEIGHT))
                            .child(render_clipboard_item(item, idx, selected, app, cx)),
                    )
                    .into_any_element(),
            )
        })
        .collect()
}

fn preview_ordered_count(
    items: &[ClipboardItem],
    dragging_id: Option<i64>,
    drop_target_id: Option<i64>,
) -> usize {
    preview_ordered_items(items, dragging_id, drop_target_id).len()
}

/// Like `@dnd-kit` `arrayMove`: move dragged item to the `over` item's index.
fn preview_ordered_items(
    items: &[ClipboardItem],
    dragging_id: Option<i64>,
    drop_target_id: Option<i64>,
) -> Vec<&ClipboardItem> {
    let mut ordered: Vec<&ClipboardItem> = items.iter().collect();
    let (Some(drag_id), Some(drop_id)) = (dragging_id, drop_target_id) else {
        return ordered;
    };
    if drag_id == drop_id {
        return ordered;
    }
    let Some(from) = ordered.iter().position(|item| item.id == drag_id) else {
        return ordered;
    };
    let Some(to) = ordered.iter().position(|item| item.id == drop_id) else {
        return ordered;
    };
    let item = ordered.remove(from);
    ordered.insert(to, item);
    ordered
}

/// Resolve drop target from pointer Y against the **original** list geometry + scroll.
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
    let mut best_idx = if y <= 0. {
        0
    } else {
        ((y / row) as usize).min(n - 1)
    };

    if let Some(current_id) = app.clipboard_drop_target_id
        && let Some(current_idx) = items.iter().position(|item| item.id == current_id)
        && current_idx != best_idx
    {
        let current_top = current_idx as f32 * row;
        let current_bottom = current_top + ITEM_HEIGHT;
        let margin = ITEM_HEIGHT * 0.25;
        if y >= current_top - margin && y <= current_bottom + margin {
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
