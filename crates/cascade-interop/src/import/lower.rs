//! Chart IR → [`Definition`].
//!
//! Per machine:
//!
//! 1. Names: machine names are unique across the import, state names among
//!    siblings, trigger names per machine, event names globally; every
//!    invalid or colliding name is sanitized with a warning.
//! 2. States become a [`StateDef`] tree. An explicit initial state is kept;
//!    a compound state whose first child is a history state gets an explicit
//!    initial (its first non-history child), since Cascade would otherwise
//!    default to the history state.
//! 3. Transitions: one [`TransitionDef`] per edge, with full state paths.
//!    Machine-level edges (XState root `on`) become one transition whose
//!    `from` lists every top-level normal state. Targetless edges become
//!    self-transitions. Edges out of final or history states are dropped.
//! 4. Emits: the edge's own emits plus the entry/exit actions its states
//!    run, in execution order (exits innermost first, then the transition's
//!    actions, then entries outermost first, following the target's default
//!    entry chain).
//!
//! Then [`super::wiring`] adds controllers and external sources.

use std::collections::{HashMap, HashSet};

use cascade_core::definition::{MachineDef, StateDef, StateKindDef, TransitionDef};
use cascade_core::{Definition, SourceSpan, Spanned};

use crate::error::InteropError;
use crate::import::chart::{Chart, Delay, Edge, Emit, NodeIx, NodeKind, TriggerSpec};
use crate::import::names::{KeyedNames, NameTable, sanitize};
use crate::import::wiring::{self, Wiring};
use crate::import::{ImportOptions, NameKind, Warnings};

/// Everything an importer hands to the lowering.
pub(crate) struct LowerInput {
    /// Display name of the input format, for errors.
    pub format: &'static str,
    pub system: Option<String>,
    pub charts: Vec<Chart>,
    pub wiring: Wiring,
}

/// Where the triggers of a machine come from, for external source wiring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TriggerOrigin {
    /// A named event: routed from an emit, or else from the environment.
    Event,
    /// A delay: fired by the clock.
    Delay,
    /// An invoked actor finishing or failing.
    Actor(String),
    /// A compound state completing (approximated as an environment trigger).
    Completion,
}

/// One lowered machine plus what wiring needs to know about it.
pub(crate) struct LoweredMachine {
    pub raw_name: String,
    pub def: MachineDef,
    /// Raw event name → this machine's trigger name, for named events.
    pub event_triggers: HashMap<String, String>,
    /// Every trigger with where it comes from, in first-use order.
    pub triggers: Vec<(String, TriggerOrigin, String)>,
    /// Every emit on a transition (after entry/exit attribution).
    pub emits: Vec<(Emit, String)>,
}

pub(crate) fn lower(
    input: LowerInput,
    options: &ImportOptions,
    warnings: &mut Warnings,
) -> Result<Definition, InteropError> {
    let mut lowerer = Lowerer { format: input.format, warnings, events: KeyedNames::new(NameKind::Event) };
    let mut machine_names = NameTable::new(NameKind::Machine);
    let mut machines = Vec::with_capacity(input.charts.len());
    for chart in &input.charts {
        let name = machine_names.claim(chart.name(), &chart.root().location, lowerer.warnings);
        machines.push(lowerer.machine(chart, name)?);
    }
    let wired = wiring::wire(input.wiring, &machines, options, lowerer.warnings, &mut lowerer.events, input.format)?;
    Ok(Definition {
        system: input.system.map(Spanned::synthetic),
        machines: machines.into_iter().map(|m| m.def).collect(),
        events: wired.events,
        controllers: wired.controllers,
        external: wired.external,
    })
}

fn synthetic(value: impl Into<String>) -> Spanned<String> {
    Spanned::synthetic(value.into())
}

/// Per-machine names: local state names and full paths by node.
struct StateNames {
    local: Vec<String>,
    path: Vec<String>,
}

struct Lowerer<'a> {
    format: &'static str,
    warnings: &'a mut Warnings,
    events: KeyedNames<String>,
}

