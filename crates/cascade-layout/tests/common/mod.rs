//! Shared helpers for the layout integration tests: graph builders, a
//! deterministic pseudo-random generator and an invariant checker that every
//! layout must pass.

#![allow(dead_code)]

use cascade_layout::{
    EdgeEnd, EdgeId, EdgeRouting, FlowDirection, GroupId, LayoutEdge, LayoutGraph, LayoutHints, LayoutNode,
    LayoutOptions, LayoutResult, NodeId, Point, PortSide, Rect, Size,
};

pub const EPS: f32 = 0.01;

/// Small deterministic generator (PCG-style LCG) so generated graphs are the
/// same on every run.
pub struct Lcg(u64);

impl Lcg {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1))
    }

    pub fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }

    /// Uniform in `0..n` (n > 0).
    pub fn below(&mut self, n: u32) -> u32 {
        self.next_u32() % n.max(1)
    }

    pub fn chance(&mut self, percent: u32) -> bool {
        self.below(100) < percent
    }
}

pub fn node(g: &mut LayoutGraph, key: &str, w: f32, h: f32) -> NodeId {
    g.add_node(LayoutNode::new(key, Size::new(w, h))).expect("unique key")
}

pub fn edge(g: &mut LayoutGraph, a: NodeId, b: NodeId) -> EdgeId {
    g.add_edge(LayoutEdge::new(EdgeEnd::node(a), EdgeEnd::node(b))).expect("valid edge")
}

pub fn run(g: &LayoutGraph) -> LayoutResult {
    cascade_layout::layout(g, &LayoutOptions::default(), &LayoutHints::default()).expect("layout")
}

pub fn run_with(g: &LayoutGraph, options: &LayoutOptions, hints: &LayoutHints) -> LayoutResult {
    cascade_layout::layout(g, options, hints).expect("layout")
}

/// A graph rebuilt from `(key, size, group index)` nodes and `(from, to)` key
/// edges, so tests can add or drop elements and keep keys stable.
pub struct Spec {
    pub groups: Vec<&'static str>,
    pub nodes: Vec<(String, Size, Option<usize>)>,
    pub edges: Vec<(String, String)>,
}

impl Spec {
    pub fn new() -> Self {
        Self { groups: Vec::new(), nodes: Vec::new(), edges: Vec::new() }
    }

    pub fn group(mut self, key: &'static str) -> Self {
        self.groups.push(key);
        self
    }

    pub fn node(mut self, key: &str, w: f32, h: f32) -> Self {
        self.nodes.push((key.to_string(), Size::new(w, h), None));
        self
    }

    pub fn node_in(mut self, key: &str, w: f32, h: f32, group: usize) -> Self {
        self.nodes.push((key.to_string(), Size::new(w, h), Some(group)));
        self
    }

    pub fn edge(mut self, a: &str, b: &str) -> Self {
        self.edges.push((a.to_string(), b.to_string()));
        self
    }

    pub fn without_node(mut self, key: &str) -> Self {
        self.nodes.retain(|(k, _, _)| k != key);
        self.edges.retain(|(a, b)| a != key && b != key);
        self
    }

    pub fn without_edge(mut self, a: &str, b: &str) -> Self {
        self.edges.retain(|(x, y)| !(x == a && y == b));
        self
    }

    pub fn build(&self) -> LayoutGraph {
        let mut g = LayoutGraph::new();
        let groups: Vec<GroupId> = self
            .groups
            .iter()
            .map(|k| {
                g.add_group(cascade_layout::LayoutGroup {
                    key: (*k).to_string(),
                    padding: cascade_layout::Insets::uniform(10.0),
                    header: 20.0,
                })
            })
            .collect();
        for (key, size, group) in &self.nodes {
            let mut n = LayoutNode::new(key.clone(), *size);
            if let Some(gi) = group {
                n = n.in_group(groups[*gi]);
            }
            g.add_node(n).expect("unique key");
        }
        for (a, b) in &self.edges {
            let a = g.node_by_key(a).expect("edge source exists");
            let b = g.node_by_key(b).expect("edge target exists");
            g.add_edge(LayoutEdge::new(EdgeEnd::node(a), EdgeEnd::node(b))).expect("edge");
        }
        g
    }
}

impl Default for Spec {
    fn default() -> Self {
        Self::new()
    }
}

