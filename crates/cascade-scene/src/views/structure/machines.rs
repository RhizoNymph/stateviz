//! Drafting one machine of the structure view: its lane, states, nested
//! bands and transition pills, honouring collapse and the entity filter.

use std::collections::{BTreeSet, HashMap, VecDeque};

use cascade_core::model::StateKind;
use cascade_core::{CausalGraph, ElementKey, ElementRef, MachineId, Model, StateId};
use cascade_layout::{Insets, LayerConstraint, Port, PortSide};

use crate::emphasis::Anchor;
use crate::scene::{Arrow, EdgeKind, HitTarget};
use crate::views::draft::{DraftEdge, DraftGraph, DraftGroup, DraftNode, EdgeText, Meta, standard_ports};
use crate::views::style::{Painter, StateMark, bracketed};

/// Pill ports (see `draft::standard_ports`): state → pill enters West,
/// pill → state leaves East.
pub(super) const PORT_WEST: u16 = 0;
pub(super) const PORT_EAST: u16 = 1;
pub(super) const PORT_NORTH: u16 = 2;
pub(super) const PORT_SOUTH: u16 = 3;
/// Edit mode only: a second South port where emits leave for a gutter
/// below, right of [`PORT_SOUTH`] where fires and triggers from below
/// arrive.
pub(super) const PORT_SOUTH_OUT: u16 = 4;
/// Edit mode only: a second North port where emits leave for a gutter
/// above, right of [`PORT_NORTH`] where fires and triggers from above
/// arrive.
pub(super) const PORT_NORTH_OUT: u16 = 5;

/// Pill ports in edit mode: the standard four plus [`PORT_SOUTH_OUT`] and
/// [`PORT_NORTH_OUT`].
fn edit_pill_ports() -> Vec<Port> {
    let mut ports = standard_ports();
    ports.push(Port { side: PortSide::South });
    ports.push(Port { side: PortSide::North });
    ports
}

/// Collapsed composite states and machines.
pub(super) struct Collapse {
    machines: BTreeSet<MachineId>,
    states: BTreeSet<StateId>,
}

impl Collapse {
    /// Keys that name no element, or a state that is not compound, are
    /// ignored.
    pub fn resolve(model: &Model, keys: &BTreeSet<ElementKey>) -> Self {
        let mut machines = BTreeSet::new();
        let mut states = BTreeSet::new();
        for key in keys {
            match model.resolve_key(key) {
                Some(ElementRef::Machine(m)) => {
                    machines.insert(m);
                }
                Some(ElementRef::State(s)) if model.state(s).is_compound() => {
                    states.insert(s);
                }
                _ => {}
            }
        }
        Self { machines, states }
    }

    pub fn machine(&self, m: MachineId) -> bool {
        self.machines.contains(&m)
    }

    /// Drawn: no strict ancestor is collapsed.
    fn visible(&self, model: &Model, s: StateId) -> bool {
        !model.ancestors(s).any(|a| self.states.contains(&a))
    }

    /// The node that stands for `s`: its outermost collapsed
    /// ancestor-or-self, else itself.
    fn rep(&self, model: &Model, s: StateId) -> StateId {
        std::iter::once(s).chain(model.ancestors(s)).filter(|a| self.states.contains(a)).last().unwrap_or(s)
    }
}

/// Where a transition attaches for cross-lane links.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EndKind {
    /// The transition's own pill, or its junction in arrow mode; both have
    /// ports.
    Pill,
    /// A collapsed state or machine standing in for it.
    Other,
    /// A hidden machine's stub.
    Stub,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Endpoint {
    pub node: usize,
    pub kind: EndKind,
    /// Roughly which column of its lane the node sits in (0 for anything
    /// but a pill), so the build canvas can line wiring up with it before
    /// layout.
    pub column: f32,
}

impl Endpoint {
    fn other(node: usize, kind: EndKind) -> Self {
        Self { node, kind, column: 0.0 }
    }
}

/// What was drafted for a machine, for building its lane after layout.
pub(super) enum MachinePlan {
    Hidden { machine: MachineId, stub: usize },
    Collapsed { machine: MachineId, group: usize },
    Expanded { machine: MachineId, top: usize, bands: Vec<(StateId, usize)> },
}

