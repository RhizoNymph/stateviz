//! A view's graph before layout, and turning it into scene items.
//!
//! Graph views (causal, structure) describe their nodes and edges as a
//! [`DraftGraph`]: each item carries its final look and a [`Meta`] naming
//! the model elements it stands for. [`DraftGraph::hide`] removes items for
//! hide mode and records the cut links, [`realize`] lays the rest out
//! (through the layout cache) and positions every label, and the emphasis
//! pass then decorates the scene item by item using the metas.

use std::collections::{BTreeMap, HashMap};

use cascade_core::ElementRef;
use cascade_layout::{
    EdgeEnd, GroupId, Insets, LayerConstraint, LayoutEdge, LayoutGraph, LayoutGroup, LayoutNode, LayoutOptions, Point,
    Port, PortSide, Rect, Size,
};

use crate::color::{Rgba, Theme};
use crate::emphasis::Anchor;
use crate::pins::LayoutSidecar;
use crate::scene::{
    Arrow, Dash, EdgeKind, Emphasis, FontWeight, HitTarget, Label, Layer, Overlay, Scene, SceneEdge, SceneNode, Stroke,
};
use crate::text::TextMeasure;
use crate::view_state::ViewKind;
use crate::views::SceneError;
use crate::views::cache::LayoutCache;
use crate::views::style::NodeLook;

/// What a scene item stands for.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Meta {
    /// The elements the item is. Selection and search match any of them;
    /// the first is the primary one (diff status, finding outline).
    pub elements: Vec<ElementRef>,
    /// Further elements whose findings badge this item (e.g. a handler's
    /// rules).
    pub badge_elements: Vec<ElementRef>,
    /// How the item relates to a cone or path focus.
    pub anchor: Anchor,
}

impl Meta {
    pub fn new(elements: Vec<ElementRef>, anchor: Anchor) -> Self {
        Self { elements, badge_elements: Vec::new(), anchor }
    }

    pub fn free() -> Self {
        Self::default()
    }
}

/// An edge's meta plus the scene nodes it joins, for diff inheritance.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EdgeInfo {
    pub meta: Meta,
    pub ends: Option<(usize, usize)>,
}

#[derive(Clone, Debug)]
pub(crate) struct DraftNode {
    /// Layout key: an element key string, unique in the graph.
    pub key: String,
    pub group: Option<usize>,
    pub layer: LayerConstraint,
    pub ports: Vec<Port>,
    pub look: NodeLook,
    pub target: HitTarget,
    pub meta: Meta,
}

/// Text along an edge (a guard, or event and controller names).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EdgeText {
    pub text: String,
    pub font_size: f32,
    pub color: Rgba,
}

#[derive(Clone, Debug)]
pub(crate) struct DraftEdge {
    pub from: usize,
    pub from_port: Option<u16>,
    pub to: usize,
    pub to_port: Option<u16>,
    pub kind: EdgeKind,
    pub stroke: Stroke,
    pub arrow: Arrow,
    pub label: Option<EdgeText>,
    pub target: HitTarget,
    pub meta: Meta,
    /// The edge lies on a causal cycle: when the layout reverses it to
    /// break the cycle, it is drawn red as a cascade-cycle back edge.
    pub on_cycle: bool,
}

/// A layout group (lane or band). Groups without nodes are dropped.
#[derive(Clone, Debug)]
pub(crate) struct DraftGroup {
    pub key: String,
    pub padding: Insets,
    pub header: f32,
}

/// A link cut by hide mode: drawn as a short stub leaving `node`.
#[derive(Clone, Debug)]
pub(crate) struct Cut {
    /// The remaining endpoint (index after hiding).
    pub node: usize,
    /// The remaining endpoint is the link's source.
    pub outgoing: bool,
    pub port: Option<u16>,
    pub kind: EdgeKind,
    pub stroke: Stroke,
    /// The hidden endpoint, so a click can bring it back into view.
    pub target: HitTarget,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DraftGraph {
    pub nodes: Vec<DraftNode>,
    pub edges: Vec<DraftEdge>,
    pub groups: Vec<DraftGroup>,
}

impl DraftGraph {
    pub fn add_node(&mut self, node: DraftNode) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    pub fn add_group(&mut self, group: DraftGroup) -> usize {
        self.groups.push(group);
        self.groups.len() - 1
    }

