use gpui::*;

use crate::clipboard::ClipboardItem;
use crate::ui::clipboard_item_card::{
    DRAG_CARD_WIDTH, ITEM_HEIGHT, DragPreviewCard, drag_preview_card_from_item,
    render_drag_preview_ghost,
};

/// Screen origin for the tear-off drag ghost (cursor centered on card).
pub fn tearoff_preview_origin(screen: Point<Pixels>) -> Point<Pixels> {
    point(
        screen.x - px(DRAG_CARD_WIDTH / 2.),
        screen.y - px(ITEM_HEIGHT / 2.),
    )
}

pub fn tearoff_preview_window_options(origin: Point<Pixels>) -> WindowOptions {
    WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            origin,
            size(px(DRAG_CARD_WIDTH), px(ITEM_HEIGHT)),
        ))),
        kind: WindowKind::PopUp,
        focus: false,
        is_resizable: false,
        is_movable: false,
        ..Default::default()
    }
}

/// Follower card shown while dragging outside the main window.
pub struct TearoffDragPreview {
    card: DragPreviewCard,
}

impl TearoffDragPreview {
    pub fn new(item: ClipboardItem) -> Self {
        Self {
            card: drag_preview_card_from_item(&item),
        }
    }
}

impl Render for TearoffDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render_drag_preview_ghost(
            self.card.content_type,
            &self.card.lines,
            &self.card.time_text,
            self.card.is_pinned,
            self.card.file_count,
            cx,
        )
    }
}
