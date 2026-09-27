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
    /// Locked screens: a wide perimeter band around the account form.
    Identity,
    /// Unlocked window: one thin ring shared by the sidebar and workspace.
    Window,
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
/// `ambient` is the throttled ambient clock in seconds; `None` holds the lattice still.
pub fn field(kind: Field, enabled: bool, ambient: Option<f32>) -> AnyElement {
    let body = div().absolute().inset_0().overflow_hidden();
    if !enabled {
        return body.child(field_at(kind, 1., None)).into_any_element();
    }
    body.with_animation(
        ("honeycomb-field", kind as usize),
        Animation::new(Duration::from_millis(timing::FIELD_ENTER_MS)),
        move |el, t| el.child(field_at(kind, t, ambient)),
    )
    .into_any_element()
}

/// Busy-only, clockwise pulsing light moving through the perimeter lattice.
/// There is no timer or minimum duration between the operation and its next step.
pub fn activity(kind: Field, enabled: bool, ambient: Option<f32>) -> AnyElement {
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
                        paint_activity(bounds, kind, t, ambient, window);
                    },
                )
                .absolute()
                .size_full(),
            )
        },
    )
    .into_any_element()
}

/// Advances the ambient clock by one throttled tick. Long stalls are capped so a
/// resumed window continues smoothly; every ambient period divides the cycle.
pub fn advance_ambient(seconds: f32, elapsed: Duration) -> f32 {
    (seconds + elapsed.as_secs_f32().min(timing::AMBIENT_MAX_STEP))
        .rem_euclid(timing::AMBIENT_CYCLE)
}

/// A slow Lissajous drift of the whole lattice, a few pixels at most.
fn drift(ambient: Option<f32>) -> Point<f32> {
    let Some(seconds) = ambient else {
        return point(0., 0.);
    };
    let tau = std::f32::consts::TAU;
    point(
        timing::DRIFT_AMPLITUDE * (tau * seconds / timing::DRIFT_PERIOD_X).sin(),
        timing::DRIFT_AMPLITUDE * 0.75 * (tau * seconds / timing::DRIFT_PERIOD_Y).sin(),
    )
}

/// A diagonal band of light sweeping across the embossed highlights.
fn shimmer(ambient: Option<f32>, at: Point<f32>) -> f32 {
    ambient.map_or(1., |seconds| {
        let wave = 0.5
            + 0.5
                * (std::f32::consts::TAU
                    * ((at.x + at.y) / timing::SHIMMER_WAVELENGTH
                        - seconds / timing::SHIMMER_PERIOD))
                    .cos();
        1. - timing::SHIMMER_DEPTH * (1. - wave)
    })
}