pub fn rect_of(g: &LayoutGraph, r: &LayoutResult, key: &str) -> Rect {
    r.node(g.node_by_key(key).expect("key")).rect
}

/// Strict interior intersection of an axis-aligned or diagonal segment with
/// a rect shrunk by `shrink` on every side.
pub fn segment_hits_interior(a: Point, b: Point, rect: &Rect, shrink: f32) -> bool {
    let (l, t, r, bt) = (rect.left() + shrink, rect.top() + shrink, rect.right() - shrink, rect.bottom() - shrink);
    if l >= r || t >= bt {
        return false;
    }
    // Liang-Barsky clipping against the open rectangle.
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let mut t0 = 0.0f32;
    let mut t1 = 1.0f32;
    for (p, q) in [(-dx, a.x - l), (dx, r - a.x), (-dy, a.y - t), (dy, bt - a.y)] {
        if p.abs() < 1e-9 {
            if q <= 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                if t > t0 {
                    t0 = t;
                }
            } else if t < t1 {
                t1 = t;
            }
        }
    }
    t0 < t1
}

pub fn expected_side(g: &LayoutGraph, e: EdgeId, source: bool, direction: FlowDirection) -> PortSide {
    let edge = g.edge(e);
    let end = if source { edge.source } else { edge.target };
    if let Some(p) = end.port {
        return g.node(end.node).ports[usize::from(p)].side;
    }
    let self_loop = edge.source.node == edge.target.node;
    let out = source || self_loop;
    match (direction, out) {
        (FlowDirection::LeftToRight, true) => PortSide::East,
        (FlowDirection::LeftToRight, false) => PortSide::West,
        (FlowDirection::TopToBottom, true) => PortSide::South,
        (FlowDirection::TopToBottom, false) => PortSide::North,
    }
}

pub fn on_side(p: Point, rect: &Rect, side: PortSide) -> bool {
    let within_y = p.y >= rect.top() - EPS && p.y <= rect.bottom() + EPS;
    let within_x = p.x >= rect.left() - EPS && p.x <= rect.right() + EPS;
    match side {
        PortSide::East => (p.x - rect.right()).abs() < EPS && within_y,
        PortSide::West => (p.x - rect.left()).abs() < EPS && within_y,
        PortSide::North => (p.y - rect.top()).abs() < EPS && within_x,
        PortSide::South => (p.y - rect.bottom()).abs() < EPS && within_x,
    }
}

/// Direction pointing out of a node through `side`.
pub fn outward(side: PortSide) -> (f32, f32) {
    match side {
        PortSide::East => (1.0, 0.0),
        PortSide::West => (-1.0, 0.0),
        PortSide::North => (0.0, -1.0),
        PortSide::South => (0.0, 1.0),
    }
}

fn leaves_outward(from: Point, next: Point, side: PortSide) -> bool {
    let (ox, oy) = outward(side);
    let dx = next.x - from.x;
    let dy = next.y - from.y;
    dx * ox + dy * oy > EPS && (dx * oy - dy * ox).abs() < EPS
}

/// Whether segment `a`–`b` runs straight across `group` along the stacking
/// axis (vertically with left-to-right flow, where groups stack top to
/// bottom), from one side of the group to the other.
pub fn passes_across(a: Point, b: Point, group: &Rect, direction: FlowDirection) -> bool {
    let (along_a, along_b, across_a, across_b, lo, hi) = match direction {
        FlowDirection::LeftToRight => (a.y, b.y, a.x, b.x, group.top(), group.bottom()),
        FlowDirection::TopToBottom => (a.x, b.x, a.y, b.y, group.left(), group.right()),
    };
    (across_a - across_b).abs() < EPS && along_a.min(along_b) <= lo + 0.5 && along_a.max(along_b) >= hi - 0.5
}

/// What the invariant checker should verify for one layout.
#[derive(Clone, Copy, Debug)]
pub struct Checks {
    /// Node rects never overlap (pinned nodes may overlap each other only).
    pub overlaps: bool,
    /// Routes never cross a node interior.
    pub avoidance: bool,
    /// Routes enter a foreign group's interior only straight across it
    /// along the stacking axis, from one side of the group to the other:
    /// through a passage, which `avoidance` keeps clear of nodes (pins may
    /// drag a group over other bands, making this impossible).
    pub foreign_groups: bool,
    /// Group rects contain their nodes and do not overlap each other.
    pub groups: bool,
    /// Label boxes do not overlap node interiors.
    pub labels: bool,
}

