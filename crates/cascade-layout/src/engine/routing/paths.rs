//! Turning resolved tracks into polylines.

use std::collections::BTreeMap;

use crate::geometry::{Point, Rect};

use super::super::context::{Columns, Ctx};
use super::super::frame::Side;
use super::channels::{Leg, SegKey, end_line, leg};
use super::cross::CrossGeometry;

/// Drop repeated points and middle points lying between their neighbours on
/// a straight line, keeping at least two points. Never changes the drawn
/// path.
pub(crate) fn simplify(points: Vec<Point>) -> Vec<Point> {
    const EPS: f32 = 1e-3;
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for p in points {
        if out.last().is_some_and(|q: &Point| (q.x - p.x).abs() < EPS && (q.y - p.y).abs() < EPS) {
            continue;
        }
        while out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let same_x = (a.x - b.x).abs() < EPS && (b.x - p.x).abs() < EPS;
            let same_y = (a.y - b.y).abs() < EPS && (b.y - p.y).abs() < EPS;
            let between = (b.x - a.x) * (p.x - b.x) >= 0.0 && (b.y - a.y) * (p.y - b.y) >= 0.0;
            if (same_x || same_y) && between {
                out.pop();
            } else {
                break;
            }
        }
        if out.last().is_some_and(|q: &Point| (q.x - p.x).abs() < EPS && (q.y - p.y).abs() < EPS) {
            continue;
        }
        out.push(p);
    }
    if out.len() == 1 {
        out.push(out[0]);
    }
    out
}

fn stub(ctx: &Ctx<'_, '_>, edge: usize, source: bool, at: Point) -> Option<Point> {
    let side = ctx.p.edges[edge].end(source).side;
    matches!(side, Side::North | Side::South).then(|| Point::new(at.x, end_line(ctx, edge, source)))
}

fn ends(ctx: &Ctx<'_, '_>, edge: usize) -> (Point, Point) {
    let e = &ctx.p.edges[edge];
    (
        ctx.slots.attach(ctx.p, edge, true, ctx.node_rect(e.source.node)),
        ctx.slots.attach(ctx.p, edge, false, ctx.node_rect(e.target.node)),
    )
}

/// Orthogonal route of chain `chain` in `band` through its tracks.
pub(crate) fn chain_orthogonal(
    ctx: &Ctx<'_, '_>,
    band: usize,
    chain: usize,
    seg_x: &BTreeMap<SegKey, f32>,
) -> Vec<Point> {
    let bg = &ctx.bands[band];
    let c = &bg.chains[chain];
    let (start, end) = ends(ctx, c.edge);
    let last = c.items.len() - 1;
    let mut pts = vec![start];
    pts.extend(stub(ctx, c.edge, true, start));
    for k in 0..last {
        if let Some(&x) = seg_x.get(&SegKey::Link { chain, link: k }) {
            let ya = if k == 0 { end_line(ctx, c.edge, true) } else { bg.items[c.items[k]].line() };
            let yb = if k + 1 == last { end_line(ctx, c.edge, false) } else { bg.items[c.items[k + 1]].line() };
            pts.push(Point::new(x, ya));
            pts.push(Point::new(x, yb));
        }
    }
    pts.extend(stub(ctx, c.edge, false, end));
    pts.push(end);
    simplify(pts)
}

/// Polyline route of a chain: straight segments through the dummies'
/// points, turning around mid-channel where the chain doubles back.
pub(crate) fn chain_polyline(ctx: &Ctx<'_, '_>, band: usize, chain: usize, cols: &Columns) -> Vec<Point> {
    let bg = &ctx.bands[band];
    let c = &bg.chains[chain];
    let (start, end) = ends(ctx, c.edge);
    let last = c.items.len() - 1;
    let mut pts = vec![start];
    pts.extend(stub(ctx, c.edge, true, start));
    for k in 0..last {
        let (a, b) = (&bg.items[c.items[k]], &bg.items[c.items[k + 1]]);
        if a.layer == b.layer {
            let (l, r) = cols.channel(c.channels[k]);
            let xm = (l + r) / 2.0;
            let ya = if k == 0 { end_line(ctx, c.edge, true) } else { a.line() };
            let yb = if k + 1 == last { end_line(ctx, c.edge, false) } else { b.line() };
            pts.push(Point::new(xm, ya));
            pts.push(Point::new(xm, yb));
        }
        if k + 1 < last {
            pts.push(Point::new(cols.centre(b.layer), b.line()));
        }
    }
    pts.extend(stub(ctx, c.edge, false, end));
    pts.push(end);
    simplify(pts)
}

