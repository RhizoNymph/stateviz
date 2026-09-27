//! One P machine per Cascade machine.
//!
//! The machine starts in `Wiring`, deferring its triggers until the driver
//! sends the registry of all instances (`eWire`), then enters the default
//! entry of the Cascade initial state. Only atomic and final states become
//! P states; for each of them and each trigger of the machine, the handler
//! follows `Model::enabled_transitions`, so transitions declared on a
//! compound state apply to its descendants and the innermost one wins.

use cascade_core::model::StateKind;
use cascade_core::{MachineId, Model, StateId, TransitionId, TriggerId};

use super::names::{Names, REGISTRY_TYPE, WIRE_EVENT, WIRING_STATE};
use super::writer::Writer;

pub(super) fn write_machine(w: &mut Writer, model: &Model, names: &Names, mid: MachineId) {
    let machine = model.machine(mid);
    let triggers: Vec<&str> = machine.triggers.iter().map(|&t| names.trigger(t)).collect();

    w.comment(&format!("Machine {}", machine.name));
    w.open(&format!("machine {} {{", names.machine(mid)));
    w.line(&format!("var registry: {REGISTRY_TYPE};"));
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
            write_handlers(w, model, names, s, &machine.triggers);
        }
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
        && model.transition(*only).guard.is_none()
    {
        let (target, note) = enter(model, names, model.transition(*only).to);
        if let Some(note) = note {
            w.comment(&note);
        }
        let sends = send_lines(model, names, *only);
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
    let fallback = candidates.iter().rposition(|&t| model.transition(t).guard.is_none());
    w.open(&format!("on {event} do {{"));
    let mut first = true;
    for (i, &t) in candidates.iter().enumerate() {
        if Some(i) == fallback {
            continue;
        }
        if first {
            w.open("if ($) {");
            first = false;
        } else {
            w.reopen("} else if ($) {");
        }
        match &model.transition(t).guard {
            Some(guard) => w.comment(&format!("guard: {guard}")),
            None => w.comment("no guard: nondeterministic choice"),
        }
        write_branch_body(w, model, names, t);
    }
    if let Some(i) = fallback
        && let Some(&t) = candidates.get(i)
    {
        w.reopen("} else {");
        write_branch_body(w, model, names, t);
    }
    w.close();
    w.close();
}

fn write_branch_body(w: &mut Writer, model: &Model, names: &Names, t: TransitionId) {
    let (target, note) = enter(model, names, model.transition(t).to);
    if let Some(note) = note {
        w.comment(&note);
    }
    for line in send_lines(model, names, t) {
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
    for &event in &model.transition(t).emits {
        let mut controllers = Vec::new();
        for &h in &model.event(event).handlers {
            let controller = model.handler(h).controller;
            if !controllers.contains(&controller) {
                controllers.push(controller);
            }
        }
        if controllers.is_empty() {
            lines.push(format!("// emits {}: no controller subscribes", model.event(event).name));
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
