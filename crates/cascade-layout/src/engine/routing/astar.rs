//! Obstacle-avoiding orthogonal routing for edges the channel router cannot
//! handle (those touching pinned nodes) and for repairing any route that
//! would cross a node or a foreign group.
//!
//! A* over a sparse grid: the lines through every obstacle's sides (offset
//! by a margin), the ends' stub points, and on small grids the midlines
//! between them. Moving costs its length times one plus the penalties of
//! the obstacles the step runs through, plus a penalty per bend:
//!
//! - a node is prohibitive either way;
//! - a foreign group is prohibitive to run along inside, but cheap to cross
//!   straight along the stacking axis (vertically in the canonical frame),
//!   the way passages cross bands;
//! - overlapping obstacles add up.
//!
//! Crossing is never forbidden, so a route always exists. Step penalties
//! come from prefix sums over the grid, so every step costs O(1), and the
//! search buffers are reused from one search to the next ([`Router`]). The
//! search starts in a window around the two ends and widens to everything
//! if the best route in the window is blocked.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::geometry::{Insets, Point, Rect};

use super::super::frame::Side;
use super::check::{SHRINK, passes_across, segment_hits};
use super::paths::simplify;

/// Penalty per unit of length inside a node.
const NODE: i32 = 16_000;
/// Penalty per unit of length running along inside a foreign group.
const GROUP_ALONG: i32 = 1_000;
/// Penalty per unit of length crossing a foreign group along the stacking
/// axis.
const GROUP_ACROSS: i32 = 1;
const HEURISTIC_WEIGHT: f32 = 2.0;
/// Grids up to this many cells get midlines between obstacle lines.
const MIDLINE_LIMIT: usize = 50_000;
/// Grids up to this many search states use dense, reused buffers.
const DENSE_STATES: usize = 1 << 21;

/// Something to route around.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Obstacle {
    Node(Rect),
    /// A group the route does not belong to.
    Group(Rect),
}

impl Obstacle {
    pub(crate) fn rect(&self) -> Rect {
        match self {
            Obstacle::Node(r) | Obstacle::Group(r) => *r,
        }
    }

    /// Penalties for running along the main axis and across it.
    fn penalties(&self) -> (i32, i32) {
        match self {
            Obstacle::Node(_) => (NODE, NODE),
            Obstacle::Group(_) => (GROUP_ALONG, GROUP_ACROSS),
        }
    }

    /// Whether the segment `a`–`b` crosses the obstacle in a way a route
    /// should avoid.
    fn blocks(&self, a: Point, b: Point) -> bool {
        match self {
            Obstacle::Node(r) => segment_hits(a, b, r, SHRINK),
            Obstacle::Group(r) => segment_hits(a, b, r, SHRINK) && !passes_across(a, b, r),
        }
    }
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

/// A 2-D table filled from rectangle additions by prefix sums.
struct Table {
    width: usize,
    cells: Vec<i32>,
}

impl Table {
    fn new(width: usize, height: usize) -> Self {
        Self { width, cells: vec![0; (width + 1) * (height + 1)] }
    }

    /// Add `v` to every cell in columns `c0..c1` and rows `r0..r1`.
    fn add(&mut self, (c0, c1): (usize, usize), (r0, r1): (usize, usize), v: i32) {
        if c0 >= c1 || r0 >= r1 {
            return;
        }
        let w = self.width + 1;
        self.cells[r0 * w + c0] += v;
        self.cells[r0 * w + c1] -= v;
        self.cells[r1 * w + c0] -= v;
        self.cells[r1 * w + c1] += v;
    }