    /// Remove the nodes flagged in `hidden` (indexed like `nodes`) and every
    /// edge touching them. Each removed edge with exactly one remaining
    /// endpoint becomes a [`Cut`].
    pub fn hide(&mut self, hidden: &[bool]) -> Vec<Cut> {
        let is_hidden = |i: usize| hidden.get(i).copied().unwrap_or(false);
        let mut remap: Vec<Option<usize>> = Vec::with_capacity(self.nodes.len());
        let mut next = 0;
        for i in 0..self.nodes.len() {
            if is_hidden(i) {
                remap.push(None);
            } else {
                remap.push(Some(next));
                next += 1;
            }
        }
        let mut cuts = Vec::new();
        let mut kept = Vec::with_capacity(self.edges.len());
        for edge in std::mem::take(&mut self.edges) {
            match (remap[edge.from], remap[edge.to]) {
                (Some(from), Some(to)) => kept.push(DraftEdge { from, to, ..edge }),
                (Some(from), None) => cuts.push(Cut {
                    node: from,
                    outgoing: true,
                    port: edge.from_port,
                    kind: edge.kind,
                    stroke: edge.stroke,
                    target: self.nodes[edge.to].target.clone(),
                }),
                (None, Some(to)) => cuts.push(Cut {
                    node: to,
                    outgoing: false,
                    port: edge.to_port,
                    kind: edge.kind,
                    stroke: edge.stroke,
                    target: self.nodes[edge.from].target.clone(),
                }),
                (None, None) => {}
            }
        }
        self.edges = kept;
        let mut index = 0;
        self.nodes.retain(|_| {
            let keep = !is_hidden(index);
            index += 1;
            keep
        });
        cuts
    }
}

/// Inputs to [`realize`] besides the graph.
pub(crate) struct RealizeCtx<'a> {
    pub view: ViewKind,
    pub theme: &'a Theme,
    pub measure: &'a dyn TextMeasure,
    pub sidecar: &'a LayoutSidecar,
    pub options: LayoutOptions,
}

/// A laid-out graph as scene items plus the metas the emphasis pass needs.
pub(crate) struct Realized {
    pub scene: Scene,
    /// Aligned with `scene.nodes`.
    pub nodes: Vec<Meta>,
    /// Aligned with `scene.edges`.
    pub edges: Vec<EdgeInfo>,
    /// Aligned with `scene.overlays`: the node an overlay belongs to.
    pub overlay_owner: Vec<Option<usize>>,
    /// Rect of each draft group, `None` for groups dropped as empty.
    pub groups: Vec<Option<Rect>>,
}

/// Length of a hide-mode stub.
const CUT_LENGTH: f32 = 22.0;
/// Spacing between stubs leaving the same side of a node.
const CUT_SPREAD: f32 = 6.0;
/// Size of a self-loop drawn above its node.
const LOOP_SIZE: f32 = 16.0;

