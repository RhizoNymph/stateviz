//! One P machine per controller.
//!
//! A controller starts in `Wiring`, deferring its events until it receives
//! the registry, then handles each subscribed event in `Running` by sending
//! the fired trigger to the target machine's (single) instance. Target
//! selectors, conditions and `bounded` marks become comments; a condition
//! guards the send with a nondeterministic `if ($)`; fan-out (`all`) and
//! spawn (`new`) selectors get TODOs, since the skeleton has one instance
//! per machine.

use cascade_core::definition::{TargetMode, TargetSpec};
use cascade_core::model::Target;
use cascade_core::{ControllerId, Model, RuleId};

use super::names::{Names, REGISTRY_TYPE, RUNNING_STATE, WIRE_EVENT, WIRING_STATE};
use super::writer::Writer;

pub(super) fn write_controller(w: &mut Writer, model: &Model, names: &Names, cid: ControllerId) {
    let controller = model.controller(cid);
    let events: Vec<&str> = controller.handlers.iter().map(|&h| names.event(model.handler(h).event)).collect();

    w.comment(&format!("Controller {}", controller.name));
    w.open(&format!("machine {} {{", names.controller(cid)));
    w.line(&format!("var registry: {REGISTRY_TYPE};"));
    w.blank();

    w.open(&format!("start state {WIRING_STATE} {{"));
    if !events.is_empty() {
        w.line(&format!("defer {};", events.join(", ")));
    }
    w.open(&format!("on {WIRE_EVENT} goto {RUNNING_STATE} with (r: {REGISTRY_TYPE}) {{"));
    w.line("registry = r;");
    w.close();
    w.close();
    w.blank();

    w.open(&format!("state {RUNNING_STATE} {{"));
    for (i, &h) in controller.handlers.iter().enumerate() {
        if i > 0 {
            w.blank();
        }
        let handler = model.handler(h);
        let event = handler.event;
        match names.payload_type(event) {
            Some(ty) => w.open(&format!("on {} do (payload: {ty}) {{", names.event(event))),
            None => w.open(&format!("on {} do {{", names.event(event))),
        }
        if handler.rules.is_empty() {
            w.comment("no rules");
        }
        for &r in &handler.rules {
            write_rule(w, model, names, r);
        }
        w.close();
    }
    w.close();
    w.close();
}

fn write_rule(w: &mut Writer, model: &Model, names: &Names, r: RuleId) {
    let rule = model.rule(r);
    let trigger = model.trigger(rule.trigger);
    let machine = model.machine(trigger.machine);

    w.comment(&format!("rule {}: fire {}.{}", rule.ordinal, machine.name, trigger.name));
    let (mode, clauses) = match &rule.target {
        Target::One { predicates } => (TargetMode::One, predicates),
        Target::All { predicates } => (TargetMode::All, predicates),
        Target::Spawn { assignments } => (TargetMode::Spawn, assignments),
    };
    let spec = TargetSpec { mode, machine: machine.name.clone(), clauses: clauses.clone() };
    w.comment(&format!("target: {spec}"));
    if let Some(condition) = &rule.condition {
        w.comment(&format!("when: {condition}"));
    }
    if rule.bounded {
        w.comment("bounded: cascade cycles through this rule are expected");
    }
    match mode {
        TargetMode::One => {}
        TargetMode::All => w.comment(&format!(
            "TODO fan-out: fire on every matching {}; this skeleton has one instance per machine.",
            machine.name
        )),
        TargetMode::Spawn => w.comment(&format!(
            "TODO spawn: create a new {} first; this skeleton fires on the one instance.",
            machine.name
        )),
    }
    let send = format!("send registry[\"{}\"], {};", names.machine(trigger.machine), names.trigger(rule.trigger));
    if rule.condition.is_some() {
        w.open("if ($) {");
        w.line(&send);
        w.close();
    } else {
        w.line(&send);
    }
}