/// Shared inputs for drafting machines.
pub(super) struct Drafter<'a> {
    pub model: &'a Model,
    pub graph: &'a CausalGraph,
    pub painter: &'a Painter<'a>,
    pub collapse: &'a Collapse,
    /// Edit mode: pills get [`PORT_SOUTH_OUT`] too.
    pub edit: bool,
    /// Transitions as pills (`ViewState::transition_pills`), or as arrows
    /// through a junction (see `arrows`).
    pub pills: bool,
}

impl Drafter<'_> {
    fn lane_group(&self, key: String) -> DraftGroup {
        DraftGroup {
            key,
            padding: Insets { top: 8.0, right: 16.0, bottom: 12.0, left: 16.0 },
            header: self.painter.line_height(self.painter.theme.font_size) + 10.0,
        }
    }

    fn transition_nodes(&self, m: MachineId) -> Vec<cascade_core::NodeIx> {
        self.model
            .machine(m)
            .transitions
            .iter()
            .filter_map(|&t| self.graph.ix_of_element(ElementRef::Transition(t)))
            .collect()
    }

    /// A hidden machine: a stub in a thin band of its own. Its look and
    /// link count are finished once links are known.
    pub fn hidden(&self, draft: &mut DraftGraph, m: MachineId, endpoints: &mut [Option<Endpoint>]) -> MachinePlan {
        let machine = self.model.machine(m);
        let key = self.model.key_of(ElementRef::Machine(m));
        let group = draft.add_group(DraftGroup { key: key.to_string(), padding: Insets::uniform(6.0), header: 0.0 });
        let stub = draft.add_node(DraftNode {
            key: key.to_string(),
            group: Some(group),
            layer: LayerConstraint::Free,
            ports: Vec::new(),
            look: self.painter.stub(&machine.name, 0, self.painter.machine(m)),
            target: HitTarget::MachineStub { machine: machine.name.clone(), links: 0 },
            meta: Meta::new(vec![ElementRef::Machine(m)], Anchor::Nodes(self.transition_nodes(m))),
        });
        for &t in &machine.transitions {
            endpoints[t.index()] = Some(Endpoint::other(stub, EndKind::Stub));
        }
        MachinePlan::Hidden { machine: m, stub }
    }

    /// A collapsed machine: one node in a collapsed lane.
    pub fn collapsed(&self, draft: &mut DraftGraph, m: MachineId, endpoints: &mut [Option<Endpoint>]) -> MachinePlan {
        let machine = self.model.machine(m);
        let key = self.model.key_of(ElementRef::Machine(m));
        let group = draft.add_group(self.lane_group(key.to_string()));
        let detail =
            format!("{} · {}", count(machine.states.len(), "state"), count(machine.transitions.len(), "transition"));
        let node = draft.add_node(DraftNode {
            key: key.to_string(),
            group: Some(group),
            layer: LayerConstraint::Free,
            ports: Vec::new(),
            look: self.painter.collapsed_machine(&machine.name, detail, self.painter.machine(m)),
            target: HitTarget::Element(key),
            meta: Meta::new(vec![ElementRef::Machine(m)], Anchor::Nodes(self.transition_nodes(m))),
        });
        for &t in &machine.transitions {
            endpoints[t.index()] = Some(Endpoint::other(node, EndKind::Other));
        }
        MachinePlan::Collapsed { machine: m, group }
    }

    /// An expanded machine: a lane with its visible states, a band per
    /// expanded compound state, and a pill per drawn transition.
    pub fn expanded(&self, draft: &mut DraftGraph, m: MachineId, endpoints: &mut [Option<Endpoint>]) -> MachinePlan {
        let model = self.model;
        let painter = self.painter;
        let theme = painter.theme;
        let machine = model.machine(m);
        let style = painter.machine(m);
        let top = draft.add_group(self.lane_group(model.key_of(ElementRef::Machine(m)).to_string()));

        let mut bands = Vec::new();
        let mut band_of: HashMap<StateId, usize> = HashMap::new();
        for &s in &machine.states {
            if model.state(s).is_compound() && self.collapse.visible(model, s) && !self.collapse.states.contains(&s) {
                let band = draft.add_group(DraftGroup {
                    key: model.key_of(ElementRef::State(s)).to_string(),
                    padding: Insets::uniform(10.0),
                    header: painter.line_height(theme.small_font_size) + 8.0,
                });
                band_of.insert(s, band);
                bands.push((s, band));
            }
        }
        let scope_group = |s: StateId| model.state(s).parent.and_then(|p| band_of.get(&p).copied()).unwrap_or(top);
        let columns = Columns::estimate(model, m, self.collapse, &band_of, top);

        let mut state_node: HashMap<StateId, usize> = HashMap::new();
        for &s in &machine.states {
            if !self.collapse.visible(model, s) {
                continue;
            }
            let state = model.state(s);
            let initial = machine.initial == s
                || state.parent.is_some_and(
                    |p| matches!(&model.state(p).kind, StateKind::Compound { initial, .. } if *initial == s),
                );
            let mark = match state.kind {
                StateKind::Final => StateMark::Final,
                StateKind::History { deep } => StateMark::History { deep },
                StateKind::Atomic | StateKind::Compound { .. } => StateMark::Normal,
            };
            let detail = state.is_compound().then(|| {
                let inside = model.state_count_within(s);
                let arrow = if self.collapse.states.contains(&s) { "▸" } else { "▾" };
                format!("{arrow} {}", count(inside, "state"))
            });
            let key = model.key_of(ElementRef::State(s));
            let node = draft.add_node(DraftNode {
                key: key.to_string(),
                group: Some(scope_group(s)),
                layer: LayerConstraint::Free,
                ports: Vec::new(),
                look: painter.state(state.name.clone(), detail, style, initial, mark),
                target: HitTarget::Element(key),
                meta: Meta::new(vec![ElementRef::State(s)], Anchor::Nodes(Vec::new())),
            });
            state_node.insert(s, node);
        }

        for &t in &machine.transitions {
            let tr = model.transition(t);
            let (rf, rt) = (self.collapse.rep(model, tr.from), self.collapse.rep(model, tr.to));
            let (Some(&a), Some(&b)) = (state_node.get(&rf), state_node.get(&rt)) else { continue };
            let causal: Vec<_> = self.graph.ix_of_element(ElementRef::Transition(t)).into_iter().collect();
            for end in [a, b] {
                if let Anchor::Nodes(nodes) = &mut draft.nodes[end].meta.anchor {
                    nodes.extend(causal.iter().copied());
                }
            }
            if rf == rt && (rf != tr.from || rt != tr.to) {
                // Inside a collapsed state: no pill; links attach to it.
                endpoints[t.index()] = Some(Endpoint::other(a, EndKind::Other));
                continue;
            }
            let group = common_scope(model, rf, rt).and_then(|p| band_of.get(&p).copied()).unwrap_or(top);
            let key = model.key_of(ElementRef::Transition(t));
            let meta = Meta::new(vec![ElementRef::Transition(t)], Anchor::Nodes(causal));
            let trigger = model.trigger(tr.trigger).name.clone();
            // A pill reads `from → to` over the trigger and its arrow in
            // carries the guard; an arrow carries both on its way in, and
            // its junction stands where the pill would be.
            let (look, label_in) = if self.pills {
                let label = format!("{} → {}", model.state(tr.from).path, model.state(tr.to).path);
                (painter.pill(label, trigger, style), bracketed(tr.guard.as_deref()))
            } else {
                (painter.junction(), Some(arrow_label(&trigger, tr.guard.as_deref())))
            };
            let pill = draft.add_node(DraftNode {
                key: key.to_string(),
                group: Some(group),
                layer: LayerConstraint::Free,
                ports: if self.edit { edit_pill_ports() } else { standard_ports() },
                look,
                target: HitTarget::Element(key.clone()),
                meta: meta.clone(),
            });
            let arrow = |from, from_port, to, to_port, arrow, label: Option<String>| DraftEdge {
                from,
                from_port,
                to,
                to_port,
                kind: EdgeKind::Transition,
                stroke: painter.transition_stroke(style),
                arrow,
                label: label.map(|text| EdgeText { text, font_size: theme.small_font_size, color: theme.text }),
                target: HitTarget::Element(key.clone()),
                meta: meta.clone(),
                on_cycle: false,
            };
            draft.edges.push(arrow(a, None, pill, Some(PORT_WEST), Arrow::None, label_in));
            draft.edges.push(arrow(pill, Some(PORT_EAST), b, None, Arrow::End, None));
            let column = columns.pill(model, rf, group);
            endpoints[t.index()] = Some(Endpoint { node: pill, kind: EndKind::Pill, column });
        }
        MachinePlan::Expanded { machine: m, top, bands }
    }
}

