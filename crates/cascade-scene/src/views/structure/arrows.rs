//! Arrow mode: transitions drawn as arrows instead of pills.
//!
//! With `ViewState::transition_pills` off, the drafter puts a tiny junction
//! node ([`Painter::junction`]) where each pill would be, with the same
//! ports, and the same two edges: state → junction (West, carrying the
//! label `trigger [guard]`, so the layout reserves its room) and junction →
//! state (East, with the arrowhead). Layout, gutters, wiring placement,
//! stability and pins work exactly as with pills, keyed by the same
//! transition keys.
//!
//! After layout and decoration, [`fold`] turns that into the picture:
//!
//! ```text
//! state ──label──▶ junction ──▶ state      (two edges, one junction node)
//!            fold │
//!                 ▼
//! state ──label──•───────────▶ state       (one edge; the junction is gone)
//!                ↑ wires (emits, fires, triggers, links) end on the arrow, with a dot
//! ```
//!
//! - The two edges become one polyline running through the junction, with
//!   the arrowhead only at the target state. It keeps the transition's hit
//!   target and meta, so clicking it selects the transition, and emphasis
//!   (selection, focus, active) is its outline weight.
//! - Every other edge ending on a junction is snapped onto the arrow (its
//!   vertical end segment extended to the arrow's line) and gets a dot in
//!   the arrow's color where it meets it.
//! - Findings that would badge the pill turn the arrow red and put the badge
//!   (as overlays) just above the junction. A search match keeps the
//!   junction's halo, so the hit is still marked on the arrow.
//! - Junction nodes are removed from the scene, so nothing invisible is
//!   left to hit test, badge or measure, and build mode draws no connect
//!   handles on transitions: the app treats the whole arrow as the handle.

use cascade_core::{ElementRef, Severity};
use cascade_layout::{Point, Rect};

use crate::scene::{Arrow, Badge, Dash, EdgeKind, FontWeight, HitTarget, Label, Layer, Overlay, Scene, Stroke};
use crate::views::draft::{EdgeInfo, Meta};
use crate::views::style::Painter;

/// Gap between layers in arrow mode.
pub(super) const LAYER_SPACING: f32 = 32.0;
/// Radius of the dot where a wire meets an arrow.
pub(crate) const DOT_RADIUS: f32 = 3.0;
/// How far (in steps of 6 units) a self-link loop's label may move out
/// to find a clear spot.
const LOOP_LABEL_STEPS: usize = 12;
/// Coordinates closer than this count as equal.
const EPS: f32 = 0.01;

/// Whether a node stands for a transition. In arrow mode the only such
/// nodes are junctions: states, stubs, collapsed machines and wiring nodes
/// stand for other elements first.
fn is_junction(meta: &Meta) -> bool {
    matches!(meta.elements.first(), Some(ElementRef::Transition(_)))
}

/// One transition's two edges, by scene edge index.
#[derive(Clone, Copy, Debug, Default)]
struct Pair {
    into: Option<usize>,
    out_of: Option<usize>,
}

