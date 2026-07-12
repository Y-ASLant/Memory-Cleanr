//! Shared layout metrics for window sizing and spacing.

use gpui::{Pixels, px};

pub const SECTION_GAP: f32 = 6.;
pub const DIALOG_PADDING_TOP: f32 = 16.;
pub const DIALOG_PADDING_HORIZONTAL: f32 = 16.;
/// 「窗口行为」对话框宽度（相对 520px 主窗口左右各留 20px）。
pub const WINDOW_BEHAVIOR_DIALOG_WIDTH: f32 = 480.;
/// 「窗口行为」对话框估算高度，用于垂直居中定位。
pub const WINDOW_BEHAVIOR_DIALOG_ESTIMATED_HEIGHT: f32 = 248.;
pub const TITLE_BAR_H: f32 = 34.;
pub const CLEANUP_BUTTON_H: f32 = 48.;

const CARD_BORDER: f32 = 2.;
/// GroupBox outline 内层固定 `p_4()`（上下各 16px）。
const GROUP_BOX_OUTLINE_PADDING_V: f32 = 32.;
const PANEL_BORDER: f32 = 2.;
const MEMORY_HEADER_H: f32 = 20.;
const MEMORY_LINE_GAP: f32 = 4.;
const MEMORY_SUMMARY_H: f32 = 16.;
const SECTION_TITLE_H: f32 = 20.;
/// 清理区行高估算（仅用于 `expanded_window_height`，不影响实际布局）。
const HINT_H: f32 = 24.;
const CHECKBOX_ROW_H: f32 = 22.;
const CLEANUP_ROWS: f32 = 4.;
/// 提示条 + 4 行 checkbox 共 5 项，`v_flex().gap(6)` 产生 4 个间距。
const CLEANUP_ROW_GAPS: f32 = SECTION_GAP * CLEANUP_ROWS;
/// 折叠窗口高度略偏低时会裁切 footer 底边距，补回至 6px。
const COLLAPSED_FOOTER_PADDING_GUARD: f32 = 4.;
/// 展开窗口高度估算偏高时的负向微调（仅影响 `window.resize`）。
const EXPANDED_WINDOW_HEIGHT_ADJUSTMENT: f32 = -22.;

pub fn memory_section_height() -> f32 {
    use crate::ui::memory_card::{MEMORY_CARD_PY, MEMORY_RING_SIZE};

    CARD_BORDER
        + GROUP_BOX_OUTLINE_PADDING_V
        + MEMORY_CARD_PY * 2.
        + MEMORY_HEADER_H
        + MEMORY_LINE_GAP
        + MEMORY_RING_SIZE
        + MEMORY_LINE_GAP
        + MEMORY_SUMMARY_H
}

pub fn cleanup_section_height(content_padding: f32) -> f32 {
    let cleanup_areas = HINT_H + SECTION_GAP + CHECKBOX_ROW_H * CLEANUP_ROWS + CLEANUP_ROW_GAPS;

    PANEL_BORDER + content_padding * 2. + SECTION_TITLE_H + SECTION_GAP + cleanup_areas
}

pub fn collapsed_window_height(content_padding: f32) -> f32 {
    TITLE_BAR_H
        + content_padding
        + memory_section_height()
        + SECTION_GAP
        + CLEANUP_BUTTON_H
        + content_padding
        + COLLAPSED_FOOTER_PADDING_GUARD
}

pub fn expanded_window_height(content_padding: f32) -> f32 {
    TITLE_BAR_H
        + content_padding
        + memory_section_height()
        + SECTION_GAP
        + cleanup_section_height(content_padding)
        + SECTION_GAP
        + CLEANUP_BUTTON_H
        + content_padding
        + EXPANDED_WINDOW_HEIGHT_ADJUSTMENT
}

/// Vertically center a dialog in the client area below the app title bar.
pub fn centered_dialog_margin_top(
    viewport_height: Pixels,
    dialog_height: f32,
    title_bar_height: f32,
) -> Pixels {
    let usable = viewport_height - px(title_bar_height);
    let offset = (usable - px(dialog_height)) / 2.;
    (px(title_bar_height) + offset.max(px(0.))).max(px(title_bar_height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_window_is_taller_than_collapsed() {
        let collapsed = collapsed_window_height(6.);
        let expanded = expanded_window_height(6.);
        assert!(expanded > collapsed);
    }

    #[test]
    fn centered_dialog_margin_top_stays_below_title_bar() {
        let margin = centered_dialog_margin_top(px(600.), 248., TITLE_BAR_H);
        assert!(margin >= px(TITLE_BAR_H));
    }
}
