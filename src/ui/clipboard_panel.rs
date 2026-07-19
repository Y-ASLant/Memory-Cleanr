use gpui::*;
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size,
    button::Button,
    h_flex, label::Label,
    scroll::ScrollableElement as _,
    v_flex,
};

use crate::app::MemoryCleanerApp;
use crate::clipboard::ContentType;
use crate::ui::clipboard_item_card::render_clipboard_item;

/// Clipboard-only window height (width matches the main 520px window).
pub const CLIPBOARD_WINDOW_HEIGHT: f32 = 600.;
/// Search bar height.
const SEARCH_BAR_H: f32 = 36.;
/// Filter bar height.
const FILTER_BAR_H: f32 = 34.;
/// Status bar height.
const STATUS_BAR_H: f32 = 28.;

/// Render the clipboard panel (full window content when clipboard mode is active).
pub fn render_clipboard_panel(
    app: &MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let border = theme.border;
    let items = &app.clipboard_items;
    let total = items.len();
    let active_filter = app.clipboard_filter;

    v_flex()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .w_full()
        .h_full()
        .child(
            h_flex()
                .w_full()
                .h(px(SEARCH_BAR_H))
                .flex_shrink_0()
                .px_3()
                .items_center()
                .gap_2()
                .child(
                    Icon::new(IconName::Search)
                        .with_size(Size::Small)
                        .text_color(muted),
                )
                .child(
                    Label::new("搜索剪贴板…".to_string())
                        .text_sm()
                        .text_color(muted),
                ),
        )
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
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .px_2()
                    .py_1()
                    .overflow_y_scrollbar()
                    .children(items.iter().enumerate().map(|(idx, item)| {
                        let selected = app.clipboard_selected == Some(idx);
                        let drop_target = app.clipboard_drop_target_id == Some(item.id);
                        div()
                            .w_full()
                            .mb_1()
                            .child(render_clipboard_item(
                                item,
                                idx,
                                selected,
                                drop_target,
                                app,
                                cx,
                            ))
                            .into_any_element()
                    }))
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
