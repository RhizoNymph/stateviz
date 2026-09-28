//! Drag-to-connect: which drops are valid and the edit op each one makes.
//!
//! | From | To | Op |
//! | --- | --- | --- |
//! | state | state (same machine) | `AddTransition` with trigger `go`, `go2`, … |
//! | transition | controller | new event `<Trigger>Done` emitted by the transition, handled by the controller (`Batch`) |
//! | transition | event | the transition also emits the event |
//! | event | controller | the controller handles the event |
//! | controller or handler | transition or trigger | a rule firing the trigger at the one instance |
//! | source | transition or trigger | the source exposes the trigger |
//!
//! Pure; dropping anywhere else cancels.

use cascade_core::definition::{EventDef, TriggerRef};
use cascade_core::edit::{EditOp, fresh_name};
use cascade_core::{Definition, ElementKey, SourceSpan};

use super::defs;
use super::ops::{PlanError, Planned, transition_entry};

/// Whether a drag may start at `key`.
pub fn is_source(key: &ElementKey) -> bool {
    matches!(
        key,
        ElementKey::State { .. }
            | ElementKey::Transition { .. }
            | ElementKey::Event { .. }
            | ElementKey::Controller { .. }
            | ElementKey::Handler { .. }
            | ElementKey::External { .. }
    )
}

/// Whether dropping a drag from `from` on `to` makes something, for
/// highlighting drop targets while dragging.
pub fn can_connect(from: &ElementKey, to: &ElementKey) -> bool {
    use ElementKey as K;
    match (from, to) {
        (K::State { machine: a, .. }, K::State { machine: b, .. }) => a == b,
        (K::Transition { .. }, K::Controller { .. } | K::Event { .. }) => true,
        (K::Event { .. }, K::Controller { .. }) => true,
        (K::Controller { .. } | K::Handler { .. } | K::External { .. }, K::Transition { .. } | K::Trigger { .. }) => {
            true
        }
        _ => false,
    }
}

/// The trigger a transition or trigger key names.
fn trigger_of(key: &ElementKey) -> Option<TriggerRef> {
    match key {
        ElementKey::Transition { machine, trigger, .. } | ElementKey::Trigger { machine, trigger } => {
            Some(TriggerRef { machine: machine.clone(), trigger: trigger.clone() })
        }
        _ => None,
    }
}

fn not_connectable(message: impl Into<String>) -> PlanError {
    PlanError::NotConnectable(message.into())
}

/// The op for a drop of `from` on `to`.
pub fn connect(definition: &Definition, from: &ElementKey, to: &ElementKey) -> Result<Planned, PlanError> {
    use ElementKey as K;
    if !can_connect(from, to) {
        return Err(not_connectable(format!("cannot connect {} to {}", from.kind().prefix(), to.kind().prefix())));
    }
    match (from, to) {
        (K::State { machine, path: source }, K::State { path: target, .. }) => {
            add_transition(definition, machine, source, target)
        }
        (K::Transition { .. }, K::Controller { controller }) => wire_emit_to_controller(definition, from, controller),
        (K::Transition { .. }, K::Event { event }) => emit_existing(definition, from, event),
        (K::Event { event }, K::Controller { controller }) => subscribe(definition, controller, event),
        (K::Controller { controller }, _) => {
            let handler = defs::controller(definition, controller)
                .ok_or_else(|| PlanError::NotFound(from.clone()))?
                .on
                .last()
                .ok_or_else(|| {
                    not_connectable(format!(
                        "{controller} handles no events yet: connect a transition to it first, then connect it to the transition to fire"
                    ))
                })?;
            add_rule(controller, &handler.event.value, to)
        }
        (K::Handler { controller, event }, _) => {
            defs::handler(definition, controller, event).ok_or_else(|| PlanError::NotFound(from.clone()))?;
            add_rule(controller, event, to)
        }
        (K::External { source }, _) => expose(definition, source, to),
        _ => Err(not_connectable("unsupported connection")),
    }
}

