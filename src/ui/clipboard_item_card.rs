use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{
    ActiveTheme, Icon, IconName, InteractiveElementExt, Sizable, Size,
    button::{Button, ButtonVariants},
    h_flex, label::Label, v_flex,
};
use rust_i18n::t;

use crate::app::{AppEntityHolder, MemoryCleanerApp};
use crate::clipboard::{ClipboardItem, ContentType};

/// Max preview lines shown on a card.
pub const MAX_DISPLAY_LINES: usize = 4;
/// Fixed card height (keeps drag reorder layout stable).
pub const ITEM_HEIGHT: f32 = 96.;
/// Drag ghost width (matches list content area).
pub const DRAG_CARD_WIDTH: f32 = 488.;
/// Line box height for `Label::text_sm()` — icon aligns to the first line only.
const TEXT_SM_LINE_H: f32 = 20.;

/// Left half: drag reorder zone (pale yellow `#FFF9E6`).
pub(crate) fn drag_zone_bg() -> Hsla {
    hsla(45. / 360., 0.85, 0.95, 1.)
}
/// Right half: click-to-paste zone (light blue `#D6E4FF`).
pub(crate) fn paste_zone_bg() -> Hsla {
    hsla(220. / 360., 0.55, 0.92, 1.)
}

/// Drag payload for clipboard item reorder.
#[derive(Clone)]
pub struct DragClipboardItem {
    pub id: i64,
}

#[derive(Clone)]
pub(crate) struct DragPreviewCard {
    pub(crate) lines: Vec<SharedString>,
    pub(crate) time_text: SharedString,
    pub(crate) content_type: ContentType,
    pub(crate) is_pinned: bool,
    pub(crate) file_count: Option<usize>,
}

impl Render for DragPreviewCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Tear-off uses a separate follower window; hide the in-window GPUI ghost.
        if cx
            .try_global::<AppEntityHolder>()
            .is_some_and(|holder| holder.0.read(cx).clipboard_drag_tearoff)
        {
            return div().occlude().into_any_element();
        }

        render_drag_preview_ghost(
            self.content_type,
            &self.lines,
            &self.time_text,
            self.is_pinned,
            self.file_count,
            cx,
        )
        .into_any_element()
    }
}

/// Shared drag ghost chrome for in-window overlay and tear-off follower window.
pub(crate) fn render_drag_preview_ghost(
    content_type: ContentType,
    lines: &[SharedString],
    time_text: &SharedString,
    is_pinned: bool,
    file_count: Option<usize>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .relative()
        .w(px(DRAG_CARD_WIDTH))
        .h(px(ITEM_HEIGHT))
        .overflow_hidden()
        .border_1()
        .border_color(theme.primary)
        .rounded_md()
        .cursor_grabbing()
        .child(render_split_card(
            div().size_full().bg(drag_zone_bg()),
            div().size_full().bg(paste_zone_bg()),
        ))
        .child(
            div()
                .absolute()
                .inset_0()
                .px_2()
                .py_2()
                .overflow_hidden()
                .child(card_content(
                    content_type,
                    lines,
                    time_text,
                    is_pinned,
                    file_count,
                    cx,
                )),
        )
        .shadow(vec![BoxShadow {
            color: hsla(0., 0., 0., 0.16),
            offset: point(px(0.), px(6.)),
            blur_radius: px(16.),
            spread_radius: px(0.),
            inset: false,
        }])
        .opacity(0.96)
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
    let is_deleting = app.clipboard_deleting_id == Some(item.id);
    let zone_opacity =
        crate::ui::clipboard_panel::sample_clipboard_hover_opacity(app, item.id, Instant::now());
    let show_delete = zone_opacity > 0.01 && !is_deleting && app.clipboard_dragging_id.is_none();

    let border_color = if is_selected {
        theme.primary
    } else {
        theme.border
    };
    let card_bg = if is_selected {
        theme.selection
    } else {
        theme.background
    };
    // Same tokens as gpui-component ListItem so hover/press reads clearly.
    let hover_border = theme.primary.opacity(0.55);
    let danger = theme.danger;

    let time_text = format_time_ago(&item.created_at);
    let item_id = item.id;
    let preview_lines: Vec<SharedString> = display_lines(item)
        .into_iter()
        .map(SharedString::from)
        .collect();
    let file_count = item.file_paths.as_ref().map(|p| p.len());
    let drag_preview = drag_preview_card_from_item(item);
    let drag_payload = DragClipboardItem { id: item_id };
    let app_entity = cx.global::<AppEntityHolder>().0.clone();

    let content_params = (
        item.content_type,
        preview_lines,
        time_text,
        item.is_pinned,
        file_count,
    );

    let card = div()
        .id(("clipboard-item", item_id as u32))
        .relative()
        .w_full()
        .h(px(ITEM_HEIGHT))
        .overflow_hidden()
        .bg(card_bg)
        .border_1()
        .border_color(border_color)
        .rounded_md()
        .on_hover(cx.listener(move |app, hovered: &bool, _, cx| {
            if *hovered {
                if app.clipboard_hovered_id != Some(item_id) {
                    app.clipboard_hovered_id = Some(item_id);
                    crate::ui::clipboard_panel::begin_clipboard_hover_fade(app, item_id, cx);
                }
            } else if app.clipboard_hovered_id == Some(item_id) {
                app.clipboard_hovered_id = None;
                crate::ui::clipboard_panel::begin_clipboard_hover_fade(app, item_id, cx);
            }
        }))
        .when(!is_selected && !is_deleting, |el| {
            el.hover(move |style| style.border_color(hover_border))
        })
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
                .child(card_content(
                    content_params.0,
                    &content_params.1,
                    &content_params.2,
                    content_params.3,
                    content_params.4,
                    cx,
                )),
        )
        .child(render_split_card(
            div()
                .id(("clipboard-drag", item_id as u32))
                .size_full()
                .cursor_grab()
                .on_click(|_, _, cx| cx.stop_propagation())
                .on_drag(drag_payload, {
                    let preview = drag_preview.clone();
                    let app_entity = app_entity.clone();
                    move |item, offset, _window, cx| {
                                app_entity.update(cx, |app, cx| {
                                    app.clipboard_shift_anims.clear();
                                    app.clipboard_drag_tearoff = false;
                                    app.clipboard_drag_cursor_offset = Some(offset);
                                    app.clipboard_dragging_id = Some(item.id);
                            app.clipboard_drop_target_id = Some(item.id);
                            if app.clipboard_hovered_id == Some(item.id) {
                                app.clipboard_hovered_id = None;
                                crate::ui::clipboard_panel::begin_clipboard_hover_fade(
                                    app, item.id, cx,
                                );
                            }
                            crate::ui::clipboard_panel::sync_clipboard_shift_anims(app, cx);
                            crate::ui::clipboard_panel::start_clipboard_drag_tracker(app, cx);
                            cx.notify();
                        });
                        let preview = preview.clone();
                        cx.new(move |_cx| preview)
                    }
                }),
            div()
                .id(("clipboard-paste", item_id as u32))
                .size_full()
                .cursor_pointer()
                .on_click(cx.listener(move |app, _, _, cx| {
                    if app.clipboard_deleting_id.is_some() {
                        return;
                    }
                    app.clipboard_selected = Some(index);
                    app.paste_clipboard_item(item_id, cx);
                }))
                .on_double_click(cx.listener(move |app, _, window, cx| {
                    if app.clipboard_deleting_id.is_some() {
                        return;
                    }
                    app.open_clipboard_delete_confirm(item_id, window, cx);
                })),
        ))
        .when(show_delete, |el| {
            el.child(
                div()
                    .id(("clipboard-delete-wrap", item_id as u32))
                    .absolute()
                    .top(px(4.))
                    .right(px(4.))
                    .opacity(zone_opacity)
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        Button::new(("clipboard-delete", item_id as u32))
                            .ghost()
                            .xsmall()
                            .icon(
                                Icon::new(IconName::CircleX)
                                    .xsmall()
                                    .text_color(danger),
                            )
                            .tooltip(t!("clipboard.delete_tooltip").to_string())
                            .on_click(cx.listener(move |app, _, window, cx| {
                                cx.stop_propagation();
                                app.open_clipboard_delete_confirm(item_id, window, cx);
                            })),
                    ),
            )
        });

    if is_deleting {
        use crate::ui::clipboard_panel::DELETE_ANIM_MS;
        card.with_animation(
            ("clipboard-delete", item_id as u32),
            Animation::new(Duration::from_millis(DELETE_ANIM_MS)).with_easing(ease_out_quint()),
            |this, delta| this.opacity(1.0 - delta),
        )
        .into_any_element()
    } else if is_dragging {
        // ElegantClipboard / dnd-kit: keep layout slot, hide the source (ghost is DragOverlay).
        card.opacity(0.).into_any_element()
    } else {
        card.into_any_element()
    }
}

