//! Edit mode's wiring: what the machines are wired to, drawn in gutters
//! between the lanes.
//!
//! Every event (including ones nothing emits or handles, so a fresh
//! declaration is visible) is a tag, every controller a hexagon listing its
//! handlers ("on OrderPaid"), every external source a box. Each goes into
//! the gutter next to what it wires ([`super::gutters`]), in a row ordered
//! so its wires drop nearly straight, with columns that survive edits.
//!
//! Edges are the causal graph's, with handlers folded into their
//! controller, aggregated per pair of drawn endpoints and kind. Pill ends
//! face the gutter: North ports when it is above the pill's lane, South
//! when below.
//!
//! | Edge | From → to | Ports (gutter below / above) | Drawn |
//! | --- | --- | --- | --- |
//! | emit | pill → event | South-out → North / North-out → South | dashed gray |
//! | subscribe | event → controller | East → West | solid gray |
//! | fire | controller → pill | North → South / South → North | dashed in the target hue, short selector and `[when]` |
//! | trigger | source → pill | North → South / South → North | solid external neutral |
//!
//! Fire labels follow [`super::selector`]. Ends on a collapsed state or
//! machine are unported; ends on a hidden machine's stub become dotted
//! `StubLink`s and count towards its label.

use std::collections::{BTreeMap, HashMap};

use cascade_core::{CausalEdgeKind, CausalGraph, CausalNode, EdgeIx, ElementRef, MachineId, Model};
use cascade_layout::{Insets, LayerConstraint};

use crate::emphasis::Anchor;
use crate::scene::{Arrow, EdgeKind, HitTarget, Stroke};
use crate::views::draft::{DraftEdge, DraftGraph, DraftGroup, DraftNode, EdgeText, Meta, standard_ports};
use crate::views::structure::gutters::memo::{Row, WiringMemo};
use crate::views::structure::gutters::order::{WiringNode, order};
use crate::views::structure::gutters::{Assignment, Gutter, LaneEnd, Stack, Wires, assign};
use crate::views::structure::machines::{
    EndKind, Endpoint, PORT_EAST, PORT_NORTH, PORT_NORTH_OUT, PORT_SOUTH, PORT_SOUTH_OUT, PORT_WEST,
};
use crate::views::structure::selector::{self, RuleLabel};
use crate::views::style::Painter;

/// Key of the gutter above machine `name`'s lane. Gutter keys cannot clash
/// with lane keys, which are element keys (`machine:…`, `state:…`).
pub(super) fn gutter_key_above(name: &str) -> String {
    format!("gutter:above:{name}")
}

/// Key of the gutter below every lane.
pub(super) const LAST_GUTTER_KEY: &str = "gutter:last";

/// A gutter's layout group: thin, without a header.
pub(super) fn gutter_group(key: String) -> DraftGroup {
    DraftGroup { key, padding: Insets { top: 8.0, right: 16.0, bottom: 8.0, left: 16.0 }, header: 0.0 }
}

/// A fire's controller node and target machine; `None` for other wires.
type Parallel = Option<(usize, MachineId)>;

/// One drawn edge, standing for one or more causal edges.
struct Wire {
    from: usize,
    from_kind: EndKind,
    to: usize,
    to_kind: EndKind,
    kind: EdgeKind,
    causal: CausalEdgeKind,
    stroke: Stroke,
    elements: Vec<ElementRef>,
    rules: Vec<RuleLabel>,
    chains: Vec<Vec<EdgeIx>>,
    /// Fires: the machine fired into.
    target_machine: Option<MachineId>,
}

/// Inputs shared by the wiring's nodes and edges.
pub(super) struct Wiring<'a> {
    pub model: &'a Model,
    pub graph: &'a CausalGraph,
    pub painter: &'a Painter<'a>,
}

/// Which draft nodes are wiring nodes, and where every node sits.
struct Nodes {
    events: Vec<usize>,
    controllers: Vec<usize>,
    sources: Vec<usize>,
    wiring: HashMap<usize, WiringNode>,
}

impl Nodes {
    fn draft_node(&self, n: WiringNode) -> Option<usize> {
        match n {
            WiringNode::Source(i) => self.sources.get(i).copied(),
            WiringNode::Event(i) => self.events.get(i).copied(),
            WiringNode::Controller(i) => self.controllers.get(i).copied(),
        }
    }
}