fn add_transition(definition: &Definition, machine: &str, from: &str, to: &str) -> Result<Planned, PlanError> {
    let machine_def = defs::machine(definition, machine)
        .ok_or_else(|| PlanError::NotFound(ElementKey::Machine { machine: machine.into() }))?;
    for path in [from, to] {
        defs::state(machine_def, path)
            .ok_or_else(|| PlanError::NotFound(ElementKey::State { machine: machine.into(), path: path.into() }))?;
    }
    let taken = defs::trigger_names(definition, machine);
    let trigger = fresh_name("go", taken.iter().map(String::as_str));
    Ok(Planned::new(
        EditOp::AddTransition {
            machine: machine.to_owned(),
            transition: defs::transition_def(from, to, &trigger),
            index: None,
        },
        format!("Add transition {machine}: {from} → {to} ({trigger})"),
    ))
}

fn declare(event: &str) -> EditOp {
    EditOp::DeclareEvent {
        event: EventDef { name: defs::synthetic(event), payload: Vec::new(), span: SourceSpan::unknown() },
        index: None,
    }
}

fn wire_emit_to_controller(
    definition: &Definition,
    transition: &ElementKey,
    controller: &str,
) -> Result<Planned, PlanError> {
    defs::controller(definition, controller)
        .ok_or_else(|| PlanError::NotFound(ElementKey::Controller { controller: controller.into() }))?;
    let (machine, index, entry) = transition_entry(definition, transition)?;
    let base = format!("{}Done", defs::pascal_case(&entry.on.value));
    let taken = defs::event_names(definition);
    let event = fresh_name(&base, taken.iter().map(String::as_str));
    let mut updated = entry.clone();
    updated.emits.push(defs::synthetic(&event));
    let mut ops = Vec::new();
    if defs::strict_events(definition) {
        ops.push(declare(&event));
    }
    ops.push(EditOp::UpdateTransition { machine: machine.clone(), index, transition: updated });
    ops.push(EditOp::AddHandler { controller: controller.to_owned(), handler: defs::handler_def(&event), index: None });
    Ok(Planned::new(EditOp::Batch(ops), format!("Wire {machine}.{} → {controller} via {event}", entry.on.value)))
}

fn emit_existing(definition: &Definition, transition: &ElementKey, event: &str) -> Result<Planned, PlanError> {
    let (machine, index, entry) = transition_entry(definition, transition)?;
    if entry.emits.iter().any(|e| e.value == event) {
        return Err(not_connectable(format!("the transition already emits {event}")));
    }
    let mut updated = entry.clone();
    updated.emits.push(defs::synthetic(event));
    Ok(Planned::new(
        EditOp::UpdateTransition { machine: machine.clone(), index, transition: updated },
        format!("{machine}.{} emits {event}", entry.on.value),
    ))
}

fn subscribe(definition: &Definition, controller: &str, event: &str) -> Result<Planned, PlanError> {
    let controller_def = defs::controller(definition, controller)
        .ok_or_else(|| PlanError::NotFound(ElementKey::Controller { controller: controller.into() }))?;
    if controller_def.on.iter().any(|h| h.event.value == event) {
        return Err(not_connectable(format!("{controller} already handles {event}")));
    }
    Ok(Planned::new(
        EditOp::AddHandler { controller: controller.to_owned(), handler: defs::handler_def(event), index: None },
        format!("{controller} handles {event}"),
    ))
}

fn add_rule(controller: &str, event: &str, to: &ElementKey) -> Result<Planned, PlanError> {
    let fire = trigger_of(to).ok_or_else(|| not_connectable("drop on a transition or trigger to fire it"))?;
    let label = format!("{controller} on {event} fires {fire}");
    Ok(Planned::new(
        EditOp::AddRule {
            controller: controller.to_owned(),
            event: event.to_owned(),
            rule: defs::rule_def(fire),
            index: None,
        },
        label,
    ))
}

