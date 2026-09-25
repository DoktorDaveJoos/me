//! Finite native motion and the recurring honeycomb drawing language.
//! Decoration has no input handlers and never changes data positions.
use crate::assets::{Icon, IconSize, icon};
use crate::design_system::{DesignStyle, color::*, font, layout, motion as timing, radius, space};
use gpui::{
    Animation, AnimationExt, AnyElement, Bounds, ElementId, PathBuilder, Pixels, Point, Window,
    canvas, div, point, prelude::*, px, rgb,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub enum Field {
    Identity,
    Sidebar,
    Workspace,
}
#[derive(Clone, Copy)]
pub enum Frame {
    Panel,
    Focus,
    Navigation,
}

pub fn ease(value: f32) -> f32 {
    1. - (1. - value.clamp(0., 1.)).powi(3)
}
pub fn phase(start: Option<Instant>, milliseconds: u64, enabled: bool) -> f32 {
    if !enabled {
        return 1.;
    }
    start
        .map(|t| {
            (t.elapsed().as_secs_f32() / Duration::from_millis(milliseconds).as_secs_f32())
                .clamp(0., 1.)
        })
        .unwrap_or(1.)
}

/// Children and hit regions keep the same identity for the entire transition.
/// Navigation changes the wrapper key; typing and ordinary refreshes do not.
pub fn page(content: AnyElement, key: usize, enabled: bool) -> AnyElement {
    let body = div()
        .relative()
        .w_full()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(content);
    if !enabled {
        return body.into_any_element();
    }
    body.with_animation(
        ("page-arrival", key),
        Animation::new(Duration::from_millis(timing::PAGE_ENTER_MS)),
        |el, t| {
            let t = ease(t);
            el.opacity(timing::CONTENT_START_OPACITY + (1. - timing::CONTENT_START_OPACITY) * t)
                .top(px(space::SM * (1. - t)))
        },
    )
    .into_any_element()
}

pub fn overlay(content: AnyElement, key: impl Into<ElementId>, enabled: bool) -> AnyElement {
    let body = div().absolute().inset_0().child(content);
    if !enabled {
        return body.into_any_element();
    }
    body.with_animation(
        key,
        Animation::new(Duration::from_millis(timing::OVERLAY_ENTER_MS)),
        |el, t| el.opacity(ease(t)),
    )
    .into_any_element()
}

/// Add before foreground siblings; GPUI paints children in insertion order.
/// Keep activity beside this field in that same background layer.
pub fn field(kind: Field, enabled: bool) -> AnyElement {
    let body = div().absolute().inset_0().overflow_hidden();
    if !enabled {
        return body.child(field_at(kind, 1.)).into_any_element();
    }
    body.with_animation(
        ("honeycomb-field", kind as usize),
        Animation::new(Duration::from_millis(timing::FIELD_ENTER_MS)),
        move |el, t| el.child(field_at(kind, t)),
    )
    .into_any_element()
}

/// Busy-only, clockwise light moving through the existing perimeter lattice.
/// There is no timer or minimum duration between the operation and its next step.
pub fn activity(kind: Field, enabled: bool) -> AnyElement {
    let body = div().absolute().inset_0().overflow_hidden();
    if !enabled {
        return body.into_any_element();
    }
    body.with_animation(
        ("honeycomb-activity", kind as usize),
        Animation::new(Duration::from_millis(timing::ACCOUNT_ACTIVITY_MS)).repeat(),
        move |el, t| {
            el.child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        paint_activity(bounds, kind, t, window);
                    },
                )
                .absolute()
                .size_full(),
            )
        },
    )
    .into_any_element()
}

