//! Shared visual vocabulary and component primitives. No local token definitions.
pub use crate::assets::{Icon, IconSize};
pub use crate::design_system::{DesignStyle, Type, color::*, font, layout, radius, space};
use gpui::{Div, div, prelude::*, px, rgb};

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