impl Wiring<'_> {
    fn anchor(&self, elements: impl IntoIterator<Item = ElementRef>) -> Anchor {
        Anchor::Nodes(elements.into_iter().filter_map(|e| self.graph.ix_of_element(e)).collect())
    }

    /// Add the wiring's nodes and edges. `endpoints` says where each
    /// transition is drawn; `gutters` are the gutters' draft groups, top to
    /// bottom; `stubs` are hidden machines' stub nodes, whose labels get
    /// their link counts here. `memo` keeps gutter columns across builds.
    pub fn draft(
        &self,
        draft: &mut DraftGraph,
        endpoints: &[Option<Endpoint>],
        gutters: &[usize],
        stubs: &[(MachineId, usize)],
        memo: &mut WiringMemo,
    ) {
        let nodes = self.add_nodes(draft);
        let wires = self.collect_wires(&nodes, endpoints);

        let columns: HashMap<usize, f32> = endpoints.iter().flatten().map(|e| (e.node, e.column)).collect();
        let lane_end = |n: usize| -> Option<LaneEnd> {
            let position = draft.nodes.get(n)?.group?;
            Some(LaneEnd { position, column: columns.get(&n).copied().unwrap_or(0.0) })
        };
        let inputs = self.placement_inputs(&nodes, &wires, &lane_end);
        let mut is_gutter = vec![false; draft.groups.len()];
        for &g in gutters {
            if let Some(slot) = is_gutter.get_mut(g) {
                *slot = true;
            }
        }
        let stack = Stack::new(&is_gutter);
        let assignment = assign(&stack, &inputs);
        self.place(draft, &nodes, &stack, &inputs, &assignment, memo);

        self.add_edges(draft, wires, stubs);
    }

    fn add_nodes(&self, draft: &mut DraftGraph) -> Nodes {
        let model = self.model;
        let painter = self.painter;
        let node = |key: cascade_core::ElementKey, look, meta: Meta| DraftNode {
            key: key.to_string(),
            target: HitTarget::Element(key),
            group: None,
            layer: LayerConstraint::Free,
            ports: standard_ports(),
            look,
            meta,
        };
        let mut wiring = HashMap::new();
        let mut events = Vec::with_capacity(model.event_count());
        for (id, event) in model.events() {
            let element = ElementRef::Event(id);
            let meta = Meta::new(vec![element], self.anchor([element]));
            let n = draft.add_node(node(model.key_of(element), painter.tag(event.name.clone()), meta));
            wiring.insert(n, WiringNode::Event(events.len()));
            events.push(n);
        }
        let mut controllers = Vec::with_capacity(model.controller_count());
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
            let n = draft.add_node(node(model.key_of(ElementRef::Controller(id)), look, meta));
            wiring.insert(n, WiringNode::Controller(controllers.len()));
            controllers.push(n);
        }
        let mut sources = Vec::with_capacity(model.external_count());
        for (id, source) in model.externals() {
            let element = ElementRef::External(id);
            let meta = Meta::new(vec![element], self.anchor([element]));
            let n = draft.add_node(node(model.key_of(element), painter.external(source.name.clone()), meta));
            wiring.insert(n, WiringNode::Source(sources.len()));
            sources.push(n);
        }
        Nodes { events, controllers, sources, wiring }
    }

    /// The causal graph's edges between drawn ends, merged per pair of ends
    /// and kind, in the graph's order.
    fn collect_wires(&self, nodes: &Nodes, endpoints: &[Option<Endpoint>]) -> Vec<Wire> {
        let model = self.model;
        let painter = self.painter;
        let place = |n: CausalNode| -> Option<(usize, EndKind)> {
            match n {
                CausalNode::External(x) => nodes.sources.get(x.index()).map(|&i| (i, EndKind::Other)),
                CausalNode::Event(e) => nodes.events.get(e.index()).map(|&i| (i, EndKind::Other)),
                CausalNode::Handler(h) => {
                    nodes.controllers.get(model.handler(h).controller.index()).map(|&i| (i, EndKind::Other))
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
            let (kind, stroke, element, rule, target_machine) = match edge.kind {
                CausalEdgeKind::Trigger { trigger } => {
                    (EdgeKind::Trigger, painter.trigger_stroke(), ElementRef::Trigger(trigger), None, None)
                }
                CausalEdgeKind::Emit => {
                    let CausalNode::Transition(t) = self.graph.node(edge.from) else { continue };
                    (EdgeKind::Emit, painter.emit_stroke(), ElementRef::Transition(t), None, None)
                }
                CausalEdgeKind::Subscribe => {
                    let CausalNode::Handler(h) = self.graph.node(edge.to) else { continue };
                    (EdgeKind::Subscribe, painter.subscribe_stroke(), ElementRef::Handler(h), None, None)
                }
                CausalEdgeKind::Fire { rule } => {
                    let r = model.rule(rule);
                    let target = model.trigger(r.trigger).machine;
                    let label = RuleLabel::new(&r.target, r.condition.as_deref());
                    (
                        EdgeKind::Fire,
                        painter.fire_stroke(painter.machine(target)),
                        ElementRef::Rule(rule),
                        Some(label),
                        Some(target),
                    )
                }
            };
            let through_stub = from_kind == EndKind::Stub || to_kind == EndKind::Stub;
            let (kind, stroke) =
                if through_stub { (EdgeKind::StubLink, painter.stub_link_stroke(stroke)) } else { (kind, stroke) };
            let i = *at.entry((from, to, kind)).or_insert_with(|| {
                wires.push(Wire {
                    from,
                    from_kind,
                    to,
                    to_kind,
                    kind,
                    causal: edge.kind,
                    stroke,
                    elements: Vec::new(),
                    rules: Vec::new(),
                    chains: Vec::new(),
                    target_machine,
                });
                wires.len() - 1
            });
            let wire = &mut wires[i];
            if !wire.elements.contains(&element) {
                wire.elements.push(element);
            }
            wire.rules.extend(rule);
            wire.chains.push(vec![e]);
        }
        wires
    }

    /// What the placement looks at: per wiring node, the lane ends of its
    /// wires and, for controllers, their events.
    fn placement_inputs(&self, nodes: &Nodes, wires: &[Wire], lane_end: &dyn Fn(usize) -> Option<LaneEnd>) -> Wires {
        let model = self.model;
        let mut inputs = Wires {
            events: vec![Default::default(); nodes.events.len()],
            controllers: vec![Default::default(); nodes.controllers.len()],
            sources: vec![Default::default(); nodes.sources.len()],
        };
        // (event, controller) → index into the event's `handled_into`.
        let mut handled: HashMap<(usize, usize), usize> = HashMap::new();
        for wire in wires {
            let (from, to) = (nodes.wiring.get(&wire.from).copied(), nodes.wiring.get(&wire.to).copied());
            match (wire.causal, from, to) {
                (CausalEdgeKind::Emit, None, Some(WiringNode::Event(e))) => {
                    inputs.events[e].emitters.extend(lane_end(wire.from));
                }
                (CausalEdgeKind::Subscribe, Some(WiringNode::Event(e)), Some(WiringNode::Controller(c))) => {
                    if !inputs.controllers[c].events.contains(&e) {
                        inputs.controllers[c].events.push(e);
                    }
                }
                (CausalEdgeKind::Fire { .. }, Some(WiringNode::Controller(c)), None) => {
                    let end = lane_end(wire.to);
                    inputs.controllers[c].fires.extend(end);
                    for element in &wire.elements {
                        let ElementRef::Rule(r) = element else { continue };
                        let e = model.rule(*r).event.index();
                        let slot = *handled.entry((e, c)).or_insert_with(|| {
                            let list = &mut inputs.events[e].handled_into;
                            list.push(Vec::new());
                            list.len() - 1
                        });
                        inputs.events[e].handled_into[slot].extend(end);
                    }
                }
                (CausalEdgeKind::Trigger { .. }, Some(WiringNode::Source(s)), None) => {
                    inputs.sources[s].triggers.extend(lane_end(wire.to));
                }
                _ => {}
            }
        }
        inputs
    }

    /// Put every wiring node into its gutter, at its column.
    fn place(
        &self,
        draft: &mut DraftGraph,
        nodes: &Nodes,
        stack: &Stack,
        inputs: &Wires,
        assignment: &Assignment,
        memo: &mut WiringMemo,
    ) {
        let rows: BTreeMap<Gutter, Vec<WiringNode>> = order(inputs, assignment);
        let mut drafted: Vec<(usize, Vec<usize>)> = Vec::with_capacity(rows.len());
        let mut memo_rows: Vec<Row> = Vec::with_capacity(rows.len());
        for (&gutter, row) in &rows {
            let Some(group) = stack.group(gutter) else { continue };
            let members: Vec<usize> = row.iter().filter_map(|&n| nodes.draft_node(n)).collect();
            let index: HashMap<WiringNode, usize> = row.iter().enumerate().map(|(i, &n)| (n, i)).collect();
            let mut before = Vec::new();
            for (i, &n) in row.iter().enumerate() {
                let WiringNode::Controller(c) = n else { continue };
                for &e in &inputs.controllers[c].events {
                    if let Some(&j) = index.get(&WiringNode::Event(e)) {
                        before.push((j, i));
                    }
                }
            }
            memo_rows.push(Row {
                gutter: draft.groups[group].key.clone(),
                nodes: members.iter().map(|&n| draft.nodes[n].key.clone()).collect(),
                before,
            });
            drafted.push((group, members));
        }
        let columns = memo.columns(&memo_rows);
        for ((group, members), cols) in drafted.into_iter().zip(columns) {
            for (n, col) in members.into_iter().zip(cols) {
                let node = &mut draft.nodes[n];
                node.group = Some(group);
                node.layer = LayerConstraint::Exact(col);
            }
        }
    }

    fn add_edges(&self, draft: &mut DraftGraph, wires: Vec<Wire>, stubs: &[(MachineId, usize)]) {
        let model = self.model;
        let painter = self.painter;
        let theme = painter.theme;
        // Which end is the gutter's: its group lies above the other end's
        // group when its index is smaller.
        let group_of = |n: usize| draft.nodes.get(n).and_then(|d| d.group);
        let above = |gutter_node: usize, lane_node: usize| match (group_of(gutter_node), group_of(lane_node)) {
            (Some(a), Some(b)) => a < b,
            _ => false,
        };
        let labels: Vec<Option<String>> = wires.iter().map(|w| selector::merged(&w.rules)).collect();
        // Parallel fires: one controller's fires into one machine.
        let groups: Vec<(Parallel, Option<&str>)> =
            wires.iter().zip(&labels).map(|(w, l)| (w.target_machine.map(|m| (w.from, m)), l.as_deref())).collect();
        let shown = selector::shown(&groups);

        let mut edges = Vec::with_capacity(wires.len());
        let mut stub_links: HashMap<usize, u32> = HashMap::new();
        for ((wire, label), show) in wires.into_iter().zip(labels).zip(shown) {
            for end in [wire.from, wire.to] {
                if stubs.iter().any(|&(_, n)| n == end) {
                    *stub_links.entry(end).or_insert(0) += 1;
                }
            }
            let pill = |kind: EndKind, port: u16| (kind == EndKind::Pill).then_some(port);
            let (from_port, to_port) = match wire.causal {
                CausalEdgeKind::Emit => {
                    if above(wire.to, wire.from) {
                        (pill(wire.from_kind, PORT_NORTH_OUT), Some(PORT_SOUTH))
                    } else {
                        (pill(wire.from_kind, PORT_SOUTH_OUT), Some(PORT_NORTH))
                    }
                }
                CausalEdgeKind::Subscribe => (Some(PORT_EAST), Some(PORT_WEST)),
                CausalEdgeKind::Fire { .. } | CausalEdgeKind::Trigger { .. } => {
                    if above(wire.from, wire.to) {
                        (Some(PORT_SOUTH), pill(wire.to_kind, PORT_NORTH))
                    } else {
                        (Some(PORT_NORTH), pill(wire.to_kind, PORT_SOUTH))
                    }
                }
            };
            edges.push(DraftEdge {
                from: wire.from,
                from_port,
                to: wire.to,
                to_port,
                kind: wire.kind,
                stroke: wire.stroke,
                arrow: Arrow::End,
                label: label.filter(|_| show).map(|text| EdgeText {
                    text,
                    font_size: theme.small_font_size,
                    color: theme.text,
                }),
                target: wire.elements.first().map_or(HitTarget::None, |e| HitTarget::Element(model.key_of(*e))),
                meta: Meta::new(wire.elements, Anchor::Chains(wire.chains)),
                on_cycle: false,
            });
        }
        draft.edges.extend(edges);
        for &(machine, node) in stubs {
            let links = stub_links.get(&node).copied().unwrap_or(0);
            let name = &model.machine(machine).name;
            let stub = &mut draft.nodes[node];
            stub.look = painter.stub(name, links, painter.machine(machine));
            stub.target = HitTarget::MachineStub { machine: name.clone(), links };
        }
    }
}
