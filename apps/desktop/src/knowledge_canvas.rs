//! Native GPUI honeycomb geometry. The graph's integer cells are independent of
//! viewport size, zoom, and paint timing. Only visible cells are tessellated.
use crate::design_system::knowledge as map_style;
use crate::theme::*;
use gpui::{Bounds, PathBuilder, Pixels, Point, Window, point, px, rgb};
use me_core::{
    HexPosition, KnowledgeKind, KnowledgeMap, KnowledgeRelation, KnowledgeStatus, KnowledgeViewport,
};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

pub(super) fn world(position: HexPosition) -> Point<f32> {
    world_fraction(f64::from(position.q), f64::from(position.r))
}
pub(super) fn world_fraction(q: f64, r: f64) -> Point<f32> {
    point(
        (3f64.sqrt() * f64::from(map_style::CELL_RADIUS) * (q + r / 2.)) as f32,
        (1.5 * f64::from(map_style::CELL_RADIUS) * r) as f32,
    )
}
pub(super) fn axial(x: f32, y: f32) -> (f64, f64) {
    let r = f64::from(y) / (1.5 * f64::from(map_style::CELL_RADIUS));
    (
        f64::from(x) / (3f64.sqrt() * f64::from(map_style::CELL_RADIUS)) - r / 2.,
        r,
    )
}
type Vertex = (i64, i64);
fn corners(p: HexPosition) -> [Vertex; 6] {
    let (u, v) = (2 * i64::from(p.q) + i64::from(p.r), 3 * i64::from(p.r));
    [
        (u + 1, v - 1),
        (u + 1, v + 1),
        (u, v + 2),
        (u - 1, v + 1),
        (u - 1, v - 1),
        (u, v - 2),
    ]
}
fn neighbors((u, v): Vertex) -> [Vertex; 3] {
    if v.rem_euclid(3) == 1 {
        [(u, v - 2), (u - 1, v + 1), (u + 1, v + 1)]
    } else {
        [(u, v + 2), (u - 1, v - 1), (u + 1, v - 1)]
    }
}
fn vertex(p: Vertex) -> Point<f32> {
    point(
        p.0 as f32 * 3f32.sqrt() * map_style::CELL_RADIUS / 2.,
        p.1 as f32 * map_style::CELL_RADIUS / 2.,
    )
}
/// Borders are an infinite degree-three lattice. A* is bounded independently of
/// document length; a very distant endpoint stays available in the evidence pane.
fn route(a: HexPosition, b: HexPosition) -> Vec<Point<f32>> {
    let starts = corners(a);
    let goals = corners(b).into_iter().collect::<BTreeSet<_>>();
    let shared = starts
        .iter()
        .filter(|v| goals.contains(v))
        .copied()
        .collect::<Vec<_>>();
    if shared.len() == 2 {
        return vec![
            toward(vertex(shared[0]), vertex(shared[1]), radius::STANDARD),
            toward(vertex(shared[1]), vertex(shared[0]), radius::STANDARD),
        ];
    }
    let heuristic = |p: Vertex| {
        goals
            .iter()
            .map(|g| ((p.0 - g.0).abs() + (p.1 - g.1).abs()) / 3)
            .min()
            .unwrap_or(0)
    };
    let mut open = BinaryHeap::new();
    let mut cost = BTreeMap::new();
    let mut previous = BTreeMap::new();
    for p in starts {
        cost.insert(p, 0i64);
        open.push(Reverse((heuristic(p), 0i64, p)));
    }
    for _ in 0..8192 {
        let Some(Reverse((_, distance, p))) = open.pop() else {
            break;
        };
        if cost.get(&p) != Some(&distance) {
            continue;
        }
        if goals.contains(&p) {
            let mut path = vec![p];
            let mut p = p;
            while let Some(parent) = previous.get(&p).copied() {
                path.push(parent);
                p = parent;
            }
            path.reverse();
            return path.into_iter().map(vertex).collect();
        }
        for n in neighbors(p) {
            let next = distance + 1;
            if cost.get(&n).is_none_or(|c| next < *c) {
                cost.insert(n, next);
                previous.insert(n, p);
                open.push(Reverse((next + heuristic(n), next, n)));
            }
        }
    }
    Vec::new()
}
pub(super) fn routes(
    graph: &KnowledgeMap,
    selected: Option<&str>,
) -> Vec<(usize, Vec<Point<f32>>)> {
    let positions = graph
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.position))
        .collect::<BTreeMap<_, _>>();
    graph
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| Some(e.from.as_str()) == selected || Some(e.to.as_str()) == selected)
        .filter_map(|(i, e)| {
            Some((
                i,
                route(
                    *positions.get(e.from.as_str())?,
                    *positions.get(e.to.as_str())?,
                ),
            ))
        })
        .collect()
}
fn toward(a: Point<f32>, b: Point<f32>, distance: f32) -> Point<f32> {
    let length = (b.x - a.x).hypot(b.y - a.y);
    if length == 0. {
        return a;
    }
    let t = (distance / length).min(0.5);
    point(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}
fn rounded(builder: &mut PathBuilder, points: &[Point<f32>], closed: bool, bend: f32) {
    if points.len() < 2 {
        return;
    }
    if closed {
        for (i, p) in points.iter().copied().enumerate() {
            let a = toward(p, points[(i + points.len() - 1) % points.len()], bend);
            let b = toward(p, points[(i + 1) % points.len()], bend);
            if i == 0 {
                builder.move_to(point(px(a.x), px(a.y)));
            } else {
                builder.line_to(point(px(a.x), px(a.y)));
            }
            builder.curve_to(point(px(b.x), px(b.y)), point(px(p.x), px(p.y)));
        }
        builder.close();
    } else {
        builder.move_to(point(px(points[0].x), px(points[0].y)));
        for i in 1..points.len() - 1 {
            let p = points[i];
            let a = toward(p, points[i - 1], bend);
            let b = toward(p, points[i + 1], bend);
            builder.line_to(point(px(a.x), px(a.y)));
            builder.curve_to(point(px(b.x), px(b.y)), point(px(p.x), px(p.y)));
        }
        let p = points[points.len() - 1];
        builder.line_to(point(px(p.x), px(p.y)));
    }
}
fn paint_path(window: &mut Window, builder: PathBuilder, color: u32, opacity: f32) {
    if let Ok(path) = builder.build() {
        let mut c = rgb(color);
        c.a = opacity;
        window.paint_path(path, c);
    }
}
fn hex(center: Point<f32>, radius: f32) -> Vec<Point<f32>> {
    (0..6)
        .map(|i| {
            let angle = (i as f32 * 60. - 30.).to_radians();
            point(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            )
        })
        .collect()
}
pub(super) fn paint(
    bounds: Bounds<Pixels>,
    graph: &KnowledgeMap,
    view: &KnowledgeViewport,
    visible: &[usize],
    paths: &[(usize, Vec<Point<f32>>)],
    matching: Option<&BTreeSet<String>>,
    window: &mut Window,
) {
    let zoom = view.zoom as f32;
    let center = world_fraction(view.center_q, view.center_r);
    let origin = point(
        f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2. - center.x * zoom,
        f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2. - center.y * zoom,
    );
    let radius = map_style::CELL_RADIUS * zoom;
    let width = 3f32.sqrt() * radius;
    let h = f32::from(bounds.size.height);
    let w = f32::from(bounds.size.width);
    let relative_origin = point(
        origin.x - f32::from(bounds.origin.x),
        origin.y - f32::from(bounds.origin.y),
    );
    let rmin = ((-relative_origin.y - radius) / (1.5 * radius)).floor() as i32;
    let rmax = ((h - relative_origin.y + radius) / (1.5 * radius)).ceil() as i32;
    let mut grid = PathBuilder::stroke(px(map_style::BORDER));
    for r in rmin..=rmax {
        let qmin = ((-relative_origin.x - width) / width - r as f32 / 2.).floor() as i32;
        let qmax = ((w - relative_origin.x + width) / width - r as f32 / 2.).ceil() as i32;
        for q in qmin..=qmax {
            let p = world(HexPosition { q, r });
            let p = point(origin.x + p.x * zoom, origin.y + p.y * zoom);
            rounded(&mut grid, &hex(p, radius), true, radius::STANDARD * zoom);
        }
    }
    paint_path(window, grid, LINE, map_style::GRID_OPACITY);
    for index in visible {
        let n = &graph.nodes[*index];
        let p = world(n.position);
        let p = point(origin.x + p.x * zoom, origin.y + p.y * zoom);
        let selected = view.selected_node.as_deref() == Some(&n.id);
        let source = matches!(
            n.kind,
            KnowledgeKind::Document | KnowledgeKind::Credential | KnowledgeKind::Entity
        );
        let opacity = if matching.is_some_and(|m| !m.contains(&n.id)) && !selected {
            map_style::DIM_OPACITY
        } else {
            1.
        };
        let mut fill = PathBuilder::fill();
        rounded(&mut fill, &hex(p, radius), true, radius::STANDARD * zoom);
        paint_path(
            window,
            fill,
            if source {
                KNOWLEDGE_SOURCE
            } else if selected {
                HOVER
            } else {
                SURFACE
            },
            opacity,
        );
        let mut border = PathBuilder::stroke(px(map_style::BORDER));
        rounded(&mut border, &hex(p, radius), true, radius::STANDARD * zoom);
        paint_path(
            window,
            border,
            if n.status.needs_review() || n.status == KnowledgeStatus::Conflicting {
                WARNING
            } else {
                DECORATIVE
            },
            opacity,
        );
        if selected {
            let mut inner = PathBuilder::stroke(px(map_style::BORDER));
            rounded(
                &mut inner,
                &hex(p, radius - space::XS * zoom),
                true,
                radius::STANDARD * zoom,
            );
            paint_path(window, inner, ACCENT, 1.);
        }
    }
    for (index, path) in paths {
        if path.len() < 2 {
            continue;
        }
        let e = &graph.edges[*index];
        let uncertain = matches!(
            e.relation,
            KnowledgeRelation::Context | KnowledgeRelation::Unconfirmed
        );
        let color = if uncertain { WARNING } else { ACCENT };
        let points = path
            .iter()
            .map(|p| point(origin.x + p.x * zoom, origin.y + p.y * zoom))
            .collect::<Vec<_>>();
        let mut halo = PathBuilder::stroke(px(map_style::HALO_WIDTH));
        rounded(&mut halo, &points, false, radius::STANDARD * zoom);
        paint_path(window, halo, color, map_style::HALO_OPACITY);
        let mut line = PathBuilder::stroke(px(map_style::CONNECTION_WIDTH));
        if uncertain {
            line = line.dash_array(&[px(space::XS), px(space::XS)]);
        }
        rounded(&mut line, &points, false, radius::STANDARD * zoom);
        paint_path(window, line, color, 1.);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_stay_on_hex_borders_and_neighbors_have_a_real_shared_edge() {
        for q in -5..6 {
            for r in -5..6 {
                let a = HexPosition::default();
                let b = HexPosition { q, r };
                if a == b {
                    continue;
                }
                let path = route(a, b);
                assert!(path.len() >= 2);
                if a.distance(b) == 1 {
                    assert_eq!(path.len(), 2);
                } else {
                    for p in path.windows(2) {
                        let d = (p[1].x - p[0].x).hypot(p[1].y - p[0].y);
                        assert!((d - map_style::CELL_RADIUS).abs() < 0.02);
                    }
                }
            }
        }
    }
    #[test]
    fn axial_camera_conversion_round_trips() {
        let (q, r) = axial(world_fraction(12.5, -9.25).x, world_fraction(12.5, -9.25).y);
        assert!((q - 12.5).abs() < 0.0001);
        assert!((r + 9.25).abs() < 0.0001);
    }
}