/// A small progress drawing, using the same stroke and tint as ME Outline.
/// Reduced motion keeps a static arc; the adjacent label still describes work.
pub fn spinner(enabled: bool) -> AnyElement {
    let body = div().size(px(timing::SPINNER_SIZE)).flex_shrink_0();
    let drawing = |t: f32| {
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let center = point(f32::from(bounds.center().x), f32::from(bounds.center().y));
                let size = timing::SPINNER_SIZE / 2. - timing::SPINNER_INSET;
                let points = (0..=48)
                    .map(|i| {
                        let angle = (i as f32 / 48. + t) * std::f32::consts::TAU;
                        point(center.x + angle.cos() * size, center.y + angle.sin() * size)
                    })
                    .collect::<Vec<_>>();
                let mut track = PathBuilder::stroke(px(timing::TRACE_STROKE));
                stroke_points(&mut track, &points);
                paint(window, track, SURFACE, timing::SPINNER_TRACK_OPACITY);
                let mut arc = PathBuilder::stroke(px(timing::TRACE_STROKE));
                stroke_points(&mut arc, &trace(&points, timing::SPINNER_ARC));
                paint(window, arc, SURFACE, 1.);
            },
        )
        .size_full()
    };
    if !enabled {
        return body.child(drawing(0.)).into_any_element();
    }
    body.with_animation(
        "action-spinner",
        Animation::new(Duration::from_millis(timing::SPINNER_MS)).repeat(),
        move |el, t| el.child(drawing(t)),
    )
    .into_any_element()
}

fn paint_activity(bounds: Bounds<Pixels>, kind: Field, progress: f32, window: &mut Window) {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    if w <= 0. || h <= 0. {
        return;
    }
    let size = field_cell_size(kind);
    let width = 3f32.sqrt() * size;
    let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let mut lights = FadedPaths::strokes(timing::TRACE_STROKE);
    for r in -1..=(h / (1.5 * size)).ceil() as i32 + 1 {
        for q in -1..=(w / width).ceil() as i32 + 1 {
            let x = q as f32 * width + if r.rem_euclid(2) == 0 { 0. } else { width / 2. };
            let y = r as f32 * 1.5 * size;
            if !field_cell_visible(kind, x, y, w, h, size) {
                continue;
            }
            let position = (((y / h - 0.5).atan2(x / w - 0.5) + std::f32::consts::FRAC_PI_2)
                / std::f32::consts::TAU)
                .rem_euclid(1.);
            let distance = (progress - position).rem_euclid(1.);
            if distance >= timing::ACCOUNT_ACTIVITY_TAIL {
                continue;
            }
            let strength = (distance / timing::ACCOUNT_ACTIVITY_TAIL * std::f32::consts::PI)
                .sin()
                .powi(2);
            let outline = hex_outline(point(ox + x, oy + y), size, radius::STANDARD);
            lights.stroke_alpha(&outline, kind, bounds, strength);
        }
    }
    lights.paint(
        window,
        ACCENT,
        timing::TRACE_OPACITY * field_opacity(kind) / timing::FIELD_OPACITY,
    );
}

/// Also used by the synthetic gallery to inspect exact drawing phases.
pub fn field_at(kind: Field, progress: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| paint_field(bounds, kind, progress, window),
    )
    .absolute()
    .size_full()
}

pub fn frame(kind: Frame, enabled: bool) -> AnyElement {
    let body = div().absolute().inset_0().overflow_hidden();
    if !enabled {
        return body.into_any_element();
    }
    let duration = if matches!(kind, Frame::Focus) {
        timing::FOCUS_TRACE_MS
    } else {
        timing::FRAME_TRACE_MS
    };
    body.with_animation(
        ("technical-frame", kind as usize),
        Animation::new(Duration::from_millis(duration)),
        move |el, t| {
            el.child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| paint_frame(bounds, kind, t, window),
                )
                .absolute()
                .size_full(),
            )
        },
    )
    .into_any_element()
}

