//! The scene: a backend-neutral display list with hit targets.
//!
//! Builders bake every visual decision (position, color, stroke weight,
//! opacity) into the scene, so a backend only paints. Paint order is lanes,
//! under-overlays, edges, nodes, over-overlays; within each list, in order.

use serde::{Deserialize, Serialize};

use cascade_core::diff::DiffStatus;
use cascade_core::{ElementKey, Severity};
use cascade_layout::{Point, Rect};

use crate::color::Rgba;
use crate::view_state::ViewKind;

/// What a click on a scene item selects.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HitTarget {
    /// Decoration only; not clickable.
    None,
    /// A model element.
    Element(ElementKey),
    /// A hidden machine's stub node, keeping its cross links.
    MachineStub { machine: String, links: u32 },
    /// A matrix cell: causal links from `row` into `column`.
    MatrixCell { row: String, column: String, count: u32 },
    /// A step in a trace, in trace `ordering` (0, or 1 for the swapped side
    /// of a race).
    TraceStep { ordering: u8, step: u32 },
    /// A lifeline header in a trace.
    Lifeline { ordering: u8, lifeline: u32 },
    /// Build mode: drag from here to connect `element` to something (a state
    /// to a state makes a transition, a transition to a controller wires an
    /// emit, a controller to a transition wires a fire, a source to a
    /// transition exposes its trigger).
    ConnectHandle { element: ElementKey },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Dash {
    Solid,
    Dashed { on: f32, off: f32 },
    Dotted,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub color: Rgba,
    pub width: f32,
    pub dash: Dash,
}

impl Stroke {
    pub const fn solid(color: Rgba, width: f32) -> Self {
        Self { color, width, dash: Dash::Solid }
    }