impl Lowerer<'_> {
    fn unsupported(&self, location: &str, what: impl Into<String>) -> InteropError {
        InteropError::Unsupported { format: self.format, location: location.to_owned(), what: what.into() }
    }

    fn invalid(&self, location: &str, message: impl Into<String>) -> InteropError {
        InteropError::Invalid { format: self.format, location: location.to_owned(), message: message.into() }
    }

    fn machine(&mut self, chart: &Chart, name: String) -> Result<LoweredMachine, InteropError> {
        let root = chart.root();
        if root.children.is_empty() {
            return Err(self.unsupported(&root.location, "a machine without states"));
        }
        let names = self.state_names(chart);
        let states = root.children.iter().map(|&c| self.state_def(chart, c, &names)).collect::<Result<Vec<_>, _>>()?;
        let initial = self.initial_of(chart, NodeIx::ROOT, &names)?;

        let mut lowered = LoweredMachine {
            raw_name: chart.name().to_owned(),
            def: MachineDef {
                name: synthetic(name),
                color: chart.attrs.color.map(Spanned::synthetic),
                domain: chart.attrs.domain.clone().map(Spanned::synthetic),
                initial: initial.map(Spanned::synthetic),
                fields: chart.attrs.fields.iter().map(|f| synthetic(f.as_str())).collect(),
                states,
                transitions: Vec::new(),
                span: SourceSpan::unknown(),
            },
            event_triggers: HashMap::new(),
            triggers: Vec::new(),
            emits: Vec::new(),
        };
        let mut triggers: KeyedNames<(TriggerSpec, Option<NodeIx>)> = KeyedNames::new(NameKind::Trigger);
        let mut seen_triggers = HashSet::new();
        self.warn_unattributed_start_emits(chart);
        for (ix, node) in chart.nodes.iter().enumerate() {
            let source = NodeIx(ix);
            for edge in &node.edges {
                let Some(from) = self.edge_sources(chart, source, edge, &names) else {
                    continue;
                };
                let trigger = self.trigger_name(chart, source, edge, &names, &mut triggers);
                let origin = match &edge.trigger {
                    TriggerSpec::Event(raw) => {
                        lowered.event_triggers.entry(raw.clone()).or_insert_with(|| trigger.clone());
                        TriggerOrigin::Event
                    }
                    TriggerSpec::After(_) => TriggerOrigin::Delay,
                    TriggerSpec::ActorDone(actor) | TriggerSpec::ActorError(actor) => {
                        TriggerOrigin::Actor(actor.clone())
                    }
                    TriggerSpec::StateDone => TriggerOrigin::Completion,
                };
                if seen_triggers.insert(trigger.clone()) {
                    lowered.triggers.push((trigger.clone(), origin, edge.location.clone()));
                }
                let emits = attributed_emits(chart, source, edge);
                let mut emit_names: Vec<Spanned<String>> = Vec::new();
                for emit in &emits {
                    let name = self.events.name(&emit.event, &emit.event, &edge.location, self.warnings);
                    if !emit_names.iter().any(|e| e.value == name) {
                        emit_names.push(synthetic(name));
                    }
                    lowered.emits.push((emit.clone(), edge.location.clone()));
                }
                let targets: Vec<(Vec<String>, String)> = match self.edge_target(chart, edge, &names) {
                    Some(to) => vec![(from, to)],
                    // Targetless: a self-transition on each source.
                    None => from.into_iter().map(|f| (vec![f.clone()], f)).collect(),
                };
                for (from, to) in targets {
                    lowered.def.transitions.push(TransitionDef {
                        from: from.into_iter().map(synthetic).collect(),
                        to: synthetic(to),
                        on: synthetic(trigger.as_str()),
                        guard: edge.guard.clone().map(Spanned::synthetic),
                        emits: emit_names.clone(),
                        bounded: edge.bounded,
                        span: SourceSpan::unknown(),
                    });
                }
            }
        }
        Ok(lowered)
    }

    fn state_names(&mut self, chart: &Chart) -> StateNames {
        let mut names =
            StateNames { local: vec![String::new(); chart.nodes.len()], path: vec![String::new(); chart.nodes.len()] };
        // Nodes are created parents first, so one pass in index order sees
        // every parent's name before its children.
        for (ix, node) in chart.nodes.iter().enumerate() {
            let mut siblings = NameTable::new(NameKind::State);
            for &child in &node.children {
                let raw = &chart.node(child).name;
                let local = siblings.claim(raw, &chart.node(child).location, self.warnings);
                names.path[child.0] = if ix == 0 { local.clone() } else { format!("{}.{local}", names.path[ix]) };
                names.local[child.0] = local;
            }
        }
        names
    }

    fn state_def(&mut self, chart: &Chart, ix: NodeIx, names: &StateNames) -> Result<StateDef, InteropError> {
        let node = chart.node(ix);
        let kind = match node.kind {
            NodeKind::Normal => StateKindDef::Normal,
            NodeKind::Final => StateKindDef::Final,
            NodeKind::History { deep: false } => StateKindDef::History,
            NodeKind::History { deep: true } => StateKindDef::DeepHistory,
        };
        if kind != StateKindDef::Normal && !node.children.is_empty() {
            return Err(self.invalid(&node.location, format!("a {} state cannot have child states", kind.name())));
        }
        let initial = self.initial_of(chart, ix, names)?;
        let states = node.children.iter().map(|&c| self.state_def(chart, c, names)).collect::<Result<Vec<_>, _>>()?;
        Ok(StateDef {
            name: synthetic(names.local[ix.0].as_str()),
            kind: Spanned::synthetic(kind),
            initial: initial.map(Spanned::synthetic),
            states,
            span: SourceSpan::unknown(),
        })
    }

    /// The `initial:` to write for a node: a path for the machine, a child
    /// name for a compound state, or `None` to keep Cascade's default (the
    /// first child).
    fn initial_of(&mut self, chart: &Chart, ix: NodeIx, names: &StateNames) -> Result<Option<String>, InteropError> {
        let node = chart.node(ix);
        let Some(&first) = node.children.first() else {
            return Ok(None);
        };
        let name_of = |n: NodeIx| if ix == NodeIx::ROOT { names.path[n.0].clone() } else { names.local[n.0].clone() };
        match node.initial {
            Some(initial) => {
                if matches!(chart.node(initial).kind, NodeKind::History { .. }) {
                    return Err(self.unsupported(&node.location, "an initial state that is a history state"));
                }
                if ix == NodeIx::ROOT {
                    return Ok(Some(names.path[initial.0].clone()));
                }
                match chart.child_toward(ix, initial) {
                    Some(child) if child == initial => Ok(Some(name_of(child))),
                    Some(child) => {
                        self.warnings.approximated(
                            node.location.as_str(),
                            format!("initial state `{}`, which is not a direct child", chart.display_path(initial)),
                            format!(
                                "`{}` starts in `{}` and enters its default child",
                                chart.display_path(ix),
                                names.local[child.0]
                            ),
                        );
                        Ok(Some(name_of(child)))
                    }
                    None => Err(self.invalid(
                        &node.location,
                        format!(
                            "initial state `{}` is not inside `{}`",
                            chart.display_path(initial),
                            chart.display_path(ix)
                        ),
                    )),
                }
            }
            None if matches!(chart.node(first).kind, NodeKind::History { .. }) => match chart.default_child(ix) {
                Some(child) => Ok(Some(name_of(child))),
                None => Err(self.invalid(&node.location, "every child state is a history state")),
            },
            None => Ok(None),
        }
    }

    /// The `from` paths of an edge, or `None` (with a warning) when the edge
    /// cannot be represented.
    fn edge_sources(&mut self, chart: &Chart, source: NodeIx, edge: &Edge, names: &StateNames) -> Option<Vec<String>> {
        if source == NodeIx::ROOT {
            let from: Vec<String> = chart
                .root()
                .children
                .iter()
                .filter(|&&c| chart.node(c).kind == NodeKind::Normal)
                .map(|&c| names.path[c.0].clone())
                .collect();
            if from.is_empty() {
                self.warnings
                    .ignored(edge.location.as_str(), "a machine-level transition: no top-level state can take it");
                return None;
            }
            return Some(from);
        }
        match chart.node(source).kind {
            NodeKind::Normal => Some(vec![names.path[source.0].clone()]),
            NodeKind::Final => {
                self.warnings.ignored(edge.location.as_str(), "a transition out of a final state");
                None
            }
            NodeKind::History { .. } => {
                self.warnings.ignored(edge.location.as_str(), "a transition out of a history state");
                None
            }
        }
    }

    /// The target path, or `None` for a targetless edge.
    fn edge_target(&mut self, chart: &Chart, edge: &Edge, names: &StateNames) -> Option<String> {
        let target = edge.target?;
        if target != NodeIx::ROOT {
            return Some(names.path[target.0].clone());
        }
        // A transition to the machine itself restarts it.
        let mut entry = chart.root().initial.or_else(|| chart.default_child(NodeIx::ROOT))?;
        if matches!(chart.node(entry).kind, NodeKind::History { .. }) {
            entry = chart.default_child(NodeIx::ROOT)?;
        }
        self.warnings.approximated(
            edge.location.as_str(),
            "a transition to the machine itself",
            format!("it targets the initial state `{}`", names.path[entry.0]),
        );
        Some(names.path[entry.0].clone())
    }

    fn trigger_name(
        &mut self,
        chart: &Chart,
        source: NodeIx,
        edge: &Edge,
        names: &StateNames,
        triggers: &mut KeyedNames<(TriggerSpec, Option<NodeIx>)>,
    ) -> String {
        let (key, raw) = match &edge.trigger {
            TriggerSpec::Event(event) => ((edge.trigger.clone(), None), event.clone()),
            TriggerSpec::After(Delay::Millis(ms)) => ((edge.trigger.clone(), None), format!("after_{ms}ms")),
            TriggerSpec::After(Delay::Named(delay)) => {
                ((edge.trigger.clone(), None), format!("after_{}", sanitize(delay)))
            }
            TriggerSpec::ActorDone(actor) => ((edge.trigger.clone(), None), format!("{}_done", sanitize(actor))),
            TriggerSpec::ActorError(actor) => ((edge.trigger.clone(), None), format!("{}_error", sanitize(actor))),
            TriggerSpec::StateDone => {
                let state = if source == NodeIx::ROOT { sanitize(chart.name()) } else { names.local[source.0].clone() };
                self.warnings.approximated(
                    edge.location.as_str(),
                    format!("the completion (`onDone`) of `{}`", chart.display_path(source)),
                    format!("trigger `{state}_done`, fired by the environment"),
                );
                ((TriggerSpec::StateDone, Some(source)), format!("{state}_done"))
            }
        };
        triggers.name(&key, &raw, &edge.location, self.warnings)
    }

    /// Entry actions of the initial configuration run before any transition,
    /// so their emits have nowhere to go.
    fn warn_unattributed_start_emits(&mut self, chart: &Chart) {
        let mut node = NodeIx::ROOT;
        loop {
            let n = chart.node(node);
            for emit in &n.entry {
                self.warnings.ignored(
                    n.location.as_str(),
                    format!("event `{}` emitted when the machine starts (no transition to attach it to)", emit.event),
                );
            }
            match chart.default_child(node) {
                Some(child) => node = child,
                None => break,
            }
        }
    }
}