/// Lay out `draft` and turn it into scene items.
pub(crate) fn realize(
    draft: DraftGraph,
    cuts: Vec<Cut>,
    ctx: &RealizeCtx<'_>,
    cache: &mut LayoutCache,
) -> Result<Realized, SceneError> {
    let DraftGraph { nodes, edges, groups } = draft;

    // Groups that hold at least one node, in draft order.
    let mut used = vec![false; groups.len()];
    for node in &nodes {
        if let Some(g) = node.group.and_then(|g| used.get_mut(g)) {
            *g = true;
        }
    }
    let mut lg = LayoutGraph::new();
    let group_ids: Vec<Option<GroupId>> = groups
        .iter()
        .zip(&used)
        .map(|(g, &u)| {
            u.then(|| lg.add_group(LayoutGroup { key: g.key.clone(), padding: g.padding, header: g.header }))
        })
        .collect();

    let mut node_ids = Vec::with_capacity(nodes.len());
    for node in &nodes {
        let mut ln =
            LayoutNode::new(node.key.clone(), node.look.size).with_layer(node.layer).with_ports(node.ports.clone());
        if let Some(g) = node.group.and_then(|g| group_ids.get(g).copied().flatten()) {
            ln = ln.in_group(g);
        }
        node_ids.push(lg.add_node(ln)?);
    }

    // Self-loops are drawn by hand; everything else goes to the layout.
    let mut edge_ids = Vec::with_capacity(edges.len());
    for edge in &edges {
        if edge.from == edge.to {
            edge_ids.push(None);
            continue;
        }
        let source = EdgeEnd { node: node_ids[edge.from], port: edge.from_port };
        let target = EdgeEnd { node: node_ids[edge.to], port: edge.to_port };
        let mut le = LayoutEdge::new(source, target);
        if let Some(text) = &edge.label {
            le = le.with_label(label_size(ctx.measure, text));
        }
        edge_ids.push(Some(lg.add_edge(le)?));
    }

    let pins: BTreeMap<String, Point> = ctx
        .sidecar
        .pins_for(ctx.view)
        .map(|(k, p)| (k.to_string(), p))
        .filter(|(k, _)| lg.node_by_key(k).is_some())
        .collect();
    let result = cache.layout(ctx.view, lg, ctx.options, pins)?;

    let mut scene = Scene::empty(ctx.view, ctx.theme.background);
    let mut node_metas = Vec::with_capacity(nodes.len());
    let mut overlay_owner = Vec::new();
    let mut rects = Vec::with_capacity(nodes.len());
    for (i, (node, id)) in nodes.into_iter().zip(&node_ids).enumerate() {
        let rect = result.node(*id).rect;
        rects.push(rect);
        for circle in &node.look.circles {
            let c = rect.origin.offset(circle.center.x, circle.center.y);
            scene.overlays.push(Overlay::Rect {
                rect: Rect::new(c.x - circle.radius, c.y - circle.radius, 2.0 * circle.radius, 2.0 * circle.radius),
                fill: None,
                stroke: Some(circle.stroke),
                radius: circle.radius,
                opacity: 1.0,
                layer: Layer::Over,
                target: HitTarget::None,
            });
            overlay_owner.push(Some(i));
        }
        scene.nodes.push(SceneNode {
            target: node.target,
            shape: node.look.shape,
            rect,
            fill: node.look.fill,
            stroke: node.look.stroke,
            border: node.look.border,
            labels: node
                .look
                .labels
                .into_iter()
                .map(|l| Label {
                    text: l.text,
                    origin: rect.origin.offset(l.offset.x, l.offset.y),
                    font_size: l.font_size,
                    color: l.color,
                    weight: l.weight,
                })
                .collect(),
            badge: None,
            opacity: 1.0,
            emphasis: Emphasis::Normal,
            diff: None,
        });
        node_metas.push(node.meta);
    }

    let mut edge_infos = Vec::with_capacity(edges.len());
    for (edge, id) in edges.into_iter().zip(edge_ids) {
        let (points, reversed, label_rect) = match id {
            Some(id) => {
                let route = result.edge(id);
                (route.points.clone(), route.reversed, route.label)
            }
            None => {
                let label_box =
                    edge.label.as_ref().map(|t| self_loop_label(rects[edge.from], label_size(ctx.measure, t), &rects));
                (self_loop(rects[edge.from]), false, label_box)
            }
        };
        let back_edge = edge.on_cycle && reversed;
        let stroke = if back_edge { Stroke { color: ctx.theme.finding, ..edge.stroke } } else { edge.stroke };
        let label = edge.label.map(|text| place_edge_label(ctx.measure, text, &points, label_rect));
        scene.edges.push(SceneEdge {
            target: edge.target,
            kind: edge.kind,
            points,
            stroke,
            arrow: edge.arrow,
            label,
            opacity: 1.0,
            emphasis: Emphasis::Normal,
            back_edge,
            diff: None,
        });
        edge_infos.push(EdgeInfo { meta: edge.meta, ends: Some((edge.from, edge.to)) });
    }

    draw_cuts(&mut scene, &mut edge_infos, &cuts, &rects, ctx);

    let group_rects = group_ids.iter().map(|g| g.map(|g| result.group(g))).collect();
    scene.bounds = result.bounds;
    Ok(Realized { scene, nodes: node_metas, edges: edge_infos, overlay_owner, groups: group_rects })
}

