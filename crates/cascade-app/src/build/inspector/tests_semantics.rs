//! Inspector tests for typed fields, trigger kinds, `sets:`/`with:` rows
//! and live type feedback.

use cascade_core::definition::{FieldDef, TransitionDef, TriggerSpec};
use cascade_core::edit::EditOp;
use cascade_core::expr::{Assignment, Duration, FieldType, Literal, Operand};

use super::*;

const TEXT: &str = "\
events:
  Paid:
    payload:
      orderId: string
      amount: number
  Shipped:
    payload: [orderId]
machines:
  Order:
    fields:
      orderId: string
      tier: enum [basic, gold]
    states: [draft, placed, paid, expired]
    transitions:
      - { from: draft, to: placed, on: place }
      - from: placed
        to: paid
        on: pay
        guard: \"event.amount > 0\"
        sets: { tier: gold }
        emits:
          - event: Paid
            with: { orderId: self.orderId }
      - { from: placed, to: expired, after: 30m }
  Shipment:
    fields: [orderId]
    states: [idle, moving]
    transitions:
      - from: idle
        to: moving
        on: { event: Paid, where: \"orderId == event.orderId\" }
      - { from: moving, to: idle, on: stop }
controllers:
  Fulfil:
    on:
      Paid:
        - fire: Shipment.stop
          when: \"event.amount > 10\"
";

fn def() -> Definition {
    cascade_core::parse_definition(TEXT).expect("parses")
}

#[test]
fn the_fixture_loads() {
    cascade_core::resolve(def()).expect("resolves");
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

fn place() -> ElementKey {
    transition("Order", "draft", "placed", "place")
}

fn pay() -> ElementKey {
    transition("Order", "placed", "paid", "pay")
}

fn expire() -> ElementKey {
    transition("Order", "placed", "expired", "after-30m")
}

fn subscribed() -> ElementKey {
    transition("Shipment", "idle", "moving", "Paid")
}

fn rule() -> ElementKey {
    ElementKey::Rule { controller: "Fulfil".into(), event: "Paid".into(), ordinal: 0 }
}

fn set(field: FieldId, value: &str, key: &ElementKey) -> Result<Option<EditOp>, FieldError> {
    field_op(&def(), key, field, &FieldValue::Text(value.into())).map(|p| p.map(|p| p.op))
}

/// The edited transition entry of an `UpdateTransition`.
fn edited(field: FieldId, value: &str, key: &ElementKey) -> TransitionDef {
    match set(field, value, key) {
        Ok(Some(EditOp::UpdateTransition { transition, .. })) => transition,
        other => panic!("expected an UpdateTransition, got {other:?}"),
    }
}

fn text_of(inspection: &Inspection, id: FieldId) -> Option<String> {
    inspection.fields.iter().find(|f| f.id == id).and_then(|f| match &f.input {
        FieldInput::Text(t) => Some(t.clone()),
        _ => None,
    })
}

fn ids(inspection: &Inspection) -> Vec<FieldId> {
    inspection.fields.iter().map(|f| f.id).collect()
}

fn check_text(key: &ElementKey, field: FieldId, text: &str) -> Result<(), String> {
    check(&def(), key, field, text)
}

// --- Typed fields and payloads ---------------------------------------------------

#[test]
fn typed_fields_show_their_types() {
    let i = inspect(&def(), &ElementKey::Machine { machine: "Order".into() }).expect("inspects");
    assert_eq!(text_of(&i, FieldId::MachineFields).as_deref(), Some("orderId: string, tier: enum [basic, gold]"));
    let untyped = inspect(&def(), &ElementKey::Machine { machine: "Shipment".into() }).expect("inspects");
    assert_eq!(text_of(&untyped, FieldId::MachineFields).as_deref(), Some("orderId"));
    let paid = inspect(&def(), &ElementKey::Event { event: "Paid".into() }).expect("inspects");
    assert_eq!(text_of(&paid, FieldId::EventPayload).as_deref(), Some("orderId: string, amount: number"));
}

#[test]
fn typed_field_edits_make_typed_ops() {
    let order = ElementKey::Machine { machine: "Order".into() };
    assert_eq!(set(FieldId::MachineFields, "orderId: string, tier: enum [basic, gold]", &order), Ok(None));
    assert_eq!(
        set(FieldId::MachineFields, "orderId: string, tier: enum [basic, gold, vip], gift: bool", &order),
        Ok(Some(EditOp::SetMachineFields {
            machine: "Order".into(),
            fields: vec![
                FieldDef::typed("orderId", FieldType::String),
                FieldDef::typed("tier", FieldType::Enum(vec!["basic".into(), "gold".into(), "vip".into()])),
                FieldDef::typed("gift", FieldType::Bool),
            ],
        }))
    );
    // A typed list with an untyped name: that field is `any`.
    let shipment = ElementKey::Machine { machine: "Shipment".into() };
    assert_eq!(
        set(FieldId::MachineFields, "orderId, weight: number", &shipment),
        Ok(Some(EditOp::SetMachineFields {
            machine: "Shipment".into(),
            fields: vec![FieldDef::typed("orderId", FieldType::Any), FieldDef::typed("weight", FieldType::Number)],
        }))
    );
    assert!(matches!(set(FieldId::MachineFields, "a: integer", &order), Err(FieldError::Invalid(_))));
    assert!(validate(FieldId::MachineFields, "a: enum [x, y], b").is_ok());
    assert!(validate(FieldId::MachineFields, "a: enum [x, y").is_err());
}

#[test]
fn typed_payload_edits_redeclare_the_event() {
    let paid = ElementKey::Event { event: "Paid".into() };
    match set(FieldId::EventPayload, "orderId: string, amount: number, note", &paid) {
        Ok(Some(EditOp::Batch(ops))) => match &ops[1] {
            EditOp::DeclareEvent { event, index: Some(0) } => assert_eq!(
                event.payload,
                vec![
                    FieldDef::typed("orderId", FieldType::String),
                    FieldDef::typed("amount", FieldType::Number),
                    FieldDef::typed("note", FieldType::Any),
                ]
            ),
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(set(FieldId::EventPayload, "orderId: string, amount: number", &paid), Ok(None));
}

#[test]
fn removing_a_field_that_is_used_is_reported_live() {
    let order = ElementKey::Machine { machine: "Order".into() };
    let message = check_text(&order, FieldId::MachineFields, "orderId: string").expect_err("tier is used");
    assert!(message.contains("tier"), "{message}");
    assert_eq!(check_text(&order, FieldId::MachineFields, "orderId: string, tier: enum [basic, gold]"), Ok(()));
    let enum_message = check_text(&order, FieldId::MachineFields, "orderId: string, tier: enum [basic]")
        .expect_err("gold is no longer a member");
    assert!(enum_message.contains("gold"), "{enum_message}");
}

// --- Trigger kinds ---------------------------------------------------------------------

#[test]
fn transitions_show_their_trigger_kind() {
    let i = inspect(&def(), &pay()).expect("inspects");
    match i.fields.iter().find(|f| f.id == FieldId::TransitionKind).map(|f| &f.input) {
        Some(FieldInput::Choice { value, options }) => {
            assert_eq!(value, "command");
            assert_eq!(options.iter().map(|o| o.0.as_str()).collect::<Vec<_>>(), ["command", "event", "timer"]);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(text_of(&i, FieldId::TransitionTrigger).as_deref(), Some("pay"));
    assert!(!ids(&i).contains(&FieldId::TransitionEvent));
    assert!(!ids(&i).contains(&FieldId::TransitionAfter));

    let timer = inspect(&def(), &expire()).expect("inspects");
    assert_eq!(text_of(&timer, FieldId::TransitionAfter).as_deref(), Some("30m"));
    assert!(!ids(&timer).contains(&FieldId::TransitionTrigger));

    let sub = inspect(&def(), &subscribed()).expect("inspects");
    assert_eq!(text_of(&sub, FieldId::TransitionEvent).as_deref(), Some("Paid"));
    assert_eq!(text_of(&sub, FieldId::TransitionWhere).as_deref(), Some("orderId == event.orderId"));
}

#[test]
fn a_draft_kind_shows_blank_fields_and_a_prompt() {
    let i = inspect_with(&def(), &place(), Some(TriggerKindTag::Event)).expect("inspects");
    assert_eq!(text_of(&i, FieldId::TransitionEvent).as_deref(), Some(""));
    assert_eq!(text_of(&i, FieldId::TransitionWhere).as_deref(), Some(""));
    assert!(!ids(&i).contains(&FieldId::TransitionTrigger));
    assert!(i.notes.iter().any(|n| n == TriggerKindTag::Event.prompt()));
    let same = inspect_with(&def(), &place(), Some(TriggerKindTag::Command)).expect("inspects");
    assert!(same.notes.is_empty(), "the draft is the actual kind: {:?}", same.notes);
    assert!(matches!(set(FieldId::TransitionKind, "event", &place()), Err(FieldError::Invalid(_))));
}

#[test]
fn trigger_kind_edits() {
    let sub = edited(FieldId::TransitionEvent, "Shipped", &place());
    assert!(matches!(&sub.trigger, TriggerSpec::Event { event, filter: None } if event.value == "Shipped"));

    let timer = edited(FieldId::TransitionAfter, "5m", &place());
    assert!(matches!(&timer.trigger, TriggerSpec::After(d) if d.value == Duration::from_secs(300)));
    assert_eq!(set(FieldId::TransitionAfter, "30m", &expire()), Ok(None));
    assert!(matches!(set(FieldId::TransitionAfter, "0s", &expire()), Err(FieldError::Invalid(_))));
    assert!(matches!(set(FieldId::TransitionAfter, "soon", &expire()), Err(FieldError::Invalid(_))));

    let command = edited(FieldId::TransitionTrigger, "lapse", &expire());
    assert!(matches!(&command.trigger, TriggerSpec::Command(n) if n.value == "lapse"));
    // The subscribed event's name as a command makes it a command.
    let same_name = edited(FieldId::TransitionTrigger, "Paid", &subscribed());
    assert!(matches!(&same_name.trigger, TriggerSpec::Command(n) if n.value == "Paid"));

    // Changing the event keeps the filter; `where` needs a subscription.
    let moved = edited(FieldId::TransitionEvent, "Shipped", &subscribed());
    assert!(matches!(&moved.trigger, TriggerSpec::Event { filter: Some(_), .. }));
    let unfiltered = edited(FieldId::TransitionWhere, "", &subscribed());
    assert!(matches!(&unfiltered.trigger, TriggerSpec::Event { filter: None, .. }));
    let filtered = edited(FieldId::TransitionWhere, "orderId == event.orderId and event.amount > 1", &subscribed());
    match &filtered.trigger {
        TriggerSpec::Event { filter: Some(f), .. } => {
            assert_eq!(f.value.display_filter().to_string(), "orderId == event.orderId and event.amount > 1");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(set(FieldId::TransitionWhere, "orderId == event.orderId", &pay()), Err(FieldError::Invalid(_))));
    assert!(validate(FieldId::TransitionWhere, "orderId ==").is_err());
}

// --- sets and with rows ------------------------------------------------------------------

#[test]
fn sets_and_with_rows_are_listed() {
    let i = inspect(&def(), &pay()).expect("inspects");
    assert_eq!(text_of(&i, FieldId::TransitionSet(0)).as_deref(), Some("tier = gold"));
    assert_eq!(text_of(&i, FieldId::TransitionSetNew).as_deref(), Some(""));
    let with = FieldId::EmitWith { emit: 0, row: 0 };
    assert_eq!(text_of(&i, with).as_deref(), Some("orderId = self.orderId"));
    let label = |id| i.fields.iter().find(|f| f.id == id).map(|f| f.label.clone());
    assert_eq!(label(with).as_deref(), Some("With Paid"));
    assert_eq!(label(FieldId::EmitWithNew { emit: 0 }).as_deref(), Some("+ With Paid"));
    // Order: guard, sets, emits, with rows, bounded.
    let order = ids(&i);
    let pos = |id| order.iter().position(|f| *f == id).expect("listed");
    assert!(pos(FieldId::TransitionGuard) < pos(FieldId::TransitionSet(0)));
    assert!(pos(FieldId::TransitionSetNew) < pos(FieldId::TransitionEmits));
    assert!(pos(FieldId::TransitionEmits) < pos(with));
    assert!(pos(FieldId::EmitWithNew { emit: 0 }) < pos(FieldId::TransitionBounded));
}

#[test]
fn set_rows_edit_add_and_remove() {
    let gold = || Assignment::new("tier", Operand::Lit(Literal::new("gold")));
    let replaced = edited(FieldId::TransitionSet(0), "tier = basic", &pay());
    assert_eq!(
        replaced.sets.iter().map(|s| s.value.clone()).collect::<Vec<_>>(),
        [Assignment::new("tier", Operand::Lit(Literal::new("basic")))]
    );
    let removed = edited(FieldId::TransitionSet(0), "  ", &pay());
    assert!(removed.sets.is_empty());
    let added = edited(FieldId::TransitionSetNew, "orderId = event.orderId", &pay());
    assert_eq!(
        added.sets.iter().map(|s| s.value.clone()).collect::<Vec<_>>(),
        [gold(), Assignment::new("orderId", Operand::Event("orderId".into()))]
    );
    assert_eq!(set(FieldId::TransitionSetNew, "", &pay()), Ok(None));
    assert_eq!(set(FieldId::TransitionSet(0), "tier = gold", &pay()), Ok(None));
    assert!(matches!(set(FieldId::TransitionSet(4), "tier = gold", &pay()), Err(FieldError::Invalid(_))));
    assert!(matches!(set(FieldId::TransitionSetNew, "tier gold", &pay()), Err(FieldError::Invalid(_))));
    assert!(validate(FieldId::TransitionSet(0), "").is_ok(), "blank removes the row");
}

#[test]
fn with_rows_edit_add_and_remove() {
    let row = FieldId::EmitWith { emit: 0, row: 0 };
    let replaced = edited(row, "orderId = event.orderId", &pay());
    assert_eq!(replaced.emits[0].with[0].value, Assignment::new("orderId", Operand::Event("orderId".into())));
    let removed = edited(row, "", &pay());
    assert!(removed.emits[0].with.is_empty());
    assert_eq!(removed.emits[0].event.value, "Paid");
    let added = edited(FieldId::EmitWithNew { emit: 0 }, "amount = event.amount", &pay());
    assert_eq!(added.emits[0].with.len(), 2);
    assert!(matches!(
        set(FieldId::EmitWithNew { emit: 3 }, "amount = 1", &pay()),
        Err(FieldError::Invalid(m)) if m.contains("no emit")
    ));
    // Renaming the emits list keeps the `with:` of events that stay.
    let kept = edited(FieldId::TransitionEmits, "Paid, Shipped", &pay());
    assert_eq!(kept.emits[0].with.len(), 1);
    assert!(kept.emits[1].with.is_empty());
}

// --- Live feedback -------------------------------------------------------------------------

#[test]
fn guard_feedback_parses_then_types() {
    assert_eq!(check_text(&pay(), FieldId::TransitionGuard, "self.tier == gold"), Ok(()));
    assert_eq!(check_text(&pay(), FieldId::TransitionGuard, "event.amount > 0"), Ok(()), "unchanged");
    assert_eq!(check_text(&pay(), FieldId::TransitionGuard, ""), Ok(()));
    let parse = check_text(&pay(), FieldId::TransitionGuard, "event.amount >").expect_err("incomplete");
    assert!(parse.contains("column"), "{parse}");
    let member = check_text(&pay(), FieldId::TransitionGuard, "self.tier == platinum").expect_err("not a member");
    assert!(member.contains("platinum"), "{member}");
    let unknown = check_text(&pay(), FieldId::TransitionGuard, "self.nope == a").expect_err("unknown field");
    assert!(unknown.contains("nope"), "{unknown}");
}

#[test]
fn filter_condition_and_row_feedback() {
    let unknown = check_text(&subscribed(), FieldId::TransitionWhere, "orderId == event.missing")
        .expect_err("Paid declares no `missing`");
    assert!(unknown.contains("missing"), "{unknown}");
    assert_eq!(
        check_text(&subscribed(), FieldId::TransitionWhere, "orderId == event.orderId and event.amount > 1"),
        Ok(())
    );
    assert!(check_text(&pay(), FieldId::TransitionSetNew, "tier = vip").is_err(), "vip is not a tier");
    assert_eq!(check_text(&pay(), FieldId::TransitionSetNew, "tier = basic"), Ok(()));
    assert!(check_text(&pay(), FieldId::TransitionSetNew, "nope = event.amount").is_err());
    assert_eq!(check_text(&rule(), FieldId::RuleWhen, "event.amount > 10"), Ok(()));
    assert!(check_text(&rule(), FieldId::RuleWhen, "event.amount > abc").is_err(), "a number against a word");
    assert!(check_text(&rule(), FieldId::RuleWhen, "self.orderId == a").is_err(), "rules have no self");
    assert!(check_text(&rule(), FieldId::RuleTarget, "Shipment where nope == event.orderId").is_err());
    assert_eq!(check_text(&rule(), FieldId::RuleTarget, "Shipment where orderId == event.orderId"), Ok(()));
}

#[test]
fn feedback_reports_names_that_are_taken() {
    let order = ElementKey::Machine { machine: "Order".into() };
    assert!(check_text(&order, FieldId::MachineName, "Shipment").is_err());
    assert_eq!(check_text(&order, FieldId::MachineName, "Purchase"), Ok(()));
}