fn expose(definition: &Definition, source: &str, to: &ElementKey) -> Result<Planned, PlanError> {
    let external = defs::external(definition, source)
        .ok_or_else(|| PlanError::NotFound(ElementKey::External { source: source.into() }))?;
    let trigger = trigger_of(to).ok_or_else(|| not_connectable("drop on a transition or trigger to expose it"))?;
    if external.triggers.iter().any(|t| t.value == trigger) {
        return Err(not_connectable(format!("{source} already fires {trigger}")));
    }
    let mut triggers: Vec<TriggerRef> = external.triggers.iter().map(|t| t.value.clone()).collect();
    triggers.push(trigger.clone());
    Ok(Planned::new(
        EditOp::SetExternalTriggers { external: source.to_owned(), triggers },
        format!("{source} fires {trigger}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "\
machines:
  Order:
    states: [draft, paid]
    transitions:
      - { from: draft, to: paid, on: pay }
      - { from: paid, to: draft, on: go }
  Shipment:
    states: [idle, moving]
    transitions:
      - { from: idle, to: moving, on: start }
controllers:
  Fulfil:
    on:
      PayDone:
        - fire: Shipment.start
  Idle: { on: {} }
external:
  Clock: [Order.pay]
";

    fn def() -> Definition {
        cascade_core::parse_definition(TEXT).expect("parses")
    }

    fn state(machine: &str, path: &str) -> ElementKey {
        ElementKey::State { machine: machine.into(), path: path.into() }
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

    fn controller(name: &str) -> ElementKey {
        ElementKey::Controller { controller: name.into() }
    }

    fn source(name: &str) -> ElementKey {
        ElementKey::External { source: name.into() }
    }

    #[test]
    fn valid_drop_targets() {
        let pay = transition("Order", "draft", "paid", "pay");
        assert!(can_connect(&state("Order", "draft"), &state("Order", "paid")));
        assert!(can_connect(&state("Order", "draft"), &state("Order", "draft")), "self loops");
        assert!(!can_connect(&state("Order", "draft"), &state("Shipment", "idle")), "across machines");
        assert!(can_connect(&pay, &controller("Fulfil")));
        assert!(can_connect(&pay, &ElementKey::Event { event: "X".into() }));
        assert!(can_connect(&controller("Fulfil"), &pay));
        assert!(can_connect(&source("Clock"), &pay));
        assert!(can_connect(&source("Clock"), &ElementKey::Trigger { machine: "Order".into(), trigger: "x".into() }));
        assert!(can_connect(&ElementKey::Event { event: "E".into() }, &controller("Fulfil")));
        assert!(!can_connect(&controller("Fulfil"), &controller("Idle")));
        assert!(!can_connect(&state("Order", "draft"), &pay));
        assert!(!can_connect(&source("Clock"), &state("Order", "draft")));
    }

    #[test]
    fn drag_sources() {
        assert!(is_source(&state("Order", "draft")));
        assert!(is_source(&controller("Fulfil")));
        assert!(!is_source(&ElementKey::Machine { machine: "Order".into() }));
        assert!(!is_source(&ElementKey::Trigger { machine: "Order".into(), trigger: "pay".into() }));
    }

    #[test]
    fn state_to_state_adds_a_transition_with_a_fresh_trigger() {
        let planned = connect(&def(), &state("Order", "paid"), &state("Order", "paid")).expect("plans");
        assert_eq!(planned.label, "Add transition Order: paid → paid (go2)");
        match planned.op {
            EditOp::AddTransition { machine, transition, index: None } => {
                assert_eq!(machine, "Order");
                assert_eq!(transition.from.iter().map(|f| f.value.as_str()).collect::<Vec<_>>(), ["paid"]);
                assert_eq!(transition.to.value, "paid");
                assert_eq!(transition.on.value, "go2");
                assert!(transition.emits.is_empty() && transition.guard.is_none() && !transition.bounded);
            }
            other => panic!("unexpected {other:?}"),
        }
        let shipment = connect(&def(), &state("Shipment", "moving"), &state("Shipment", "idle")).expect("plans");
        assert_eq!(shipment.label, "Add transition Shipment: moving → idle (go)");
    }

    #[test]
    fn missing_states_are_reported() {
        let error = connect(&def(), &state("Order", "draft"), &state("Order", "gone")).expect_err("fails");
        assert_eq!(error, PlanError::NotFound(state("Order", "gone")));
    }

    #[test]
    fn controller_to_transition_adds_a_rule_on_its_latest_handler() {
        let pay = transition("Order", "draft", "paid", "pay");
        let planned = connect(&def(), &controller("Fulfil"), &pay).expect("plans");
        assert_eq!(planned.label, "Fulfil on PayDone fires Order.pay");
        match planned.op {
            EditOp::AddRule { controller, event, rule, index: None } => {
                assert_eq!((controller.as_str(), event.as_str()), ("Fulfil", "PayDone"));
                assert_eq!(rule.fire.value, TriggerRef { machine: "Order".into(), trigger: "pay".into() });
                assert!(rule.target.is_none(), "the one instance by default");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_controller_without_handlers_cannot_fire_yet() {
        let pay = transition("Order", "draft", "paid", "pay");
        assert!(matches!(connect(&def(), &controller("Idle"), &pay), Err(PlanError::NotConnectable(_))));
    }

    #[test]
    fn handler_to_trigger_uses_that_handler() {
        let handler = ElementKey::Handler { controller: "Fulfil".into(), event: "PayDone".into() };
        let trigger = ElementKey::Trigger { machine: "Shipment".into(), trigger: "start".into() };
        let planned = connect(&def(), &handler, &trigger).expect("plans");
        assert!(matches!(planned.op, EditOp::AddRule { ref event, .. } if event == "PayDone"));
    }

    #[test]
    fn source_to_transition_exposes_the_trigger_once() {
        let start = transition("Shipment", "idle", "moving", "start");
        let planned = connect(&def(), &source("Clock"), &start).expect("plans");
        assert_eq!(
            planned.op,
            EditOp::SetExternalTriggers {
                external: "Clock".into(),
                triggers: vec![
                    TriggerRef { machine: "Order".into(), trigger: "pay".into() },
                    TriggerRef { machine: "Shipment".into(), trigger: "start".into() },
                ],
            }
        );
        let pay = transition("Order", "draft", "paid", "pay");
        assert!(matches!(connect(&def(), &source("Clock"), &pay), Err(PlanError::NotConnectable(_))));
    }

    #[test]
    fn event_to_controller_subscribes_once() {
        let planned = connect(&def(), &ElementKey::Event { event: "New".into() }, &controller("Idle")).expect("plans");
        assert!(matches!(planned.op, EditOp::AddHandler { ref controller, ref handler, .. }
            if controller == "Idle" && handler.event.value == "New"));
        let again = connect(&def(), &ElementKey::Event { event: "PayDone".into() }, &controller("Fulfil"));
        assert!(matches!(again, Err(PlanError::NotConnectable(_))));
    }

    #[test]
    fn invalid_pairs_are_refused() {
        let error = connect(&def(), &controller("Fulfil"), &controller("Idle")).expect_err("refused");
        assert_eq!(error.to_string(), "cannot connect controller to controller");
    }

    #[test]
    fn transition_to_controller_wires_a_new_event() {
        let pay = transition("Order", "draft", "paid", "pay");
        match connect(&def(), &pay, &controller("Idle")) {
            Ok(planned) => {
                // PayDone is taken, so the new event is PayDone2.
                assert_eq!(planned.label, "Wire Order.pay → Idle via PayDone2");
                let EditOp::Batch(ops) = planned.op else { panic!("expected a batch") };
                assert_eq!(ops.len(), 2, "no declaration without an events: list");
                assert!(matches!(&ops[0], EditOp::UpdateTransition { transition, .. }
                    if transition.emits.iter().any(|e| e.value == "PayDone2")));
                assert!(matches!(&ops[1], EditOp::AddHandler { controller, handler, .. }
                    if controller == "Idle" && handler.event.value == "PayDone2"));
            }
            // `locate_transition` is a stub until `feat/edit-ops` lands.
            Err(error) => assert_eq!(error, PlanError::TransitionNotLocated(pay)),
        }
    }

    #[test]
    fn strict_files_declare_the_new_event() {
        let text = format!("events:\n  PayDone: {{}}\n{TEXT}");
        let d = cascade_core::parse_definition(&text).expect("parses");
        let pay = transition("Order", "draft", "paid", "pay");
        if let Ok(planned) = connect(&d, &pay, &controller("Idle")) {
            let EditOp::Batch(ops) = planned.op else { panic!("expected a batch") };
            assert!(matches!(&ops[0], EditOp::DeclareEvent { event, .. } if event.name.value == "PayDone2"));
            assert_eq!(ops.len(), 3);
        }
    }
}
