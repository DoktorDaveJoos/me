//! Shared visual vocabulary and component primitives. No local token definitions.
pub use crate::assets::{Icon, IconSize};
pub use crate::design_system::{DesignStyle, Type, color::*, font, layout, radius, space};
use gpui::{Animation, AnimationExt, Div, Transformation, div, prelude::*, px, rgb};

pub fn eyebrow(text: impl Into<gpui::SharedString>) -> Div {
    div()
        .type_style(Type::Caption)
        .font_family(font::MONO)
        .text_color(rgb(MUTED))
        .child(text.into())
}

pub fn heading(text: impl Into<gpui::SharedString>) -> Div {
    div().type_style(Type::Title).child(text.into())
}

/// Primary action styling; handlers and disabled/busy behavior belong to the caller.
pub fn primary_action() -> Div {
    div()
        .h(px(layout::CONTROL))
        .px(px(space::LG))
        .rounded(px(radius::STANDARD))
        .bg(rgb(INK))
        .text_color(rgb(SURFACE))
        .type_style(Type::Small)
        .flex()
        .items_center()
        .justify_center()
        .gap(px(space::SM))
        .cursor_pointer()
}

/// Dialogs share their surface, border, inset and content rhythm.
pub fn modal_panel(width: f32) -> Div {
    div()
        .w(px(width))
        .max_w_full()
        .p(px(space::XXL))
        .rounded(px(radius::STANDARD))
        .bg(rgb(SURFACE))
        .border_1()
        .border_color(rgb(LINE))
        .flex()
        .flex_col()
        .gap(px(space::XXL))
}

/// Progress is completed work, never elapsed time or model confidence. Unknown
/// totals use a moving marker with an explicit Working label in the caller.
pub fn progress_bar(fraction: Option<f32>, color: u32, activity: f32) -> Div {
    div()
        .w_full()
        .h(px(layout::PROGRESS_HEIGHT))
        .flex_shrink_0()
        .rounded(px(radius::STANDARD))
        .bg(rgb(LINE))
        .overflow_hidden()
        .relative()
        .child(
            div()
                .absolute()
                .top_0()
                .h_full()
                .rounded(px(radius::STANDARD))
                .bg(rgb(color))
                .w(gpui::relative(fraction.unwrap_or(0.2).clamp(0., 1.)))
                .left(gpui::relative(if fraction.is_some() {
                    0.
                } else {
                    (activity % 1.) * 0.8
                })),
        )
}

/// A one-shot entrance: a slide and a foreshortened turn, settling before typing.
/// The form itself never moves and the animation never gates interaction.
pub fn fingerprint_intro() -> impl IntoElement {
    crate::assets::icon(Icon::Fingerprint, IconSize::Hero, DECORATIVE).with_animation(
        "vault-fingerprint-intro",
        Animation::new(std::time::Duration::from_millis(
            crate::design_system::motion::IDENTITY_ENTER_MS,
        )),
        |icon, progress| {
            let eased = 1. - (1. - progress).powi(3);
            let remaining = 1. - eased;
            let turn = remaining * std::f32::consts::TAU;
            icon.opacity(eased).with_transformation(
                Transformation::translate(gpui::point(px(-space::SECTION * remaining), px(0.)))
                    .with_scaling(gpui::size(turn.cos().abs().max(0.08), 1.))
                    .with_rotation(gpui::Radians(-remaining * std::f32::consts::FRAC_PI_6)),
            )
        },
    )
}

/// Secondary actions share the same control geometry as primary actions.
pub fn secondary_action() -> Div {
    primary_action()
        .bg(rgb(SURFACE))
        .text_color(rgb(INK))
        .border_1()
        .border_color(rgb(LINE))
}
