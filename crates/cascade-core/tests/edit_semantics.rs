//! Edit ops on the semantics-phase parts of a definition: typed fields,
//! guards, `sets`, emits `with`, subscriptions, timers, conditions and
//! target filters. Each op is applied, checked to resolve, undone and
//! redone.

mod edit_support;

use cascade_core::definition::{EmitDef, FieldDef, TargetSpec, TriggerSpec};
use cascade_core::edit::{EditError, EditOp, locate_transition};
use cascade_core::expr::{
    Assignment, CmpOp, Condition, Duration, Expr, ExprContext, FieldType, Guard, Literal, Operand, parse_expr,
};
use cascade_core::{ElementKey, ElementRef, Spanned, resolve};
use edit_support::*;

const SAGA: &str = r#"
machines:
  Order:
    fields: { orderId: string, amount: number }
    states: [draft, paid]
    transitions:
      - from: draft
        to: paid
        on: pay
        guard: "event.amount > 0"
        sets: { amount: event.amount }
        emits: [{ event: OrderPaid, with: { orderId: self.orderId } }]
  Checkout:
    fields: [orderId]
    states: [waiting, paid, expired]
    transitions:
      - { from: waiting, to: paid, on: { event: OrderPaid, where: "orderId == event.orderId" } }
      - { from: waiting, to: expired, after: 30m }
controllers:
  Audit:
    on:
      OrderPaid:
        - fire: Order.pay
          target: Order where orderId == event.orderId
          when: "?worth a look"
events:
  OrderPaid: { payload: { orderId: string } }
external:
  Customer: [Order.pay]
"#;

fn expr(text: &str) -> Expr {
    match parse_expr(text, ExprContext::Plain) {
        Ok(e) => e,
        Err(err) => panic!("{text}: {err}"),
    }
}

fn filter(text: &str) -> Expr {
    match parse_expr(text, ExprContext::Filter) {
        Ok(e) => e,
        Err(err) => panic!("{text}: {err}"),
    }
}

#[test]
fn update_guard_sets_and_emits() {
    let def = parse(SAGA);
    let mut t = machine(&def, "Order").transitions[0].clone();
    t.guard = Some(Spanned::synthetic(Guard::Expr(expr("event.amount >= 10 and self.amount == 0"))));
    t.sets.push(Spanned::synthetic(Assignment::new("orderId", Operand::Event("orderId".into()))));
    t.emits[0].with.clear();
    t.emits.push(EmitDef::plain("OrderPaid2"));
    let mut def2 = def.clone();
    def2.events.push(cascade_core::definition::EventDef {
        name: s("OrderPaid2"),
        payload: Vec::new(),
        span: cascade_core::SourceSpan::unknown(),
    });
    let applied = ok(&def2, EditOp::UpdateTransition { machine: "Order".into(), index: 0, transition: t.clone() });
    assert_eq!(machine(&applied.definition, "Order").transitions[0], t);

    // Choices and else.
    let mut choice = machine(&def, "Order").transitions[0].clone();
    choice.guard = Some(Spanned::synthetic(Guard::Choice("card declined".into())));
    ok(&def, EditOp::UpdateTransition { machine: "Order".into(), index: 0, transition: choice.clone() });
    choice.guard = Some(Spanned::synthetic(Guard::Else));
    ok(&def, EditOp::UpdateTransition { machine: "Order".into(), index: 0, transition: choice });
}