/// Fixed 50/50 split — absolute halves so wide clipped content cannot expand one side.
pub(crate) fn render_split_card(
    left: impl IntoElement,
    right: impl IntoElement,
) -> impl IntoElement {
    div()
        .relative()
        .w_full()
        .h_full()
        .child(
            div()
                .absolute()
                .top(px(0.))
                .bottom(px(0.))
                .left(px(0.))
                .w(relative(0.5))
                .overflow_hidden()
                .child(left),
        )
        .child(
            div()
                .absolute()
                .top(px(0.))
                .bottom(px(0.))
                .left(relative(0.5))
                .w(relative(0.5))
                .overflow_hidden()
                .child(right),
        )
}

/// Yellow/blue zone tint shown on card hover (list + pinned desktop cards).
pub(crate) fn render_zone_overlay() -> impl IntoElement {
    render_split_card(
        div().size_full().bg(drag_zone_bg()),
        div().size_full().bg(paste_zone_bg()),
    )
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
            div()
                .flex_shrink_0()
                .h(px(TEXT_SM_LINE_H))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(icon)
                        .with_size(Size::Small)
                        .text_color(theme.muted_foreground),
                ),
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
                    h_flex().gap_2().items_center().child(
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

/// Drag ghost payload for in-window overlay and tear-off follower window.
pub(crate) fn drag_preview_card_from_item(item: &ClipboardItem) -> DragPreviewCard {
    let preview_lines: Vec<SharedString> = display_lines(item)
        .into_iter()
        .map(SharedString::from)
        .collect();
    DragPreviewCard {
        lines: preview_lines,
        time_text: format_time_ago(&item.created_at).into(),
        content_type: item.content_type,
        is_pinned: item.is_pinned,
        file_count: item.file_paths.as_ref().map(|p| p.len()),
    }
}

/// Shared card body for list rows and pinned desktop cards.
pub fn render_card_content(item: &ClipboardItem, cx: &App) -> AnyElement {
    let preview_lines: Vec<SharedString> = display_lines(item)
        .into_iter()
        .map(SharedString::from)
        .collect();
    let time_text = format_time_ago(&item.created_at);
    let file_count = item.file_paths.as_ref().map(|p| p.len());
    card_content(
        item.content_type,
        &preview_lines,
        &time_text,
        item.is_pinned,
        file_count,
        cx,
    )
    .into_any_element()
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
