//! Trace view: in a concrete scenario, what happens in what order?
//!
//! A sequence diagram per trace, time running down, placed directly (no
//! layered layout): one column per lifeline in `Trace::lifelines` order
//! (sources, instances, controllers), one row per step. Two traces (a
//! race's `as_queued` and `swapped`) sit side by side, each titled with its
//! ordering label.
//!
//! | Step | Drawn as |
//! | --- | --- |
//! | `ExternalFire` | message source → instance, trigger name, solid neutral |
//! | `Transition` | state-change box on the instance (`pending → paid`), pale machine fill |
//! | `Emit` | event tag on the emitting instance |
//! | `Deliver` | message emitter → controller, event name, dashed gray |
//! | `Fire` | message controller → instance, trigger name, dashed in the target's hue |
//! | `Spawn` | creation message; the new lifeline starts at that row |
//! | `Dropped` | red-outlined ✕ note on the instance |
//! | `NoTarget`, `Ambiguous` | dashed note on the controller |
//!
//! Selection, cones and search emphasise the steps whose elements they
//! cover; hide mode dims here, since removing steps would break the time
//! axis.

use std::collections::HashMap;

use cascade_core::{ElementRef, NodeIx};
use cascade_layout::{Point, Rect, Size};
use cascade_sim::{Lifeline, LifelineIx, Trace, TraceStepKind};

use crate::color::machine_styles;
use crate::emphasis::{Anchor, Interaction};
use crate::scene::{
    Arrow, Border, Dash, EdgeKind, Emphasis, FontWeight, HitTarget, Label, Layer, Overlay, Scene, SceneEdge, SceneNode,
    Shape, Stroke,
};
use crate::view_state::ViewKind;
use crate::views::SceneInput;
use crate::views::decorate::{Decor, FindingIndex, scene_bounds};
use crate::views::draft::{EdgeInfo, Meta};
use crate::views::style::{NodeLook, Painter, STUB_RADIUS, StateMark};

/// Narrowest lifeline column.
const COLUMN_MIN: f32 = 110.0;
/// Room around the widest box or header in a column.
const COLUMN_PAD: f32 = 24.0;
/// Height of one step row.
const ROW: f32 = 40.0;
/// Gap between two traces side by side.
const BLOCK_GAP: f32 = 72.0;
/// Gap below a trace's title.
const TITLE_GAP: f32 = 10.0;
/// Length of a message from outside the diagram (a delivery whose emitter
/// is unknown).
const FOUND_MESSAGE: f32 = 48.0;

pub(super) fn build(input: &SceneInput<'_>, interaction: &Interaction) -> Scene {
    let mut scene = Scene::empty(ViewKind::Trace, input.theme.background);
    if input.traces.is_empty() {
        scene.notes.push("Pick a scenario to trace.".to_owned());
        return scene;
    }
    let painter =
        Painter { theme: input.theme, measure: input.measure, styles: machine_styles(input.model, input.theme) };
    let mut out = Items { scene, nodes: Vec::new(), edges: Vec::new() };
    let mut left = 0.0;
    for (ordering, trace) in input.traces.iter().enumerate() {
        let ordering = u8::try_from(ordering).unwrap_or(u8::MAX);
        let block = Block { input, painter: &painter, trace, ordering };
        left = block.draw(&mut out, left) + BLOCK_GAP;
    }
    let Items { mut scene, nodes, edges } = out;
    let no_findings = FindingIndex::new(&[]);
    let decor = Decor { model: input.model, theme: input.theme, interaction, findings: no_findings, diff: None };
    let owners = vec![None; scene.overlays.len()];
    decor.apply(&mut scene, &nodes, &edges, &owners);
    scene.bounds = scene_bounds(&scene, input.measure);
    scene.notes = interaction.notes().to_vec();
    scene
}

/// The scene under construction plus the metas the decoration pass needs.
struct Items {
    scene: Scene,
    nodes: Vec<Meta>,
    edges: Vec<EdgeInfo>,
}

