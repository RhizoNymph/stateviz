//! Obstacle-avoiding orthogonal routing for edges the channel router cannot
//! handle (those touching pinned nodes) and for repairing any route that
//! would cross a node or a foreign group.
//!
//! A* over a sparse grid: the lines through every obstacle's sides (offset
//! by a margin), the ends' stub points, and the midlines between them.
//! Moving costs its length plus a penalty per bend; crossing an obstacle
//! is allowed but penalised by the obstacle's weight (overlapping obstacles
//! add up, and nodes weigh far more than groups), so a route always exists
//! and prefers crossing a group over crossing a node. The search
//! starts in a window around the two ends and widens to everything if the
//! best route in the window is blocked.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::geometry::{Point, Rect};

use super::super::frame::Side;
use super::check::{SHRINK, segment_hits};
use super::paths::simplify;

const BLOCKED: f32 = 1000.0;
const HEURISTIC_WEIGHT: f32 = 2.0;

/// Something to route around. Crossing it costs `weight × BLOCKED` per
/// unit of length.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Obstacle {
    pub rect: Rect,
    pub weight: u16,
}

fn dir_of(side: Side) -> usize {
    match side {
        Side::North => 0,
        Side::East => 1,
        Side::South => 2,
        Side::West => 3,
    }
}

