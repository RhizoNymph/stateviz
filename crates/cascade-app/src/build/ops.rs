//! Toolbar actions as edit ops: add a machine, state, controller or source
//! with a fresh default name, and delete the selected element.
//!
//! Pure: each function reads the definition (and the selection) and
//! returns a [`Planned`] op with a label for the undo history, or a
//! [`PlanError`] the host shows. `edit::apply` does the rest, including
//! cascading removals.

use cascade_core::definition::TransitionDef;
use cascade_core::edit::{EditOp, fresh_name, locate_transition};
use cascade_core::{Definition, ElementKey};

use super::defs;

/// An op ready to commit, with its undo-history label.
#[derive(Clone, Debug, PartialEq)]
pub struct Planned {
    pub op: EditOp,
    pub label: String,
}

impl Planned {
    pub fn new(op: EditOp, label: impl Into<String>) -> Planned {
        Planned { op, label: label.into() }
    }
}

/// Why a toolbar action or gesture produced no op.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("select {0} first")]
    NeedsSelection(&'static str),
    #[error("{0} is not in the definition")]
    NotFound(ElementKey),
    /// `edit::locate_transition` did not find the entry (or is a stub).
    #[error("cannot find the `transitions:` entry of {0}")]
    TransitionNotLocated(ElementKey),
    #[error("{0}")]
    NotDeletable(&'static str),
    #[error("{0}")]
    NotConnectable(String),
}

/// Add a machine `Machine`, `Machine2`, … with one state.
pub fn add_machine(definition: &Definition) -> Planned {
    let taken = defs::machine_names(definition);
    let name = fresh_name("Machine", taken.iter().map(String::as_str));
    let machine = defs::machine_def(&name, "idle");
    Planned::new(EditOp::AddMachine { machine, index: None }, format!("Add machine {name}"))
}

pub fn add_controller(definition: &Definition) -> Planned {
    let taken = defs::controller_names(definition);
    let name = fresh_name("Controller", taken.iter().map(String::as_str));
    Planned::new(
        EditOp::AddController { controller: defs::controller_def(&name), index: None },
        format!("Add controller {name}"),
    )
}

pub fn add_external(definition: &Definition) -> Planned {
    let taken = defs::external_names(definition);
    let name = fresh_name("Source", taken.iter().map(String::as_str));
    Planned::new(EditOp::AddExternal { external: defs::external_def(&name), index: None }, format!("Add source {name}"))
}

/// Where "Add state" puts the new state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateTarget {
    pub machine: String,
    /// Parent state path; `None` for the top level.
    pub parent: Option<String>,
}

/// Into the selected machine; into the selected compound state; next to
/// the selected atomic state; into the machine of a selected transition or
/// trigger. With no usable selection, into the only machine if there is
/// just one.
pub fn state_target(definition: &Definition, selection: Option<&ElementKey>) -> Result<StateTarget, PlanError> {
    let top = |machine: &str| Ok(StateTarget { machine: machine.to_owned(), parent: None });
    match selection {
        Some(ElementKey::Machine { machine }) => {
            defs::machine(definition, machine)
                .ok_or_else(|| PlanError::NotFound(ElementKey::Machine { machine: machine.clone() }))?;
            top(machine)
        }
        Some(key @ ElementKey::State { machine, path }) => {
            let state = defs::machine(definition, machine)
                .and_then(|m| defs::state(m, path))
                .ok_or_else(|| PlanError::NotFound(key.clone()))?;
            let parent =
                if state.states.is_empty() { defs::parent_path(path).map(str::to_owned) } else { Some(path.clone()) };
            Ok(StateTarget { machine: machine.clone(), parent })
        }
        Some(ElementKey::Transition { machine, .. } | ElementKey::Trigger { machine, .. })
            if defs::machine(definition, machine).is_some() =>
        {
            top(machine)
        }
        _ => match definition.machines.as_slice() {
            [only] => top(&only.name.value),
            _ => Err(PlanError::NeedsSelection("a machine or a state to add a state to")),
        },
    }
}