/// Estimated columns of a machine's pills, from breadth-first depth in
/// each band: the layered layout puts a band's states in roughly these
/// layers from its initial state, a pill between two of them.
struct Columns {
    /// Per band (layout group): each drawn state's depth from the band's
    /// initial state. States nothing reaches count as depth 0.
    depth: HashMap<(usize, StateId), u32>,
    /// The band of each drawn compound state's children.
    band_of: HashMap<StateId, usize>,
    top: usize,
}

impl Columns {
    fn estimate(
        model: &Model,
        m: MachineId,
        collapse: &Collapse,
        band_of: &HashMap<StateId, usize>,
        top: usize,
    ) -> Self {
        let machine = model.machine(m);
        let mut this = Self { depth: HashMap::new(), band_of: band_of.clone(), top };
        let mut next: HashMap<usize, Vec<(StateId, StateId)>> = HashMap::new();
        for &t in &machine.transitions {
            let tr = model.transition(t);
            let (rf, rt) = (collapse.rep(model, tr.from), collapse.rep(model, tr.to));
            let group = common_scope(model, rf, rt).and_then(|p| band_of.get(&p).copied()).unwrap_or(top);
            if let (Some(a), Some(b)) = (this.project(model, rf, group), this.project(model, rt, group))
                && a != b
            {
                next.entry(group).or_default().push((a, b));
            }
        }
        let mut starts: Vec<(usize, StateId)> = band_of
            .iter()
            .filter_map(|(&s, &g)| match &model.state(s).kind {
                StateKind::Compound { initial, .. } => Some((g, *initial)),
                StateKind::Atomic | StateKind::Final | StateKind::History { .. } => None,
            })
            .collect();
        starts.extend(this.project(model, machine.initial, top).map(|s| (top, s)));
        for (group, start) in starts {
            let edges = next.get(&group).map(Vec::as_slice).unwrap_or(&[]);
            let mut queue = VecDeque::from([(start, 0u32)]);
            while let Some((s, d)) = queue.pop_front() {
                if this.depth.contains_key(&(group, s)) {
                    continue;
                }
                this.depth.insert((group, s), d);
                queue.extend(edges.iter().filter(|(a, _)| *a == s).map(|&(_, b)| (b, d + 1)));
            }
        }
        this
    }

