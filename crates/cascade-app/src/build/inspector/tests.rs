use cascade_core::edit::EditOp;

use super::*;
use crate::build::ops::Planned;

const TEXT: &str = "\
events:
  Paid: { payload: [orderId] }
machines:
  Order:
    color: blue
    fields: [orderId]
    states:
      - draft
      - placed:
          states: [waiting, paid]
    transitions:
      - { from: draft, to: placed, on: place }
      - { from: placed.waiting, to: placed.paid, on: pay, emits: [Paid] }
  Shipment:
    states: [idle, moving]
    transitions:
      - { from: idle, to: moving, on: start }
controllers:
  Fulfil:
    on:
      Paid:
        - fire: Shipment.start
          target: Shipment where orderId == event.orderId
external:
  Clock: [Order.place]
";

fn def() -> Definition {
    cascade_core::parse_definition(TEXT).expect("parses")
}

fn machine() -> ElementKey {
    ElementKey::Machine { machine: "Order".into() }
}

fn state(path: &str) -> ElementKey {
    ElementKey::State { machine: "Order".into(), path: path.into() }
}

fn rule() -> ElementKey {
    ElementKey::Rule { controller: "Fulfil".into(), event: "Paid".into(), ordinal: 0 }
}

fn set(field: FieldId, value: &str, key: &ElementKey) -> Result<Option<EditOp>, FieldError> {
    field_op(&def(), key, field, &FieldValue::Text(value.into())).map(|p| p.map(|p| p.op))
}

fn value_of(inspection: &Inspection, id: FieldId) -> Option<&FieldInput> {
    inspection.fields.iter().find(|f| f.id == id).map(|f| &f.input)
}