pub fn seal(enabled: bool) -> AnyElement {
    let body = div().absolute().inset_0();
    let render = |t| {
        canvas(
            |_, _, _| (),
            move |b, _, w, _| {
                let center = point(
                    f32::from(b.origin.x + b.size.width / 2.),
                    f32::from(b.origin.y + b.size.height / 2.),
                );
                let points = hex_outline(center, timing::SEAL_RADIUS, radius::STANDARD);
                let mut base = PathBuilder::stroke(px(timing::FIELD_STROKE));
                stroke_points(&mut base, &trace(&points, ease(t)));
                paint(w, base, DECORATIVE, timing::SEAL_OPACITY);
                if t < 1. {
                    let mut tip = PathBuilder::stroke(px(timing::TRACE_STROKE));
                    let start = (ease(t) - timing::TRACE_TAIL).max(0.);
                    stroke_points(&mut tip, &trace_range(&points, start, ease(t)));
                    paint(w, tip, ACCENT, (1. - t) * timing::TRACE_OPACITY);
                }
            },
        )
        .absolute()
        .size_full()
    };
    if !enabled {
        return body.child(render(1.)).into_any_element();
    }
    body.with_animation(
        "identity-seal",
        Animation::new(Duration::from_millis(timing::FIELD_ENTER_MS)),
        move |el, t| el.child(render(t)),
    )
    .into_any_element()
}

/// The selection clock is owned by the page, so typing, revealing a password,
/// saving, scrolling and clicking the current login cannot restart the artwork.
/// Values are never used to seed geometry or motion.
pub fn login_identity(
    progress: f32,
    logo: Option<std::sync::Arc<gpui::RenderImage>>,
) -> impl IntoElement {
    let has_logo = logo.is_some();
    div()
        .relative()
        .w_full()
        .h(px(layout::LOGIN_IDENTITY_HEIGHT))
        .flex_shrink_0()
        .overflow_hidden()
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    paint_login_identity(bounds, progress, has_logo, window)
                },
            )
            .absolute()
            .size_full(),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(login_logo(
                    logo,
                    layout::LOGIN_LOGO_DETAIL,
                    IconSize::Large,
                    SURFACE,
                )),
        )
}

/// Hexagonal backplate belongs to the drawing language, not control geometry.
/// The surrounding list hit target remains the shared 8 px rectangle.
pub fn login_badge(
    active: bool,
    progress: f32,
    logo: Option<std::sync::Arc<gpui::RenderImage>>,
) -> impl IntoElement {
    div()
        .relative()
        .size(px(layout::CONTROL))
        .flex_shrink_0()
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let center = point(f32::from(bounds.center().x), f32::from(bounds.center().y));
                    let size = f32::from(bounds.size.height) / 2. - timing::FRAME_INSET;
                    let points = hex_outline(center, size, radius::STANDARD);
                    let mut fill = PathBuilder::fill();
                    stroke_points(&mut fill, &points);
                    fill.close();
                    paint(
                        window,
                        fill,
                        if active { ACCENT } else { HOVER },
                        if active { timing::LOGIN_BADGE_TINT } else { 1. },
                    );
                    let mut border = PathBuilder::stroke(px(timing::FIELD_STROKE));
                    stroke_points(&mut border, &points);
                    paint(window, border, if active { ACCENT } else { DECORATIVE }, 1.);
                    if active && progress < 1. {
                        let end = ease(progress);
                        let mut front = PathBuilder::stroke(px(timing::TRACE_STROKE));
                        stroke_points(
                            &mut front,
                            &trace_range(&points, (end - timing::TRACE_TAIL).max(0.), end),
                        );
                        paint(
                            window,
                            front,
                            ACCENT,
                            timing::TRACE_OPACITY * (1. - progress),
                        );
                    }
                },
            )
            .absolute()
            .size_full(),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(login_logo(
                    logo,
                    layout::LOGIN_LOGO_LIST,
                    IconSize::Medium,
                    if active { ACCENT } else { MUTED },
                )),
        )
}