impl Items {
    fn node(&mut self, look: NodeLook, center: Point, target: HitTarget, meta: Meta) -> Rect {
        let rect = Rect::new(
            center.x - look.size.width / 2.0,
            center.y - look.size.height / 2.0,
            look.size.width,
            look.size.height,
        );
        self.scene.nodes.push(SceneNode {
            target,
            shape: look.shape,
            rect,
            fill: look.fill,
            stroke: look.stroke,
            border: look.border,
            labels: look
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
        self.nodes.push(meta);
        rect
    }

    fn message(&mut self, points: Vec<Point>, stroke: Stroke, label: Option<Label>, target: HitTarget, meta: Meta) {
        self.scene.edges.push(SceneEdge {
            target,
            kind: EdgeKind::Message,
            points,
            stroke,
            arrow: Arrow::End,
            label,
            opacity: 1.0,
            emphasis: Emphasis::Normal,
            back_edge: false,
            diff: None,
        });
        self.edges.push(EdgeInfo { meta, ends: None });
    }
}

/// One trace's sequence diagram.
struct Block<'a> {
    input: &'a SceneInput<'a>,
    painter: &'a Painter<'a>,
    trace: &'a Trace,
    ordering: u8,
}

impl Block<'_> {
    fn causal(&self, element: ElementRef) -> Anchor {
        Anchor::Nodes(self.input.graph.ix_of_element(element).into_iter().collect::<Vec<NodeIx>>())
    }

    fn lifeline_target(&self, i: usize) -> HitTarget {
        HitTarget::Lifeline { ordering: self.ordering, lifeline: u32::try_from(i).unwrap_or(u32::MAX) }
    }

    fn step_target(&self, i: usize) -> HitTarget {
        HitTarget::TraceStep { ordering: self.ordering, step: u32::try_from(i).unwrap_or(u32::MAX) }
    }

    fn header_look(&self, lifeline: &Lifeline) -> (NodeLook, Meta) {
        let model = self.input.model;
        let painter = self.painter;
        match lifeline {
            Lifeline::External { source } => (
                painter.external(model.external(*source).name.clone()),
                Meta::new(vec![ElementRef::External(*source)], Anchor::Free),
            ),
            Lifeline::Controller { controller } => (
                painter.hexagon(model.controller(*controller).name.clone()),
                Meta::new(vec![ElementRef::Controller(*controller)], Anchor::Free),
            ),
            Lifeline::Instance { machine, name } => {
                let style = painter.machine(*machine);
                let mut look = painter.external(format!("{name}: {}", model.machine(*machine).name));
                look.fill = Some(style.hue);
                look.stroke = Stroke::solid(painter.hue_outline(style), painter.theme.stroke_width);
                for l in &mut look.labels {
                    l.color = style.on_hue;
                }
                (look, Meta::new(vec![ElementRef::Machine(*machine)], Anchor::Free))
            }
        }
    }

    fn note(&self, text: String, stroke: Stroke, fill: Option<crate::color::Rgba>) -> NodeLook {
        let mut look = self.painter.external(text);
        look.shape = Shape::RoundedRect { radius: STUB_RADIUS };
        look.stroke = stroke;
        look.fill = fill;
        look.border = Border::Single;
        look
    }

    fn label(&self, text: String, from_x: f32, to_x: f32, y: f32) -> Label {
        let theme = self.painter.theme;
        let size = Size::new(
            self.painter.text_width(&text, theme.small_font_size),
            self.painter.line_height(theme.small_font_size),
        );
        Label {
            origin: Point::new((from_x + to_x) / 2.0 - size.width / 2.0, y - size.height - 1.0),
            text,
            font_size: theme.small_font_size,
            color: theme.text,
            weight: FontWeight::Normal,
        }
    }

