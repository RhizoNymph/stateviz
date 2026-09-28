//! `stateDiagram-v2`: every machine's states and transitions.
//!
//! Representation:
//!
//! - Front matter `title:` when the definition names the system.
//! - Each machine is a composite `state "Order" as Order { … }`; compound
//!   states are nested composites, other states `state "draft" as
//!   Order_draft`. History states are labelled `H` (shallow) or `H*` (deep).
//! - `[*] --> x` inside each composite marks its initial child; the
//!   machine's initial state (a path) is lifted to its top-level ancestor.
//!   Final states get `x --> [*]` inside their parent composite.
//! - Transitions are labelled `trigger [guard]`.
//!
//! Limits:
//!
//! - Mermaid cannot draw a transition between internal states of different
//!   composites, so each transition is drawn in the innermost composite that
//!   contains both endpoints, from and to the endpoints' ancestors that are
//!   direct children of it. When an endpoint was lifted, the label carries
//!   the real endpoints: `finish (running.computing → done)`.
//! - A machine initial deeper than the top level shows as its top-level
//!   ancestor (the compound's own `[*]` marker continues from there).
//! - Emits, events, controllers and external sources are not drawn; the
//!   causal diagram shows those.

use std::collections::HashMap;
use std::fmt::Write as _;

use cascade_core::model::StateKind;
use cascade_core::{Model, StateId, TransitionId};

use super::escape;
use super::ids::{IdAllocator, sanitize};

const INDENT: &str = "    ";

/// Every machine's structure as a Mermaid `stateDiagram-v2`.
pub(crate) fn structure(model: &Model) -> String {
    let mut out = String::new();
    if let Some(system) = &model.definition().system {
        out.push_str("---\n");
        let _ = writeln!(out, "title: {}", escape::yaml_string(&system.value));
        out.push_str("---\n");
    }
    out.push_str("stateDiagram-v2\n");

    let ids = Ids::allocate(model);
    for (index, (mid, machine)) in model.machines().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let mut placed: HashMap<Option<StateId>, Vec<TransitionId>> = HashMap::new();
        for &t in &machine.transitions {
            let transition = model.transition(t);
            placed.entry(container(model, transition.from, transition.to)).or_default().push(t);
        }
        let _ = writeln!(out, "{INDENT}state \"{}\" as {} {{", escape::label(&machine.name), ids.machines[mid.index()]);
        let initial = top_level_ancestor(model, machine.initial);
        let mut writer = Writer { model, ids: &ids, placed: &placed, out: &mut out };
        writer.composite(None, &machine.top_states, initial, 2);
        let _ = writeln!(out, "{INDENT}}}");
    }
    out
}

/// Ids for every machine and state, allocated in definition order so the
/// output is deterministic.
struct Ids {
    machines: Vec<String>,
    states: Vec<String>,
}

impl Ids {
    fn allocate(model: &Model) -> Self {
        let mut allocator = IdAllocator::default();
        let mut machines = Vec::with_capacity(model.machine_count());
        let mut states = vec![String::new(); model.state_count()];
        for (_, machine) in model.machines() {
            machines.push(allocator.allocate(&machine.name));
            let prefix = sanitize(&machine.name);
            for &s in &machine.states {
                let path = model.state(s).path.replace('.', "_");
                states[s.index()] = allocator.allocate(&format!("{prefix}_{path}"));
            }
        }
        Self { machines, states }
    }

    fn state(&self, id: StateId) -> &str {
        &self.states[id.index()]
    }
}

struct Writer<'a> {
    model: &'a Model,
    ids: &'a Ids,
    placed: &'a HashMap<Option<StateId>, Vec<TransitionId>>,
    out: &'a mut String,
}

impl Writer<'_> {
    /// The body of a composite: initial marker, child states (recursing into
    /// compound ones), the transitions placed here, and final markers.
    fn composite(&mut self, container: Option<StateId>, children: &[StateId], initial: StateId, depth: usize) {
        let pad = INDENT.repeat(depth);
        let _ = writeln!(self.out, "{pad}[*] --> {}", self.ids.state(initial));
        for &child in children {
            let state = self.model.state(child);
            let id = self.ids.state(child);
            match &state.kind {
                StateKind::Compound { children: nested, initial: nested_initial } => {
                    let _ = writeln!(self.out, "{pad}state \"{}\" as {id} {{", escape::label(&state.name));
                    self.composite(Some(child), nested, *nested_initial, depth + 1);
                    let _ = writeln!(self.out, "{pad}}}");
                }
                StateKind::History { deep } => {
                    let label = if *deep { "H*" } else { "H" };
                    let _ = writeln!(self.out, "{pad}state \"{label}\" as {id}");
                }
                StateKind::Atomic | StateKind::Final => {
                    let _ = writeln!(self.out, "{pad}state \"{}\" as {id}", escape::label(&state.name));
                }
            }
        }
        for &t in self.placed.get(&container).map(Vec::as_slice).unwrap_or_default() {
            let line = self.transition_line(container, t);
            let _ = writeln!(self.out, "{pad}{line}");
        }
        for &child in children {
            if self.model.state(child).is_final() {
                let _ = writeln!(self.out, "{pad}{} --> [*]", self.ids.state(child));
            }
        }
    }

    fn transition_line(&self, container: Option<StateId>, t: TransitionId) -> String {
        let model = self.model;
        let transition = model.transition(t);
        let from = lift(model, transition.from, container);
        let to = lift(model, transition.to, container);
        let mut label = model.trigger(transition.trigger).name.clone();
        if let Some(guard) = &transition.guard {
            let _ = write!(label, " [{guard}]");
        }
        if from != transition.from || to != transition.to {
            let _ = write!(label, " ({} → {})", model.state(transition.from).path, model.state(transition.to).path);
        }
        format!("{} --> {} : {}", self.ids.state(from), self.ids.state(to), escape::transition_label(&label))
    }
}

/// The innermost state that is a proper ancestor of both endpoints, or
/// `None` for the machine itself.
fn container(model: &Model, from: StateId, to: StateId) -> Option<StateId> {
    model.ancestors(from).find(|&a| a != to && model.is_ancestor_or_self(a, to))
}

/// The ancestor-or-self of `state` that is a direct child of `container`.
fn lift(model: &Model, state: StateId, container: Option<StateId>) -> StateId {
    std::iter::once(state).chain(model.ancestors(state)).find(|&s| model.state(s).parent == container).unwrap_or(state)
}

fn top_level_ancestor(model: &Model, state: StateId) -> StateId {
    lift(model, state, None)
}