/// Recipe cells are decorative children of stable rectangular click targets.
/// Only a successful generation changes the finite animation key; secrets never
/// influence geometry. Reduced motion renders the completed outline immediately.
pub fn password_cell(
    label: &'static str,
    active: bool,
    index: usize,
    generation: usize,
    animated: bool,
) -> AnyElement {
    let render = move |phase: f32| {
        div()
            .relative()
            .size(px(layout::CONTROL_LARGE))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let center =
                            point(f32::from(bounds.center().x), f32::from(bounds.center().y));
                        let size = f32::from(bounds.size.height) / 2. - timing::FRAME_INSET;
                        let points = hex_outline(center, size, radius::STANDARD);
                        let mut fill = PathBuilder::fill();
                        stroke_points(&mut fill, &points);
                        fill.close();
                        paint(
                            window,
                            fill,
                            if active { ACCENT } else { HOVER },
                            if active { timing::LOGIN_BADGE_TINT } else { 1. },
                        );
                        let mut border = PathBuilder::stroke(px(timing::TRACE_STROKE));
                        stroke_points(&mut border, &trace(&points, ease(phase)));
                        paint(window, border, if active { ACCENT } else { DECORATIVE }, 1.);
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_family(font::MONO)
                    .type_style(crate::design_system::Type::Small)
                    .text_color(rgb(if active { ACCENT } else { MUTED }))
                    .child(label),
            )
    };
    if !animated || generation == 0 {
        return render(1.).into_any_element();
    }
    div()
        .with_animation(
            (
                "password-cell",
                generation.wrapping_mul(4).wrapping_add(index),
            ),
            Animation::new(Duration::from_millis(timing::FRAME_TRACE_MS)),
            move |el, t| el.child(render(t)),
        )
        .into_any_element()
}

fn login_logo(
    logo: Option<std::sync::Arc<gpui::RenderImage>>,
    size: f32,
    fallback: IconSize,
    color: u32,
) -> AnyElement {
    if let Some(logo) = logo {
        gpui::img(logo)
            .size(px(size))
            .object_fit(gpui::ObjectFit::Contain)
            .into_any_element()
    } else {
        icon(Icon::Key, fallback, color).into_any_element()
    }
}

pub fn login_content_opacity(progress: f32, rank: usize) -> f32 {
    let elapsed = progress * timing::LOGIN_ENTER_MS as f32;
    let delay = rank.min(timing::LOGIN_STAGGER_LIMIT) as f32 * timing::LOGIN_STAGGER_MS as f32;
    let phase = ((elapsed - delay) / timing::LOGIN_CONTENT_MS as f32).clamp(0., 1.);
    timing::LOGIN_CONTENT_OPACITY + (1. - timing::LOGIN_CONTENT_OPACITY) * ease(phase)
}

pub fn frame_at(kind: Frame, progress: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            paint_frame(bounds, kind, progress, window);
        },
    )
    // Anchor to the parent bounds, not the padded flex-content position.
    .absolute()
    .inset_0()
    .size_full()
}

fn bloom(progress: f32) -> f32 {
    let t = progress.clamp(0., 1.) - 1.;
    1. + (timing::LOGIN_BLOOM_OVERSHOOT + 1.) * t.powi(3)
        + timing::LOGIN_BLOOM_OVERSHOOT * t.powi(2)
}

