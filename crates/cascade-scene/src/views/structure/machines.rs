//! Drafting one machine of the structure view: its lane, states, nested
//! bands and transition pills, honouring collapse and the entity filter.

use std::collections::{BTreeSet, HashMap};

use cascade_core::model::StateKind;
use cascade_core::{CausalGraph, ElementKey, ElementRef, MachineId, Model, StateId};
use cascade_layout::{Insets, LayerConstraint};

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
    /// The transition's own pill, which has ports.
    Pill,
    /// A collapsed state or machine standing in for it.
    Other,
    /// A hidden machine's stub.
    Stub,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Endpoint {
    pub node: usize,
    pub kind: EndKind,
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
            endpoints[t.index()] = Some(Endpoint { node: stub, kind: EndKind::Stub });
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
            endpoints[t.index()] = Some(Endpoint { node, kind: EndKind::Other });
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
                endpoints[t.index()] = Some(Endpoint { node: a, kind: EndKind::Other });
                continue;
            }
            let group = common_scope(model, rf, rt).and_then(|p| band_of.get(&p).copied()).unwrap_or(top);
            let key = model.key_of(ElementRef::Transition(t));
            let label = format!("{} → {}", model.state(tr.from).path, model.state(tr.to).path);
            let meta = Meta::new(vec![ElementRef::Transition(t)], Anchor::Nodes(causal));
            let pill = draft.add_node(DraftNode {
                key: key.to_string(),
                group: Some(group),
                layer: LayerConstraint::Free,
                ports: standard_ports(),
                look: painter.pill(label, model.trigger(tr.trigger).name.clone(), style),
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
            draft.edges.push(arrow(a, None, pill, Some(PORT_WEST), Arrow::None, bracketed(tr.guard.as_deref())));
            draft.edges.push(arrow(pill, Some(PORT_EAST), b, None, Arrow::End, None));
            endpoints[t.index()] = Some(Endpoint { node: pill, kind: EndKind::Pill });
        }
        MachinePlan::Expanded { machine: m, top, bands }
    }
}

/// The deepest compound state containing both `a` and `b` (strictly), or
/// `None` for the machine's top level.
fn common_scope(model: &Model, a: StateId, b: StateId) -> Option<StateId> {
    let above_b: Vec<StateId> = model.ancestors(b).collect();
    model.ancestors(a).find(|s| above_b.contains(s))
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