impl Checks {
    pub const ALL: Checks =
        Checks { overlaps: true, avoidance: true, foreign_groups: true, groups: true, labels: true };
}

/// Check the invariants every layout must satisfy. Returns a description of
/// the first violation.
pub fn check(
    g: &LayoutGraph,
    options: &LayoutOptions,
    hints: &LayoutHints,
    r: &LayoutResult,
    checks: Checks,
) -> Result<(), String> {
    if r.node_count() != g.node_count() {
        return Err(format!("{} placements for {} nodes", r.node_count(), g.node_count()));
    }
    let pinned = |id: NodeId| hints.pins.contains_key(&g.node(id).key);
    let rects: Vec<Rect> = g.nodes().map(|(id, _)| r.node(id).rect).collect();

    for (id, n) in g.nodes() {
        let rect = rects[id.index()];
        if !(rect.left().is_finite() && rect.top().is_finite()) {
            return Err(format!("node {} has a non-finite rect", n.key));
        }
        if (rect.size.width - n.size.width).abs() > EPS || (rect.size.height - n.size.height).abs() > EPS {
            return Err(format!("node {} was resized", n.key));
        }
        if let Some(pin) = hints.pins.get(&n.key)
            && (rect.origin.x != pin.x || rect.origin.y != pin.y)
        {
            return Err(format!("pinned node {} is at {:?}, not {:?}", n.key, rect.origin, pin));
        }
    }

    if checks.overlaps {
        for (a, na) in g.nodes() {
            for (b, nb) in g.nodes() {
                if b.index() <= a.index() || (pinned(a) && pinned(b)) {
                    continue;
                }
                let (ra, rb) = (rects[a.index()], rects[b.index()]);
                let shrunk =
                    Rect::new(ra.left() + EPS, ra.top() + EPS, ra.size.width - 2.0 * EPS, ra.size.height - 2.0 * EPS);
                if ra.size.width > 2.0 * EPS && ra.size.height > 2.0 * EPS && shrunk.intersects(&rb) {
                    return Err(format!("nodes {} {:?} and {} {:?} overlap", na.key, ra, nb.key, rb));
                }
            }
        }
    }

    for (e, edge) in g.edges() {
        let route = r.edge(e);
        let name = format!("edge {} {} -> {}", e.index(), g.node(edge.source.node).key, g.node(edge.target.node).key);
        if route.points.len() < 2 {
            return Err(format!("{name} has {} points", route.points.len()));
        }
        if route.points.iter().any(|p| !(p.x.is_finite() && p.y.is_finite())) {
            return Err(format!("{name} has a non-finite point"));
        }
        if options.routing == EdgeRouting::Orthogonal {
            for w in route.points.windows(2) {
                if (w[0].x - w[1].x).abs() > EPS && (w[0].y - w[1].y).abs() > EPS {
                    return Err(format!("{name} has a diagonal segment {:?} -> {:?}", w[0], w[1]));
                }
            }
        }
        let first = route.points[0];
        let last = route.points[route.points.len() - 1];
        let s_side = expected_side(g, e, true, options.direction);
        let t_side = expected_side(g, e, false, options.direction);
        let s_rect = rects[edge.source.node.index()];
        let t_rect = rects[edge.target.node.index()];
        if !on_side(first, &s_rect, s_side) {
            return Err(format!("{name} starts at {first:?}, not on {s_side:?} of {s_rect:?}"));
        }
        if !on_side(last, &t_rect, t_side) {
            return Err(format!("{name} ends at {last:?}, not on {t_side:?} of {t_rect:?}"));
        }
        if options.routing == EdgeRouting::Orthogonal {
            if !leaves_outward(first, route.points[1], s_side) {
                return Err(format!("{name} does not leave {s_side:?} outward: {:?}", route.points));
            }
            let before = route.points[route.points.len() - 2];
            if !leaves_outward(last, before, t_side) {
                return Err(format!("{name} does not enter {t_side:?} from outside: {:?}", route.points));
            }
        }
        if route.label.is_some() != edge.label.is_some() {
            return Err(format!("{name} label box presence does not match its label"));
        }
        if let (Some(boxed), Some(size)) = (route.label, edge.label) {
            if (boxed.size.width - size.width).abs() > EPS || (boxed.size.height - size.height).abs() > EPS {
                return Err(format!("{name} label box {boxed:?} does not have the label's size {size:?}"));
            }
            if checks.labels {
                for (id, n) in g.nodes() {
                    let rr = rects[id.index()];
                    let shrunk = Rect::new(rr.left() + 0.5, rr.top() + 0.5, rr.size.width - 1.0, rr.size.height - 1.0);
                    if shrunk.size.width > 0.0 && shrunk.size.height > 0.0 && shrunk.intersects(&boxed) {
                        return Err(format!("{name} label {boxed:?} overlaps node {}", n.key));
                    }
                }
            }
        }
        if checks.avoidance {
            // A node overlapping one of the edge's own ends (pins may
            // overlap) cannot always be avoided.
            let exempt = |r: &Rect| r.intersects(&s_rect) || r.intersects(&t_rect);
            for w in route.points.windows(2) {
                for (id, n) in g.nodes() {
                    let rr = rects[id.index()];
                    let own = id == edge.source.node || id == edge.target.node;
                    if !own && exempt(&rr) {
                        continue;
                    }
                    if segment_hits_interior(w[0], w[1], &rr, 0.5) {
                        return Err(format!("{name} crosses node {} with {:?} -> {:?}", n.key, w[0], w[1]));
                    }
                }
                for (gid, group) in g.groups().filter(|_| checks.foreign_groups) {
                    let own =
                        g.node(edge.source.node).group == Some(gid) || g.node(edge.target.node).group == Some(gid);
                    let gr = r.group(gid);
                    if !own
                        && segment_hits_interior(w[0], w[1], &gr, 0.5)
                        && !passes_across(w[0], w[1], &gr, options.direction)
                    {
                        return Err(format!(
                            "{name} crosses foreign group {} with {:?} -> {:?}",
                            group.key, w[0], w[1]
                        ));
                    }
                }
            }
        }
    }

    if checks.groups {
        for (gid, group) in g.groups() {
            let gr = r.group(gid);
            for (id, n) in g.nodes() {
                if n.group == Some(gid) {
                    let nr = rects[id.index()];
                    let inside = nr.left() >= gr.left() - EPS
                        && nr.right() <= gr.right() + EPS
                        && nr.top() >= gr.top() - EPS
                        && nr.bottom() <= gr.bottom() + EPS;
                    if !inside {
                        return Err(format!("group {} {:?} does not contain node {} {:?}", group.key, gr, n.key, nr));
                    }
                }
            }
            for (other, og) in g.groups() {
                if other.index() > gid.index() {
                    let or = r.group(other);
                    let shrunk = Rect::new(
                        gr.left() + EPS,
                        gr.top() + EPS,
                        gr.size.width - 2.0 * EPS,
                        gr.size.height - 2.0 * EPS,
                    );
                    if shrunk.size.width > 0.0 && shrunk.size.height > 0.0 && shrunk.intersects(&or) {
                        return Err(format!("groups {} {:?} and {} {:?} overlap", group.key, gr, og.key, or));
                    }
                }
            }
        }
    }

    let b = r.bounds;
    let inside_bounds =
        |p: Point| p.x >= b.left() - EPS && p.x <= b.right() + EPS && p.y >= b.top() - EPS && p.y <= b.bottom() + EPS;
    for rect in &rects {
        if !inside_bounds(rect.origin) || !inside_bounds(Point::new(rect.right(), rect.bottom())) {
            return Err(format!("node rect {rect:?} outside bounds {b:?}"));
        }
    }
    for (e, _) in g.edges() {
        if let Some(p) = r.edge(e).points.iter().find(|p| !inside_bounds(**p)) {
            return Err(format!("route point {p:?} of edge {} outside bounds {b:?}", e.index()));
        }
    }
    Ok(())
}

pub fn assert_ok(g: &LayoutGraph, options: &LayoutOptions, hints: &LayoutHints, r: &LayoutResult) {
    if let Err(msg) = check(g, options, hints, r, Checks::ALL) {
        panic!("layout invariant violated: {msg}");
    }
}
