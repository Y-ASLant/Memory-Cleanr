use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{
    ActiveTheme, Icon, IconName, InteractiveElementExt, Sizable, Size, h_flex, label::Label,
    v_flex,
};

use crate::app::{AppEntityHolder, MemoryCleanerApp};
use crate::clipboard::{ClipboardItem, ContentType};

/// Max preview lines shown on a card.
pub const MAX_DISPLAY_LINES: usize = 4;
/// Fixed card height (keeps drag reorder layout stable).
pub const ITEM_HEIGHT: f32 = 96.;
/// Drag ghost width (matches list content area).
pub const DRAG_CARD_WIDTH: f32 = 488.;

/// Drag payload for clipboard item reorder.
#[derive(Clone)]
pub struct DragClipboardItem {
    pub id: i64,
}

#[derive(Clone)]
struct DragPreviewCard {
    lines: Vec<SharedString>,
    time_text: SharedString,
    content_type: ContentType,
    is_pinned: bool,
    file_count: Option<usize>,
}

impl Render for DragPreviewCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Keep the ghost non-interactive so list `on_drag_move` still receives pointer events
        // (same idea as dnd-kit DragOverlay not blocking collision).
        let theme = cx.theme();
        h_flex()
            .w(px(DRAG_CARD_WIDTH))
            .h(px(ITEM_HEIGHT))
            .py_2()
            .px_2()
            .gap_2()
            .items_start()
            .overflow_hidden()
            .bg(theme.background)
            .border_1()
            .border_color(theme.primary)
            .rounded_md()
            .child(
                div()
                    .w(px(20.))
                    .flex_shrink_0()
                    .pt_1()
                    .child(drag_handle_icon(theme.muted_foreground)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(card_content(
                        self.content_type,
                        &self.lines,
                        &self.time_text,
                        self.is_pinned,
                        self.file_count,
                        cx,
                    )),
            )
            .with_animation(
                "clipboard-drag-ghost",
                Animation::new(Duration::from_millis(140)).with_easing(ease_out_quint()),
                |this, delta| {
                    let shadow = vec![BoxShadow {
                        color: hsla(0., 0., 0., 0.14 * delta),
                        offset: point(px(0.), px(2. + 6. * delta)),
                        blur_radius: px(6. + 12. * delta),
                        spread_radius: px(0.),
                        inset: false,
                    }];
                    this.opacity(0.55 + 0.4 * delta).shadow(shadow)
                },
            )
    }
}

/// Render a single clipboard item card.
pub fn render_clipboard_item(
    item: &ClipboardItem,
    index: usize,
    is_selected: bool,
    app: &MemoryCleanerApp,
    cx: &mut Context<MemoryCleanerApp>,
) -> impl IntoElement {
    let theme = cx.theme();
    let is_dragging = app.clipboard_dragging_id == Some(item.id);

    // Soft insertion hole (follows via arrayMove). Fade in so the gap doesn't pop in.
    if is_dragging {
        let theme = cx.theme();
        return div()
            .id(("clipboard-item-slot", item.id as u32))
            .w_full()
            .h(px(ITEM_HEIGHT))
            .rounded_md()
            .border_1()
            .border_color(theme.primary.opacity(0.35))
            .bg(theme.primary.opacity(0.07))
            .with_animation(
                ("clipboard-drop-slot", item.id as u32),
                Animation::new(Duration::from_millis(120)).with_easing(ease_out_quint()),
                |this, delta| this.opacity(0.35 + 0.65 * delta),
            )
            .into_any_element();
    }

    let bg = if is_selected {
        theme.selection
    } else {
        theme.background
    };
    let border_color = if is_selected {
        theme.primary
    } else {
        theme.border
    };

    let time_text = format_time_ago(&item.created_at);
    let item_id = item.id;
    let preview_lines: Vec<SharedString> = display_lines(item)
        .into_iter()
        .map(SharedString::from)
        .collect();
    let file_count = item.file_paths.as_ref().map(|p| p.len());
    let drag_preview = DragPreviewCard {
        lines: preview_lines.clone(),
        time_text: time_text.clone().into(),
        content_type: item.content_type,
        is_pinned: item.is_pinned,
        file_count,
    };
    let drag_payload = DragClipboardItem { id: item_id };
    let app_entity = cx.global::<AppEntityHolder>().0.clone();

    h_flex()
        .id(("clipboard-item", item_id as u32))
        .w_full()
        .h(px(ITEM_HEIGHT))
        .py_2()
        .px_2()
        .gap_2()
        .items_start()
        .overflow_hidden()
        .bg(bg)
        .border_1()
        .border_color(border_color)
        .rounded_md()
        .child(
            div()
                .id(("clipboard-drag", item_id as u32))
                .w(px(20.))
                .flex_shrink_0()
                .pt_1()
                .cursor_grab()
                .on_drag(drag_payload, {
                    let preview = drag_preview.clone();
                    let app_entity = app_entity.clone();
                    move |item, _offset, _window, cx| {
                        app_entity.update(cx, |app, cx| {
                            app.clipboard_dragging_id = Some(item.id);
                            // Start with over = self so the hole begins under the card.
                            app.clipboard_drop_target_id = Some(item.id);
                            cx.notify();
                        });
                        let preview = preview.clone();
                        cx.new(move |_cx| preview)
                    }
                })
                .child(drag_handle_icon(theme.muted_foreground)),
        )
        .child(
            h_flex()
                .id(("clipboard-body", item_id as u32))
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .cursor_pointer()
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.clipboard_selected = Some(index);
                    app.paste_clipboard_item(item_id, cx);
                }))
                .on_double_click(cx.listener(move |app, _, _, cx| {
                    app.delete_clipboard_item(item_id, cx);
                }))
                .child(card_content(
                    item.content_type,
                    &preview_lines,
                    &time_text,
                    item.is_pinned,
                    file_count,
                    cx,
                )),
        )
        .into_any_element()
}

