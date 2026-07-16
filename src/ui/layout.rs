//! Shared layout metrics for window sizing and spacing.

pub const SECTION_GAP: f32 = 6.;
pub const DIALOG_PADDING_TOP: f32 = 16.;
pub const DIALOG_PADDING_HORIZONTAL: f32 = 16.;
/// 「窗口行为」对话框宽度（相对 520px 主窗口左右各留 20px）。
pub const WINDOW_BEHAVIOR_DIALOG_WIDTH: f32 = 480.;
pub const TITLE_BAR_H: f32 = 34.;
pub const CLEANUP_BUTTON_H: f32 = 48.;

const CARD_BORDER: f32 = 2.;
/// GroupBox outline 内容区 `p_2()`（上下各 8px）。
const GROUP_BOX_OUTLINE_PADDING_V: f32 = 16.;
const MEMORY_HEADER_H: f32 = 20.;
const MEMORY_LINE_GAP: f32 = 4.;
const MEMORY_SUMMARY_H: f32 = 16.;
const SECTION_TITLE_H: f32 = 20.;
/// 清理区行高估算（仅用于 `expanded_window_height`，不影响实际布局）。
const HINT_H: f32 = 24.;
const CHECKBOX_ROW_H: f32 = 22.;
const CLEANUP_ROWS: f32 = 4.;
/// Small outline Tag 行高（与 gpui-component `Tag::small` 一致）。
pub const EXCLUSION_TAG_ROW_HEIGHT: f32 = 26.;
/// 标签 flex_wrap 间距（横/纵）。
pub const EXCLUSION_TAG_GAP: f32 = 6.;
pub const EXCLUSION_TAG_VISIBLE_ROWS: f32 = 3.;
pub const EXCLUSION_LIST_PADDING: f32 = 6.;
pub const EXCLUSION_FOOTER_GAP: f32 = 6.;
pub const EXCLUSION_SELECTOR_H: f32 = 32.;
/// 提示条 + 4 行 checkbox 共 5 项，`v_flex().gap(6)` 产生 4 个间距。
const CLEANUP_ROW_GAPS: f32 = SECTION_GAP * CLEANUP_ROWS;
/// 折叠窗口高度略偏低时会裁切 footer 底边距，补回至 6px。
const COLLAPSED_FOOTER_PADDING_GUARD: f32 = 4.;

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

pub fn cleanup_section_height() -> f32 {
    let cleanup_areas = section_card_height(
        HINT_H + SECTION_GAP + CHECKBOX_ROW_H * CLEANUP_ROWS + CLEANUP_ROW_GAPS,
    );
    let exclusion_list = process_exclusion_list_max_height();
    let process_exclusion =
        section_card_height(exclusion_list + EXCLUSION_FOOTER_GAP + EXCLUSION_SELECTOR_H);

    process_exclusion + SECTION_GAP + cleanup_areas
}

fn section_card_height(body: f32) -> f32 {
    CARD_BORDER + GROUP_BOX_OUTLINE_PADDING_V + SECTION_TITLE_H + SECTION_GAP + body
}

fn process_exclusion_list_inner_height() -> f32 {
    EXCLUSION_TAG_ROW_HEIGHT * EXCLUSION_TAG_VISIBLE_ROWS
        + EXCLUSION_TAG_GAP * (EXCLUSION_TAG_VISIBLE_ROWS - 1.)
}

pub fn process_exclusion_list_max_height() -> f32 {
    process_exclusion_list_inner_height() + EXCLUSION_LIST_PADDING * 2.
}

/// 进程下拉菜单最大高度（向上展开，避免遮挡下方控件）。
pub const PROCESS_PICKER_MENU_MAX_H: f32 = 180.;

pub fn collapsed_window_height(content_padding: f32) -> f32 {
    TITLE_BAR_H
        + content_padding
        + memory_section_height()
        + SECTION_GAP
        + CLEANUP_BUTTON_H
        + content_padding
        + COLLAPSED_FOOTER_PADDING_GUARD
}

pub fn expanded_window_height(_content_padding: f32) -> f32 {
    630.
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
    fn process_exclusion_list_fits_three_tag_rows() {
        let inner = process_exclusion_list_inner_height();
        let total = process_exclusion_list_max_height();
        assert_eq!(
            inner,
            EXCLUSION_TAG_ROW_HEIGHT * 3. + EXCLUSION_TAG_GAP * 2.
        );
        assert_eq!(total, inner + EXCLUSION_LIST_PADDING * 2.);
        assert_eq!(total, 102.);
    }
}