/// The events an edge emits once entry and exit actions are attributed to
/// it: exits (innermost first), the edge's own actions, then entries
/// (outermost first, down the target's default entry chain).
pub(crate) fn attributed_emits(chart: &Chart, source: NodeIx, edge: &Edge) -> Vec<Emit> {
    let Some(target) = edge.target else {
        return edge.emits.clone();
    };
    let source_chain = chart.ancestors_or_self(source);
    let internal = !edge.reenter && chart.is_descendant_or_self(target, source);
    // The domain is the innermost state that is neither exited nor entered.
    let domain = if internal {
        source
    } else {
        let target_chain = chart.ancestors_or_self(target);
        let common = source_chain.iter().copied().find(|n| target_chain.contains(n)).unwrap_or(NodeIx::ROOT);
        if (common == source || common == target) && common != NodeIx::ROOT {
            chart.node(common).parent.unwrap_or(NodeIx::ROOT)
        } else {
            common
        }
    };
    if internal && target == source {
        return edge.emits.clone();
    }

    let mut out = Vec::new();
    for &node in source_chain.iter().take_while(|&&n| n != domain) {
        out.extend(chart.node(node).exit.iter().cloned());
    }
    out.extend(edge.emits.iter().cloned());
    let mut entered: Vec<NodeIx> = chart.ancestors_or_self(target).into_iter().take_while(|&n| n != domain).collect();
    entered.reverse();
    if !matches!(chart.node(target).kind, NodeKind::History { .. }) {
        let mut node = target;
        while let Some(child) = chart.default_child(node) {
            entered.push(child);
            node = child;
        }
    }
    for node in entered {
        out.extend(chart.node(node).entry.iter().cloned());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::chart::Route;

    fn emit(event: &str) -> Emit {
        Emit { event: event.to_owned(), route: Route::Own }
    }

    fn edge(target: Option<NodeIx>, reenter: bool) -> Edge {
        Edge {
            trigger: TriggerSpec::Event("go".into()),
            target,
            guard: None,
            emits: vec![emit("T")],
            reenter,
            bounded: false,
            location: "test".into(),
        }
    }

    /// root ─┬─ a (exit Xa) ─┬─ a1 (exit Xa1)
    ///       │               └─ a2 (entry Ea2)
    ///       └─ b (entry Eb) ─── b1 (entry Eb1)
    fn chart() -> (Chart, [NodeIx; 5]) {
        let mut c = Chart::new("M".into(), "m".into());
        let a = c.add_child(NodeIx::ROOT, "a".into(), "a".into(), NodeKind::Normal);
        let a1 = c.add_child(a, "a1".into(), "a1".into(), NodeKind::Normal);
        let a2 = c.add_child(a, "a2".into(), "a2".into(), NodeKind::Normal);
        let b = c.add_child(NodeIx::ROOT, "b".into(), "b".into(), NodeKind::Normal);
        let b1 = c.add_child(b, "b1".into(), "b1".into(), NodeKind::Normal);
        c.node_mut(a).exit.push(emit("Xa"));
        c.node_mut(a).entry.push(emit("Ea"));
        c.node_mut(a1).exit.push(emit("Xa1"));
        c.node_mut(a2).entry.push(emit("Ea2"));
        c.node_mut(b).entry.push(emit("Eb"));
        c.node_mut(b1).entry.push(emit("Eb1"));
        (c, [a, a1, a2, b, b1])
    }

    fn events(emits: &[Emit]) -> Vec<&str> {
        emits.iter().map(|e| e.event.as_str()).collect()
    }

    #[test]
    fn cross_branch_transition_exits_then_enters_down_the_default_chain() {
        let (c, [_, a1, _, b, _]) = chart();
        let got = attributed_emits(&c, a1, &edge(Some(b), false));
        assert_eq!(events(&got), ["Xa1", "Xa", "T", "Eb", "Eb1"]);
    }

    #[test]
    fn sibling_transition_inside_a_compound_keeps_the_parent() {
        let (c, [_, a1, a2, _, _]) = chart();
        let got = attributed_emits(&c, a1, &edge(Some(a2), false));
        assert_eq!(events(&got), ["Xa1", "T", "Ea2"]);
    }

    #[test]
    fn self_transitions_reenter_only_when_asked() {
        let (c, [a, ..]) = chart();
        assert_eq!(events(&attributed_emits(&c, a, &edge(Some(a), false))), ["T"]);
        // Re-entering `a` runs its exit, then its entry and its default child's.
        assert_eq!(events(&attributed_emits(&c, a, &edge(Some(a), true))), ["Xa", "T", "Ea"]);
    }

    #[test]
    fn internal_transition_to_a_child_does_not_exit_the_parent() {
        let (c, [a, _, a2, _, _]) = chart();
        assert_eq!(events(&attributed_emits(&c, a, &edge(Some(a2), false))), ["T", "Ea2"]);
        assert_eq!(events(&attributed_emits(&c, a, &edge(Some(a2), true))), ["Xa", "T", "Ea", "Ea2"]);
    }

    #[test]
    fn targetless_transitions_only_run_their_own_actions() {
        let (c, [_, a1, ..]) = chart();
        assert_eq!(events(&attributed_emits(&c, a1, &edge(None, true))), ["T"]);
    }

    #[test]
    fn machine_level_transitions_enter_the_target() {
        let (c, [.., b, _]) = chart();
        assert_eq!(events(&attributed_emits(&c, NodeIx::ROOT, &edge(Some(b), false))), ["T", "Eb", "Eb1"]);
    }
}