    /// What one step draws: a box on a lifeline, or a message.
    fn step_shape(&self, step: &cascade_sim::TraceStep) -> StepShape {
        let model = self.input.model;
        let painter = self.painter;
        let theme = painter.theme;
        match &step.kind {
            TraceStepKind::ExternalFire { source, target, trigger } => {
                let external = external_of(self.trace, *source).map(ElementRef::External);
                let anchor = external.map_or(Anchor::Free, |e| self.causal(e));
                StepShape::Message {
                    from: End::Lifeline(*source),
                    to: End::Lifeline(*target),
                    text: model.trigger(*trigger).name.clone(),
                    stroke: painter.trigger_stroke(),
                    meta: Meta::new(std::iter::once(ElementRef::Trigger(*trigger)).chain(external).collect(), anchor),
                }
            }
            TraceStepKind::Transition { instance, transition, from, to } => {
                let machine = model.transition(*transition).machine;
                let text = format!("{} → {}", model.state(*from).path, model.state(*to).path);
                let el = ElementRef::Transition(*transition);
                StepShape::Box {
                    lifeline: *instance,
                    look: painter.state(text, None, painter.machine(machine), false, StateMark::Normal),
                    meta: Meta::new(vec![el], self.causal(el)),
                }
            }
            TraceStepKind::Dropped { instance, trigger, state } => {
                let text = format!("✕ {} dropped in {}", model.trigger(*trigger).name, model.state(*state).path);
                StepShape::Box {
                    lifeline: *instance,
                    look: self.note(text, Stroke::solid(theme.finding, theme.stroke_width), Some(theme.background)),
                    meta: Meta::new(vec![ElementRef::Trigger(*trigger)], Anchor::Free),
                }
            }
            TraceStepKind::Emit { instance, event } => {
                let el = ElementRef::Event(*event);
                StepShape::Box {
                    lifeline: *instance,
                    look: painter.tag(model.event(*event).name.clone()),
                    meta: Meta::new(vec![el], self.causal(el)),
                }
            }
            TraceStepKind::Deliver { controller, event, handler } => {
                let h = model.handler(*handler);
                StepShape::Message {
                    from: emitter(self.trace, step.cause).map_or(End::Found(*controller), End::Lifeline),
                    to: End::Lifeline(*controller),
                    text: model.event(*event).name.clone(),
                    stroke: painter.emit_stroke(),
                    meta: Meta::new(
                        vec![
                            ElementRef::Handler(*handler),
                            ElementRef::Event(*event),
                            ElementRef::Controller(h.controller),
                        ],
                        self.causal(ElementRef::Handler(*handler)),
                    ),
                }
            }
            TraceStepKind::Fire { controller, target, rule } => {
                let r = model.rule(*rule);
                let machine = model.trigger(r.trigger).machine;
                StepShape::Message {
                    from: End::Lifeline(*controller),
                    to: End::Lifeline(*target),
                    text: model.trigger(r.trigger).name.clone(),
                    stroke: painter.fire_stroke(painter.machine(machine)),
                    meta: Meta::new(vec![ElementRef::Rule(*rule)], self.causal(ElementRef::Handler(r.handler))),
                }
            }
            TraceStepKind::Spawn { controller, instance, rule } => {
                let r = model.rule(*rule);
                let machine = model.trigger(r.trigger).machine;
                let name = match self.trace.lifelines.get(instance.index()) {
                    Some(Lifeline::Instance { name, .. }) => name.clone(),
                    _ => model.machine(machine).name.clone(),
                };
                StepShape::Message {
                    from: End::Lifeline(*controller),
                    to: End::Header(*instance),
                    text: format!("new {name}"),
                    stroke: painter.fire_stroke(painter.machine(machine)),
                    meta: Meta::new(vec![ElementRef::Rule(*rule)], self.causal(ElementRef::Handler(r.handler))),
                }
            }
            TraceStepKind::NoTarget { controller, rule } | TraceStepKind::Ambiguous { controller, rule, .. } => {
                let r = model.rule(*rule);
                let t = model.trigger(r.trigger);
                let machine = &model.machine(t.machine).name;
                let text = match &step.kind {
                    TraceStepKind::Ambiguous { candidates, .. } => {
                        format!("ambiguous: {} {machine} match for {}", candidates.len(), t.name)
                    }
                    _ => format!("no {machine} matched for {}", t.name),
                };
                StepShape::Box {
                    lifeline: *controller,
                    look: self.note(
                        text,
                        Stroke::dashed(theme.neutral, theme.stroke_width),
                        Some(theme.neutral.mix(theme.background, 0.88)),
                    ),
                    meta: Meta::new(vec![ElementRef::Rule(*rule)], self.causal(ElementRef::Handler(r.handler))),
                }
            }
        }
    }

