//! Edit mode's wiring band: what the machines are wired to.
//!
//! Two layout groups below the machine lanes, so the layout engine places
//! them, routes the edges between them and the lanes, and keeps them stable
//! across edits like any lane:
//!
//! - **External sources** (right below the lanes): one box per source.
//! - **Events and controllers** (last): one event tag per event (including
//!   ones nothing emits or handles, so a fresh declaration is visible), and
//!   one hexagon per controller listing its handlers ("on OrderPaid").
//!   Subscriptions run East → West inside the band, so events sit left of
//!   the controllers that handle them. This band grows most while
//!   building; being last, its growth moves nothing else.
//!
//! Edges are the causal graph's, with handlers folded into their
//! controller, aggregated per pair of drawn endpoints and kind:
//!
//! | Edge | From → to | Ports | Drawn |
//! | --- | --- | --- | --- |
//! | emit | pill South-out → event North | 4 → 2 | dashed gray |
//! | subscribe | event East → controller West | 1 → 0 | solid gray |
//! | fire | controller North → pill South-in | 2 → 3 | dashed in the target hue, selector and `[when]` |
//! | trigger | source North → pill South-in | 2 → 3 | solid external neutral |
//!
//! Ends on a collapsed state or machine are unported; ends on a hidden
//! machine's stub become dotted `StubLink`s and count towards its label.

use std::collections::HashMap;

use cascade_core::model::Target;
use cascade_core::{CausalEdgeKind, CausalGraph, CausalNode, EdgeIx, ElementRef, MachineId, Model, RuleId};
use cascade_layout::{Insets, LayerConstraint};

use crate::emphasis::Anchor;
use crate::scene::{Arrow, EdgeKind, HitTarget, Stroke};
use crate::views::draft::{DraftEdge, DraftGraph, DraftGroup, DraftNode, EdgeText, Meta, standard_ports};
use crate::views::structure::machines::{
    EndKind, Endpoint, PORT_EAST, PORT_NORTH, PORT_SOUTH, PORT_SOUTH_OUT, PORT_WEST,
};
use crate::views::style::{Painter, bracketed};

/// Layout group keys of the band. They cannot clash with lane keys, which
/// are element keys (`machine:…`, `state:…`).
const WIRING_KEY: &str = "band:wiring";
const SOURCES_KEY: &str = "band:sources";
/// Band titles.
pub(super) const WIRING_TITLE: &str = "Events and controllers";
pub(super) const SOURCES_TITLE: &str = "External sources";

/// The band's groups, for drawing their lanes after layout.
pub(super) struct WiringPlan {
    pub wiring: usize,
    pub sources: usize,
}

/// One drawn edge, standing for one or more causal edges.
struct Wire {
    from: usize,
    from_port: Option<u16>,
    to: usize,
    to_port: Option<u16>,
    kind: EdgeKind,
    stroke: Stroke,
    elements: Vec<ElementRef>,
    labels: Vec<String>,
    chains: Vec<Vec<EdgeIx>>,
}

/// Inputs shared by the band's nodes and edges.
pub(super) struct Wiring<'a> {
    pub model: &'a Model,
    pub graph: &'a CausalGraph,
    pub painter: &'a Painter<'a>,
}

