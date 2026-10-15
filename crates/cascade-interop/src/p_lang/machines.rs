//! One P machine per Cascade machine.
//!
//! The machine starts in `Wiring`, deferring its triggers until the driver
//! sends the registry of all instances (`eWire`), then enters the default
//! entry of the Cascade initial state. Only atomic and final states become
//! P states; for each of them and each trigger of the machine, the handler
//! follows `Model::enabled_transitions`, so transitions declared on a
//! compound state apply to its descendants and the innermost one wins.
//!
//! Declared fields are machine variables; guards over them are P
//! conditions and `sets` assign them (see [`super::fields`]). Emits also go
//! to subscribing machines as their subscription trigger. A state that
//! arms timers (its own or its ancestors') sends them to itself on entry,
//! nondeterministically.

use cascade_core::expr::{Guard, Operand};
use cascade_core::model::{StateKind, TriggerKind};
use cascade_core::{MachineId, Model, StateId, TransitionId, TriggerId};

use super::fields::{self, PField, PKind};
use super::names::{Names, REGISTRY_TYPE, WIRE_EVENT, WIRING_STATE};
use super::writer::Writer;

pub(super) fn write_machine(w: &mut Writer, model: &Model, names: &Names, mid: MachineId) {
    let machine = model.machine(mid);
    let triggers: Vec<&str> = machine.triggers.iter().map(|&t| names.trigger(t)).collect();

    w.comment(&format!("Machine {}", machine.name));
    w.open(&format!("machine {} {{", names.machine(mid)));
    w.line(&format!("var registry: {REGISTRY_TYPE};"));
    for field in names.machine_fields(mid) {
        w.line(&format!("var {}: {};", field.ident, field.kind.p_type()));
    }
    w.blank();

    let (entry, note) = enter(model, names, machine.initial);
    w.open(&format!("start state {WIRING_STATE} {{"));
    if !triggers.is_empty() {
        w.line(&format!("defer {};", triggers.join(", ")));
    }
    if let Some(note) = note {
        w.comment(&note);
    }
    w.open(&format!("on {WIRE_EVENT} goto {entry} with (r: {REGISTRY_TYPE}) {{"));
    w.line("registry = r;");
    w.close();
    w.close();

    for &s in &machine.states {
        let state = model.state(s);
        let Some(name) = names.state(s) else { continue };
        w.blank();
        w.open(&format!("state {name} {{"));
        if state.is_final() {
            if !triggers.is_empty() {
                w.comment("Final state: every trigger is dropped.");
                w.line(&format!("ignore {};", triggers.join(", ")));
            }
        } else {
            write_timers(w, model, names, s);
            write_handlers(w, model, names, s, &machine.triggers);
        }
        w.close();
    }
    w.close();
}

/// Timers armed by entering `state` (its own and its ancestors'), as
/// nondeterministic self-sends.
fn write_timers(w: &mut Writer, model: &Model, names: &Names, state: StateId) {
    let mut timers: Vec<TriggerId> = Vec::new();
    for s in std::iter::once(state).chain(model.ancestors(state)) {
        for &t in &model.machine(model.state(state).machine).transitions {
            let transition = model.transition(t);
            if transition.from == s
                && matches!(model.trigger(transition.trigger).kind, TriggerKind::Timer(_))
                && !timers.contains(&transition.trigger)
            {
                timers.push(transition.trigger);
            }
        }
    }
    if timers.is_empty() {
        return;
    }
    w.open("entry {");
    for trigger in timers {
        w.comment(&format!(
            "timer {}: may fire while this state is current (TODO: a timer machine to cancel it on exit)",
            model.trigger(trigger).name
        ));
        w.open("if ($) {");
        w.line(&format!("send this, {};", names.trigger(trigger)));
        w.close();
    }
    w.close();
}

fn write_handlers(w: &mut Writer, model: &Model, names: &Names, state: StateId, triggers: &[TriggerId]) {
    let mut ignored = Vec::new();
    for &trigger in triggers {
        let candidates = model.enabled_transitions(state, trigger);
        if candidates.is_empty() {
            ignored.push(names.trigger(trigger));
        } else {
            write_handler(w, model, names, names.trigger(trigger), &candidates);
        }
    }
    if !ignored.is_empty() {
        w.comment("Cascade drops triggers a state does not accept; remove this line to have P report them.");
        w.line(&format!("ignore {};", ignored.join(", ")));
    }
}

/// The handler for one trigger in one state: a plain `goto` for a single
/// unguarded transition, otherwise a `do` block choosing nondeterministically
/// (`$`) between the candidates, the guard text as a comment.
fn write_handler(w: &mut Writer, model: &Model, names: &Names, event: &str, candidates: &[TransitionId]) {
    if let [only] = candidates
        && unconditional(model, *only)
    {
        let (target, note) = enter(model, names, model.transition(*only).to);
        if let Some(note) = note {
            w.comment(&note);
        }
        let mut sends = set_lines(model, names, *only);
        sends.extend(send_lines(model, names, *only));
        if sends.is_empty() {
            w.line(&format!("on {event} goto {target};"));
        } else {
            w.open(&format!("on {event} goto {target} with {{"));
            for line in &sends {
                w.line(line);
            }
            w.close();
        }
        return;
    }

    // The last unguarded candidate is the fallback `else`; every other
    // candidate is a nondeterministic branch.
    // An `else` guard is the fallback when there is one.
    let fallback = candidates
        .iter()
        .rposition(|&t| model.transition(t).guard == Some(Guard::Else) && model.transition(t).filter.is_none())
        .or_else(|| candidates.iter().rposition(|&t| unconditional(model, t)));
    w.open(&format!("on {event} do {{"));
    let mut first = true;
    for (i, &t) in candidates.iter().enumerate() {
        if Some(i) == fallback {
            continue;
        }
        let test = p_guard(model, names, t).unwrap_or_else(|| "$".to_owned());
        if first {
            w.open(&format!("if ({test}) {{"));
            first = false;
        } else {
            w.reopen(&format!("}} else if ({test}) {{"));
        }
        let transition = model.transition(t);
        if let Some(filter) = &transition.filter {
            w.comment(&format!("where: {}", filter.display_filter()));
        }
        match &transition.guard {
            Some(guard) => w.comment(&format!("guard: {guard}")),
            None if transition.filter.is_none() => w.comment("no guard: nondeterministic choice"),
            None => {}
        }
        write_branch_body(w, model, names, t);
    }
    if let Some(i) = fallback
        && let Some(&t) = candidates.get(i)
    {
        w.reopen("} else {");
        if model.transition(t).guard == Some(Guard::Else) {
            w.comment("guard: else");
        }
        write_branch_body(w, model, names, t);
    }
    w.close();
    w.close();
}

