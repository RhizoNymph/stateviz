//! Drag-to-connect onto a transition from an event or from another
//! machine's transition: subscribe the transition to the event (its
//! trigger becomes `on: { event: E }`), or wire it through a controller
//! that already handles the event (a rule firing the transition's command).
//!
//! | From | Offers |
//! | --- | --- |
//! | event `E` | subscribe to `E`; then one rule per controller handling `E` (command triggers only) |
//! | transition emitting `E1, E2…` | for each emitted event, as above |
//! | transition emitting nothing | emit a new `<Trigger>Done` (declared in strict files) and subscribe to it |
//!
//! Pure. A transition already subscribed to `E` is not offered `E` again.

use cascade_core::definition::{EmitDef, TriggerSpec};
use cascade_core::edit::{EditOp, fresh_name};
use cascade_core::{Definition, ElementKey};

use super::connect::{add_rule, declare, not_connectable};
use super::defs;
use super::ops::{PlanError, Planned, transition_entry};

/// `Order: placed → paid`, for labels.
fn describe(key: &ElementKey) -> String {
    match key {
        ElementKey::Transition { machine, from, to, .. } => format!("{machine}: {from} → {to}"),
        other => other.to_string(),
    }
}

/// The op that makes `transition` subscribe to `event`; `None` when it
/// already does.
fn subscribe_op(definition: &Definition, transition: &ElementKey, event: &str) -> Result<Option<EditOp>, PlanError> {
    let (machine, index, entry) = transition_entry(definition, transition)?;
    if matches!(&entry.trigger, TriggerSpec::Event { event: e, .. } if e.value == event) {
        return Ok(None);
    }
    let mut updated = entry.clone();
    updated.trigger = TriggerSpec::Event { event: defs::synthetic(event), filter: None };
    Ok(Some(EditOp::UpdateTransition { machine, index, transition: updated }))
}

/// Rules firing the transition's command from each controller that
/// handles `event`. Subscriptions and timers cannot be fired by rules.
fn rule_offers(definition: &Definition, event: &str, transition: &ElementKey) -> Result<Vec<Planned>, PlanError> {
    let (_, _, entry) = transition_entry(definition, transition)?;
    if !matches!(entry.trigger, TriggerSpec::Command(_)) {
        return Ok(Vec::new());
    }
    definition
        .controllers
        .iter()
        .filter(|c| c.on.iter().any(|h| h.event.value == event))
        .map(|c| add_rule(&c.name.value, event, transition))
        .collect()
}

/// Offers for an event dropped on a transition.
pub fn event_offers(definition: &Definition, event: &str, transition: &ElementKey) -> Result<Vec<Planned>, PlanError> {
    let mut offers = Vec::new();
    if let Some(op) = subscribe_op(definition, transition, event)? {
        offers.push(Planned::new(op, format!("{} subscribes to {event}", describe(transition))));
    }
    offers.extend(rule_offers(definition, event, transition)?);
    if offers.is_empty() {
        return Err(not_connectable(format!("{} already subscribes to {event}", describe(transition))));
    }
    Ok(offers)
}

/// Offers for a transition dropped on a transition of another machine.
pub fn transition_offers(
    definition: &Definition,
    from: &ElementKey,
    to: &ElementKey,
) -> Result<Vec<Planned>, PlanError> {
    let (source_machine, source_index, source) = transition_entry(definition, from)?;
    if source.emits.is_empty() {
        let base = format!("{}Done", defs::pascal_case(&source.trigger.name()));
        let taken = defs::event_names(definition);
        let event = fresh_name(&base, taken.iter().map(String::as_str));
        let mut emitting = source.clone();
        emitting.emits.push(EmitDef::plain(event.clone()));
        let mut ops = Vec::new();
        if defs::strict_events(definition) {
            ops.push(declare(&event));
        }
        ops.push(EditOp::UpdateTransition { machine: source_machine, index: source_index, transition: emitting });
        if let Some(op) = subscribe_op(definition, to, &event)? {
            ops.push(op);
        }
        return Ok(vec![Planned::new(
            EditOp::Batch(ops),
            format!("{} emits {event}; {} subscribes to it", describe(from), describe(to)),
        )]);
    }
    let mut offers = Vec::new();
    for emit in &source.emits {
        match event_offers(definition, &emit.event.value, to) {
            Ok(more) => offers.extend(more),
            Err(PlanError::NotConnectable(_)) => {}
            Err(other) => return Err(other),
        }
    }
    if offers.is_empty() {
        return Err(not_connectable(format!("{} already subscribes to what {} emits", describe(to), describe(from))));
    }
    Ok(offers)
}

#[cfg(test)]
mod tests {
    use cascade_core::definition::TriggerRef;

    use super::*;
    use crate::build::connect::{can_connect, connect, offers};

    const TEXT: &str = "\
machines:
  Order:
    states: [draft, paid, done]
    transitions:
      - { from: draft, to: paid, on: pay, emits: [Paid, Billed] }
      - { from: paid, to: done, on: finish }
  Shipment:
    states: [idle, moving, stopped]
    transitions:
      - { from: idle, to: moving, on: start }
      - { from: moving, to: stopped, on: { event: Paid } }
      - { from: stopped, to: idle, after: 5m }
      - { from: moving, to: moving, on: load }
controllers:
  Fulfil:
    on:
      Paid:
        - fire: Shipment.load
";

    fn def() -> Definition {
        cascade_core::parse_definition(TEXT).expect("parses")
    }

    fn transition(machine: &str, from: &str, to: &str, trigger: &str) -> ElementKey {
        ElementKey::Transition {
            machine: machine.into(),
            from: from.into(),
            to: to.into(),
            trigger: trigger.into(),
            ordinal: 0,
        }
    }