impl Wiring<'_> {
    fn group(&self, key: &str) -> DraftGroup {
        DraftGroup {
            key: key.to_owned(),
            padding: Insets { top: 8.0, right: 16.0, bottom: 12.0, left: 16.0 },
            header: self.painter.line_height(self.painter.theme.font_size) + 10.0,
        }
    }

    fn anchor(&self, elements: impl IntoIterator<Item = ElementRef>) -> Anchor {
        Anchor::Nodes(elements.into_iter().filter_map(|e| self.graph.ix_of_element(e)).collect())
    }

    /// Add the band's groups, nodes and edges. `endpoints` says where each
    /// transition is drawn; `stubs` are hidden machines' stub nodes, whose
    /// labels get their link counts here.
    pub fn draft(
        &self,
        draft: &mut DraftGraph,
        endpoints: &[Option<Endpoint>],
        stubs: &[(MachineId, usize)],
    ) -> WiringPlan {
        let model = self.model;
        let painter = self.painter;
        // Sources first: the events-and-controllers band grows most while
        // building, and at the bottom its growth moves nothing else.
        let sources = draft.add_group(self.group(SOURCES_KEY));
        let wiring = draft.add_group(self.group(WIRING_KEY));
        let node = |key: cascade_core::ElementKey, group: usize, look, meta: Meta| DraftNode {
            key: key.to_string(),
            target: HitTarget::Element(key),
            group: Some(group),
            layer: LayerConstraint::Free,
            ports: standard_ports(),
            look,
            meta,
        };

        let mut event_node = Vec::with_capacity(model.event_count());
        for (id, event) in model.events() {
            let element = ElementRef::Event(id);
            let meta = Meta::new(vec![element], self.anchor([element]));
            let key = model.key_of(element);
            event_node.push(draft.add_node(node(key, wiring, painter.tag(event.name.clone()), meta)));
        }
        let mut controller_node = Vec::with_capacity(model.controller_count());
        for (id, controller) in model.controllers() {
            let handlers = controller.handlers.iter().map(|&h| ElementRef::Handler(h));
            let mut meta = Meta::new(
                std::iter::once(ElementRef::Controller(id)).chain(handlers.clone()).collect(),
                self.anchor(handlers),
            );
            meta.badge_elements = controller
                .handlers
                .iter()
                .flat_map(|&h| model.handler(h).rules.iter().map(|&r| ElementRef::Rule(r)))
                .collect();
            let lines = controller
                .handlers
                .iter()
                .map(|&h| format!("on {}", model.event(model.handler(h).event).name))
                .collect();
            let look = painter.controller(controller.name.clone(), lines);
            let key = model.key_of(ElementRef::Controller(id));
            controller_node.push(draft.add_node(node(key, wiring, look, meta)));
        }
        let mut source_node = Vec::with_capacity(model.external_count());
        for (id, source) in model.externals() {
            let element = ElementRef::External(id);
            let meta = Meta::new(vec![element], self.anchor([element]));
            let key = model.key_of(element);
            source_node.push(draft.add_node(node(key, sources, painter.external(source.name.clone()), meta)));
        }

        let place = |n: CausalNode| -> Option<(usize, EndKind)> {
            match n {
                CausalNode::External(x) => source_node.get(x.index()).map(|&i| (i, EndKind::Other)),
                CausalNode::Event(e) => event_node.get(e.index()).map(|&i| (i, EndKind::Other)),
                CausalNode::Handler(h) => {
                    controller_node.get(model.handler(h).controller.index()).map(|&i| (i, EndKind::Other))
                }
                CausalNode::Transition(t) => endpoints.get(t.index()).copied().flatten().map(|e| (e.node, e.kind)),
            }
        };
        let mut wires: Vec<Wire> = Vec::new();
        let mut at: HashMap<(usize, usize, EdgeKind), usize> = HashMap::new();
        for (e, edge) in self.graph.edges() {
            let (Some((from, from_kind)), Some((to, to_kind))) =
                (place(self.graph.node(edge.from)), place(self.graph.node(edge.to)))
            else {
                continue;
            };
            let pill = |kind: EndKind, port: u16| (kind == EndKind::Pill).then_some(port);
            let (kind, from_port, to_port, stroke, element, label) = match edge.kind {
                CausalEdgeKind::Trigger { trigger } => (
                    EdgeKind::Trigger,
                    Some(PORT_NORTH),
                    pill(to_kind, PORT_SOUTH),
                    painter.trigger_stroke(),
                    ElementRef::Trigger(trigger),
                    None,
                ),
                CausalEdgeKind::Emit => {
                    let CausalNode::Transition(t) = self.graph.node(edge.from) else { continue };
                    (
                        EdgeKind::Emit,
                        pill(from_kind, PORT_SOUTH_OUT),
                        Some(PORT_NORTH),
                        painter.emit_stroke(),
                        ElementRef::Transition(t),
                        None,
                    )
                }
                CausalEdgeKind::Subscribe => {
                    let CausalNode::Handler(h) = self.graph.node(edge.to) else { continue };
                    (
                        EdgeKind::Subscribe,
                        Some(PORT_EAST),
                        Some(PORT_WEST),
                        painter.subscribe_stroke(),
                        ElementRef::Handler(h),
                        None,
                    )
                }
                CausalEdgeKind::Fire { rule } => {
                    let target = model.trigger(model.rule(rule).trigger).machine;
                    (
                        EdgeKind::Fire,
                        Some(PORT_NORTH),
                        pill(to_kind, PORT_SOUTH),
                        painter.fire_stroke(painter.machine(target)),
                        ElementRef::Rule(rule),
                        fire_label(model, rule),
                    )
                }
            };
            let through_stub = from_kind == EndKind::Stub || to_kind == EndKind::Stub;
            let (kind, stroke) =
                if through_stub { (EdgeKind::StubLink, painter.stub_link_stroke(stroke)) } else { (kind, stroke) };
            let i = *at.entry((from, to, kind)).or_insert_with(|| {
                wires.push(Wire {
                    from,
                    from_port,
                    to,
                    to_port,
                    kind,
                    stroke,
                    elements: Vec::new(),
                    labels: Vec::new(),
                    chains: Vec::new(),
                });
                wires.len() - 1
            });
            let wire = &mut wires[i];
            if !wire.elements.contains(&element) {
                wire.elements.push(element);
            }
            if let Some(label) = label
                && !wire.labels.contains(&label)
            {
                wire.labels.push(label);
            }
            wire.chains.push(vec![e]);
        }

        let mut stub_links: HashMap<usize, u32> = HashMap::new();
        for wire in wires {
            for end in [wire.from, wire.to] {
                if stubs.iter().any(|&(_, n)| n == end) {
                    *stub_links.entry(end).or_insert(0) += 1;
                }
            }
            let theme = painter.theme;
            draft.edges.push(DraftEdge {
                from: wire.from,
                from_port: wire.from_port,
                to: wire.to,
                to_port: wire.to_port,
                kind: wire.kind,
                stroke: wire.stroke,
                arrow: Arrow::End,
                label: (!wire.labels.is_empty()).then(|| EdgeText {
                    text: wire.labels.join(", "),
                    font_size: theme.small_font_size,
                    color: theme.text,
                }),
                target: wire.elements.first().map_or(HitTarget::None, |e| HitTarget::Element(model.key_of(*e))),
                meta: Meta::new(wire.elements, Anchor::Chains(wire.chains)),
                on_cycle: false,
            });
        }
        for &(machine, node) in stubs {
            let links = stub_links.get(&node).copied().unwrap_or(0);
            let name = &model.machine(machine).name;
            let stub = &mut draft.nodes[node];
            stub.look = painter.stub(name, links, painter.machine(machine));
            stub.target = HitTarget::MachineStub { machine: name.clone(), links };
        }
        WiringPlan { wiring, sources }
    }
}

/// A fire's label: its target selector without the machine (the pill it
/// points at names that), then `[when]`. `None` for a plain singleton fire.
fn fire_label(model: &Model, rule: RuleId) -> Option<String> {
    let rule = model.rule(rule);
    let clauses = |clauses: &[cascade_core::definition::FieldClause], joiner: &str, op: &str| {
        clauses.iter().map(|c| format!("{} {op} {}", c.field, c.value)).collect::<Vec<_>>().join(joiner)
    };
    let selector = match &rule.target {
        Target::One { predicates } if predicates.is_empty() => None,
        Target::One { predicates } => Some(format!("where {}", clauses(predicates, " and ", "=="))),
        Target::All { predicates } if predicates.is_empty() => Some("all".to_owned()),
        Target::All { predicates } => Some(format!("all where {}", clauses(predicates, " and ", "=="))),
        Target::Spawn { assignments } if assignments.is_empty() => Some("new".to_owned()),
        Target::Spawn { assignments } => Some(format!("new with {}", clauses(assignments, ", ", "="))),
    };
    let parts: Vec<String> = [selector, bracketed(rule.condition.as_deref())].into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}