    fn finish(mut self) -> Vec<i32> {
        let w = self.width + 1;
        let h = self.cells.len() / w;
        for r in 0..h {
            for c in 1..w {
                self.cells[r * w + c] += self.cells[r * w + c - 1];
            }
        }
        for r in 1..h {
            for c in 0..w {
                self.cells[r * w + c] += self.cells[(r - 1) * w + c];
            }
        }
        let mut out = Vec::with_capacity(self.width * (h - 1));
        for r in 0..h - 1 {
            out.extend_from_slice(&self.cells[r * w..r * w + self.width]);
        }
        out
    }
}

/// Grid lines through obstacle sides (plus midlines on small grids), with
/// the penalty of every step between neighbouring grid points.
struct Grid {
    xs: Vec<f32>,
    ys: Vec<f32>,
    /// Penalty of the step from `(i, j)` to `(i + 1, j)`, at `j * (nx - 1) + i`.
    along: Vec<i32>,
    /// Penalty of the step from `(i, j)` to `(i, j + 1)`, at `j * nx + i`.
    across: Vec<i32>,
}

impl Grid {
    fn new(window: Rect, s1: Point, e1: Point, obstacles: &[Obstacle], margin: f32) -> Self {
        let mut xs = vec![window.left(), window.right(), s1.x, e1.x];
        let mut ys = vec![window.top(), window.bottom(), s1.y, e1.y];
        for o in obstacles {
            let r = o.rect();
            xs.extend([r.left() - margin, r.right() + margin]);
            ys.extend([r.top() - margin, r.bottom() + margin]);
        }
        xs.retain(|x| *x >= window.left() - 0.01 && *x <= window.right() + 0.01);
        ys.retain(|y| *y >= window.top() - 0.01 && *y <= window.bottom() + 0.01);
        let midlines = xs.len().saturating_mul(ys.len()) <= MIDLINE_LIMIT;
        coords(&mut xs, midlines);
        coords(&mut ys, midlines);
        let (nx, ny) = (xs.len(), ys.len());
        let mut along = Table::new(nx.saturating_sub(1), ny);
        let mut across = Table::new(nx, ny.saturating_sub(1));
        for o in obstacles {
            let r = o.rect();
            let (p_along, p_across) = o.penalties();
            // Lines strictly inside the obstacle, and the steps between
            // lines that overlap it.
            let rows = (ys.partition_point(|&y| y <= r.top()), ys.partition_point(|&y| y < r.bottom()));
            let cols = (xs.partition_point(|&x| x <= r.left()), xs.partition_point(|&x| x < r.right()));
            let steps_x = (cols.0.saturating_sub(1), cols.1.min(nx.saturating_sub(1)));
            let steps_y = (rows.0.saturating_sub(1), rows.1.min(ny.saturating_sub(1)));
            along.add(steps_x, rows, p_along);
            across.add(cols, steps_y, p_across);
        }
        Self { xs, ys, along: along.finish(), across: across.finish() }
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
        let (len, penalty) = if STEP[d].1 == 0 {
            let lo = i.min(ni);
            (self.xs[lo + 1] - self.xs[lo], self.along[j * (nx - 1) + lo])
        } else {
            let lo = j.min(nj);
            (self.ys[lo + 1] - self.ys[lo], self.across[lo * nx + i])
        };
        Some((ni, nj, len * (1.0 + penalty as f32)))
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

/// Best cost and predecessor of every visited search state: dense buffers
/// reused across searches (reset by bumping a stamp), or a hash map on
/// grids too large for them.
#[derive(Default)]
struct States {
    g: Vec<f32>,
    prev: Vec<usize>,
    stamp: Vec<u32>,
    current: u32,
    sparse: Option<HashMap<usize, (f32, usize)>>,
}

impl States {
    fn reset(&mut self, n: usize) {
        if n > DENSE_STATES {
            self.sparse = Some(HashMap::new());
            return;
        }
        self.sparse = None;
        if self.g.len() < n {
            self.g.resize(n, f32::INFINITY);
            self.prev.resize(n, usize::MAX);
            self.stamp.resize(n, 0);
        }
        self.current = self.current.wrapping_add(1);
        if self.current == 0 {
            self.stamp.iter_mut().for_each(|s| *s = 0);
            self.current = 1;
        }
    }

    fn get(&self, s: usize) -> Option<(f32, usize)> {
        match &self.sparse {
            Some(map) => map.get(&s).copied(),
            None => (self.stamp[s] == self.current).then(|| (self.g[s], self.prev[s])),
        }
    }

    fn insert(&mut self, s: usize, value: (f32, usize)) {
        match &mut self.sparse {
            Some(map) => {
                map.insert(s, value);
            }
            None => {
                self.g[s] = value.0;
                self.prev[s] = value.1;
                self.stamp[s] = self.current;
            }
        }
    }
}

/// The obstacle router, holding search buffers reused across routes.
#[derive(Default)]
pub(crate) struct Router {
    states: States,
    heap: BinaryHeap<Entry>,
}

impl Router {
    /// Search for a route in `params.window`. Returns the polyline and
    /// whether it had to cross an obstacle.
    fn search(&mut self, ends: Ends, obstacles: &[Obstacle], params: Params) -> Option<(Vec<Point>, bool)> {
        let Ends { start, start_side, end, end_side } = ends;
        let Params { margin, bend, window } = params;
        let (so, eo) = (start_side.outward(), end_side.outward());
        let s1 = Point::new(start.x + so.0 * margin, start.y + so.1 * margin);
        let e1 = Point::new(end.x + eo.0 * margin, end.y + eo.1 * margin);
        // An obstacle around either end (overlapping pins, a group dragged
        // over an end) cannot be avoided; searching around it would only
        // flood the grid before giving in.
        let inside = |p: Point, r: &Rect| p.x > r.left() && p.x < r.right() && p.y > r.top() && p.y < r.bottom();
        let avoidable: Vec<Obstacle> =
            obstacles.iter().copied().filter(|o| !inside(s1, &o.rect()) && !inside(e1, &o.rect())).collect();
        let obstacles = &avoidable[..];
        let grid = Grid::new(window, s1, e1, obstacles, margin);
        let (nx, ny) = (grid.xs.len(), grid.ys.len());
        if nx < 2 || ny < 2 {
            return None;
        }
        let (si, sj) = (index_of(&grid.xs, s1.x), index_of(&grid.ys, s1.y));
        let (ei, ej) = (index_of(&grid.xs, e1.x), index_of(&grid.ys, e1.y));
        let into = (dir_of(end_side) + 2) % 4;
        let state = |i: usize, j: usize, d: usize| (j * nx + i) * 4 + d;
        self.states.reset(nx * ny * 4);
        self.heap.clear();
        // Weighted A*: an inflated heuristic (with the bend any misaligned
        // goal needs) trades strict optimality for far fewer expansions in
        // cluttered layouts; routes stay short and tidy.
        let h = |i: usize, j: usize| {
            let (dx, dy) = ((grid.xs[i] - grid.xs[ei]).abs(), (grid.ys[j] - grid.ys[ej]).abs());
            let turn = if dx > 0.01 && dy > 0.01 { bend } else { 0.0 };
            HEURISTIC_WEIGHT * (dx + dy + turn)
        };
        let d0 = dir_of(start_side);
        let s0 = state(si, sj, d0);
        self.states.insert(s0, (0.0, usize::MAX));
        self.heap.push(Entry { f: h(si, sj), g: 0.0, state: s0 });
        let mut best: Option<(f32, usize)> = None;
        while let Some(Entry { f, g: gc, state: s }) = self.heap.pop() {
            if self.states.get(s).is_some_and(|(g, _)| gc > g) {
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
                if self.states.get(ns).is_none_or(|(g, _)| ng < g) {
                    self.states.insert(ns, (ng, s));
                    self.heap.push(Entry { f: ng + h(ni, nj), g: ng, state: ns });
                }
            }
        }
        let (_, mut s) = best?;
        let mut cells = Vec::new();
        loop {
            let cell = s / 4;
            cells.push(Point::new(grid.xs[cell % nx], grid.ys[cell / nx]));
            match self.states.get(s) {
                Some((_, prev)) if prev != usize::MAX => s = prev,
                _ => break,
            }
        }
        cells.reverse();
        let mut pts = vec![start];
        pts.extend(cells);
        pts.push(end);
        let pts = simplify(pts);
        let blocked = pts.windows(2).any(|w| obstacles.iter().any(|o| o.blocks(w[0], w[1])));
        Some((pts, blocked))
    }

    /// Obstacle-avoiding orthogonal route; see the module docs.
    pub(crate) fn route(&mut self, ends: Ends, obstacles: &[Obstacle], margin: f32) -> Vec<Point> {
        let Ends { start, end, .. } = ends;
        let bend = margin * 2.0 + 10.0;
        let pad = margin * 4.0 + 120.0;
        let span = Rect::new(start.x.min(end.x), start.y.min(end.y), (start.x - end.x).abs(), (start.y - end.y).abs());
        let near = span.outset(Insets::uniform(pad));
        let local: Vec<Obstacle> = obstacles.iter().copied().filter(|o| o.rect().intersects(&near)).collect();
        let window = local.iter().fold(near, |w, o| w.union(&o.rect())).outset(Insets::uniform(margin * 3.0));
        let first = self.search(ends, &local, Params { margin, bend, window });
        match first {
            Some((pts, false)) => pts,
            other => {
                let all =
                    obstacles.iter().fold(window, |w, o| w.union(&o.rect())).outset(Insets::uniform(margin * 3.0));
                match self.search(ends, obstacles, Params { margin, bend, window: all }) {
                    Some((pts, _)) => pts,
                    None => other.map_or_else(|| simplify(vec![start, end]), |(pts, _)| pts),
                }
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

    fn route(ends: Ends, obstacles: &[Obstacle]) -> Vec<Point> {
        Router::default().route(ends, obstacles, 8.0)
    }

    fn east_to_west(from: Point, to: Point) -> Ends {
        Ends { start: from, start_side: Side::East, end: to, end_side: Side::West }
    }

    #[test]
    fn goes_around_a_wall() {
        let wall = Rect::new(40.0, -100.0, 20.0, 200.0);
        let pts = route(east_to_west(Point::new(0.0, 0.0), Point::new(100.0, 0.0)), &[Obstacle::Node(wall)]);
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
        );
        assert!(pts[1].y < 0.0 && pts[1].x == 0.0, "{pts:?}");
        let before = pts[pts.len() - 2];
        assert!(before.y > 50.0 && before.x == 50.0, "{pts:?}");
    }

    #[test]
    fn prefers_crossing_a_group_to_crossing_a_node_inside_it() {
        // The target sits inside a big foreign group, with a node between.
        let group = Obstacle::Group(Rect::new(50.0, -200.0, 400.0, 400.0));
        let blocker = Rect::new(150.0, -30.0, 40.0, 60.0);
        let pts = route(east_to_west(Point::new(0.0, 0.0), Point::new(300.0, 0.0)), &[group, Obstacle::Node(blocker)]);
        assert!(!hits(&pts, &blocker), "{pts:?}");
    }

    #[test]
    fn crosses_a_foreign_group_straight_rather_than_around_it() {
        // A wide band between the ends: straight across it beats going round.
        let band = Rect::new(-1000.0, 40.0, 2000.0, 60.0);
        let ends = Ends {
            start: Point::new(0.0, 0.0),
            start_side: Side::South,
            end: Point::new(0.0, 140.0),
            end_side: Side::North,
        };
        let pts = route(ends, &[Obstacle::Group(band)]);
        assert_eq!(pts, vec![Point::new(0.0, 0.0), Point::new(0.0, 140.0)]);
    }

    #[test]
    fn does_not_run_along_inside_a_foreign_group() {
        let band = Rect::new(-100.0, -50.0, 400.0, 100.0);
        let pts = route(east_to_west(Point::new(-200.0, 0.0), Point::new(400.0, 0.0)), &[Obstacle::Group(band)]);
        assert!(pts.windows(2).all(|w| !Obstacle::Group(band).blocks(w[0], w[1])), "{pts:?}");
    }

    #[test]
    fn straight_when_clear() {
        let pts = route(east_to_west(Point::new(0.0, 0.0), Point::new(100.0, 0.0)), &[]);
        assert_eq!(pts, vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0)]);
    }

    #[test]
    fn step_penalties_match_obstacles() {
        let node = Rect::new(10.0, 10.0, 20.0, 20.0);
        let grid = Grid::new(
            Rect::new(0.0, 0.0, 40.0, 40.0),
            Point::new(0.0, 0.0),
            Point::new(40.0, 40.0),
            &[Obstacle::Node(node)],
            2.0,
        );
        let inside = |v: f32, lo: f32, hi: f32| v > lo && v < hi;
        for (j, &y) in grid.ys.iter().enumerate() {
            for i in 0..grid.xs.len() - 1 {
                let (a, b) = (grid.xs[i], grid.xs[i + 1]);
                let expected = if inside(y, 10.0, 30.0) && a < 30.0 && b > 10.0 { NODE } else { 0 };
                assert_eq!(grid.along[j * (grid.xs.len() - 1) + i], expected, "row {y} step {a}..{b}");
            }
        }
    }
}
