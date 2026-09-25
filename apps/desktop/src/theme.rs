//! Shared visual vocabulary and component primitives. No local token definitions.
pub use crate::assets::{Icon, IconSize};
pub use crate::design_system::{DesignStyle, Type, color::*, font, layout, radius, space};
use gpui::{Div, Transformation, div, prelude::*, px, rgb};
#[path = "motion.rs"]
pub mod motion;

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

/// Brand typography is intentionally separate from the Geist interface family.
pub fn wordmark(compact: bool) -> Div {
    div()
        .font_family(font::BRAND)
        .type_style(if compact {
            Type::BrandSmall
        } else {
            Type::Brand
        })
        .flex_shrink_0()
        .child("ME.")
}

pub fn brand_signature(compact: bool, window: &gpui::Window) -> Div {
    // One native text drawing gives both sizes an identical typographic baseline.
    // Flex-box baselines are element-box baselines in this GPUI version.
    let shape = |text: &'static str, role: Type, family: &'static str, color: u32| {
        let (size, leading, weight) = role.metrics();
        let mut face = gpui::font(family);
        face.weight = weight;
        let run = gpui::TextRun {
            len: text.len(),
            font: face,
            color: rgb(color).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window
            .text_system()
            .shape_line(text.into(), px(size), &[run], None);
        (line, px(leading))
    };
    let role = if compact {
        Type::BrandSmall
    } else {
        Type::Brand
    };
    let (brand, height) = shape("ME.", role, font::BRAND, INK);
    let (tagline, tagline_height) = shape("All about you.", Type::Small, font::SANS, MUTED);
    let baseline = (height - brand.ascent - brand.descent) / 2. + brand.ascent;
    let tagline_baseline =
        (tagline_height - tagline.ascent - tagline.descent) / 2. + tagline.ascent;
    let width = brand.width + px(space::MD) + tagline.width;
    div().w(width).h(height).flex_shrink_0().child(
        gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                let _ = brand.paint(bounds.origin, height, window, cx);
                let origin = bounds.origin
                    + gpui::point(brand.width + px(space::MD), baseline - tagline_baseline);
                let _ = tagline.paint(origin, tagline_height, window, cx);
            },
        )
        .size_full(),
    )
}

/// The window owns the entrance clock, so remounting a setup step cannot replay it.
pub fn onboarding_fingerprint(progress: f32) -> gpui::AnyElement {
    div()
        .size(px(layout::FINGERPRINT_ART_SIZE))
        .child(onboarding_fingerprint_at(progress))
        .into_any_element()
}