/// The side of a node a port index sits on, by the views' convention.
pub(crate) fn port_side(port: u16) -> PortSide {
    match port {
        0 => PortSide::West,
        1 => PortSide::East,
        2 => PortSide::North,
        _ => PortSide::South,
    }
}

/// The views' port list: West (in), East (out), North, South.
pub(crate) fn standard_ports() -> Vec<Port> {
    vec![
        Port { side: PortSide::West },
        Port { side: PortSide::East },
        Port { side: PortSide::North },
        Port { side: PortSide::South },
    ]
}

fn draw_cuts(scene: &mut Scene, infos: &mut Vec<EdgeInfo>, cuts: &[Cut], rects: &[Rect], ctx: &RealizeCtx<'_>) {
    // Fan stubs leaving the same side of the same node.
    let mut seen: HashMap<(usize, u8), u32> = HashMap::new();
    for cut in cuts {
        let Some(&rect) = rects.get(cut.node) else { continue };
        let side = cut.port.map(port_side).unwrap_or(if cut.outgoing { PortSide::East } else { PortSide::West });
        let side_ix = side as u8;
        let n = seen.entry((cut.node, side_ix)).or_insert(0);
        let k = *n;
        *n += 1;
        // 0, +1, -1, +2, -2, …
        let step = if k % 2 == 1 { (k / 2 + 1) as f32 } else { -((k / 2) as f32) };
        let c = rect.center();
        let (anchor, dir) = match side {
            PortSide::East => (Point::new(rect.right(), clamp_y(c.y + step * CUT_SPREAD, rect)), (1.0, 0.0)),
            PortSide::West => (Point::new(rect.left(), clamp_y(c.y + step * CUT_SPREAD, rect)), (-1.0, 0.0)),
            PortSide::North => (Point::new(clamp_x(c.x + step * CUT_SPREAD, rect), rect.top()), (0.0, -1.0)),
            PortSide::South => (Point::new(clamp_x(c.x + step * CUT_SPREAD, rect), rect.bottom()), (0.0, 1.0)),
        };
        let far = anchor.offset(dir.0 * CUT_LENGTH, dir.1 * CUT_LENGTH);
        let points = if cut.outgoing { vec![anchor, far] } else { vec![far, anchor] };
        scene.edges.push(SceneEdge {
            target: cut.target.clone(),
            kind: cut.kind,
            points,
            stroke: Stroke { dash: Dash::Dotted, width: ctx.theme.stroke_width, ..cut.stroke },
            arrow: Arrow::End,
            label: None,
            opacity: 1.0,
            emphasis: Emphasis::Normal,
            back_edge: false,
            diff: None,
        });
        infos.push(EdgeInfo::default());
    }
}

fn clamp_y(y: f32, r: Rect) -> f32 {
    y.clamp(r.top() + 2.0, (r.bottom() - 2.0).max(r.top() + 2.0))
}

fn clamp_x(x: f32, r: Rect) -> f32 {
    x.clamp(r.left() + 2.0, (r.right() - 2.0).max(r.left() + 2.0))
}

/// A loop over a node's top-right corner, entering its East side.
fn self_loop(r: Rect) -> Vec<Point> {
    let start = Point::new(r.right() - (r.size.width / 4.0).min(24.0), r.top());
    let top = r.top() - LOOP_SIZE;
    let side = r.right() + LOOP_SIZE;
    vec![
        start,
        Point::new(start.x, top),
        Point::new(side, top),
        Point::new(side, r.center().y),
        Point::new(r.right(), r.center().y),
    ]
}