    fn event(name: &str) -> ElementKey {
        ElementKey::Event { event: name.into() }
    }

    fn pay() -> ElementKey {
        transition("Order", "draft", "paid", "pay")
    }

    fn finish() -> ElementKey {
        transition("Order", "paid", "done", "finish")
    }

    fn start() -> ElementKey {
        transition("Shipment", "idle", "moving", "start")
    }

    fn stop() -> ElementKey {
        transition("Shipment", "moving", "stopped", "Paid")
    }

    fn timer() -> ElementKey {
        transition("Shipment", "stopped", "idle", "after-5m")
    }

    /// The trigger the op gives `machine`'s transition `index`.
    fn trigger_set(op: &EditOp, machine_name: &str) -> Option<TriggerSpec> {
        match op {
            EditOp::UpdateTransition { machine, transition, .. } if machine == machine_name => {
                Some(transition.trigger.clone())
            }
            EditOp::Batch(ops) => ops.iter().find_map(|o| trigger_set(o, machine_name)),
            _ => None,
        }
    }

    fn subscribed_to(spec: &Option<TriggerSpec>, name: &str) -> bool {
        matches!(spec, Some(TriggerSpec::Event { event, filter: None }) if event.value == name)
    }

    #[test]
    fn events_and_foreign_transitions_drop_on_transitions() {
        assert!(can_connect(&event("Paid"), &start()));
        assert!(can_connect(&pay(), &start()));
        assert!(!can_connect(&pay(), &finish()), "same machine");
        assert!(!can_connect(&event("Paid"), &ElementKey::Machine { machine: "Order".into() }));
    }

    #[test]
    fn an_event_on_a_command_offers_subscribe_then_controller_rules() {
        let all = offers(&def(), &event("Paid"), &start()).expect("offers");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].label, "Shipment: idle → moving subscribes to Paid");
        assert!(subscribed_to(&trigger_set(&all[0].op, "Shipment"), "Paid"));
        assert!(matches!(&all[1].op, EditOp::AddRule { controller, event, rule, .. }
            if controller == "Fulfil" && event == "Paid"
                && rule.fire.value == TriggerRef { machine: "Shipment".into(), trigger: "start".into() }));
        // `connect` takes the first offer.
        assert_eq!(connect(&def(), &event("Paid"), &start()).expect("plans"), all[0]);
    }

    #[test]
    fn timers_and_subscriptions_only_offer_subscribing() {
        let on_timer = offers(&def(), &event("Paid"), &timer()).expect("offers");
        assert_eq!(on_timer.len(), 1, "rules cannot fire a timer");
        assert!(subscribed_to(&trigger_set(&on_timer[0].op, "Shipment"), "Paid"));
        let other = offers(&def(), &event("Billed"), &stop()).expect("offers");
        assert_eq!(other.len(), 1);
        assert!(subscribed_to(&trigger_set(&other[0].op, "Shipment"), "Billed"));
    }

    #[test]
    fn subscribing_twice_is_refused() {
        assert!(
            matches!(offers(&def(), &event("Paid"), &stop()), Err(PlanError::NotConnectable(m)) if m.contains("already"))
        );
    }

    #[test]
    fn an_emitting_transition_offers_each_of_its_events() {
        let all = offers(&def(), &pay(), &start()).expect("offers");
        let labels: Vec<&str> = all.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Shipment: idle → moving subscribes to Paid",
                "Fulfil on Paid fires Shipment.start",
                "Shipment: idle → moving subscribes to Billed",
            ]
        );
        // Already subscribed to Paid: only Billed is left.
        let rest = offers(&def(), &pay(), &stop()).expect("offers");
        assert_eq!(rest.len(), 1);
        assert!(subscribed_to(&trigger_set(&rest[0].op, "Shipment"), "Billed"));
    }

    #[test]
    fn a_silent_transition_emits_a_new_event_to_subscribe_to() {
        let all = offers(&def(), &finish(), &start()).expect("offers");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].label, "Order: paid → done emits FinishDone; Shipment: idle → moving subscribes to it");
        let EditOp::Batch(ops) = &all[0].op else { panic!("expected a batch") };
        assert_eq!(ops.len(), 2, "no declaration without an events: list");
        assert!(matches!(&ops[0], EditOp::UpdateTransition { machine, transition, .. }
            if machine == "Order" && transition.emits.iter().any(|e| e.event.value == "FinishDone")));
        assert!(subscribed_to(&trigger_set(&ops[1], "Shipment"), "FinishDone"));
    }

    #[test]
    fn strict_files_declare_the_new_event_first() {
        let text = format!("events:\n  Paid: {{}}\n  Billed: {{}}\n{TEXT}");
        let d = cascade_core::parse_definition(&text).expect("parses");
        let all = offers(&d, &finish(), &start()).expect("offers");
        let EditOp::Batch(ops) = &all[0].op else { panic!("expected a batch") };
        assert!(matches!(&ops[0], EditOp::DeclareEvent { event, .. } if event.name.value == "FinishDone"));
        assert_eq!(ops.len(), 3);
    }

    #[test]
    fn every_offer_applies() {
        let d = def();
        for (from, to) in [(event("Paid"), start()), (pay(), start()), (finish(), start()), (event("Billed"), timer())]
        {
            for planned in offers(&d, &from, &to).expect("offers") {
                cascade_core::edit::apply(&d, &planned.op)
                    .unwrap_or_else(|e| panic!("{} does not apply: {e}", planned.label));
            }
        }
    }
}
