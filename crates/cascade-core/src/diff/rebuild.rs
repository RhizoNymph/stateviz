//! Rebuilding definition entries from a resolved model, for ghosts.
//!
//! Everything is written in canonical form: full state paths, explicit
//! initial states, one transition per source state, and no source spans
//! (ghosts do not exist in the file being viewed).

use crate::definition::{
    ControllerDef, EventDef, ExternalDef, HandlerDef, MachineDef, RuleDef, StateDef, StateKindDef, TargetMode,
    TargetSpec, TransitionDef, TriggerRef,
};
use crate::ids::{ControllerId, EventId, ExternalId, HandlerId, MachineId, RuleId, StateId, TransitionId, TriggerId};
use crate::model::{Model, StateKind, Target};
use crate::span::{SourceSpan, Spanned};

fn ghost<T>(value: T) -> Spanned<T> {
    Spanned::synthetic(value)
}

pub(super) fn machine_def(model: &Model, id: MachineId) -> MachineDef {
    let m = model.machine(id);
    MachineDef {
        name: ghost(m.name.clone()),
        color: m.color.map(ghost),
        domain: m.domain.clone().map(ghost),
        initial: Some(ghost(model.state(m.initial).path.clone())),
        fields: m.fields.iter().cloned().map(ghost).collect(),
        states: m.top_states.iter().map(|&s| state_def(model, s)).collect(),
        transitions: m.transitions.iter().map(|&t| transition_def(model, t)).collect(),
        span: SourceSpan::unknown(),
    }
}

/// The state and its whole subtree; a compound state names its initial
/// child explicitly.
pub(super) fn state_def(model: &Model, id: StateId) -> StateDef {
    let s = model.state(id);
    let (kind, initial, states) = match &s.kind {
        StateKind::Atomic => (StateKindDef::Normal, None, Vec::new()),
        StateKind::Compound { children, initial } => (
            StateKindDef::Normal,
            Some(ghost(model.state(*initial).name.clone())),
            children.iter().map(|&c| state_def(model, c)).collect(),
        ),
        StateKind::Final => (StateKindDef::Final, None, Vec::new()),
        StateKind::History { deep: false } => (StateKindDef::History, None, Vec::new()),
        StateKind::History { deep: true } => (StateKindDef::DeepHistory, None, Vec::new()),
    };
    StateDef { name: ghost(s.name.clone()), kind: ghost(kind), initial, states, span: SourceSpan::unknown() }
}

pub(super) fn transition_def(model: &Model, id: TransitionId) -> TransitionDef {
    let t = model.transition(id);
    TransitionDef {
        from: vec![ghost(model.state(t.from).path.clone())],
        to: ghost(model.state(t.to).path.clone()),
        on: ghost(model.trigger(t.trigger).name.clone()),
        guard: t.guard.clone().map(ghost),
        emits: t.emits.iter().map(|&e| ghost(model.event(e).name.clone())).collect(),
        bounded: t.bounded,
        span: SourceSpan::unknown(),
    }
}

/// A removed event, declared (with its old payload when it had one).
pub(super) fn event_def(model: &Model, id: EventId) -> EventDef {
    let e = model.event(id);
    EventDef {
        name: ghost(e.name.clone()),
        payload: if e.declared { e.payload.iter().cloned().map(ghost).collect() } else { Vec::new() },
        span: SourceSpan::unknown(),
    }
}

pub(super) fn controller_def(model: &Model, id: ControllerId) -> ControllerDef {
    let c = model.controller(id);
    ControllerDef {
        name: ghost(c.name.clone()),
        on: c.handlers.iter().map(|&h| handler_def(model, h)).collect(),
        span: SourceSpan::unknown(),
    }
}

pub(super) fn handler_def(model: &Model, id: HandlerId) -> HandlerDef {
    let h = model.handler(id);
    HandlerDef {
        event: ghost(model.event(h.event).name.clone()),
        rules: h.rules.iter().map(|&r| rule_def(model, r)).collect(),
        span: SourceSpan::unknown(),
    }
}

pub(super) fn rule_def(model: &Model, id: RuleId) -> RuleDef {
    let r = model.rule(id);
    let fire = trigger_ref(model, r.trigger);
    let target = target_spec(&fire.machine, &r.target).map(ghost);
    RuleDef {
        fire: ghost(fire),
        target,
        when: r.condition.clone().map(ghost),
        bounded: r.bounded,
        span: SourceSpan::unknown(),
    }
}

pub(super) fn external_def(model: &Model, id: ExternalId) -> ExternalDef {
    let x = model.external(id);
    ExternalDef {
        name: ghost(x.name.clone()),
        triggers: x.triggers.iter().map(|&t| ghost(trigger_ref(model, t))).collect(),
        span: SourceSpan::unknown(),
    }
}

pub(super) fn trigger_ref(model: &Model, id: TriggerId) -> TriggerRef {
    let t = model.trigger(id);
    TriggerRef { machine: model.machine(t.machine).name.clone(), trigger: t.name.clone() }
}

/// The selector that resolves to `target`; `None` for "the one instance".
fn target_spec(machine: &str, target: &Target) -> Option<TargetSpec> {
    let (mode, clauses) = match target {
        Target::One { predicates } if predicates.is_empty() => return None,
        Target::One { predicates } => (TargetMode::One, predicates),
        Target::All { predicates } => (TargetMode::All, predicates),
        Target::Spawn { assignments } => (TargetMode::Spawn, assignments),
    };
    Some(TargetSpec { mode, machine: machine.to_owned(), clauses: clauses.clone() })
}