/// Fold every junction of a decorated arrow-mode scene into its arrow.
/// `nodes` and `edges` are aligned with `scene.nodes` and `scene.edges` and
/// stay aligned.
pub(super) fn fold(scene: &mut Scene, nodes: &mut Vec<Meta>, edges: &mut Vec<EdgeInfo>, painter: &Painter<'_>) {
    let junction: Vec<bool> = nodes.iter().map(is_junction).collect();
    if !junction.contains(&true) {
        return;
    }
    let is_j = |i: usize| junction.get(i).copied().unwrap_or(false);

    let mut pairs = vec![Pair::default(); junction.len()];
    for (i, (edge, info)) in scene.edges.iter().zip(edges.iter()).enumerate() {
        let Some((a, b)) = info.ends else { continue };
        if edge.kind != EdgeKind::Transition {
            continue;
        }
        if is_j(b) && !is_j(a) {
            pairs[b].into = Some(i);
        } else if is_j(a) && !is_j(b) {
            pairs[a].out_of = Some(i);
        }
    }
    let in_pair = |i: usize| pairs.iter().any(|p| p.into == Some(i) || p.out_of == Some(i));

    // Merge each pair into one arrow.
    let mut removed_edges = vec![false; scene.edges.len()];
    let mut overlays = Vec::new();
    for (j, pair) in pairs.iter().enumerate() {
        let (into, out_of) = match (pair.into, pair.out_of) {
            (Some(into), Some(out_of)) => (into, out_of),
            // One side was cut by hide mode: the edge left keeps the head.
            (Some(only), None) | (None, Some(only)) => {
                scene.edges[only].arrow = Arrow::End;
                continue;
            }
            (None, None) => continue,
        };
        let tail = scene.edges[out_of].clone();
        let arrow = &mut scene.edges[into];
        arrow.points = join(&arrow.points, &tail.points);
        arrow.arrow = Arrow::End;
        arrow.label = arrow.label.take().or(tail.label);
        removed_edges[out_of] = true;
        let ends = edges[into].ends.zip(edges[out_of].ends).map(|((a, _), (_, b))| (a, b));
        edges[into].ends = ends;

        let node = &scene.nodes[j];
        if let Some(badge) = &node.badge {
            arrow.stroke.color = painter.theme.finding;
            overlays.extend(badge_overlays(painter, badge, node.rect, &arrow.target, arrow.opacity));
        }
    }

    // Wires ending on a junction: onto the arrow's line, with a dot.
    let line = |j: usize, scene: &Scene| {
        let rect = scene.nodes[j].rect;
        let arrow = pairs[j].into.or(pairs[j].out_of).map(|i| scene.edges[i].points.clone()).unwrap_or_default();
        (rect, arrow)
    };
    let mut dots: Vec<(Point, usize)> = Vec::new();
    for (i, info) in edges.iter().enumerate() {
        let Some((a, b)) = info.ends else { continue };
        if in_pair(i) {
            continue;
        }
        for (j, end) in [(a, End::First), (b, End::Last)] {
            if is_j(j) {
                let (rect, arrow) = line(j, scene);
                dots.push((snap(&mut scene.edges[i].points, end, rect, &arrow), j));
            }
        }
    }

    // Dots, in the color and opacity of the arrow they sit on.
    let arrow_of = |j: usize| pairs[j].into.or(pairs[j].out_of).map(|i| &scene.edges[i]);
    for (at, j) in dots {
        let (color, opacity) = arrow_of(j).map_or((painter.theme.text_muted, 1.0), |e| (e.stroke.color, e.opacity));
        overlays.push(Overlay::Rect {
            rect: Rect::new(at.x - DOT_RADIUS, at.y - DOT_RADIUS, 2.0 * DOT_RADIUS, 2.0 * DOT_RADIUS),
            fill: Some(color),
            stroke: None,
            radius: DOT_RADIUS,
            opacity,
            layer: Layer::Over,
            target: HitTarget::None,
        });
    }
    scene.overlays.extend(overlays);

    // A link from a transition to itself is a small hand-drawn loop on the
    // junction; its label was placed clear of nodes only, and the arrow's
    // own label now sits nearby. Move it clear of labels too.
    for (i, info) in edges.iter().enumerate() {
        if let Some((a, b)) = info.ends
            && a == b
            && is_j(a)
        {
            clear_loop_label(scene, i, &junction, painter);
        }
    }

    // Drop the folded edges and the junctions, keeping metas aligned.
    let mut index = 0;
    scene.edges.retain(|_| {
        let keep = !removed_edges[index];
        index += 1;
        keep
    });
    let mut index = 0;
    edges.retain(|_| {
        let keep = !removed_edges[index];
        index += 1;
        keep
    });
    let remap: Vec<Option<usize>> = {
        let mut next = 0;
        junction
            .iter()
            .map(|&j| {
                if j {
                    None
                } else {
                    next += 1;
                    Some(next - 1)
                }
            })
            .collect()
    };
    for info in edges.iter_mut() {
        info.ends =
            info.ends.and_then(|(a, b)| Some((remap.get(a).copied().flatten()?, remap.get(b).copied().flatten()?)));
    }
    let mut index = 0;
    scene.nodes.retain(|_| {
        let keep = !junction[index];
        index += 1;
        keep
    });
    let mut index = 0;
    nodes.retain(|_| {
        let keep = !junction[index];
        index += 1;
        keep
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    First,
    Last,
}

/// Move a wire's end on junction `rect` onto the arrow's line through the
/// junction (`arrow`, the merged polyline), extending a vertical end
/// segment (North and South ports). Returns where the wire now meets the
/// arrow.
fn snap(points: &mut [Point], end: End, rect: Rect, arrow: &[Point]) -> Point {
    let n = points.len();
    if n < 2 {
        return points.first().copied().unwrap_or(rect.center());
    }
    let (at, next) = match end {
        End::First => (0, 1),
        End::Last => (n - 1, n - 2),
    };
    if (points[at].x - points[next].x).abs() < EPS {
        let y = line_y(arrow, points[at], rect).unwrap_or(rect.center().y);
        // Only extend toward the line, never back across the wire.
        if (y - points[at].y) * (points[at].y - points[next].y) >= 0.0 {
            points[at].y = y;
        }
    }
    points[at]
}

/// Where the arrow crosses the vertical through `end.x` inside the
/// junction's rect (with a little slack): the crossing nearest the wire's
/// `end`, so the wire stops at the first arrow run it meets.
fn line_y(arrow: &[Point], end: Point, rect: Rect) -> Option<f32> {
    let slack = rect.size.height / 2.0;
    let x = end.x;
    arrow
        .windows(2)
        .filter_map(|w| {
            let (a, b) = (w[0], w[1]);
            let (lo, hi) = (a.x.min(b.x), a.x.max(b.x));
            if (b.x - a.x).abs() < EPS || x < lo - EPS || x > hi + EPS {
                return None;
            }
            let y = a.y + (b.y - a.y) * (x - a.x) / (b.x - a.x);
            (y >= rect.top() - slack && y <= rect.bottom() + slack).then_some(y)
        })
        .min_by(|a, b| (a - end.y).abs().total_cmp(&(b - end.y).abs()))
}

/// Put edge `i`'s label (a self-link loop's) at the first spot around its
/// loop that clears every drawn node and every other edge label, unless it
/// already does.
fn clear_loop_label(scene: &mut Scene, i: usize, junction: &[bool], painter: &Painter<'_>) {
    let Some(label) = &scene.edges[i].label else { return };
    let size = (painter.text_width(&label.text, label.font_size), painter.line_height(label.font_size));
    let rect_at = |origin: Point| Rect::new(origin.x, origin.y, size.0, size.1);
    let label_rect = |l: &Label| {
        Rect::new(l.origin.x, l.origin.y, painter.text_width(&l.text, l.font_size), painter.line_height(l.font_size))
    };
    let clear = |r: Rect| {
        let nodes = scene.nodes.iter().enumerate().filter(|(n, _)| !junction.get(*n).copied().unwrap_or(false));
        nodes.clone().all(|(_, n)| !n.rect.intersects(&r))
            && nodes.flat_map(|(_, n)| &n.labels).all(|l| !label_rect(l).intersects(&r))
            && scene
                .edges
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .filter_map(|(_, e)| e.label.as_ref())
                .all(|l| !label_rect(l).intersects(&r))
    };
    if clear(rect_at(label.origin)) {
        return;
    }
    let points = &scene.edges[i].points;
    let Some(first) = points.first() else { return };
    let (mut left, mut top, mut right, mut bottom) = (first.x, first.y, first.x, first.y);
    for p in points {
        left = left.min(p.x);
        top = top.min(p.y);
        right = right.max(p.x);
        bottom = bottom.max(p.y);
    }
    let (w, h) = size;
    let mid = (left + right) / 2.0;
    // Beside, above or below the loop, moving further out in small steps
    // until a spot is clear.
    let mut candidates = (0..=LOOP_LABEL_STEPS).flat_map(|k| {
        let d = k as f32 * 6.0;
        [
            Point::new(mid - w / 2.0, top - h - 2.0 - d),
            Point::new(right + 4.0 + d, top - h / 2.0),
            Point::new(left - w - 4.0 - d, top - h / 2.0),
            Point::new(mid - w / 2.0, bottom + 4.0 + d),
            Point::new(right + 4.0, top - h - 2.0 - d),
            Point::new(left - w - 4.0, top - h - 2.0 - d),
            Point::new(right + 4.0, bottom + 2.0 + d),
            Point::new(left - w - 4.0, bottom + 2.0 + d),
        ]
    });
    if let Some(origin) = candidates.find(|&c| clear(rect_at(c)))
        && let Some(label) = &mut scene.edges[i].label
    {
        label.origin = origin;
    }
}

/// The arrow through a junction: the way in, then the way out, dropping
/// the junction's two side points where the line runs straight through.
fn join(into: &[Point], out_of: &[Point]) -> Vec<Point> {
    let mut points: Vec<Point> = into.iter().chain(out_of).copied().collect();
    points.dedup_by(|b, a| a.distance(*b) < EPS);
    let mut i = 1;
    while i + 1 < points.len() {
        if collinear(points[i - 1], points[i], points[i + 1]) {
            points.remove(i);
        } else {
            i += 1;
        }
    }
    points
}

/// Whether `b` lies on the straight run from `a` to `c` (between them).
fn collinear(a: Point, b: Point, c: Point) -> bool {
    let cross = (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
    let dot = (b.x - a.x) * (c.x - b.x) + (b.y - a.y) * (c.y - b.y);
    cross.abs() < EPS && dot >= 0.0
}

/// A finding badge for an arrow, drawn as overlays (edges have no badge
/// field): a circle just above and right of the junction with the count,
/// filled red for errors, dashed for notes, like a node's badge.
fn badge_overlays(
    painter: &Painter<'_>,
    badge: &Badge,
    junction: Rect,
    target: &HitTarget,
    opacity: f32,
) -> [Overlay; 2] {
    let theme = painter.theme;
    let r = badge.radius;
    let center = Point::new(junction.right() + r * 0.5, junction.top() - r - 2.0);
    let (fill, ink) = match badge.severity {
        Severity::Error => (theme.finding, theme.background),
        Severity::Warning | Severity::Info => (theme.background, theme.finding),
    };
    let dash = if badge.severity == Severity::Info { Dash::Dashed { on: 2.0, off: 2.0 } } else { Dash::Solid };
    let text = badge.count.to_string();
    let size = r * 1.2;
    let width = painter.text_width(&text, size);
    let height = painter.line_height(size);
    [
        Overlay::Rect {
            rect: Rect::new(center.x - r, center.y - r, 2.0 * r, 2.0 * r),
            fill: Some(fill),
            stroke: Some(Stroke { color: theme.finding, width: 1.5, dash }),
            radius: r,
            opacity,
            layer: Layer::Over,
            target: target.clone(),
        },
        Overlay::Text {
            label: Label {
                text,
                origin: Point::new(center.x - width / 2.0, center.y - height / 2.0),
                font_size: size,
                color: ink,
                weight: FontWeight::Bold,
            },
            opacity,
            layer: Layer::Over,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn joining_runs_straight_through_the_junction() {
        let into = [p(0.0, 10.0), p(40.0, 10.0)];
        let out_of = [p(48.0, 10.0), p(80.0, 10.0), p(80.0, 30.0)];
        assert_eq!(join(&into, &out_of), [p(0.0, 10.0), p(80.0, 10.0), p(80.0, 30.0)]);
    }

    #[test]
    fn joining_keeps_bends_and_doubling_back() {
        let into = [p(0.0, 0.0), p(0.0, 10.0), p(40.0, 10.0)];
        let out_of = [p(48.0, 10.0), p(30.0, 10.0)];
        // The loop back (a self-loop's way out) is not collinear-between.
        assert_eq!(join(&into, &out_of), [p(0.0, 0.0), p(0.0, 10.0), p(48.0, 10.0), p(30.0, 10.0)]);
    }

    #[test]
    fn vertical_wire_ends_reach_the_arrow_line() {
        let junction = Rect::new(100.0, 100.0, 8.0, 8.0);
        let arrow = [p(0.0, 104.5), p(200.0, 104.5)];
        let mut from_above = vec![p(102.0, 40.0), p(102.0, 100.0)];
        assert_eq!(snap(&mut from_above, End::Last, junction, &arrow), p(102.0, 104.5), "onto the arrow itself");
        let mut to_below = vec![p(106.0, 108.0), p(106.0, 160.0), p(20.0, 160.0)];
        assert_eq!(snap(&mut to_below, End::First, junction, &arrow), p(106.0, 104.5));
        let mut sideways = vec![p(108.0, 104.0), p(150.0, 104.0)];
        assert_eq!(snap(&mut sideways, End::First, junction, &arrow), p(108.0, 104.0), "horizontal ends stay");
        let mut no_arrow = vec![p(102.0, 40.0), p(102.0, 100.0)];
        assert_eq!(snap(&mut no_arrow, End::Last, junction, &[]), p(102.0, 104.0), "the junction's centre");
    }
}