/// Brightness of the activity wave; `progress` is its lap, so the pulse loops seamlessly.
fn pulse(progress: f32) -> f32 {
    let wave = 0.5 - 0.5 * (std::f32::consts::TAU * timing::ACTIVITY_PULSES * progress).cos();
    timing::ACTIVITY_PULSE_FLOOR + (1. - timing::ACTIVITY_PULSE_FLOOR) * wave
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

fn paint_activity(
    bounds: Bounds<Pixels>,
    kind: Field,
    progress: f32,
    ambient: Option<f32>,
    window: &mut Window,
) {
    let mask = Mask::of(kind, window);
    let size = field_cell_size(kind);
    let pulse = pulse(progress);
    let mut lights = FadedPaths::strokes(timing::TRACE_STROKE);
    let mut glow = FadedPaths::fills();
    // Angles are measured around the window center, so the wave laps the whole
    // window even when the sidebar and workspace paint separate regions.
    for (_, _, center) in cells(mask, bounds, drift(ambient)) {
        let position = (((center.y / mask.h - 0.5).atan2(center.x / mask.w - 0.5)
            + std::f32::consts::FRAC_PI_2)
            / std::f32::consts::TAU)
            .rem_euclid(1.);
        let distance = (progress - position).rem_euclid(1.);
        if distance >= timing::ACCOUNT_ACTIVITY_TAIL {
            continue;
        }
        let strength = (distance / timing::ACCOUNT_ACTIVITY_TAIL * std::f32::consts::PI)
            .sin()
            .powi(2)
            * pulse;
        let outline = hex_outline(center, size, radius::STANDARD);
        lights.stroke_alpha(&outline, mask, strength);
        glow.fill(&outline, mask.fade(center) * strength);
    }
    let emphasis = field_opacity(kind) / timing::FIELD_OPACITY;
    glow.paint(window, ACCENT, timing::ACTIVITY_FILL_OPACITY * emphasis);
    lights.paint(window, ACCENT, timing::TRACE_OPACITY * emphasis);
}

/// Also used by the synthetic gallery to inspect exact drawing phases.
pub fn field_at(kind: Field, progress: f32, ambient: Option<f32>) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| paint_field(bounds, kind, progress, ambient, window),
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
                        paint(window, fill, if active { ACCENT } else { HOVER }, 1.);
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
                    .text_color(rgb(if active { SURFACE } else { MUTED }))
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

/// All drawing layers, including busy highlights, share one continuous mask in
/// window coordinates, fading inward from every window edge.
#[derive(Clone, Copy)]
struct Mask {
    kind: Field,
    w: f32,
    h: f32,
}
impl Mask {
    fn of(kind: Field, window: &Window) -> Self {
        let viewport = window.viewport_size();
        Self {
            kind,
            w: f32::from(viewport.width),
            h: f32::from(viewport.height),
        }
    }
    fn edge(self, at: Point<f32>) -> f32 {
        at.x.min(self.w - at.x).min(at.y).min(self.h - at.y)
    }
    fn depth(self, at: Point<f32>) -> f32 {
        self.edge(at).max(0.) / field_band(self.kind)
    }
    fn fade(self, at: Point<f32>) -> f32 {
        let t = self.depth(at).clamp(0., 1.);
        1. - t * t * (3. - 2. * t)
    }
}

/// Lattice cells that can touch `bounds`, as (column, row, window-space center).
/// The lattice is anchored to the window rather than the painting element, so
/// cells continue seamlessly across the sidebar border.
fn cells(
    mask: Mask,
    bounds: Bounds<Pixels>,
    offset: Point<f32>,
) -> impl Iterator<Item = (i32, i32, Point<f32>)> {
    let size = field_cell_size(mask.kind);
    let width = 3f32.sqrt() * size;
    let (left, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (right, bottom) = (
        left + f32::from(bounds.size.width),
        top + f32::from(bounds.size.height),
    );
    let rows = ((top - size - offset.y) / (1.5 * size)).floor() as i32 - 1
        ..=((bottom + size - offset.y) / (1.5 * size)).ceil() as i32 + 1;
    let columns = ((left - size - offset.x) / width).floor() as i32 - 1
        ..=((right + size - offset.x) / width).ceil() as i32 + 1;
    let empty = right <= left || bottom <= top;
    rows.flat_map(move |r| columns.clone().map(move |q| (q, r)))
        .filter(move |_| !empty)
        .filter_map(move |(q, r)| {
            let shift = if r.rem_euclid(2) == 0 { 0. } else { width / 2. };
            let center = point(
                q as f32 * width + shift + offset.x,
                r as f32 * 1.5 * size + offset.y,
            );
            // Keep cells whose outer strokes still meet the band or the region,
            // even when their center is outside. Center culling truncates hexagons.
            let touches = center.x + size >= left
                && center.x - size <= right
                && center.y + size >= top
                && center.y - size <= bottom;
            (touches && mask.edge(center) <= field_band(mask.kind) + size).then_some((q, r, center))
        })
}
fn field_band(kind: Field) -> f32 {
    match kind {
        Field::Identity => timing::FIELD_BAND,
        Field::Window => timing::WINDOW_FIELD_BAND,
    }
}
fn field_cell_size(kind: Field) -> f32 {
    match kind {
        Field::Identity => timing::FIELD_CELL_RADIUS,
        Field::Window => timing::WINDOW_CELL_RADIUS,
    }
}
fn field_opacity(kind: Field) -> f32 {
    match kind {
        Field::Identity => timing::FIELD_OPACITY,
        Field::Window => timing::WINDOW_FIELD_OPACITY,
    }
}
fn shifted(points: &[Point<f32>], by: f32) -> Vec<Point<f32>> {
    points.iter().map(|p| point(p.x + by, p.y + by)).collect()
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
    fn stroke(&mut self, points: &[Point<f32>], mask: Mask) {
        self.stroke_alpha(points, mask, 1.);
    }
    /// Whole-cell strokes at one alpha: cheap for the faint emboss layers.
    fn outline(&mut self, points: &[Point<f32>], alpha: f32) {
        if points.len() > 1
            && let Some(path) = self.bucket(alpha)
        {
            stroke_points(path, points);
        }
    }
    fn fill(&mut self, points: &[Point<f32>], alpha: f32) {
        if points.len() > 2
            && let Some(path) = self.bucket(alpha)
        {
            stroke_points(path, points);
            path.close();
        }
    }
    fn stroke_alpha(&mut self, points: &[Point<f32>], mask: Mask, opacity: f32) {
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
                if let Some(path) = self.bucket(mask.fade(mid) * opacity) {
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

/// Four fronts converge from the viewport perimeter. Afterwards only the
/// throttled ambient clock moves the lattice, and only while it is running.
fn paint_field(
    bounds: Bounds<Pixels>,
    kind: Field,
    progress: f32,
    ambient: Option<f32>,
    window: &mut Window,
) {
    let mask = Mask::of(kind, window);
    let size = field_cell_size(kind);
    let mut shadows = FadedPaths::strokes(timing::FIELD_SHADOW_STROKE);
    let mut structure = FadedPaths::strokes(timing::FIELD_STROKE);
    let mut highlights = FadedPaths::strokes(timing::FIELD_STROKE);
    let mut fronts = FadedPaths::strokes(timing::TRACE_STROKE);
    let mut cubes = FadedPaths::strokes(timing::FIELD_STROKE);
    let mut dots = FadedPaths::fills();
    for (q, r, center) in cells(mask, bounds, drift(ambient)) {
        let stagger = ((q * 17 + r * 31).rem_euclid(11) as f32) / 11. * timing::FIELD_STAGGER;
        let delay = mask.depth(center).clamp(0., 1.) * timing::FIELD_SPREAD + stagger;
        let local = ((progress - delay) / timing::CELL_DRAW_FRACTION).clamp(0., 1.);
        if local <= 0. {
            continue;
        }
        let mut outline = hex_outline(center, size, radius::STANDARD);
        if (q + r).rem_euclid(2) == 0 {
            outline.reverse();
        }
        let revealed = trace(&outline, local);
        let fade = mask.fade(center);
        // Embossed depth: a soft drop line below-right and a light edge above-left.
        shadows.outline(&shifted(&revealed, timing::FIELD_SHADOW_OFFSET), fade);
        structure.stroke(&revealed, mask);
        highlights.outline(
            &shifted(&revealed, -timing::FIELD_HIGHLIGHT_OFFSET),
            fade * shimmer(ambient, center),
        );
        if local < 1. {
            fronts.stroke(
                &trace_range(&outline, (local - timing::TRACE_TAIL).max(0.), local),
                mask,
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
                cubes.stroke(&trace(&[center, vertex], local), mask);
            }
            dots.dot(center, fade * local);
        }
    }
    let alpha = field_opacity(kind);
    shadows.paint(window, INK, alpha * timing::FIELD_SHADOW_OPACITY);
    structure.paint(window, LINE, alpha);
    highlights.paint(window, SURFACE, alpha * timing::FIELD_HIGHLIGHT_OPACITY);
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
    fn activity_pulse_is_bounded_and_loops_with_each_lap() {
        let mut lowest = f32::MAX;
        let mut highest = f32::MIN;
        for step in 0..=1000 {
            let value = pulse(step as f32 / 1000.);
            lowest = lowest.min(value);
            highest = highest.max(value);
        }
        assert!((lowest - timing::ACTIVITY_PULSE_FLOOR).abs() < 1e-3);
        assert!((highest - 1.).abs() < 1e-3);
        assert!((pulse(0.) - pulse(1.)).abs() < 1e-5);
    }
    #[test]
    fn ambient_motion_is_small_still_when_disabled_and_wraps_seamlessly() {
        assert_eq!(drift(None), point(0., 0.));
        assert_eq!(drift(Some(0.)), point(0., 0.));
        assert_eq!(shimmer(None, point(40., 90.)), 1.);
        for period in [
            timing::DRIFT_PERIOD_X,
            timing::DRIFT_PERIOD_Y,
            timing::SHIMMER_PERIOD,
        ] {
            assert_eq!((timing::AMBIENT_CYCLE / period).fract(), 0.);
        }
        let end = drift(Some(timing::AMBIENT_CYCLE));
        assert!(end.x.abs() < 1e-3 && end.y.abs() < 1e-3);
        for step in 0..=720 {
            let seconds = step as f32 / 10.;
            let offset = drift(Some(seconds));
            assert!(offset.x.hypot(offset.y) <= timing::DRIFT_AMPLITUDE * 1.25 + 1e-4);
            let light = shimmer(Some(seconds), point(120., 80.));
            assert!((1. - timing::SHIMMER_DEPTH - 1e-4..=1. + 1e-4).contains(&light));
        }
        let almost = timing::AMBIENT_CYCLE - 0.01;
        let wrapped = advance_ambient(almost, Duration::from_millis(66));
        assert!((wrapped - 0.056).abs() < 1e-3);
        // A stalled or long-inactive window resumes with a single small step.
        assert_eq!(
            advance_ambient(1., Duration::from_secs(30)),
            1. + timing::AMBIENT_MAX_STEP
        );
    }
    #[test]
    fn window_lattice_is_continuous_across_sidebar_and_workspace() {
        use gpui::{Bounds, size};
        let mask = Mask {
            kind: Field::Window,
            w: 1120.,
            h: 820.,
        };
        let region = |x: f32, w: f32| Bounds::new(point(px(x), px(0.)), size(px(w), px(820.)));
        let offset = drift(Some(7.3));
        let whole = cells(mask, region(0., 1120.), offset).collect::<Vec<_>>();
        let sidebar = cells(mask, region(0., 248.), offset).collect::<Vec<_>>();
        let workspace = cells(mask, region(248., 872.), offset).collect::<Vec<_>>();
        assert!(!whole.is_empty());
        // Every cell painted by a region is the identical window-anchored cell,
        // and together the regions cover the whole-window ring.
        for cell in sidebar.iter().chain(&workspace) {
            assert!(whole.contains(cell));
        }
        for cell in &whole {
            assert!(sidebar.contains(cell) || workspace.contains(cell));
        }
        let shared = sidebar.iter().filter(|c| workspace.contains(c)).count();
        assert!(shared > 0, "cells straddling the border are drawn by both");
        // The ring stays thin: no cell sits deep inside the content area.
        for (_, _, center) in &whole {
            assert!(mask.edge(*center) <= timing::WINDOW_FIELD_BAND + timing::WINDOW_CELL_RADIUS);
        }
        let empty = Bounds::new(point(px(10.), px(10.)), size(px(0.), px(0.)));
        assert_eq!(cells(mask, empty, offset).count(), 0);
    }
    #[test]
    fn reduced_motion_and_finished_phases_are_static() {
        let now = Instant::now();
        assert_eq!(phase(Some(now), 800, false), 1.);
        assert_eq!(phase(None, 800, true), 1.);
        assert_eq!(phase(Some(now - Duration::from_secs(2)), 800, true), 1.);
    }
}