#[test]
fn edits_that_would_not_type_check_are_rejected() {
    let def = parse(SAGA);
    let mut t = machine(&def, "Order").transitions[0].clone();
    t.guard = Some(Spanned::synthetic(Guard::Expr(expr("self.amount == lots"))));
    assert!(matches!(
        rejected(&def, EditOp::UpdateTransition { machine: "Order".into(), index: 0, transition: t.clone() }),
        EditError::Invalid(_)
    ));
    t.guard = Some(Spanned::synthetic(Guard::Expr(Expr::And(vec![expr("self.amount == 1")]))));
    assert!(matches!(
        rejected(&def, EditOp::UpdateTransition { machine: "Order".into(), index: 0, transition: t.clone() }),
        EditError::Invalid(_)
    ));
    t.guard = Some(Spanned::synthetic(Guard::Choice(" padded ".into())));
    assert!(matches!(
        rejected(&def, EditOp::UpdateTransition { machine: "Order".into(), index: 0, transition: t.clone() }),
        EditError::Invalid(_)
    ));
    t.guard = None;
    t.sets = vec![Spanned::synthetic(Assignment::new("nope", Operand::Lit(Literal::new("1"))))];
    assert!(matches!(
        rejected(&def, EditOp::UpdateTransition { machine: "Order".into(), index: 0, transition: t.clone() }),
        EditError::Invalid(_)
    ));
    t.sets = vec![Spanned::synthetic(Assignment::new("bad name", Operand::Lit(Literal::new("1"))))];
    assert_eq!(
        rejected(&def, EditOp::UpdateTransition { machine: "Order".into(), index: 0, transition: t }),
        EditError::InvalidName("bad name".into())
    );
}

#[test]
fn add_subscription_and_timer_transitions() {
    let def = parse(SAGA);
    let sub = transition(&["paid"], "waiting", "ignored");
    let sub = cascade_core::definition::TransitionDef {
        trigger: TriggerSpec::Event { event: s("OrderPaid"), filter: Some(Spanned::synthetic(filter("orderId == x"))) },
        ..sub
    };
    let applied = ok(&def, EditOp::AddTransition { machine: "Checkout".into(), transition: sub, index: None });
    assert!(applied.touched.contains(&"transition:Checkout:paid->waiting@OrderPaid".parse().expect("key")));

    let timer = cascade_core::definition::TransitionDef {
        trigger: TriggerSpec::After(Spanned::synthetic(Duration::from_secs(90))),
        ..transition(&["paid"], "expired", "ignored")
    };
    let applied = ok(&def, EditOp::AddTransition { machine: "Checkout".into(), transition: timer, index: None });
    let key: ElementKey = "transition:Checkout:paid->expired@after-90s".parse().expect("key");
    assert!(applied.touched.contains(&key));
    assert_eq!(locate_transition(&applied.definition, &key), Some(("Checkout".into(), 2)));

    // A zero delay cannot be written back.
    let zero = cascade_core::definition::TransitionDef {
        trigger: TriggerSpec::After(Spanned::synthetic(Duration::ZERO)),
        ..transition(&["paid"], "expired", "ignored")
    };
    assert!(matches!(
        rejected(&def, EditOp::AddTransition { machine: "Checkout".into(), transition: zero, index: None }),
        EditError::Invalid(_)
    ));
    // Rules may not fire a subscription trigger.
    let mut bad = rule("Checkout", "OrderPaid");
    bad.target = None;
    assert!(matches!(
        rejected(
            &def,
            EditOp::AddRule { controller: "Audit".into(), event: "OrderPaid".into(), rule: bad, index: None }
        ),
        EditError::Invalid(_)
    ));
}

#[test]
fn rename_event_renames_subscriptions_and_their_trigger() {
    let def = parse(SAGA);
    let applied = ok(&def, EditOp::RenameEvent { from: "OrderPaid".into(), to: "Paid".into() });
    let checkout = machine(&applied.definition, "Checkout");
    assert!(matches!(&checkout.transitions[0].trigger, TriggerSpec::Event { event, .. } if event.value == "Paid"));
    assert_eq!(machine(&applied.definition, "Order").transitions[0].emits[0].event.value, "Paid");
    assert!(applied.touched.contains(&"transition:Checkout:waiting->paid@Paid".parse().expect("key")));
    let model = match resolve(applied.definition.clone()) {
        Ok(m) => m,
        Err(err) => panic!("{err}"),
    };
    let c = model.machine_by_name("Checkout").expect("Checkout");
    assert!(model.trigger_by_name(c, "Paid").is_some());
    assert!(model.trigger_by_name(c, "OrderPaid").is_none());
}