    /// Lifeline centers and the block's right edge. Each lifeline gets a
    /// column as wide as its header and the boxes on it; columns then move
    /// apart until every message label fits between its two lifelines.
    fn columns(&self, headers: &[(NodeLook, Meta)], shapes: &[StepShape], left: f32) -> (Vec<f32>, f32) {
        let n = headers.len();
        let small = self.painter.theme.small_font_size;
        let mut widths: Vec<f32> = headers.iter().map(|(l, _)| (l.size.width + COLUMN_PAD).max(COLUMN_MIN)).collect();
        // Minimum center distance between two lifelines, per message.
        let mut spans: Vec<(usize, usize, f32)> = Vec::new();
        for shape in shapes {
            match shape {
                StepShape::Box { lifeline, look, .. } => {
                    if let Some(w) = widths.get_mut(lifeline.index()) {
                        *w = w.max(look.size.width + COLUMN_PAD);
                    }
                }
                StepShape::Message { from, to, text, .. } => {
                    let (a, b) = (from.lifeline().index(), to.lifeline().index());
                    if a != b && a < n && b < n {
                        let need = self.painter.text_width(text, small) + 2.0 * crate::views::style::PAD_X;
                        spans.push((a.min(b), a.max(b), need));
                    }
                }
            }
        }
        let mut centers: Vec<f32> = Vec::with_capacity(n);
        for k in 0..n {
            let mut x = match k {
                0 => left + widths[0] / 2.0,
                _ => centers[k - 1] + (widths[k - 1] + widths[k]) / 2.0,
            };
            for &(lo, _, need) in spans.iter().filter(|(_, hi, _)| *hi == k) {
                x = x.max(centers[lo] + need);
            }
            centers.push(x);
        }
        let right = centers.last().zip(widths.last()).map_or(left + COLUMN_MIN, |(c, w)| c + w / 2.0);
        (centers, right)
    }

    /// Draw the block with its left edge at `left`; returns its right edge.
    fn draw(&self, out: &mut Items, left: f32) -> f32 {
        let model = self.input.model;
        let painter = self.painter;
        let theme = painter.theme;
        let trace = self.trace;

        let title = trace.ordering.clone().unwrap_or_else(|| trace.scenario.clone());
        let title_h = painter.line_height(theme.font_size);
        out.scene.overlays.push(Overlay::Text {
            label: Label {
                text: title,
                origin: Point::new(left, 0.0),
                font_size: theme.font_size,
                color: theme.text,
                weight: FontWeight::Bold,
            },
            opacity: 1.0,
            layer: Layer::Over,
        });
        let top = title_h + TITLE_GAP;

        let headers: Vec<(NodeLook, Meta)> = trace.lifelines.iter().map(|l| self.header_look(l)).collect();
        let shapes: Vec<StepShape> = trace.steps.iter().map(|s| self.step_shape(s)).collect();
        let (centers, right) = self.columns(&headers, &shapes, left);
        let x = |ix: LifelineIx| centers.get(ix.index()).copied().unwrap_or(left);
        let header_h = headers.iter().map(|(l, _)| l.size.height).fold(0.0, f32::max);
        let row_y = |i: usize| top + header_h + ROW * (i as f32 + 1.0);

        // Spawned instances start at the row that creates them.
        let spawned: HashMap<LifelineIx, usize> = trace
            .steps
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match s.kind {
                TraceStepKind::Spawn { instance, .. } => Some((instance, i)),
                _ => None,
            })
            .collect();
        let mut header_rects: Vec<Rect> = Vec::with_capacity(headers.len());
        for (i, (look, meta)) in headers.into_iter().enumerate() {
            let ix = LifelineIx(u32::try_from(i).unwrap_or(u32::MAX));
            let cy = spawned.get(&ix).map_or(top + header_h / 2.0, |&s| row_y(s));
            header_rects.push(out.node(look, Point::new(x(ix), cy), self.lifeline_target(i), meta));
        }