/// Explicit phase also lets the native gallery inspect the real artwork mid-flight.
pub fn onboarding_fingerprint_at(progress: f32) -> impl IntoElement {
    use crate::design_system::motion as timing;
    let t = motion::ease(progress);
    let remaining = 1. - t;
    let angle = remaining * std::f32::consts::TAU;
    let scale = timing::FINGERPRINT_MIN_SCALE + (1. - timing::FINGERPRINT_MIN_SCALE) * t;
    crate::assets::icon(Icon::Fingerprint, IconSize::Identity, DECORATIVE)
        .opacity(t * timing::FINGERPRINT_OPACITY)
        .with_transformation(
            Transformation::translate(gpui::point(
                px(-timing::FINGERPRINT_TRAVEL * remaining
                    + timing::FINGERPRINT_ORBIT_HEIGHT * angle.sin() * remaining),
                px(-timing::FINGERPRINT_ORBIT_HEIGHT * angle.sin() * remaining),
            ))
            .with_scaling(gpui::size(
                scale * angle.cos().abs().max(timing::FINGERPRINT_MIN_WIDTH),
                scale,
            ))
            .with_rotation(gpui::Radians(-timing::FINGERPRINT_TILT * remaining)),
        )
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

/// Checkbox row: the whole label is a hit target; the caller owns focus and toggling.
pub fn checkbox(checked: bool, label: &'static str) -> Div {
    div()
        .min_h(px(layout::CONTROL_LARGE))
        .py(px(space::SM))
        .px(px(space::XS))
        .rounded(px(radius::STANDARD))
        .flex()
        .items_center()
        .gap(px(space::MD))
        .type_style(Type::Small)
        .cursor_pointer()
        .child(
            div()
                .size(px(layout::CHECKBOX))
                .flex_shrink_0()
                .rounded(px(radius::STANDARD))
                .border_2()
                .border_color(rgb(if checked { ACCENT } else { INK }))
                .bg(rgb(if checked { ACCENT } else { SURFACE }))
                .flex()
                .items_center()
                .justify_center()
                .when(checked, |s| {
                    s.child(crate::assets::icon(Icon::Check, IconSize::Medium, SURFACE))
                }),
        )
        .child(div().flex_1().min_w_0().child(label))
}

/// Dialogs share their surface, border, inset and content rhythm.
pub fn modal_panel(width: f32, animated: bool) -> Div {
    div()
        .relative()
        .child(motion::frame(motion::Frame::Panel, animated))
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
pub fn fingerprint_intro(progress: f32) -> impl IntoElement {
    let icon = crate::assets::icon(Icon::Fingerprint, IconSize::Hero, DECORATIVE);
    let eased = motion::ease(progress);
    let remaining = 1. - eased;
    let turn = remaining * std::f32::consts::TAU;
    let icon = icon.opacity(eased).with_transformation(
        Transformation::translate(gpui::point(px(-space::SECTION * remaining), px(0.)))
            .with_scaling(gpui::size(turn.cos().abs().max(0.08), 1.))
            .with_rotation(gpui::Radians(-remaining * std::f32::consts::FRAC_PI_6)),
    );
    div()
        .relative()
        .size(px(layout::IDENTITY_ICON))
        .child(motion::seal(false))
        .child(icon)
}

/// Secondary actions share the same control geometry as primary actions.
pub fn secondary_action() -> Div {
    primary_action()
        .bg(rgb(SURFACE))
        .text_color(rgb(INK))
        .border_1()
        .border_color(rgb(LINE))
}

/// Keep the action's footprint stable while showing real, indeterminate work.
pub fn action_indicator(busy: bool, animated: bool) -> gpui::AnyElement {
    if busy {
        motion::spinner(animated)
    } else {
        crate::assets::icon(
            crate::assets::Icon::Arrow,
            crate::assets::IconSize::Medium,
            SURFACE,
        )
        .into_any_element()
    }
}

/// Compact, wrapping metadata. Uses the shared rectangular radius.
pub fn tag_chip(label: impl Into<gpui::SharedString>) -> impl IntoElement {
    div()
        .max_w_full()
        .px(px(space::SM))
        .py(px(space::XS))
        .rounded(px(radius::STANDARD))
        .bg(rgb(HOVER))
        .type_style(Type::Small)
        .text_color(rgb(ACCENT))
        .child(
            div()
                .min_w_0()
                .whitespace_normal()
                .overflow_hidden()
                .child(label.into()),
        )
}

/// Compact actions retain a generous hit area and a descriptive hover label.
pub fn icon_action(
    id: impl Into<gpui::ElementId>,
    icon: Icon,
    label: &'static str,
) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .size(px(layout::CONTROL_COMPACT))
        .flex_shrink_0()
        .rounded(px(radius::STANDARD))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(rgb(HOVER)))
        .tooltip(move |_, cx| cx.new(|_| ActionTooltip(label)).into())
        .child(crate::assets::icon(icon, IconSize::Medium, MUTED))
}
struct ActionTooltip(&'static str);
impl gpui::Render for ActionTooltip {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .px(px(space::SM))
            .py(px(space::XS))
            .rounded(px(radius::STANDARD))
            .bg(rgb(INK))
            .text_color(rgb(SURFACE))
            .font_family(font::SANS)
            .type_style(Type::Small)
            .child(self.0)
    }
}