#[test]
fn removing_a_subscribed_strict_event_is_rejected() {
    let def = parse(&SAGA.replace("events:\n  OrderPaid", "events:\n  Other: {}\n  OrderPaid"));
    assert!(matches!(
        rejected(&def, EditOp::RemoveEventDeclaration { event: "OrderPaid".into() }),
        EditError::Invalid(_)
    ));
}

#[test]
fn typed_fields_round_trip() {
    let def = parse(SAGA);
    let fields = vec![
        FieldDef::typed("orderId", FieldType::String),
        FieldDef::typed("amount", FieldType::Number),
        FieldDef::typed("tier", FieldType::Enum(vec!["basic".into(), "gold".into()])),
    ];
    let applied = ok(&def, EditOp::SetMachineFields { machine: "Order".into(), fields: fields.clone() });
    assert_eq!(machine(&applied.definition, "Order").fields, fields);
    // Back to untyped (amount is still read as `any`).
    ok(&def, EditOp::SetMachineFields { machine: "Order".into(), fields: vec!["orderId".into(), "amount".into()] });
    // Dropping a field an expression reads is rejected.
    assert!(matches!(
        rejected(&def, EditOp::SetMachineFields { machine: "Order".into(), fields: vec!["orderId".into()] }),
        EditError::Invalid(_)
    ));
    // Changing a type so that an assignment no longer fits is rejected:
    // `with: { orderId: self.orderId }` feeds a string payload field.
    assert!(matches!(
        rejected(
            &def,
            EditOp::SetMachineFields {
                machine: "Order".into(),
                fields: vec![
                    FieldDef::typed("orderId", FieldType::Number),
                    FieldDef::typed("amount", FieldType::Number)
                ],
            }
        ),
        EditError::Invalid(_)
    ));
}

#[test]
fn rule_conditions_and_targets() {
    let def = parse(SAGA);
    let mut r = handler(&def, "Audit", "OrderPaid").rules[0].clone();
    r.when = Some(Spanned::synthetic(Condition::Expr(expr("event.orderId != x"))));
    r.target = Some(Spanned::synthetic(TargetSpec::All {
        machine: "Order".into(),
        filter: Some(filter("orderId == event.orderId and amount > 3")),
    }));
    ok(&def, EditOp::UpdateRule { controller: "Audit".into(), event: "OrderPaid".into(), index: 0, rule: r.clone() });
    r.target = Some(Spanned::synthetic(TargetSpec::Spawn {
        machine: "Order".into(),
        assignments: vec![Assignment::new("orderId", Operand::Event("orderId".into()))],
    }));
    ok(&def, EditOp::UpdateRule { controller: "Audit".into(), event: "OrderPaid".into(), index: 0, rule: r.clone() });
    r.when = Some(Spanned::synthetic(Condition::Expr(Expr::Cmp {
        left: Operand::SelfField("orderId".into()),
        op: CmpOp::Eq,
        right: Operand::Lit(Literal::new("x")),
    })));
    assert!(matches!(
        rejected(&def, EditOp::UpdateRule { controller: "Audit".into(), event: "OrderPaid".into(), index: 0, rule: r }),
        EditError::Invalid(_)
    ));
}

#[test]
fn rename_machine_updates_target_specs() {
    let def = parse(SAGA);
    let applied = ok(&def, EditOp::RenameMachine { from: "Order".into(), to: "Purchase".into() });
    let r = &handler(&applied.definition, "Audit", "OrderPaid").rules[0];
    assert_eq!(
        r.target.as_ref().map(|t| t.value.to_string()).as_deref(),
        Some("Purchase where orderId == event.orderId")
    );
}

#[test]
fn every_transition_key_is_located() {
    let def = parse(SAGA);
    let model = match resolve(def.clone()) {
        Ok(m) => m,
        Err(err) => panic!("{err}"),
    };
    for t in model.transition_ids() {
        let key = model.key_of(ElementRef::Transition(t));
        assert!(locate_transition(&def, &key).is_some(), "{key}");
    }
}