/// Add a state `state`, `state2`, … (fresh among all the machine's state
/// names) under `target`.
pub fn add_state(definition: &Definition, target: &StateTarget) -> Result<Planned, PlanError> {
    let machine = defs::machine(definition, &target.machine)
        .ok_or_else(|| PlanError::NotFound(ElementKey::Machine { machine: target.machine.clone() }))?;
    let taken = defs::state_names(machine);
    let name = fresh_name("state", taken.iter().map(String::as_str));
    let path = defs::join_path(target.parent.as_deref(), &name);
    Ok(Planned::new(
        EditOp::AddState {
            machine: target.machine.clone(),
            parent: target.parent.clone(),
            state: defs::state_def(&name),
            index: None,
        },
        format!("Add state {}.{path}", target.machine),
    ))
}

/// The `transitions:` entry of a transition key.
pub fn transition_entry<'a>(
    definition: &'a Definition,
    key: &ElementKey,
) -> Result<(String, usize, &'a TransitionDef), PlanError> {
    let (machine, index) =
        locate_transition(definition, key).ok_or_else(|| PlanError::TransitionNotLocated(key.clone()))?;
    let entry = defs::machine(definition, &machine)
        .and_then(|m| m.transitions.get(index))
        .ok_or_else(|| PlanError::TransitionNotLocated(key.clone()))?;
    Ok((machine, index, entry))
}

/// Delete `key` (removal cascades in `edit::apply`). Deleting an event
/// removes it from every `emits:`, every handler subscribed to it and its
/// declaration, as one batch.
pub fn delete(definition: &Definition, key: &ElementKey) -> Result<Planned, PlanError> {
    let missing = || PlanError::NotFound(key.clone());
    let planned = match key {
        ElementKey::Machine { machine } => {
            defs::machine(definition, machine).ok_or_else(missing)?;
            Planned::new(EditOp::RemoveMachine { machine: machine.clone() }, format!("Delete machine {machine}"))
        }
        ElementKey::State { machine, path } => {
            defs::machine(definition, machine).and_then(|m| defs::state(m, path)).ok_or_else(missing)?;
            Planned::new(
                EditOp::RemoveState { machine: machine.clone(), path: path.clone() },
                format!("Delete state {machine}.{path}"),
            )
        }
        ElementKey::Transition { machine, from, to, trigger, .. } => {
            let (machine_name, index, _) = transition_entry(definition, key)?;
            Planned::new(
                EditOp::RemoveTransition { machine: machine_name, index },
                format!("Delete transition {machine}: {from} → {to} ({trigger})"),
            )
        }
        ElementKey::Trigger { .. } => {
            return Err(PlanError::NotDeletable(
                "a trigger exists through its transitions, fires and sources; delete those instead",
            ));
        }
        ElementKey::Event { event } => delete_event(definition, event).ok_or_else(missing)?,
        ElementKey::Controller { controller } => {
            defs::controller(definition, controller).ok_or_else(missing)?;
            Planned::new(
                EditOp::RemoveController { controller: controller.clone() },
                format!("Delete controller {controller}"),
            )
        }
        ElementKey::Handler { controller, event } => {
            defs::handler(definition, controller, event).ok_or_else(missing)?;
            Planned::new(
                EditOp::RemoveHandler { controller: controller.clone(), event: event.clone() },
                format!("Delete handler {controller} on {event}"),
            )
        }
        ElementKey::Rule { controller, event, ordinal } => {
            defs::rule(definition, controller, event, *ordinal).ok_or_else(missing)?;
            let index = usize::try_from(*ordinal).map_err(|_| missing())?;
            Planned::new(
                EditOp::RemoveRule { controller: controller.clone(), event: event.clone(), index },
                format!("Delete rule {controller}/{event}#{ordinal}"),
            )
        }
        ElementKey::External { source } => {
            defs::external(definition, source).ok_or_else(missing)?;
            Planned::new(EditOp::RemoveExternal { external: source.clone() }, format!("Delete source {source}"))
        }
    };
    Ok(planned)
}

