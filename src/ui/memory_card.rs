use crate::memory::{MemorySection, MemoryStatus};
use rust_i18n::t;

use gpui::*;
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, h_flex, label::Label, progress::ProgressCircle,
    v_flex,
};

pub const MEMORY_RING_SIZE: f32 = 108.;

/// ProgressCircle applies a 0.75 scale to custom sizes internally.
const PROGRESS_CIRCLE_LAYOUT_SIZE: Pixels = px(MEMORY_RING_SIZE / 0.75);

/// 卡片容器上下内边距（app 中 GroupBox 内 v_flex 使用）。
pub const MEMORY_CARD_PY: f32 = 2.;

fn usage_color(percent: f32, cx: &App) -> Hsla {
    let theme = cx.theme();
    if percent >= 90.0 {
        theme.danger
    } else if percent >= 70.0 {
        theme.warning
    } else {
        theme.chart_2
    }
}

fn render_usage_ring(
    id: &'static str,
    section: &MemorySection,
    animated_percent: f32,
    cx: &App,
) -> impl IntoElement {
    let unavailable = section.is_unavailable();
    let (display_percent, color, label_color, label_text) = if unavailable {
        (
            0.0,
            cx.theme().muted_foreground,
            cx.theme().muted_foreground,
            "—".to_string(),
        )
    } else {
        (
            animated_percent,
            usage_color(section.used_percent, cx),
            cx.theme().foreground,
            format!("{}%", animated_percent.round() as u32),
        )
    };

    ProgressCircle::new(id)
        .with_size(Size::Size(PROGRESS_CIRCLE_LAYOUT_SIZE))
        .value(display_percent)
        .color(color)
        .child(
            Label::new(label_text)
                .text_lg()
                .font_weight(FontWeight::BOLD)
                .text_color(label_color),
        )
}

pub fn render_memory_card(
    section: &MemorySection,
    id: &'static str,
    is_physical: bool,
    animated_percent: f32,
    animated_used: u64,
    animated_avail: u64,
    cx: &App,
) -> impl IntoElement {
    let unavailable = section.is_unavailable();

    let icon = if is_physical {
        IconName::Cpu
    } else {
        IconName::HardDrive
    };

    let ring = render_usage_ring(id, section, animated_percent, cx);
    let summary = if unavailable {
        t!("memory.unavailable").to_string()
    } else {
        t!(
            "memory.used_avail",
            used = MemoryStatus::format_bytes(animated_used),
            avail = MemoryStatus::format_bytes(animated_avail),
        )
        .to_string()
    };
    let muted = cx.theme().foreground.opacity(0.82);

    v_flex()
        .w_full()
        .items_center()
        .gap_1()
        .child(
            h_flex()
                .items_center()
                .gap_1()
                .child(Icon::new(icon).small())
                .child(
                    Label::new(section.header())
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD),
                ),
        )
        .child(ring)
        .child(Label::new(summary).text_xs().text_color(if unavailable {
            cx.theme().warning
        } else {
            muted
        }))
}