/// Self-loop around its node: East/East and West/West loops use their
/// channel track (or a fixed offset on a pinned node); other side pairs go
/// around the node's corners on their own stub levels.
pub(crate) fn self_loop(ctx: &Ctx<'_, '_>, edge: usize, seg_x: Option<&BTreeMap<SegKey, f32>>) -> Vec<Point> {
    let e = &ctx.p.edges[edge];
    let rect = ctx.node_rect(e.source.node);
    let (start, end) = ends(ctx, edge);
    let es = ctx.p.spacing.edge;
    let track = seg_x.and_then(|m| m.get(&SegKey::Loop { edge }).copied());
    if (start.x - end.x).abs() < 1e-3 && (start.y - end.y).abs() < 1e-3 {
        return lasso(start, e.source.side, es.max(2.0));
    }
    let pts = match (e.source.side, e.target.side) {
        (Side::East, Side::East) => {
            let x = track.unwrap_or(rect.right() + es);
            vec![start, Point::new(x, start.y), Point::new(x, end.y), end]
        }
        (Side::West, Side::West) => {
            let x = track.unwrap_or(rect.left() - es);
            vec![start, Point::new(x, start.y), Point::new(x, end.y), end]
        }
        (s, t) => {
            let [ln, ls] = ctx.slots.loop_level[edge];
            let ring = Ring {
                rect,
                offset: es / 2.0,
                north: rect.top() - ln.max(1) as f32 * es,
                south: rect.bottom() + ls.max(1) as f32 * es,
            };
            ring.around((s, start), (t, end))
        }
    };
    simplify(pts)
}

/// A loop from a port back into the same port: out, across, and back in
/// along the way it left.
fn lasso(p: Point, side: Side, d: f32) -> Vec<Point> {
    let (ox, oy) = side.outward();
    let (tx, ty) = (oy.abs(), ox.abs());
    let at = |o: f32, t: f32| Point::new(p.x + ox * o + tx * t, p.y + oy * o + ty * t);
    vec![p, at(2.0 * d, 0.0), at(2.0 * d, d), at(d, d), at(d, 0.0), p]
}

/// The path a corner self-loop follows around its node: vertical legs
/// `offset` beside the East/West sides, horizontal legs at the `north` and
/// `south` stub levels.
struct Ring {
    rect: Rect,
    offset: f32,
    north: f32,
    south: f32,
}

impl Ring {
    fn at(&self, side: Side, p: Point) -> Point {
        match side {
            Side::East => Point::new(self.rect.right() + self.offset, p.y),
            Side::West => Point::new(self.rect.left() - self.offset, p.y),
            Side::North => Point::new(p.x, self.north),
            Side::South => Point::new(p.x, self.south),
        }
    }

    /// From the point on side `s` around the node's corners to the point
    /// on side `t`.
    fn around(&self, (s, sp): (Side, Point), (t, tp): (Side, Point)) -> Vec<Point> {
        let (east, west) = (self.rect.right() + self.offset, self.rect.left() - self.offset);
        let ne = Point::new(east, self.north);
        let nw = Point::new(west, self.north);
        let se = Point::new(east, self.south);
        let sw = Point::new(west, self.south);
        use Side::{East, North, South, West};
        let corners: Vec<Point> = match (s, t) {
            (East, North) | (North, East) => vec![ne],
            (West, North) | (North, West) => vec![nw],
            (East, South) | (South, East) => vec![se],
            (West, South) | (South, West) => vec![sw],
            (East, West) => vec![ne, nw],
            (West, East) => vec![nw, ne],
            (North, South) => vec![ne, se],
            (South, North) => vec![se, ne],
            _ => Vec::new(),
        };
        let mut pts = vec![sp, self.at(s, sp)];
        pts.extend(corners);
        pts.push(self.at(t, tp));
        pts.push(tp);
        pts
    }
}

/// Route of a cross-band edge from its resolved geometry: out of the
/// source (straight from a direct leg, or by its stub and channel track),
/// alternating gap runs and verticals, and into the target the same way.
pub(crate) fn cross_band(ctx: &Ctx<'_, '_>, edge: usize, g: &CrossGeometry) -> Vec<Point> {
    let (start, end) = ends(ctx, edge);
    let (Some(&x_s), Some(&x_t)) = (g.xs.first(), g.xs.last()) else { return simplify(vec![start, end]) };
    let mut pts = vec![start];
    if leg(ctx, edge, true) == Leg::Channel {
        pts.extend(stub(ctx, edge, true, start));
        pts.push(Point::new(x_s, end_line(ctx, edge, true)));
    }
    for (k, run) in g.runs.iter().enumerate() {
        if let (Some(y), Some(&a), Some(&b)) = (run, g.xs.get(k), g.xs.get(k + 1)) {
            pts.push(Point::new(a, *y));
            pts.push(Point::new(b, *y));
        }
    }
    if leg(ctx, edge, false) == Leg::Channel {
        pts.push(Point::new(x_t, end_line(ctx, edge, false)));
        pts.extend(stub(ctx, edge, false, end));
    }
    pts.push(end);
    simplify(pts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplify_removes_duplicates_and_straight_runs() {
        let pts = vec![
            Point::new(0.0, 0.0),
            Point::new(0.0, 0.0),
            Point::new(5.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 5.0),
            Point::new(10.0, 3.0),
            Point::new(20.0, 3.0),
        ];
        // The back-tracking point at (10, 5) is part of the drawn path.
        assert_eq!(
            simplify(pts),
            vec![
                Point::new(0.0, 0.0),
                Point::new(10.0, 0.0),
                Point::new(10.0, 5.0),
                Point::new(10.0, 3.0),
                Point::new(20.0, 3.0)
            ]
        );
        assert_eq!(simplify(vec![Point::new(1.0, 1.0)]).len(), 2);
    }
}