fn drag_handle_icon(muted: Hsla) -> impl IntoElement {
    Icon::new(IconName::Menu)
        .with_size(Size::Small)
        .text_color(muted)
}

fn card_content(
    content_type: ContentType,
    lines: &[SharedString],
    time_text: &str,
    is_pinned: bool,
    file_count: Option<usize>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let icon = match content_type {
        ContentType::Text => IconName::File,
        ContentType::File => IconName::FolderOpen,
    };

    h_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .items_start()
        .overflow_hidden()
        .child(
            Icon::new(icon)
                .with_size(Size::Small)
                .text_color(theme.muted_foreground)
                .flex_shrink_0(),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .overflow_hidden()
                .children(lines.iter().map(|line| {
                    Label::new(line.clone())
                        .text_sm()
                        .text_color(theme.foreground)
                        .truncate()
                        .into_any_element()
                }))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            Label::new(time_text.to_string())
                                .text_xs()
                                .text_color(theme.muted_foreground),
                        ),
                ),
        )
        .when(is_pinned, |el| {
            el.child(
                Icon::new(IconName::Star)
                    .with_size(Size::XSmall)
                    .text_color(theme.primary)
                    .flex_shrink_0(),
            )
        })
        .children(file_count.map(|count| {
            Label::new(format!("{count} 个文件"))
                .text_xs()
                .text_color(theme.muted_foreground)
                .into_any_element()
        }))
}

fn display_lines(item: &ClipboardItem) -> Vec<String> {
    let source = item
        .text_content
        .as_deref()
        .filter(|t| !t.is_empty())
        .unwrap_or(item.preview.as_str());

    let mut lines: Vec<String> = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(MAX_DISPLAY_LINES)
        .map(str::to_string)
        .collect();

    if lines.is_empty() {
        lines.push(item.preview.clone());
        return lines;
    }

    let total_lines = source.lines().filter(|line| !line.trim().is_empty()).count();
    if total_lines > MAX_DISPLAY_LINES
        && let Some(last) = lines.last_mut()
    {
        last.push('…');
    }

    lines
}

/// Format a datetime string as a relative time ago.
fn format_time_ago(created_at: &str) -> String {
    let now = chrono::Local::now();
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%d %H:%M:%S") {
        let local_dt = dt.and_local_timezone(chrono::Local).single();
        if let Some(local_dt) = local_dt {
            let duration = now.signed_duration_since(local_dt);
            let secs = duration.num_seconds();
            if secs < 60 {
                "刚刚".into()
            } else if secs < 3600 {
                format!("{} 分钟前", secs / 60)
            } else if secs < 86400 {
                format!("{} 小时前", secs / 3600)
            } else {
                format!("{} 天前", secs / 86400)
            }
        } else {
            created_at.to_string()
        }
    } else {
        created_at.to_string()
    }
}