fn delete_event(definition: &Definition, event: &str) -> Option<Planned> {
    let mut ops = Vec::new();
    for machine in &definition.machines {
        for (index, transition) in machine.transitions.iter().enumerate() {
            if transition.emits.iter().any(|e| e.value == event) {
                let mut updated = transition.clone();
                updated.emits.retain(|e| e.value != event);
                ops.push(EditOp::UpdateTransition { machine: machine.name.value.clone(), index, transition: updated });
            }
        }
    }
    for controller in &definition.controllers {
        if controller.on.iter().any(|h| h.event.value == event) {
            ops.push(EditOp::RemoveHandler { controller: controller.name.value.clone(), event: event.to_owned() });
        }
    }
    if definition.events.iter().any(|e| e.name.value == event) {
        ops.push(EditOp::RemoveEventDeclaration { event: event.to_owned() });
    }
    let op = match ops.len() {
        0 => return None,
        1 => ops.pop()?,
        _ => EditOp::Batch(ops),
    };
    Some(Planned::new(op, format!("Delete event {event}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "\
events:
  Paid: { payload: [orderId] }
machines:
  Order:
    states:
      - draft
      - placed:
          states: [waiting, paid]
    transitions:
      - { from: draft, to: placed, on: place }
      - { from: placed.waiting, to: placed.paid, on: pay, emits: [Paid] }
  Shipment:
    states: [idle, moving]
controllers:
  Fulfil:
    on:
      Paid:
        - fire: Shipment.start
external:
  Clock: [Order.place]
";

    fn def() -> Definition {
        cascade_core::parse_definition(TEXT).expect("parses")
    }

    fn state_key(machine: &str, path: &str) -> ElementKey {
        ElementKey::State { machine: machine.into(), path: path.into() }
    }

    #[test]
    fn adds_use_fresh_default_names() {
        let mut d = def();
        let planned = add_machine(&d);
        assert_eq!(planned.label, "Add machine Machine");
        match &planned.op {
            EditOp::AddMachine { machine, index: None } => {
                assert_eq!(machine.name.value, "Machine");
                assert_eq!(machine.states.len(), 1);
            }
            other => panic!("unexpected {other:?}"),
        }
        d.machines.push(defs::machine_def("Machine", "idle"));
        assert_eq!(add_machine(&d).label, "Add machine Machine2");
        assert_eq!(add_controller(&d).label, "Add controller Controller");
        assert_eq!(add_external(&d).label, "Add source Source");
        d.external.push(defs::external_def("Source"));
        assert_eq!(add_external(&d).label, "Add source Source2");
    }

    #[test]
    fn state_target_follows_the_selection() {
        let d = def();
        let machine = ElementKey::Machine { machine: "Order".into() };
        assert_eq!(state_target(&d, Some(&machine)), Ok(StateTarget { machine: "Order".into(), parent: None }));
        assert_eq!(
            state_target(&d, Some(&state_key("Order", "placed"))),
            Ok(StateTarget { machine: "Order".into(), parent: Some("placed".into()) }),
            "into a compound state"
        );
        assert_eq!(
            state_target(&d, Some(&state_key("Order", "placed.paid"))),
            Ok(StateTarget { machine: "Order".into(), parent: Some("placed".into()) }),
            "next to an atomic state"
        );
        assert_eq!(
            state_target(&d, Some(&state_key("Order", "draft"))),
            Ok(StateTarget { machine: "Order".into(), parent: None })
        );
        let trigger = ElementKey::Trigger { machine: "Shipment".into(), trigger: "start".into() };
        assert_eq!(state_target(&d, Some(&trigger)), Ok(StateTarget { machine: "Shipment".into(), parent: None }));
        assert_eq!(state_target(&d, None), Err(PlanError::NeedsSelection("a machine or a state to add a state to")));
        assert!(matches!(state_target(&d, Some(&state_key("Order", "nope"))), Err(PlanError::NotFound(_))));
    }

    #[test]
    fn with_one_machine_no_selection_is_needed() {
        let d = cascade_core::parse_definition(crate::build::disk::NEW_FILE_TEXT).expect("parses");
        assert_eq!(state_target(&d, None), Ok(StateTarget { machine: "Machine".into(), parent: None }));
        let planned = add_state(&d, &StateTarget { machine: "Machine".into(), parent: None }).expect("plans");
        assert_eq!(planned.label, "Add state Machine.state");
    }

    #[test]
    fn new_state_names_avoid_every_state_name_in_the_machine() {
        let mut d = def();
        let placed = StateTarget { machine: "Order".into(), parent: Some("placed".into()) };
        if let Some(order) = d.machines.first_mut() {
            order.states.push(defs::state_def("state"));
        }
        let planned = add_state(&d, &placed).expect("plans");
        assert_eq!(planned.label, "Add state Order.placed.state2");
        match planned.op {
            EditOp::AddState { machine, parent, state, index } => {
                assert_eq!((machine.as_str(), parent.as_deref(), index), ("Order", Some("placed"), None));
                assert_eq!(state.name.value, "state2");
            }
            other => panic!("unexpected {other:?}"),
        }
        let missing = StateTarget { machine: "Nope".into(), parent: None };
        assert!(matches!(add_state(&d, &missing), Err(PlanError::NotFound(_))));
    }

    #[test]
    fn delete_maps_each_kind_to_its_remove_op() {
        let d = def();
        let cases = [
            (ElementKey::Machine { machine: "Order".into() }, EditOp::RemoveMachine { machine: "Order".into() }),
            (
                state_key("Order", "placed.paid"),
                EditOp::RemoveState { machine: "Order".into(), path: "placed.paid".into() },
            ),
            (
                ElementKey::Controller { controller: "Fulfil".into() },
                EditOp::RemoveController { controller: "Fulfil".into() },
            ),
            (
                ElementKey::Handler { controller: "Fulfil".into(), event: "Paid".into() },
                EditOp::RemoveHandler { controller: "Fulfil".into(), event: "Paid".into() },
            ),
            (
                ElementKey::Rule { controller: "Fulfil".into(), event: "Paid".into(), ordinal: 0 },
                EditOp::RemoveRule { controller: "Fulfil".into(), event: "Paid".into(), index: 0 },
            ),
            (ElementKey::External { source: "Clock".into() }, EditOp::RemoveExternal { external: "Clock".into() }),
        ];
        for (key, op) in cases {
            assert_eq!(delete(&d, &key).map(|p| p.op), Ok(op), "{key}");
        }
    }

    #[test]
    fn delete_labels_name_the_element() {
        let d = def();
        let planned = delete(&d, &state_key("Order", "draft")).expect("plans");
        assert_eq!(planned.label, "Delete state Order.draft");
    }

    #[test]
    fn deleting_an_event_unhooks_it_everywhere() {
        let d = def();
        let planned = delete(&d, &ElementKey::Event { event: "Paid".into() }).expect("plans");
        let EditOp::Batch(ops) = planned.op else { panic!("expected a batch") };
        assert_eq!(ops.len(), 3);
        match &ops[0] {
            EditOp::UpdateTransition { machine, index, transition } => {
                assert_eq!((machine.as_str(), *index), ("Order", 1));
                assert!(transition.emits.is_empty());
                assert_eq!(transition.on.value, "pay");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(ops[1], EditOp::RemoveHandler { controller: "Fulfil".into(), event: "Paid".into() });
        assert_eq!(ops[2], EditOp::RemoveEventDeclaration { event: "Paid".into() });
    }

    #[test]
    fn missing_elements_and_triggers_are_not_deleted() {
        let d = def();
        assert!(matches!(delete(&d, &state_key("Order", "gone")), Err(PlanError::NotFound(_))));
        assert!(matches!(delete(&d, &ElementKey::Event { event: "Nope".into() }), Err(PlanError::NotFound(_))));
        let trigger = ElementKey::Trigger { machine: "Order".into(), trigger: "pay".into() };
        assert!(matches!(delete(&d, &trigger), Err(PlanError::NotDeletable(_))));
        let rule = ElementKey::Rule { controller: "Fulfil".into(), event: "Paid".into(), ordinal: 3 };
        assert!(matches!(delete(&d, &rule), Err(PlanError::NotFound(_))));
    }

    #[test]
    fn deleting_a_transition_goes_through_locate_transition() {
        let d = def();
        let key = ElementKey::Transition {
            machine: "Order".into(),
            from: "draft".into(),
            to: "placed".into(),
            trigger: "place".into(),
            ordinal: 0,
        };
        match delete(&d, &key) {
            Ok(planned) => assert_eq!(planned.op, EditOp::RemoveTransition { machine: "Order".into(), index: 0 }),
            // `locate_transition` is a stub until `feat/edit-ops` lands.
            Err(error) => assert_eq!(error, PlanError::TransitionNotLocated(key)),
        }
    }
}