    pub const fn dashed(color: Rgba, width: f32) -> Self {
        Self { color, width, dash: Dash::Dashed { on: 6.0, off: 4.0 } }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    /// Transition: fully rounded ends.
    Pill,
    /// Event: a rectangle with a pointed right end.
    Tag,
    /// Controller.
    Hexagon,
    /// State.
    RoundedRect { radius: f32 },
    /// External source, matrix cell, lifeline header.
    Rect,
    /// A hidden machine collapsed to a stub: rounded, dashed by the builder.
    Stub,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Border {
    Single,
    /// Final states.
    Double,
    /// Initial states: a thick bar on the left edge of this width.
    ThickLeft(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontWeight {
    Normal,
    Bold,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub text: String,
    /// Top-left of the text's line box.
    pub origin: Point,
    pub font_size: f32,
    pub color: Rgba,
    pub weight: FontWeight,
}

/// A count of findings on an element, drawn as a red-outlined circle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Badge {
    pub count: u32,
    pub severity: Severity,
    pub center: Point,
    pub radius: f32,
}

/// Why an item is drawn the way it is, for backends that add hover or
/// focus effects. Visual consequences are already baked into strokes and
/// opacity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Emphasis {
    Normal,
    /// Selected (one of up to two selections).
    Selected,
    /// Inside the active cone or path query.
    Focused,
    /// Outside the active cone or path query.
    Dimmed,
    /// Matches the search query.
    SearchMatch,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneNode {
    pub target: HitTarget,
    pub shape: Shape,
    pub rect: Rect,
    pub fill: Option<Rgba>,
    pub stroke: Stroke,
    pub border: Border,
    pub labels: Vec<Label>,
    pub badge: Option<Badge>,
    pub opacity: f32,
    pub emphasis: Emphasis,
    pub diff: Option<DiffStatus>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeKind {
    /// Within a machine: state → state (solid, machine hue).
    Transition,
    /// External source → transition.
    Trigger,
    /// Transition → event (dashed, gray).
    Emit,
    /// Event → controller.
    Subscribe,
    /// Controller → transition (dashed, target machine hue).
    Fire,
    /// A message between lifelines in the trace view.
    Message,
    /// A link into or out of a hidden machine's stub.
    StubLink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Arrow {
    None,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneEdge {
    pub target: HitTarget,
    pub kind: EdgeKind,
    /// Polyline, at least two points.
    pub points: Vec<Point>,
    pub stroke: Stroke,
    pub arrow: Arrow,
    /// A guard like `[amount > 0]`, or a message label in the trace view.
    pub label: Option<Label>,
    pub opacity: f32,
    pub emphasis: Emphasis,
    /// Reversed to break a cycle; the builder already colored it red.
    pub back_edge: bool,
    pub diff: Option<DiffStatus>,
}

/// A machine lane (structure view) or other large background region.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub target: HitTarget,
    pub rect: Rect,
    pub fill: Rgba,
    pub stroke: Stroke,
    pub title: Label,
    pub opacity: f32,
    pub collapsed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Layer {
    /// Painted after lanes, before edges.
    Under,
    /// Painted after nodes.
    Over,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Overlay {
    Line {
        from: Point,
        to: Point,
        stroke: Stroke,
        opacity: f32,
        layer: Layer,
    },
    Rect {
        rect: Rect,
        fill: Option<Rgba>,
        stroke: Option<Stroke>,
        radius: f32,
        opacity: f32,
        layer: Layer,
        target: HitTarget,
    },
    Text {
        label: Label,
        opacity: f32,
        layer: Layer,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub view: ViewKind,
    /// Bounding box of everything drawn.
    pub bounds: Rect,
    pub background: Rgba,
    pub lanes: Vec<Lane>,
    pub edges: Vec<SceneEdge>,
    pub nodes: Vec<SceneNode>,
    pub overlays: Vec<Overlay>,
    /// Status messages for the host (e.g. "no scenario selected").
    pub notes: Vec<String>,
}

impl Scene {
    pub fn empty(view: ViewKind, background: Rgba) -> Self {
        Self {
            view,
            bounds: Rect::default(),
            background,
            lanes: Vec::new(),
            edges: Vec::new(),
            nodes: Vec::new(),
            overlays: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// The topmost clickable item at `p` (scene coordinates). Nodes win over
    /// edges, edges over overlays, overlays over lanes. `tolerance` is the
    /// pick distance for edges in scene units.
    pub fn hit_test(&self, p: Point, tolerance: f32) -> Option<&HitTarget> {
        let clickable = |t: &HitTarget| !matches!(t, HitTarget::None);
        if let Some(node) = self.nodes.iter().rev().find(|n| clickable(&n.target) && n.rect.contains(p)) {
            return Some(&node.target);
        }
        if let Some(edge) = self
            .edges
            .iter()
            .rev()
            .filter(|e| clickable(&e.target))
            .map(|e| (e, polyline_distance(&e.points, p)))
            .filter(|(_, d)| *d <= tolerance)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(e, _)| e)
        {
            return Some(&edge.target);
        }
        for overlay in self.overlays.iter().rev() {
            if let Overlay::Rect { rect, target, .. } = overlay
                && clickable(target)
                && rect.contains(p)
            {
                return Some(target);
            }
        }
        self.lanes.iter().rev().find(|l| clickable(&l.target) && l.rect.contains(p)).map(|l| &l.target)
    }

    /// The rectangle of the first node or lane for `target`, for centering
    /// the viewport on a search result or finding.
    pub fn locate(&self, target: &HitTarget) -> Option<Rect> {
        self.nodes
            .iter()
            .find(|n| &n.target == target)
            .map(|n| n.rect)
            .or_else(|| self.lanes.iter().find(|l| &l.target == target).map(|l| l.rect))
    }
}

/// Shortest distance from `p` to a polyline.
pub fn polyline_distance(points: &[Point], p: Point) -> f32 {
    points.windows(2).map(|w| segment_distance(w[0], w[1], p)).fold(f32::INFINITY, f32::min)
}

fn segment_distance(a: Point, b: Point, p: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    p.distance(Point::new(a.x + t * dx, a.y + t * dy))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(target: HitTarget, rect: Rect) -> SceneNode {
        SceneNode {
            target,
            shape: Shape::Rect,
            rect,
            fill: None,
            stroke: Stroke::solid(Rgba::hex(0), 1.0),
            border: Border::Single,
            labels: Vec::new(),
            badge: None,
            opacity: 1.0,
            emphasis: Emphasis::Normal,
            diff: None,
        }
    }

    #[test]
    fn hit_test_prefers_topmost_node_then_edges() {
        let a = HitTarget::Element(ElementKey::Event { event: "A".into() });
        let b = HitTarget::Element(ElementKey::Event { event: "B".into() });
        let e = HitTarget::Element(ElementKey::Event { event: "E".into() });
        let mut scene = Scene::empty(ViewKind::Causal, Rgba::hex(0xFFFFFF));
        scene.nodes.push(node(a.clone(), Rect::new(0.0, 0.0, 10.0, 10.0)));
        scene.nodes.push(node(b.clone(), Rect::new(5.0, 5.0, 10.0, 10.0)));
        scene.edges.push(SceneEdge {
            target: e.clone(),
            kind: EdgeKind::Emit,
            points: vec![Point::new(0.0, 50.0), Point::new(100.0, 50.0)],
            stroke: Stroke::solid(Rgba::hex(0), 1.0),
            arrow: Arrow::End,
            label: None,
            opacity: 1.0,
            emphasis: Emphasis::Normal,
            back_edge: false,
            diff: None,
        });
        assert_eq!(scene.hit_test(Point::new(7.0, 7.0), 3.0), Some(&b));
        assert_eq!(scene.hit_test(Point::new(1.0, 1.0), 3.0), Some(&a));
        assert_eq!(scene.hit_test(Point::new(50.0, 52.0), 3.0), Some(&e));
        assert_eq!(scene.hit_test(Point::new(50.0, 60.0), 3.0), None);
        assert_eq!(scene.locate(&a), Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
    }

    #[test]
    fn distance_to_polyline() {
        let pts = [Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0)];
        assert_eq!(polyline_distance(&pts, Point::new(5.0, 3.0)), 3.0);
        assert_eq!(polyline_distance(&pts, Point::new(12.0, 5.0)), 2.0);
    }
}