    /// The ancestor-or-self of `s` drawn in `group`.
    fn project(&self, model: &Model, s: StateId, group: usize) -> Option<StateId> {
        std::iter::once(s).chain(model.ancestors(s)).find(|&a| {
            let own = model.state(a).parent.and_then(|p| self.band_of.get(&p).copied()).unwrap_or(self.top);
            own == group
        })
    }

    /// A pill leaving `from` in `group` sits right of its source state.
    fn pill(&self, model: &Model, from: StateId, group: usize) -> f32 {
        let depth = self.project(model, from, group).and_then(|s| self.depth.get(&(group, s)).copied()).unwrap_or(0);
        (2 * depth + 1) as f32
    }
}

/// The deepest compound state containing both `a` and `b` (strictly), or
/// `None` for the machine's top level.
fn common_scope(model: &Model, a: StateId, b: StateId) -> Option<StateId> {
    let above_b: Vec<StateId> = model.ancestors(b).collect();
    model.ancestors(a).find(|s| above_b.contains(s))
}

/// An arrow's label: the trigger, then ` [guard]` when there is one.
fn arrow_label(trigger: &str, guard: Option<&str>) -> String {
    match bracketed(guard) {
        Some(guard) => format!("{trigger} {guard}"),
        None => trigger.to_owned(),
    }
}

/// "1 state", "3 states".
fn count(n: usize, noun: &str) -> String {
    if n == 1 { format!("1 {noun}") } else { format!("{n} {noun}s") }
}

/// Number of states nested (at any depth) inside `s`.
trait StatesWithin {
    fn state_count_within(&self, s: StateId) -> usize;
}

impl StatesWithin for Model {
    fn state_count_within(&self, s: StateId) -> usize {
        self.state(s).children().iter().map(|&c| 1 + self.state_count_within(c)).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_labels_read_trigger_then_guard() {
        assert_eq!(arrow_label("pay", None), "pay");
        assert_eq!(arrow_label("pay", Some("  ")), "pay");
        assert_eq!(arrow_label("pay", Some("amount > 0")), "pay [amount > 0]");
    }
}