/// Neither a guard nor a subscription filter.
fn unconditional(model: &Model, t: TransitionId) -> bool {
    let transition = model.transition(t);
    transition.guard.is_none() && transition.filter.is_none()
}

/// A transition's guard as a P condition over the machine's variables,
/// when it is an expression over typed fields (and there is no filter).
fn p_guard(model: &Model, names: &Names, t: TransitionId) -> Option<String> {
    let transition = model.transition(t);
    let (Some(Guard::Expr(expr)), None) = (&transition.guard, &transition.filter) else {
        return None;
    };
    let vars = names.machine_fields(transition.machine);
    let scope = |o: &Operand| -> Option<(String, &PKind)> {
        match o {
            Operand::SelfField(f) => vars.iter().find(|v| v.cascade == *f).map(|v| (v.ident.clone(), &v.kind)),
            Operand::Event(_) | Operand::Lit(_) => None,
        }
    };
    fields::condition(expr, &scope)
}

/// `sets` as assignments to the machine's variables.
fn set_lines(model: &Model, names: &Names, t: TransitionId) -> Vec<String> {
    let transition = model.transition(t);
    let vars: &[PField] = names.machine_fields(transition.machine);
    let var = |name: &str| vars.iter().find(|v| v.cascade == name);
    transition
        .sets
        .iter()
        .map(|a| {
            let value = var(&a.field).and_then(|target| match &a.value {
                Operand::Lit(lit) => fields::literal(lit, &target.kind),
                Operand::SelfField(f) => {
                    var(f).filter(|v| v.kind == target.kind || target.kind == PKind::Any).map(|v| v.ident.clone())
                }
                Operand::Event(_) => None,
            });
            match (var(&a.field), value) {
                (Some(target), Some(value)) => format!("{} = {value};", target.ident),
                _ => {
                    format!("// TODO sets {} = {} (no P value here: the trigger carries no payload)", a.field, a.value)
                }
            }
        })
        .collect()
}

fn write_branch_body(w: &mut Writer, model: &Model, names: &Names, t: TransitionId) {
    let (target, note) = enter(model, names, model.transition(t).to);
    if let Some(note) = note {
        w.comment(&note);
    }
    for line in set_lines(model, names, t).into_iter().chain(send_lines(model, names, t)) {
        w.line(&line);
    }
    w.line(&format!("goto {target};"));
}

/// The P state entered when a transition targets `state`: compound states
/// enter their default atomic state; history states are approximated by
/// their parent's default entry (or the machine's initial state), with a
/// note saying so.
fn enter<'a>(model: &Model, names: &'a Names, state: StateId) -> (&'a str, Option<String>) {
    let s = model.state(state);
    let (entered, note) = match &s.kind {
        StateKind::History { .. } => {
            let (base, from) = match s.parent {
                Some(parent) => (model.default_entry(parent), model.state(parent).path.clone()),
                None => (model.default_entry(model.machine(s.machine).initial), "the machine".to_owned()),
            };
            (base, Some(format!("{}: history approximated by the default entry of {from}", s.path)))
        }
        StateKind::Atomic | StateKind::Compound { .. } | StateKind::Final => (model.default_entry(state), None),
    };
    (names.state(entered).unwrap_or(WIRING_STATE), note)
}

/// `send` statements delivering a transition's emitted events to every
/// controller subscribed to them.
fn send_lines(model: &Model, names: &Names, t: TransitionId) -> Vec<String> {
    let mut lines = Vec::new();
    for event in model.transition(t).emits.iter().map(|e| e.event) {
        let mut controllers = Vec::new();
        for &h in &model.event(event).handlers {
            let controller = model.handler(h).controller;
            if !controllers.contains(&controller) {
                controllers.push(controller);
            }
        }
        let subscribers = &model.event(event).subscribers;
        if controllers.is_empty() && subscribers.is_empty() {
            lines.push(format!("// emits {}: no controller subscribes", model.event(event).name));
        }
        for &trigger in subscribers {
            let machine = names.machine(model.trigger(trigger).machine);
            lines.push(format!("send registry[\"{machine}\"], {};", names.trigger(trigger)));
        }
        for controller in controllers {
            let target = names.controller(controller);
            lines.push(match names.payload_type(event) {
                Some(ty) => format!("send registry[\"{target}\"], {}, default({ty});", names.event(event)),
                None => format!("send registry[\"{target}\"], {};", names.event(event)),
            });
        }
    }
    lines
}