/// Where a hand-drawn self-loop's label goes: beside the loop's outer
/// corner, else above the loop, else left of it at the same height,
/// whichever first stays clear of every node (the first if none does).
fn self_loop_label(r: Rect, size: Size, nodes: &[Rect]) -> Rect {
    let side = r.right() + LOOP_SIZE;
    let top = r.top() - LOOP_SIZE;
    let candidates = [
        Rect::new(side + 4.0, top - size.height / 2.0, size.width, size.height),
        Rect::new(side - size.width / 2.0 - LOOP_SIZE / 2.0, top - size.height - 2.0, size.width, size.height),
        Rect::new(r.right() - size.width - LOOP_SIZE, top - size.height / 2.0, size.width, size.height),
    ];
    candidates.iter().copied().find(|c| nodes.iter().all(|n| !n.intersects(c))).unwrap_or(candidates[0])
}

pub(crate) fn label_size(measure: &dyn TextMeasure, text: &EdgeText) -> Size {
    Size::new(measure.width(&text.text, text.font_size), measure.line_height(text.font_size))
}

/// Put an edge label in its box (reserved by the layout, or chosen beside a
/// hand-drawn self-loop), otherwise just above the middle of the polyline.
pub(crate) fn place_edge_label(
    measure: &dyn TextMeasure,
    text: EdgeText,
    points: &[Point],
    reserved: Option<Rect>,
) -> Label {
    let size = label_size(measure, &text);
    let origin = match reserved {
        Some(r) => Point::new(r.center().x - size.width / 2.0, r.center().y - size.height / 2.0),
        None => {
            let mid = polyline_midpoint(points);
            Point::new(mid.x - size.width / 2.0, mid.y - size.height - 2.0)
        }
    };
    Label { text: text.text, origin, font_size: text.font_size, color: text.color, weight: FontWeight::Normal }
}

/// The point halfway along a polyline.
pub(crate) fn polyline_midpoint(points: &[Point]) -> Point {
    let total: f32 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
    let mut remaining = total / 2.0;
    for w in points.windows(2) {
        let len = w[0].distance(w[1]);
        if len > 0.0 && remaining <= len {
            let t = remaining / len;
            return Point::new(w[0].x + (w[1].x - w[0].x) * t, w[0].y + (w[1].y - w[0].y) * t);
        }
        remaining -= len;
    }
    points.first().copied().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Border, Shape};

    fn node(key: &str) -> DraftNode {
        DraftNode {
            key: key.to_owned(),
            group: None,
            layer: LayerConstraint::Free,
            ports: Vec::new(),
            look: NodeLook {
                shape: Shape::Rect,
                size: Size::new(40.0, 20.0),
                fill: None,
                stroke: Stroke::solid(Rgba::hex(0), 1.0),
                border: Border::Single,
                labels: Vec::new(),
                circles: Vec::new(),
            },
            target: HitTarget::None,
            meta: Meta::free(),
        }
    }

    fn edge(from: usize, to: usize) -> DraftEdge {
        DraftEdge {
            from,
            from_port: None,
            to,
            to_port: None,
            kind: EdgeKind::Emit,
            stroke: Stroke::solid(Rgba::hex(0), 1.0),
            arrow: Arrow::End,
            label: None,
            target: HitTarget::None,
            meta: Meta::free(),
            on_cycle: false,
        }
    }

    #[test]
    fn hiding_remaps_edges_and_records_cuts() {
        let mut g = DraftGraph::default();
        for k in ["a", "b", "c", "d"] {
            g.add_node(node(k));
        }
        g.edges = vec![edge(0, 1), edge(1, 2), edge(2, 3), edge(3, 1)];
        let cuts = g.hide(&[false, true, false, false]);
        assert_eq!(g.nodes.iter().map(|n| n.key.as_str()).collect::<Vec<_>>(), ["a", "c", "d"]);
        assert_eq!(g.edges.iter().map(|e| (e.from, e.to)).collect::<Vec<_>>(), [(1, 2)]);
        let summary: Vec<(usize, bool)> = cuts.iter().map(|c| (c.node, c.outgoing)).collect();
        assert_eq!(summary, [(0, true), (1, false), (2, true)]);
    }

    #[test]
    fn midpoint_of_a_bent_polyline() {
        let pts = [Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0)];
        assert_eq!(polyline_midpoint(&pts), Point::new(10.0, 0.0));
        assert_eq!(polyline_midpoint(&[Point::new(1.0, 1.0)]), Point::new(1.0, 1.0));
    }
}