fn paint_login_identity(
    bounds: Bounds<Pixels>,
    progress: f32,
    has_logo: bool,
    window: &mut Window,
) {
    let center = point(f32::from(bounds.center().x), f32::from(bounds.center().y));
    let size = timing::LOGIN_CELL_RADIUS;
    let pitch = 3f32.sqrt() * size;
    let spread = timing::LOGIN_BLOOM_SCALE + (1. - timing::LOGIN_BLOOM_SCALE) * bloom(progress);
    // A cropped honeycomb ribbon: outer cells unfold around a steady central key.
    // Nothing about a credential's length, value or strength changes this art.
    for row in -1i32..=1 {
        for column in -3i32..=3 {
            if row == 0 && column == 0 {
                continue;
            }
            let x = column as f32 + if row == 0 { 0. } else { 0.5 };
            let distance = x.abs() + row.abs() as f32 / 2.;
            let fade = (1. - distance / timing::LOGIN_FIELD_SPREAD).clamp(0., 1.);
            let local = ((progress - distance * timing::LOGIN_CELL_STAGGER)
                / timing::LOGIN_CELL_DRAW)
                .clamp(0., 1.);
            let at = point(
                center.x + x * pitch * spread,
                center.y + row as f32 * size * 1.5 * spread,
            );
            let outline = hex_outline(at, size * spread, radius::STANDARD);
            let mut base = PathBuilder::stroke(px(timing::FIELD_STROKE));
            stroke_points(&mut base, &trace(&outline, ease(local)));
            paint(window, base, DECORATIVE, fade * timing::LOGIN_FIELD_OPACITY);
            if local > 0. && local < 1. {
                let end = ease(local);
                let mut front = PathBuilder::stroke(px(timing::TRACE_STROKE));
                stroke_points(
                    &mut front,
                    &trace_range(&outline, (end - timing::TRACE_TAIL).max(0.), end),
                );
                paint(window, front, ACCENT, fade * timing::TRACE_OPACITY);
            }
            if (column + row).rem_euclid(3) == 0 {
                let mut cube = PathBuilder::stroke(px(timing::FIELD_STROKE));
                for edge in 0..3 {
                    let angle = (edge as f32 * 120. - 30.).to_radians();
                    let vertex = point(
                        at.x + angle.cos() * size * spread,
                        at.y + angle.sin() * size * spread,
                    );
                    stroke_points(&mut cube, &trace(&[at, vertex], ease(local)));
                }
                paint(window, cube, ACCENT, fade * timing::FIELD_CUBE_OPACITY);
            }
        }
    }
    let outline = hex_outline(center, size, radius::STANDARD);
    let mut fill = PathBuilder::fill();
    stroke_points(&mut fill, &outline);
    fill.close();
    paint(
        window,
        fill,
        if has_logo { SURFACE } else { PRIMARY_HOVER },
        1.,
    );
    let mut border = PathBuilder::stroke(px(timing::TRACE_STROKE));
    stroke_points(&mut border, &trace(&outline, ease(progress)));
    paint(window, border, ACCENT, 1.);
}

