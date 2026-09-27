//! Causal flow view (foundation version): every causal node, laid out left
//! to right from external sources, styled per the visual encoding. No
//! emphasis, filtering or badges yet.

use std::collections::BTreeMap;

use cascade_core::{CausalEdgeKind, CausalNode, ElementRef};
use cascade_layout::{
    EdgeEnd, LayerConstraint, LayoutEdge, LayoutGraph, LayoutHints, LayoutNode, LayoutOptions, Point, PreviousLayout,
    Size, layout,
};

use crate::color::{machine_styles, style_of};
use crate::scene::{
    Arrow, Border, EdgeKind, Emphasis, FontWeight, HitTarget, Label, Scene, SceneEdge, SceneNode, Shape, Stroke,
};
use crate::views::{SceneError, SceneInput};

const PAD_X: f32 = 14.0;
const PAD_Y: f32 = 6.0;

pub(super) fn build(
    input: &SceneInput<'_>,
    previous: Option<PreviousLayout>,
) -> Result<(Scene, PreviousLayout), SceneError> {
    let model = input.model;
    let graph = input.graph;
    let theme = input.theme;
    let styles = machine_styles(model, theme);
    let line = input.measure.line_height(theme.font_size);
    let small_line = input.measure.line_height(theme.small_font_size);

    let mut lg = LayoutGraph::new();
    let mut texts: Vec<(String, Option<String>)> = Vec::with_capacity(graph.node_count());
    for (_, node) in graph.nodes() {
        let element = node.element();
        let (primary, secondary) = match node {
            CausalNode::Transition(t) => {
                let tr = model.transition(t);
                (
                    format!(
                        "{}: {} → {}",
                        model.machine(tr.machine).name,
                        model.state(tr.from).path,
                        model.state(tr.to).path
                    ),
                    Some(model.trigger(tr.trigger).name.clone()),
                )
            }
            CausalNode::Handler(h) => (model.controller(model.handler(h).controller).name.clone(), None),
            CausalNode::Event(_) | CausalNode::External(_) => (model.label_of(element), None),
        };
        let mut width = input.measure.width(&primary, theme.font_size);
        let mut height = line;
        if let Some(s) = &secondary {
            width = width.max(input.measure.width(s, theme.small_font_size));
            height += small_line;
        }
        let extra = match node {
            CausalNode::Handler(_) => 2.0 * PAD_X,
            CausalNode::Event(_) => PAD_X,
            CausalNode::Transition(_) | CausalNode::External(_) => 0.0,
        };
        let size = Size::new(width + 2.0 * PAD_X + extra, height + 2.0 * PAD_Y);
        let layer = match node {
            CausalNode::External(_) => LayerConstraint::First,
            _ => LayerConstraint::Free,
        };
        lg.add_node(LayoutNode::new(model.key_of(element).to_string(), size).with_layer(layer))?;
        texts.push((primary, secondary));
    }
    let layout_ids: Vec<_> = lg.nodes().map(|(id, _)| id).collect();
    for (_, edge) in graph.edges() {
        lg.add_edge(LayoutEdge::new(
            EdgeEnd::node(layout_ids[edge.from.index()]),
            EdgeEnd::node(layout_ids[edge.to.index()]),
        ))?;
    }

    let pins: BTreeMap<String, Point> =
        input.sidecar.pins_for(crate::view_state::ViewKind::Causal).map(|(k, p)| (k.to_string(), p)).collect();
    let hints = LayoutHints { previous, pins };
    let result = layout(&lg, &LayoutOptions::default(), &hints)?;

    let mut scene = crate::scene::Scene::empty(input.view.view, theme.background);
    scene.bounds = result.bounds;

    for ((ix, node), (primary, secondary)) in graph.nodes().zip(texts) {
        let rect = result.node(layout_ids[ix.index()]).rect;
        let (shape, fill, stroke, text_color) = match node {
            CausalNode::Transition(t) => {
                let style = style_of(&styles, model.transition(t).machine);
                (Shape::Pill, Some(style.hue), Stroke::solid(style.hue, theme.stroke_width), style.on_hue)
            }
            CausalNode::Event(_) => (
                Shape::Tag,
                Some(theme.neutral.mix(theme.background, 0.7)),
                Stroke::solid(theme.neutral, theme.stroke_width),
                theme.text,
            ),
            CausalNode::Handler(_) => {
                (Shape::Hexagon, None, Stroke::solid(theme.controller, theme.stroke_width), theme.text)
            }
            CausalNode::External(_) => {
                (Shape::Rect, None, Stroke::solid(theme.external, theme.stroke_width), theme.text)
            }
        };
        let mut labels = vec![Label {
            text: primary,
            origin: Point::new(rect.left() + PAD_X, rect.top() + PAD_Y),
            font_size: theme.font_size,
            color: text_color,
            weight: FontWeight::Normal,
        }];
        if let Some(s) = secondary {
            labels.push(Label {
                text: s,
                origin: Point::new(rect.left() + PAD_X, rect.top() + PAD_Y + line),
                font_size: theme.small_font_size,
                color: text_color,
                weight: FontWeight::Normal,
            });
        }
        if matches!(node, CausalNode::Handler(_)) {
            for l in &mut labels {
                l.origin.x += PAD_X;
            }
        }
        scene.nodes.push(SceneNode {
            target: HitTarget::Element(model.key_of(node.element())),
            shape,
            rect,
            fill,
            stroke,
            border: Border::Single,
            labels,
            badge: None,
            opacity: 1.0,
            emphasis: Emphasis::Normal,
            diff: None,
        });
    }

    for ((_, edge), (lid, _)) in graph.edges().zip(lg.edges()) {
        let route = result.edge(lid);
        let (kind, stroke, target) = match edge.kind {
            CausalEdgeKind::Trigger { trigger } => (
                EdgeKind::Trigger,
                Stroke::solid(theme.external, theme.stroke_width),
                HitTarget::Element(model.key_of(ElementRef::Trigger(trigger))),
            ),
            CausalEdgeKind::Emit => {
                (EdgeKind::Emit, Stroke::dashed(theme.neutral, theme.stroke_width), HitTarget::None)
            }
            CausalEdgeKind::Subscribe => {
                (EdgeKind::Subscribe, Stroke::solid(theme.neutral, theme.stroke_width), HitTarget::None)
            }
            CausalEdgeKind::Fire { rule } => {
                let machine = model.trigger(model.rule(rule).trigger).machine;
                (
                    EdgeKind::Fire,
                    Stroke::dashed(style_of(&styles, machine).hue, theme.stroke_width),
                    HitTarget::Element(model.key_of(ElementRef::Rule(rule))),
                )
            }
        };
        let stroke = if route.reversed { Stroke { color: theme.finding, ..stroke } } else { stroke };
        scene.edges.push(SceneEdge {
            target,
            kind,
            points: route.points.clone(),
            stroke,
            arrow: Arrow::End,
            label: None,
            opacity: 1.0,
            emphasis: Emphasis::Normal,
            back_edge: route.reversed,
            diff: None,
        });
    }

    Ok((scene, result.to_previous(&lg)))
}