        for (i, shape) in shapes.into_iter().enumerate() {
            let y = row_y(i);
            let target = self.step_target(i);
            match shape {
                StepShape::Box { lifeline, look, meta } => {
                    out.node(look, Point::new(x(lifeline), y), target, meta);
                }
                StepShape::Message { from, to, text, stroke, meta } => {
                    let from_x = match from {
                        End::Lifeline(ix) | End::Header(ix) => x(ix),
                        End::Found(ix) => x(ix) - FOUND_MESSAGE,
                    };
                    let to_x = match to {
                        End::Lifeline(ix) => x(ix),
                        End::Found(ix) => x(ix) - FOUND_MESSAGE,
                        End::Header(ix) => {
                            let head = header_rects.get(ix.index()).copied().unwrap_or_default();
                            if head.center().x >= from_x { head.left() } else { head.right() }
                        }
                    };
                    let label = self.label(text, from_x, to_x, y);
                    out.message(line(from_x, to_x, y), stroke, Some(label), target, meta);
                }
            }
        }

        // Final states under each instance, then the lifelines themselves.
        let bottom = row_y(trace.steps.len());
        let mut final_top = bottom;
        for (&ix, &state) in &trace.final_states {
            let Some(Lifeline::Instance { machine, .. }) = trace.lifelines.get(ix.index()) else { continue };
            let look = painter.state(
                model.state(state).path.clone(),
                None,
                painter.machine(*machine),
                false,
                StateMark::Normal,
            );
            let rect =
                out.node(look, Point::new(x(ix), bottom + ROW / 2.0), self.lifeline_target(ix.index()), Meta::free());
            final_top = final_top.max(rect.top());
        }
        for (i, rect) in header_rects.iter().enumerate() {
            let ix = LifelineIx(u32::try_from(i).unwrap_or(u32::MAX));
            let end = if trace.final_states.contains_key(&ix) { final_top } else { bottom + ROW / 2.0 };
            out.scene.overlays.push(Overlay::Line {
                from: Point::new(x(ix), rect.bottom()),
                to: Point::new(x(ix), end.max(rect.bottom() + 1.0)),
                stroke: Stroke { color: theme.rule, width: 1.0, dash: Dash::Dashed { on: 4.0, off: 4.0 } },
                opacity: 1.0,
                layer: Layer::Under,
            });
        }
        right
    }
}

/// What a step draws, decided once and used for both sizing and drawing.
enum StepShape {
    /// A box centered on a lifeline.
    Box { lifeline: LifelineIx, look: NodeLook, meta: Meta },
    /// A horizontal message.
    Message { from: End, to: End, text: String, stroke: Stroke, meta: Meta },
}

/// One end of a message.
#[derive(Clone, Copy)]
enum End {
    Lifeline(LifelineIx),
    /// The side of a created instance's header.
    Header(LifelineIx),
    /// Just left of a lifeline, for a delivery whose emitter is unknown.
    Found(LifelineIx),
}

impl End {
    fn lifeline(self) -> LifelineIx {
        match self {
            End::Lifeline(ix) | End::Header(ix) | End::Found(ix) => ix,
        }
    }
}

/// A horizontal message, or a small loop when both ends share a lifeline.
fn line(from_x: f32, to_x: f32, y: f32) -> Vec<Point> {
    if (from_x - to_x).abs() < 0.5 {
        vec![
            Point::new(from_x, y - 6.0),
            Point::new(from_x + 30.0, y - 6.0),
            Point::new(from_x + 30.0, y + 6.0),
            Point::new(from_x, y + 6.0),
        ]
    } else {
        vec![Point::new(from_x, y), Point::new(to_x, y)]
    }
}

/// The instance whose emit caused a delivery.
fn emitter(trace: &Trace, cause: Option<cascade_sim::StepIx>) -> Option<LifelineIx> {
    match trace.steps.get(cause?.index())?.kind {
        TraceStepKind::Emit { instance, .. } => Some(instance),
        _ => None,
    }
}

/// The external source behind a source lifeline, if it is one.
fn external_of(trace: &Trace, ix: LifelineIx) -> Option<cascade_core::ExternalId> {
    match trace.lifelines.get(ix.index()) {
        Some(Lifeline::External { source }) => Some(*source),
        _ => None,
    }
}