/// Rounded corners share the same radius as the existing Knowledge cells.
pub fn hex_outline(center: Point<f32>, size: f32, bend: f32) -> Vec<Point<f32>> {
    let vertices = (0..6)
        .map(|i| {
            let angle = (i as f32 * 60. - 30.).to_radians();
            point(center.x + size * angle.cos(), center.y + size * angle.sin())
        })
        .collect::<Vec<_>>();
    rounded_polygon(&vertices, bend.min(size / 3.))
}
fn rounded_polygon(vertices: &[Point<f32>], bend: f32) -> Vec<Point<f32>> {
    let mut points = Vec::new();
    for (i, p) in vertices.iter().copied().enumerate() {
        let a = toward(p, vertices[(i + vertices.len() - 1) % vertices.len()], bend);
        let b = toward(p, vertices[(i + 1) % vertices.len()], bend);
        points.push(a);
        for step in 1..=6 {
            let t = step as f32 / 6.;
            let u = 1. - t;
            points.push(point(
                u * u * a.x + 2. * u * t * p.x + t * t * b.x,
                u * u * a.y + 2. * u * t * p.y + t * t * b.y,
            ));
        }
    }
    if let Some(first) = points.first().copied() {
        points.push(first);
    }
    points
}
fn toward(a: Point<f32>, b: Point<f32>, distance: f32) -> Point<f32> {
    let length = (b.x - a.x).hypot(b.y - a.y);
    let t = (distance / length.max(f32::EPSILON)).min(0.5);
    point(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

/// A distance-based reveal, including a fractional final segment. Source data
/// and stored coordinates are untouched; the final path is byte-for-byte whole.
pub fn trace(points: &[Point<f32>], progress: f32) -> Vec<Point<f32>> {
    trace_range(points, 0., progress)
}
fn trace_range(points: &[Point<f32>], start: f32, end: f32) -> Vec<Point<f32>> {
    if points.len() < 2 || end <= start {
        return Vec::new();
    }
    if start <= 0. && end >= 1. {
        return points.to_vec();
    }
    let length = points
        .windows(2)
        .map(|p| (p[1].x - p[0].x).hypot(p[1].y - p[0].y))
        .sum::<f32>();
    let (from, to) = (length * start.clamp(0., 1.), length * end.clamp(0., 1.));
    let mut walked = 0.;
    let mut result = Vec::new();
    for pair in points.windows(2) {
        let distance = (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y);
        if distance <= f32::EPSILON {
            continue;
        }
        if walked + distance > from && walked < to {
            let at = |fraction: f32| {
                point(
                    pair[0].x + (pair[1].x - pair[0].x) * fraction,
                    pair[0].y + (pair[1].y - pair[0].y) * fraction,
                )
            };
            if result.is_empty() {
                result.push(at(((from - walked) / distance).clamp(0., 1.)));
            }
            result.push(at(((to - walked) / distance).clamp(0., 1.)));
        }
        walked += distance;
        if walked >= to {
            break;
        }
    }
    result
}
fn stroke_points(builder: &mut PathBuilder, points: &[Point<f32>]) {
    if let Some(p) = points.first() {
        builder.move_to(point(px(p.x), px(p.y)));
        for p in &points[1..] {
            builder.line_to(point(px(p.x), px(p.y)));
        }
    }
}
fn paint(window: &mut Window, builder: PathBuilder, color: u32, opacity: f32) {
    if opacity <= 0. {
        return;
    }
    if let Ok(path) = builder.build() {
        let mut color = rgb(color);
        color.a = opacity.clamp(0., 1.);
        window.paint_path(path, color);
    }
}

/// All drawing layers, including busy highlights, use the same continuous mask.
fn field_depth(kind: Field, x: f32, y: f32, w: f32, h: f32) -> f32 {
    match kind {
        Field::Identity => x.min(w - x).min(y).min(h - y).max(0.) / timing::FIELD_BAND,
        Field::Sidebar => ((w - x).max(0.) / timing::SIDEBAR_FIELD_WIDTH)
            .hypot(y.max(0.) / timing::SIDEBAR_FIELD_HEIGHT),
        Field::Workspace => ((w - x).max(0.) / timing::HEADER_FIELD_WIDTH)
            .hypot(y.max(0.) / timing::HEADER_FIELD_HEIGHT),
    }
}
fn field_fade(kind: Field, x: f32, y: f32, w: f32, h: f32) -> f32 {
    let t = field_depth(kind, x, y, w, h).clamp(0., 1.);
    1. - t * t * (3. - 2. * t)
}
fn field_cell_visible(kind: Field, x: f32, y: f32, w: f32, h: f32, size: f32) -> bool {
    if matches!(kind, Field::Identity) {
        x.min(w - x).min(y).min(h - y) <= timing::FIELD_BAND + size
    } else {
        // Keep cells whose outermost strokes still meet the fade, even when their
        // center is outside it. Culling at the center produces truncated hexagons.
        field_depth(kind, x + size, y - size, w, h) < 1.
    }
}
fn field_cell_size(kind: Field) -> f32 {
    if matches!(kind, Field::Sidebar) {
        timing::SIDEBAR_CELL_RADIUS
    } else {
        timing::FIELD_CELL_RADIUS
    }
}
fn field_opacity(kind: Field) -> f32 {
    match kind {
        Field::Identity => timing::FIELD_OPACITY,
        Field::Sidebar => timing::SIDEBAR_FIELD_OPACITY,
        Field::Workspace => timing::HEADER_FIELD_OPACITY,
    }
}

/// Short segments share 32 alpha batches, keeping the fade smooth without a
/// separate GPU path for every line. All geometry stays in viewport coordinates.
struct FadedPaths(Vec<PathBuilder>);
impl FadedPaths {
    fn strokes(width: f32) -> Self {
        Self(
            (0..timing::FIELD_FADE_BUCKETS)
                .map(|_| PathBuilder::stroke(px(width)))
                .collect(),
        )
    }
    fn fills() -> Self {
        Self(
            (0..timing::FIELD_FADE_BUCKETS)
                .map(|_| PathBuilder::fill())
                .collect(),
        )
    }
    fn bucket(&mut self, alpha: f32) -> Option<&mut PathBuilder> {
        if alpha <= 0. {
            return None;
        }
        let index = ((alpha.clamp(0., 1.) * timing::FIELD_FADE_BUCKETS as f32).ceil() as usize)
            .saturating_sub(1)
            .min(timing::FIELD_FADE_BUCKETS - 1);
        self.0.get_mut(index)
    }
    fn stroke(&mut self, points: &[Point<f32>], kind: Field, bounds: Bounds<Pixels>) {
        self.stroke_alpha(points, kind, bounds, 1.);
    }
    fn stroke_alpha(
        &mut self,
        points: &[Point<f32>],
        kind: Field,
        bounds: Bounds<Pixels>,
        opacity: f32,
    ) {
        let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
        for line in points.windows(2) {
            let a = line[0];
            let b = line[1];
            let count = ((b.x - a.x).hypot(b.y - a.y) / timing::FIELD_SEGMENT_LENGTH)
                .ceil()
                .max(1.) as usize;
            for i in 0..count {
                let at = |t: f32| point(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
                let start = at(i as f32 / count as f32);
                let end = at((i + 1) as f32 / count as f32);
                let mid = at((i as f32 + 0.5) / count as f32);
                let alpha = field_fade(
                    kind,
                    mid.x - ox,
                    mid.y - oy,
                    f32::from(bounds.size.width),
                    f32::from(bounds.size.height),
                );
                if let Some(path) = self.bucket(alpha * opacity) {
                    stroke_points(path, &[start, end]);
                }
            }
        }
    }
    fn dot(&mut self, center: Point<f32>, alpha: f32) {
        if let Some(path) = self.bucket(alpha) {
            let points = (0..=12)
                .map(|i| {
                    let angle = i as f32 / 12. * std::f32::consts::TAU;
                    point(
                        center.x + angle.cos() * timing::FIELD_DOT_RADIUS,
                        center.y + angle.sin() * timing::FIELD_DOT_RADIUS,
                    )
                })
                .collect::<Vec<_>>();
            stroke_points(path, &points);
            path.close();
        }
    }
    fn paint(self, window: &mut Window, color: u32, opacity: f32) {
        for (index, path) in self.0.into_iter().enumerate() {
            paint(
                window,
                path,
                color,
                opacity * (index + 1) as f32 / timing::FIELD_FADE_BUCKETS as f32,
            );
        }
    }
}

/// Four fronts converge from the viewport perimeter, then all animation stops.
fn paint_field(bounds: Bounds<Pixels>, kind: Field, progress: f32, window: &mut Window) {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    if w <= 0. || h <= 0. {
        return;
    }
    let size = field_cell_size(kind);
    let width = 3f32.sqrt() * size;
    let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let mut structure = FadedPaths::strokes(timing::FIELD_STROKE);
    let mut fronts = FadedPaths::strokes(timing::TRACE_STROKE);
    let mut cubes = FadedPaths::strokes(timing::FIELD_STROKE);
    let mut dots = FadedPaths::fills();
    for r in -1..=(h / (1.5 * size)).ceil() as i32 + 1 {
        for q in -1..=(w / width).ceil() as i32 + 1 {
            let x = q as f32 * width + if r.rem_euclid(2) == 0 { 0. } else { width / 2. };
            let y = r as f32 * 1.5 * size;
            if !field_cell_visible(kind, x, y, w, h, size) {
                continue;
            }
            let stagger = ((q * 17 + r * 31).rem_euclid(11) as f32) / 11. * timing::FIELD_STAGGER;
            let delay =
                field_depth(kind, x, y, w, h).clamp(0., 1.) * timing::FIELD_SPREAD + stagger;
            let local = ((progress - delay) / timing::CELL_DRAW_FRACTION).clamp(0., 1.);
            if local <= 0. {
                continue;
            }
            let center = point(ox + x, oy + y);
            let mut outline = hex_outline(center, size, radius::STANDARD);
            if (q + r).rem_euclid(2) == 0 {
                outline.reverse();
            }
            let revealed = trace(&outline, local);
            structure.stroke(&revealed, kind, bounds);
            if local < 1. {
                fronts.stroke(
                    &trace_range(&outline, (local - timing::TRACE_TAIL).max(0.), local),
                    kind,
                    bounds,
                );
            }
            // Three inner edges turn selected hexagons into quiet isometric cubes.
            if (q + r * 2).rem_euclid(4) == 0 {
                for index in 0..3 {
                    let angle = (index as f32 * 120. - 30.).to_radians();
                    let distance = size - radius::STANDARD / 3.;
                    let vertex = point(
                        center.x + angle.cos() * distance,
                        center.y + angle.sin() * distance,
                    );
                    cubes.stroke(&trace(&[center, vertex], local), kind, bounds);
                }
                dots.dot(center, field_fade(kind, x, y, w, h) * local);
            }
        }
    }
    let alpha = field_opacity(kind);
    structure.paint(window, LINE, alpha);
    cubes.paint(window, LINE, alpha * timing::FIELD_CUBE_OPACITY);
    dots.paint(window, DECORATIVE, alpha);
    fronts.paint(
        window,
        ACCENT,
        timing::TRACE_OPACITY * alpha / timing::FIELD_OPACITY,
    );
}

fn paint_frame(bounds: Bounds<Pixels>, kind: Frame, progress: f32, window: &mut Window) {
    if progress >= 1. {
        return;
    }
    let inset = timing::FRAME_INSET;
    let (x, y) = (
        f32::from(bounds.origin.x) + inset,
        f32::from(bounds.origin.y) + inset,
    );
    let (w, h) = (
        f32::from(bounds.size.width) - 2. * inset,
        f32::from(bounds.size.height) - 2. * inset,
    );
    if w <= 0. || h <= 0. {
        return;
    }
    let outline = rounded_polygon(
        &[
            point(x, y),
            point(x + w, y),
            point(x + w, y + h),
            point(x, y + h),
        ],
        radius::STANDARD,
    );
    let fraction = ease(progress);
    let mut path = PathBuilder::stroke(px(timing::TRACE_STROKE));
    stroke_points(&mut path, &trace_range(&outline, 0., fraction));
    let opacity = if matches!(kind, Frame::Navigation) {
        timing::NAV_TRACE_OPACITY
    } else {
        timing::TRACE_OPACITY
    };
    paint(window, path, ACCENT, opacity * (1. - progress.powi(3)));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn route_reveal_preserves_geometry_and_finishes_exactly() {
        let path = vec![point(0., 0.), point(10., 0.), point(10., 30.)];
        assert!(trace(&path, 0.).is_empty());
        assert_eq!(
            trace(&path, 0.5),
            vec![point(0., 0.), point(10., 0.), point(10., 10.)]
        );
        assert_eq!(trace(&path, 1.), path);
        assert_eq!(
            trace_range(&path, 0.25, 0.75),
            vec![point(10., 0.), point(10., 20.)]
        );
    }
    #[test]
    fn login_motion_is_bounded_and_settles_for_all_field_counts() {
        let settled = phase(Some(Instant::now()), timing::LOGIN_ENTER_MS, false);
        assert!(bloom(0.).abs() < f32::EPSILON);
        assert_eq!(bloom(settled), 1.);
        for rank in [0, 1, 4, 100, usize::MAX] {
            assert_eq!(login_content_opacity(settled, rank), 1.);
            let mut previous = 0.;
            for step in 0..=100 {
                let opacity = login_content_opacity(step as f32 / 100., rank);
                assert!((0.0..=1.0).contains(&opacity));
                assert!(opacity >= previous);
                previous = opacity;
            }
        }
    }
    #[test]
    fn reduced_motion_and_finished_phases_are_static() {
        let now = Instant::now();
        assert_eq!(phase(Some(now), 800, false), 1.);
        assert_eq!(phase(None, 800, true), 1.);
        assert_eq!(phase(Some(now - Duration::from_secs(2)), 800, true), 1.);
    }
}
