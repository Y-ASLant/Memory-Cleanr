use std::f32::consts::TAU;

use gpui::{Animation, AnimationExt, canvas, *};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable,
    animation::ease_in_out_cubic,
    h_flex,
    label::Label,
    plot::shape::{Arc, ArcData},
    v_flex,
};

use crate::memory::MemorySection;

pub const RING_ANIM_DURATION_MS: u64 = 300;

const OUTER_RADIUS: f32 = 55.;
const INNER_RADIUS: f32 = 35.;

pub const MEMORY_RING_SIZE: f32 = OUTER_RADIUS * 2.;

/// 卡片容器上下内边距（app 中 GroupBox 内 v_flex 使用）。
pub const MEMORY_CARD_PY: f32 = 2.;

/// Ring animation endpoints; updated by the app when memory data changes.
#[derive(Debug, Clone, Copy, Default)]
pub struct RingAnim {
    pub from: f32,
    pub to: f32,
}

impl RingAnim {
    pub fn new(percent: f32) -> Self {
        let p = percent.clamp(0.0, 100.0);
        Self { from: p, to: p }
    }

    pub fn is_animating(&self) -> bool {
        (self.from - self.to).abs() > 0.01
    }
}

#[derive(Clone, Copy)]
struct RingTheme {
    chart_1: Hsla,
    chart_2: Hsla,
    warning: Hsla,
    danger: Hsla,
}

impl RingTheme {
    fn new(theme: &gpui_component::Theme) -> Self {
        Self {
            chart_1: theme.chart_1,
            chart_2: theme.chart_2,
            warning: theme.warning,
            danger: theme.danger,
        }
    }
}

fn usage_color(percent: f32, theme: RingTheme) -> Hsla {
    if percent >= 90.0 {
        theme.danger
    } else if percent >= 70.0 {
        theme.warning
    } else {
        theme.chart_2
    }
}

fn render_donut(used_percent: f32, theme: RingTheme) -> impl IntoElement {
    let used_percent = used_percent.clamp(0.0, 100.0);
    let used_color = usage_color(used_percent, theme);
    let free_color = theme.chart_1;
    let used_angle = (used_percent / 100.0) * TAU;

    canvas(
        move |bounds, _, _| (used_percent, used_color, free_color, used_angle, bounds),
        move |_, (used_percent, used_color, free_color, used_angle, bounds), window, _| {
            let arc = Arc::new()
                .inner_radius(INNER_RADIUS)
                .outer_radius(OUTER_RADIUS);

            arc.paint(
                &ArcData {
                    data: &(),
                    index: 0,
                    value: 100.,
                    start_angle: 0.,
                    end_angle: TAU,
                    pad_angle: 0.,
                },
                free_color,
                None,
                None,
                &bounds,
                window,
            );

            if used_percent > 0.01 {
                arc.paint(
                    &ArcData {
                        data: &(),
                        index: 1,
                        value: used_percent,
                        start_angle: 0.,
                        end_angle: used_angle.max(0.02),
                        pad_angle: 0.02,
                    },
                    used_color,
                    None,
                    None,
                    &bounds,
                    window,
                );
            }
        },
    )
    .absolute()
    .size_full()
}

fn render_ring(used_percent: f32, theme: RingTheme) -> impl IntoElement {
    div()
        .relative()
        .w(px(110.))
        .h(px(110.))
        .flex_shrink_0()
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Label::new(format!("{}%", used_percent.round() as u32))
                        .text_lg()
                        .font_weight(FontWeight::BOLD),
                ),
        )
        .child(render_donut(used_percent, theme))
}

fn render_animated_ring(id: &'static str, from: f32, to: f32, theme: RingTheme) -> AnyElement {
    div()
        .id(id)
        .relative()
        .w(px(110.))
        .h(px(110.))
        .flex_shrink_0()
        .with_animation(
            format!("{id}-ring-anim"),
            Animation::new(std::time::Duration::from_millis(RING_ANIM_DURATION_MS))
                .with_easing(ease_in_out_cubic),
            move |this, delta| {
                let percent = from + (to - from) * delta;
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(format!("{}%", percent.round() as u32))
                                .text_lg()
                                .font_weight(FontWeight::BOLD),
                        ),
                )
                .child(render_donut(percent, theme))
            },
        )
        .into_any_element()
}

pub fn render_memory_card(
    section: &MemorySection,
    ring: RingAnim,
    id: &'static str,
    is_physical: bool,
    cx: &App,
) -> impl IntoElement {
    let ring_theme = RingTheme::new(cx.theme());
    let unavailable = section.is_unavailable();

    let icon = if is_physical {
        IconName::Cpu
    } else {
        IconName::HardDrive
    };

    let ring = if unavailable {
        render_ring(0.0, ring_theme).into_any_element()
    } else if ring.is_animating() {
        render_animated_ring(id, ring.from, ring.to, ring_theme)
    } else {
        render_ring(ring.to, ring_theme).into_any_element()
    };

    let summary = if unavailable {
        "无法读取内存信息".into()
    } else {
        section.usage_summary()
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