const STEP: [(i64, i64); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

#[derive(Clone, Copy, PartialEq)]
struct Entry {
    f: f32,
    g: f32,
    state: usize,
}

impl Eq for Entry {}

impl Ord for Entry {
    /// Lowest `f` first; on equal `f`, highest `g` (closest to the goal)
    /// first, which keeps the search from flooding plateaus of equally
    /// good paths.
    fn cmp(&self, other: &Self) -> Ordering {
        other.f.total_cmp(&self.f).then(self.g.total_cmp(&other.g)).then(other.state.cmp(&self.state))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn coords(values: &mut Vec<f32>, midlines: bool) {
    values.sort_by(f32::total_cmp);
    values.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    if midlines {
        let mut mids: Vec<f32> = values.windows(2).filter(|w| w[1] - w[0] > 1.0).map(|w| (w[0] + w[1]) / 2.0).collect();
        values.append(&mut mids);
        values.sort_by(f32::total_cmp);
    }
}

fn index_of(values: &[f32], v: f32) -> usize {
    let i = values.partition_point(|&x| x < v - 0.005);
    i.min(values.len().saturating_sub(1))
}

/// Grid lines through obstacle sides (plus the midlines between them on
/// small grids), with each row's and column's obstacle intervals for cost
/// lookups.
struct Grid {
    xs: Vec<f32>,
    ys: Vec<f32>,
    /// Per row: (left, right, weight) of obstacles whose interior the row
    /// passes through.
    rows: Vec<Vec<(f32, f32, u32)>>,
    /// Per column: (top, bottom, weight) likewise.
    cols: Vec<Vec<(f32, f32, u32)>>,
}

const MIDLINE_LIMIT: usize = 200_000;

impl Grid {
    fn new(window: Rect, s1: Point, e1: Point, obstacles: &[Obstacle], margin: f32) -> Self {
        let mut xs = vec![window.left(), window.right(), s1.x, e1.x];
        let mut ys = vec![window.top(), window.bottom(), s1.y, e1.y];
        for o in obstacles {
            xs.extend([o.rect.left() - margin, o.rect.right() + margin]);
            ys.extend([o.rect.top() - margin, o.rect.bottom() + margin]);
        }
        xs.retain(|x| *x >= window.left() - 0.01 && *x <= window.right() + 0.01);
        ys.retain(|y| *y >= window.top() - 0.01 && *y <= window.bottom() + 0.01);
        let midlines = xs.len().saturating_mul(ys.len()) * 4 <= MIDLINE_LIMIT;
        coords(&mut xs, midlines);
        coords(&mut ys, midlines);
        let mut rows = vec![Vec::new(); ys.len()];
        let mut cols = vec![Vec::new(); xs.len()];
        for ob in obstacles {
            let o = ob.rect;
            let w = u32::from(ob.weight);
            for row in &mut rows[ys.partition_point(|&y| y <= o.top())..ys.partition_point(|&y| y < o.bottom())] {
                row.push((o.left(), o.right(), w));
            }
            for col in &mut cols[xs.partition_point(|&x| x <= o.left())..xs.partition_point(|&x| x < o.right())] {
                col.push((o.top(), o.bottom(), w));
            }
        }
        Self { xs, ys, rows, cols }
    }

    /// Cost of one step from (i, j) in direction `d`, or `None` off-grid.
    fn step(&self, i: usize, j: usize, d: usize) -> Option<(usize, usize, f32)> {
        let (nx, ny) = (self.xs.len(), self.ys.len());
        let ni = i as i64 + STEP[d].0;
        let nj = j as i64 + STEP[d].1;
        if ni < 0 || nj < 0 || ni >= nx as i64 || nj >= ny as i64 {
            return None;
        }
        let (ni, nj) = (ni as usize, nj as usize);
        let (len, weight) = if STEP[d].1 == 0 {
            let (a, b) = (self.xs[i.min(ni)], self.xs[i.max(ni)]);
            let w: u32 = self.rows[j].iter().filter(|(l, r, _)| a < *r && b > *l).map(|o| o.2).sum();
            (b - a, w)
        } else {
            let (a, b) = (self.ys[j.min(nj)], self.ys[j.max(nj)]);
            let w: u32 = self.cols[i].iter().filter(|(t, bt, _)| a < *bt && b > *t).map(|o| o.2).sum();
            (b - a, w)
        };
        Some((ni, nj, len * (1.0 + BLOCKED * weight as f32)))
    }
}

/// The ends of a route: it leaves `start` through `start_side` and enters
/// `end` through `end_side`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ends {
    pub start: Point,
    pub start_side: Side,
    pub end: Point,
    pub end_side: Side,
}

/// Costs and clearances of one search.
#[derive(Clone, Copy, Debug)]
struct Params {
    margin: f32,
    bend: f32,
    window: Rect,
}

/// Best cost and predecessor of every visited search state: dense arrays
/// on small grids, a hash map on large ones (where searches visit a small
/// fraction of the states).
enum States {
    Dense { g: Vec<f32>, prev: Vec<usize> },
    Sparse(HashMap<usize, (f32, usize)>),
}

const DENSE_STATES: usize = 1 << 20;

impl States {
    fn new(n: usize) -> Self {
        if n <= DENSE_STATES {
            States::Dense { g: vec![f32::INFINITY; n], prev: vec![usize::MAX; n] }
        } else {
            States::Sparse(HashMap::new())
        }
    }

    fn get(&self, s: usize) -> Option<(f32, usize)> {
        match self {
            States::Dense { g, prev } => g[s].is_finite().then(|| (g[s], prev[s])),
            States::Sparse(map) => map.get(&s).copied(),
        }
    }

    fn insert(&mut self, s: usize, value: (f32, usize)) {
        match self {
            States::Dense { g, prev } => {
                g[s] = value.0;
                prev[s] = value.1;
            }
            States::Sparse(map) => {
                map.insert(s, value);
            }
        }
    }
}

/// Search for a route in `params.window`. Returns the polyline and whether
/// it had to cross an obstacle.
fn search(ends: Ends, obstacles: &[Obstacle], params: Params) -> Option<(Vec<Point>, bool)> {
    let Ends { start, start_side, end, end_side } = ends;
    let Params { margin, bend, window } = params;
    let (so, eo) = (start_side.outward(), end_side.outward());
    let s1 = Point::new(start.x + so.0 * margin, start.y + so.1 * margin);
    let e1 = Point::new(end.x + eo.0 * margin, end.y + eo.1 * margin);
    // An obstacle around either end (overlapping pins, a group dragged over
    // an end) cannot be avoided; searching around it would only flood the
    // grid before giving in.
    let inside = |p: Point, r: &Rect| p.x > r.left() && p.x < r.right() && p.y > r.top() && p.y < r.bottom();
    let avoidable: Vec<Obstacle> =
        obstacles.iter().copied().filter(|o| !inside(s1, &o.rect) && !inside(e1, &o.rect)).collect();
    let obstacles = &avoidable[..];
    let grid = Grid::new(window, s1, e1, obstacles, margin);
    let (nx, ny) = (grid.xs.len(), grid.ys.len());
    if nx == 0 || ny == 0 {
        return None;
    }
    let (si, sj) = (index_of(&grid.xs, s1.x), index_of(&grid.ys, s1.y));
    let (ei, ej) = (index_of(&grid.xs, e1.x), index_of(&grid.ys, e1.y));
    let into = (dir_of(end_side) + 2) % 4;
    let state = |i: usize, j: usize, d: usize| (j * nx + i) * 4 + d;
    let mut best_g = States::new(nx * ny * 4);
    // Weighted A*: an inflated heuristic (with the bend any misaligned goal
    // needs) trades strict optimality for far fewer expansions in cluttered
    // layouts; routes stay short and tidy.
    let h = |i: usize, j: usize| {
        let (dx, dy) = ((grid.xs[i] - grid.xs[ei]).abs(), (grid.ys[j] - grid.ys[ej]).abs());
        let turn = if dx > 0.01 && dy > 0.01 { bend } else { 0.0 };
        HEURISTIC_WEIGHT * (dx + dy + turn)
    };
    let d0 = dir_of(start_side);
    let s0 = state(si, sj, d0);
    best_g.insert(s0, (0.0, usize::MAX));
    let mut heap = BinaryHeap::new();
    heap.push(Entry { f: h(si, sj), g: 0.0, state: s0 });
    let mut best: Option<(f32, usize)> = None;
    while let Some(Entry { f, g: gc, state: s }) = heap.pop() {
        if best_g.get(s).is_some_and(|(g, _)| gc > g) {
            continue;
        }
        if best.is_some_and(|(b, _)| f >= b) {
            break;
        }
        let d = s % 4;
        let cell = s / 4;
        let (i, j) = (cell % nx, cell / nx);
        if i == ei && j == ej {
            let total = gc
                + if d == into {
                    0.0
                } else if d == dir_of(end_side) {
                    3.0 * bend
                } else {
                    bend
                };
            if best.is_none_or(|(b, _)| total < b) {
                best = Some((total, s));
            }
        }
        for nd in 0..4 {
            if nd == (d + 2) % 4 {
                continue;
            }
            let Some((ni, nj, cost)) = grid.step(i, j, nd) else { continue };
            let ng = gc + cost + if nd == d { 0.0 } else { bend };
            let ns = state(ni, nj, nd);
            if best_g.get(ns).is_none_or(|(g, _)| ng < g) {
                best_g.insert(ns, (ng, s));
                heap.push(Entry { f: ng + h(ni, nj), g: ng, state: ns });
            }
        }
    }
    let (_, mut s) = best?;
    let mut cells = Vec::new();
    loop {
        let cell = s / 4;
        cells.push(Point::new(grid.xs[cell % nx], grid.ys[cell / nx]));
        match best_g.get(s) {
            Some((_, prev)) if prev != usize::MAX => s = prev,
            _ => break,
        }
    }
    cells.reverse();
    let blocked = cells.windows(2).any(|w| obstacles.iter().any(|o| segment_hits(w[0], w[1], &o.rect, SHRINK)));
    let mut pts = vec![start];
    pts.extend(cells);
    pts.push(end);
    Some((simplify(pts), blocked))
}

/// Obstacle-avoiding orthogonal route; see the module docs.
pub(crate) fn route(ends: Ends, obstacles: &[Obstacle], margin: f32) -> Vec<Point> {
    let Ends { start, end, .. } = ends;
    let bend = margin * 2.0 + 10.0;
    let pad = margin * 4.0 + 120.0;
    let span = Rect::new(start.x.min(end.x), start.y.min(end.y), (start.x - end.x).abs(), (start.y - end.y).abs());
    let near = span.outset(crate::geometry::Insets::uniform(pad));
    let local: Vec<Obstacle> = obstacles.iter().copied().filter(|o| o.rect.intersects(&near)).collect();
    let window =
        local.iter().fold(near, |w, o| w.union(&o.rect)).outset(crate::geometry::Insets::uniform(margin * 3.0));
    let first = search(ends, &local, Params { margin, bend, window });
    match first {
        Some((pts, false)) => pts,
        other => {
            let all = obstacles
                .iter()
                .fold(window, |w, o| w.union(&o.rect))
                .outset(crate::geometry::Insets::uniform(margin * 3.0));
            match search(ends, obstacles, Params { margin, bend, window: all }) {
                Some((pts, _)) => pts,
                None => other.map_or_else(|| simplify(vec![start, end]), |(pts, _)| pts),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hits(pts: &[Point], r: &Rect) -> bool {
        pts.windows(2).any(|w| {
            let (a, b) = (w[0], w[1]);
            let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
            let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
            x1 > r.left() && x0 < r.right() && y1 > r.top() && y0 < r.bottom()
        })
    }

    fn node(rect: Rect) -> Obstacle {
        Obstacle { rect, weight: 10 }
    }

    #[test]
    fn goes_around_a_wall() {
        let wall = Rect::new(40.0, -100.0, 20.0, 200.0);
        let pts = route(
            Ends {
                start: Point::new(0.0, 0.0),
                start_side: Side::East,
                end: Point::new(100.0, 0.0),
                end_side: Side::West,
            },
            &[node(wall)],
            8.0,
        );
        assert!(!hits(&pts, &wall), "{pts:?}");
        assert_eq!(pts[0], Point::new(0.0, 0.0));
        assert_eq!(*pts.last().expect("points"), Point::new(100.0, 0.0));
        for w in pts.windows(2) {
            assert!(w[0].x == w[1].x || w[0].y == w[1].y, "{pts:?}");
        }
    }

    #[test]
    fn leaves_and_enters_through_the_given_sides() {
        let pts = route(
            Ends {
                start: Point::new(0.0, 0.0),
                start_side: Side::North,
                end: Point::new(50.0, 50.0),
                end_side: Side::South,
            },
            &[],
            8.0,
        );
        assert!(pts[1].y < 0.0 && pts[1].x == 0.0, "{pts:?}");
        let before = pts[pts.len() - 2];
        assert!(before.y > 50.0 && before.x == 50.0, "{pts:?}");
    }

    #[test]
    fn prefers_crossing_a_group_to_crossing_a_node_inside_it() {
        // The target sits inside a big foreign group, with a node between.
        let group = Obstacle { rect: Rect::new(50.0, -200.0, 400.0, 400.0), weight: 1 };
        let blocker = Rect::new(150.0, -30.0, 40.0, 60.0);
        let pts = route(
            Ends {
                start: Point::new(0.0, 0.0),
                start_side: Side::East,
                end: Point::new(300.0, 0.0),
                end_side: Side::West,
            },
            &[group, node(blocker)],
            8.0,
        );
        assert!(!hits(&pts, &blocker), "{pts:?}");
    }

    #[test]
    fn straight_when_clear() {
        let pts = route(
            Ends {
                start: Point::new(0.0, 0.0),
                start_side: Side::East,
                end: Point::new(100.0, 0.0),
                end_side: Side::West,
            },
            &[],
            8.0,
        );
        assert_eq!(pts, vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0)]);
    }
}