#[test]
fn machine_inspection_lists_its_properties() {
    let i = inspect(&def(), &machine()).expect("inspects");
    assert_eq!(i.title, "Machine Order");
    assert_eq!(value_of(&i, FieldId::MachineName), Some(&FieldInput::Text("Order".into())));
    assert_eq!(value_of(&i, FieldId::MachineFields), Some(&FieldInput::Text("orderId".into())));
    match value_of(&i, FieldId::MachineColor) {
        Some(FieldInput::Choice { value, options }) => {
            assert_eq!(value, "blue");
            assert_eq!(options.len(), 9, "auto plus the eight Okabe-Ito colors");
        }
        other => panic!("unexpected {other:?}"),
    }
    match value_of(&i, FieldId::MachineInitial) {
        Some(FieldInput::Choice { value, options }) => {
            assert_eq!(value, "");
            assert_eq!(options.iter().map(|o| o.0.as_str()).collect::<Vec<_>>(), ["", "draft", "placed"]);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(i.actions, [InspectorAction::Delete]);
}

#[test]
fn machine_fields_make_their_ops() {
    assert_eq!(
        set(FieldId::MachineName, "Purchase", &machine()),
        Ok(Some(EditOp::RenameMachine { from: "Order".into(), to: "Purchase".into() }))
    );
    assert_eq!(set(FieldId::MachineName, "Order", &machine()), Ok(None), "unchanged");
    assert_eq!(
        set(FieldId::MachineColor, "vermillion", &machine()),
        Ok(Some(EditOp::SetMachineColor { machine: "Order".into(), color: Some(PaletteColor::Vermillion) }))
    );
    assert_eq!(
        set(FieldId::MachineColor, "", &machine()),
        Ok(Some(EditOp::SetMachineColor { machine: "Order".into(), color: None }))
    );
    assert_eq!(
        set(FieldId::MachineInitial, "placed", &machine()),
        Ok(Some(EditOp::SetMachineInitial { machine: "Order".into(), initial: Some("placed".into()) }))
    );
    assert!(matches!(set(FieldId::MachineInitial, "waiting", &machine()), Err(FieldError::Invalid(_))));
    assert_eq!(
        set(FieldId::MachineFields, "orderId, customerId", &machine()),
        Ok(Some(EditOp::SetMachineFields {
            machine: "Order".into(),
            fields: vec!["orderId".into(), "customerId".into()]
        }))
    );
    assert_eq!(
        set(FieldId::MachineDomain, "sales", &machine()),
        Ok(Some(EditOp::SetMachineDomain { machine: "Order".into(), domain: Some("sales".into()) }))
    );
    assert_eq!(set(FieldId::MachineDomain, "", &machine()), Ok(None));
}

#[test]
fn invalid_input_changes_nothing() {
    assert_eq!(
        set(FieldId::MachineName, "not valid", &machine()),
        Err(FieldError::Invalid("`not valid` is not a valid name (letters, digits, _ and -)".into()))
    );
    assert!(matches!(set(FieldId::MachineColor, "chartreuse", &machine()), Err(FieldError::Invalid(_))));
    assert!(matches!(set(FieldId::StateKind, "weird", &state("draft")), Err(FieldError::Invalid(_))));
    assert!(matches!(set(FieldId::RuleWhen, "x", &machine()), Err(FieldError::Invalid(_))), "wrong field");
}

#[test]
fn state_inspection_and_ops() {
    let i = inspect(&def(), &state("placed")).expect("inspects");
    assert_eq!(i.title, "State Order.placed");
    assert!(value_of(&i, FieldId::StateInitial).is_some(), "compound states pick an initial child");
    assert_eq!(i.actions, [InspectorAction::AddChildState, InspectorAction::Delete]);
    let atomic = inspect(&def(), &state("draft")).expect("inspects");
    assert!(value_of(&atomic, FieldId::StateInitial).is_none());

    assert_eq!(
        set(FieldId::StateName, "open", &state("placed")),
        Ok(Some(EditOp::RenameState { machine: "Order".into(), path: "placed".into(), to: "open".into() }))
    );
    assert_eq!(
        set(FieldId::StateKind, "final", &state("draft")),
        Ok(Some(EditOp::SetStateKind { machine: "Order".into(), path: "draft".into(), kind: StateKindDef::Final }))
    );
    assert_eq!(set(FieldId::StateKind, "normal", &state("draft")), Ok(None));
    assert_eq!(
        set(FieldId::StateInitial, "paid", &state("placed")),
        Ok(Some(EditOp::SetStateInitial {
            machine: "Order".into(),
            path: "placed".into(),
            initial: Some("paid".into())
        }))
    );
    assert!(matches!(set(FieldId::StateInitial, "draft", &state("placed")), Err(FieldError::Invalid(_))));
}

#[test]
fn event_ops() {
    let paid = ElementKey::Event { event: "Paid".into() };
    assert_eq!(
        set(FieldId::EventName, "OrderPaid", &paid),
        Ok(Some(EditOp::RenameEvent { from: "Paid".into(), to: "OrderPaid".into() }))
    );
    match set(FieldId::EventPayload, "orderId, amount", &paid) {
        Ok(Some(EditOp::Batch(ops))) => {
            assert_eq!(ops[0], EditOp::RemoveEventDeclaration { event: "Paid".into() });
            assert!(matches!(&ops[1], EditOp::DeclareEvent { event, index: Some(0) }
                if event.payload.iter().map(|p| p.value.as_str()).collect::<Vec<_>>() == ["orderId", "amount"]));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(set(FieldId::EventPayload, "orderId", &paid), Ok(None));
    let undeclared = ElementKey::Event { event: "Other".into() };
    assert!(matches!(set(FieldId::EventPayload, "x", &undeclared), Ok(Some(EditOp::DeclareEvent { index: None, .. }))));
    let i = inspect(&def(), &undeclared).expect("inspects");
    assert_eq!(i.notes.len(), 1);
}

#[test]
fn controller_inspection_lists_handlers() {
    let key = ElementKey::Controller { controller: "Fulfil".into() };
    let i = inspect(&def(), &key).expect("inspects");
    let (title, items) = i.list.expect("handlers");
    assert_eq!(title, "Handlers");
    assert_eq!(
        items,
        [ListItem {
            label: "on Paid (1 rule)".into(),
            key: ElementKey::Handler { controller: "Fulfil".into(), event: "Paid".into() }
        }]
    );
    assert_eq!(
        set(FieldId::ControllerName, "Router", &key),
        Ok(Some(EditOp::RenameController { from: "Fulfil".into(), to: "Router".into() }))
    );
    assert!(matches!(set(FieldId::ControllerAddHandler, "Shipped", &key), Ok(Some(EditOp::AddHandler { .. }))));
    assert!(matches!(set(FieldId::ControllerAddHandler, "Paid", &key), Err(FieldError::Invalid(_))));
}

#[test]
fn handler_inspection_lists_rules() {
    let key = ElementKey::Handler { controller: "Fulfil".into(), event: "Paid".into() };
    let i = inspect(&def(), &key).expect("inspects");
    let (_, items) = i.list.expect("rules");
    assert_eq!(items[0].label, "fire Shipment.start");
    assert_eq!(items[0].key, rule());
}

#[test]
fn rule_fields_validate_with_the_core_grammar() {
    let i = inspect(&def(), &rule()).expect("inspects");
    assert_eq!(
        value_of(&i, FieldId::RuleTarget),
        Some(&FieldInput::Text("Shipment where orderId == event.orderId".into()))
    );
    assert!(validate(FieldId::RuleTarget, "all Shipment where orderId == event.orderId").is_ok());
    assert!(validate(FieldId::RuleTarget, "").is_ok(), "blank is the one instance");
    assert!(validate(FieldId::RuleTarget, "Shipment where ==").is_err());
    assert!(validate(FieldId::RuleFire, "Shipment.start").is_ok());
    assert!(validate(FieldId::RuleFire, "Shipment").is_err());

    match set(FieldId::RuleTarget, "new Shipment with orderId = event.orderId", &rule()) {
        Ok(Some(EditOp::UpdateRule { controller, event, index, rule })) => {
            assert_eq!((controller.as_str(), event.as_str(), index), ("Fulfil", "Paid", 0));
            assert_eq!(
                rule.target.map(|t| t.value.to_string()),
                Some("new Shipment with orderId = event.orderId".into())
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(
        matches!(set(FieldId::RuleTarget, "", &rule()), Ok(Some(EditOp::UpdateRule { rule, .. })) if rule.target.is_none())
    );
    assert_eq!(set(FieldId::RuleTarget, "Shipment where orderId == event.orderId", &rule()), Ok(None));
    assert!(matches!(set(FieldId::RuleWhen, "stock left", &rule()), Ok(Some(EditOp::UpdateRule { rule, .. }))
        if rule.when.as_ref().map(|w| w.value.as_str()) == Some("stock left")));
    let bounded = field_op(&def(), &rule(), FieldId::RuleBounded, &FieldValue::Toggle(true));
    assert!(matches!(bounded, Ok(Some(Planned { op: EditOp::UpdateRule { rule, .. }, .. })) if rule.bounded));
    assert!(matches!(set(FieldId::RuleFire, "Shipment", &rule()), Err(FieldError::Invalid(_))));
}

#[test]
fn source_ops() {
    let key = ElementKey::External { source: "Clock".into() };
    let i = inspect(&def(), &key).expect("inspects");
    assert_eq!(value_of(&i, FieldId::SourceTriggers), Some(&FieldInput::Text("Order.place".into())));
    assert_eq!(
        set(FieldId::SourceTriggers, "Order.place, Shipment.start", &key),
        Ok(Some(EditOp::SetExternalTriggers {
            external: "Clock".into(),
            triggers: vec![
                TriggerRef { machine: "Order".into(), trigger: "place".into() },
                TriggerRef { machine: "Shipment".into(), trigger: "start".into() },
            ],
        }))
    );
    assert_eq!(set(FieldId::SourceTriggers, "Order.place", &key), Ok(None));
    assert!(matches!(set(FieldId::SourceTriggers, "Order.place, nope", &key), Err(FieldError::Invalid(_))));
    assert_eq!(
        set(FieldId::SourceName, "Timer", &key),
        Ok(Some(EditOp::RenameExternal { from: "Clock".into(), to: "Timer".into() }))
    );
}

#[test]
fn transitions_need_their_entry() {
    let key = ElementKey::Transition {
        machine: "Order".into(),
        from: "draft".into(),
        to: "placed".into(),
        trigger: "place".into(),
        ordinal: 0,
    };
    let i = inspect(&def(), &key).expect("inspects");
    match set(FieldId::TransitionTrigger, "submit", &key) {
        Ok(Some(EditOp::UpdateTransition { machine, index, transition })) => {
            assert_eq!((machine.as_str(), index, transition.on.value.as_str()), ("Order", 0, "submit"));
            assert_eq!(value_of(&i, FieldId::TransitionTrigger), Some(&FieldInput::Text("place".into())));
        }
        // `locate_transition` is a stub until `feat/edit-ops` lands.
        Err(FieldError::Plan(PlanError::TransitionNotLocated(_))) => {
            assert!(i.fields.is_empty());
            assert!(i.notes.iter().any(|n| n.starts_with("Read-only")));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn transition_field_syntax() {
    assert!(validate(FieldId::TransitionFrom, "a, b.c").is_ok());
    assert!(validate(FieldId::TransitionFrom, "").is_err());
    assert!(validate(FieldId::TransitionFrom, "a, b c").is_err());
    assert!(validate(FieldId::TransitionTo, "placed.paid").is_ok());
    assert!(validate(FieldId::TransitionTo, "placed..paid").is_err());
    assert!(validate(FieldId::TransitionEmits, "").is_ok());
    assert!(validate(FieldId::TransitionEmits, "A, B!").is_err());
    assert!(validate(FieldId::TransitionGuard, "anything at all").is_ok());
}

#[test]
fn triggers_are_read_only() {
    let key = ElementKey::Trigger { machine: "Order".into(), trigger: "pay".into() };
    let i = inspect(&def(), &key).expect("inspects");
    assert!(i.fields.is_empty() && i.actions.is_empty() && !i.notes.is_empty());
}

#[test]
fn missing_elements_are_reported() {
    assert!(matches!(inspect(&def(), &state("nope")), Err(PlanError::NotFound(_))));
    assert!(matches!(set(FieldId::StateName, "x", &state("nope")), Err(FieldError::Plan(PlanError::NotFound(_)))));
}

#[test]
fn labels_and_hints() {
    assert_eq!(FieldId::RuleTarget.label(), "Target");
    assert_eq!(FieldId::RuleTarget.hint(), "the one instance");
    assert_eq!(FieldId::StateInitial.label(), "Initial child");
}
