//! The semantics-phase definition format: typed fields and payloads,
//! subscriptions, timers, guards, conditions, `sets`, emits `with`, and
//! every diagnostic the parser and resolver report for them.

use cascade_core::definition::{TriggerSpec, fields_are_typed};
use cascade_core::expr::{CmpOp, Condition, Duration, Expr, FieldDecl, FieldType, Guard, Literal, Operand};
use cascade_core::model::{Target, TriggerKind};
use cascade_core::{DiagnosticKind, LoadError, Model, load_str, parse_definition};

fn load(text: &str) -> Model {
    match load_str(text) {
        Ok(model) => model,
        Err(err) => panic!("expected the definition to load:\n{err}"),
    }
}

fn load_err(text: &str) -> LoadError {
    match load_str(text) {
        Ok(_) => panic!("expected a load error:\n{text}"),
        Err(err) => err,
    }
}

fn kinds(err: &LoadError) -> Vec<&DiagnosticKind> {
    err.diagnostics.iter().map(|d| &d.kind).collect()
}

fn has(err: &LoadError, pred: impl Fn(&DiagnosticKind) -> bool) -> bool {
    err.diagnostics.iter().any(|d| pred(&d.kind))
}

const CHECKOUT: &str = r#"
machines:
  Order:
    fields: { orderId: string, amount: number, gift: bool, tier: "enum [basic, gold]", note: any }
    states: [draft, paid, done]
    transitions:
      - from: draft
        to: paid
        on: pay
        guard: "event.amount > 0 and self.tier == gold"
        sets: { amount: event.amount, tier: basic }
        emits:
          - OrderPaid
          - event: Receipt
            with: { total: self.amount, kind: card }
      - { from: draft, to: done, on: pay, guard: "?card declined" }
      - { from: draft, to: done, on: pay, guard: else }
  Checkout:
    fields: [orderId]
    states: [waiting, paid, expired]
    transitions:
      - from: waiting
        to: paid
        on: { event: OrderPaid, where: "orderId == event.orderId" }
      - { from: [waiting, paid], to: expired, after: 30m }
controllers:
  Billing:
    on:
      OrderPaid:
        - fire: Order.pay
          target: Order where orderId == event.orderId and event.kind == refund
          when: "event.amount >= 100"
        - fire: Order.pay
          when: "?not a gift card"
events:
  OrderPaid: { payload: { orderId: string, amount: number, kind: any } }
  Receipt: { payload: { total: number, kind: string } }
external:
  Customer: [Order.pay]
"#;

#[test]
fn typed_and_untyped_field_lists() {
    let def = match parse_definition(CHECKOUT) {
        Ok(def) => def,
        Err(err) => panic!("{err}"),
    };
    assert!(fields_are_typed(&def.machines[0].fields));
    assert!(!fields_are_typed(&def.machines[1].fields));
    let model = load(CHECKOUT);
    let order = model.machine_by_name("Order").expect("Order");
    assert_eq!(
        model.machine(order).fields,
        [
            FieldDecl::new("orderId", FieldType::String),
            FieldDecl::new("amount", FieldType::Number),
            FieldDecl::new("gift", FieldType::Bool),
            FieldDecl::new("tier", FieldType::Enum(vec!["basic".into(), "gold".into()])),
            FieldDecl::new("note", FieldType::Any),
        ]
    );
    let checkout = model.machine_by_name("Checkout").expect("Checkout");
    assert_eq!(model.machine(checkout).fields, [FieldDecl::untyped("orderId")]);
    let paid = model.event_by_name("OrderPaid").expect("OrderPaid");
    assert_eq!(model.event(paid).payload[1], FieldDecl::new("amount", FieldType::Number));
}

#[test]
fn guards_sets_and_emits_with() {
    let model = load(CHECKOUT);
    let order = model.machine_by_name("Order").expect("Order");
    let ts = &model.machine(order).transitions;
    let first = model.transition(ts[0]);
    assert_eq!(
        first.guard.as_ref().map(ToString::to_string).as_deref(),
        Some("event.amount > 0 and self.tier == gold")
    );
    assert_eq!(
        first.sets.iter().map(ToString::to_string).collect::<Vec<_>>(),
        ["amount = event.amount", "tier = basic"]
    );
    assert_eq!(first.emits.len(), 2);
    assert!(first.emits[0].with.is_empty());
    assert_eq!(model.event(first.emits[1].event).name, "Receipt");
    assert_eq!(
        first.emits[1].with.iter().map(ToString::to_string).collect::<Vec<_>>(),
        ["total = self.amount", "kind = card"]
    );
    assert_eq!(model.transition(ts[1]).guard, Some(Guard::Choice("card declined".into())));
    assert_eq!(model.transition(ts[2]).guard, Some(Guard::Else));
}

#[test]
fn trigger_kinds_and_subscribers() {
    let model = load(CHECKOUT);
    let checkout = model.machine_by_name("Checkout").expect("Checkout");
    let paid_event = model.event_by_name("OrderPaid").expect("OrderPaid");
    let sub = model.trigger_by_name(checkout, "OrderPaid").expect("subscription trigger");
    assert_eq!(model.trigger(sub).kind, TriggerKind::Event(paid_event));
    assert_eq!(model.event(paid_event).subscribers, [sub]);
    assert_eq!(model.subscription_trigger(checkout, paid_event), Some(sub));
    let t = model.trigger(sub).accepted_by[0];
    assert_eq!(
        model.transition(t).filter,
        Some(Expr::Cmp {
            left: Operand::SelfField("orderId".into()),
            op: CmpOp::Eq,
            right: Operand::Event("orderId".into())
        })
    );

    let timer = model.trigger_by_name(checkout, "after-30m").expect("timer trigger");
    assert_eq!(model.trigger(timer).kind, TriggerKind::Timer(Duration::from_secs(1800)));
    assert_eq!(model.trigger(timer).accepted_by.len(), 2, "one per source state");

    let order = model.machine_by_name("Order").expect("Order");
    let pay = model.trigger_by_name(order, "pay").expect("pay");
    assert_eq!(model.trigger(pay).kind, TriggerKind::Command);
}

#[test]
fn rule_conditions_and_target_filters() {
    let model = load(CHECKOUT);
    let rules: Vec<_> = model.rules().map(|(_, r)| r).collect();
    assert_eq!(rules[0].condition.as_ref().map(ToString::to_string).as_deref(), Some("event.amount >= 100"));
    assert_eq!(rules[1].condition, Some(Condition::Choice("not a gift card".into())));
    let Target::One { filter: Some(Expr::And(items)) } = &rules[0].target else { panic!("{:?}", rules[0].target) };
    // #32: `event.x == lit` in a selector.
    assert_eq!(
        items[1],
        Expr::Cmp { left: Operand::Event("kind".into()), op: CmpOp::Eq, right: Operand::Lit(Literal::new("refund")) }
    );
    assert_eq!(rules[1].target, Target::One { filter: None });
}

#[test]
fn types_for_expressions() {
    let model = load(CHECKOUT);
    let order = model.machine_by_name("Order").expect("Order");
    let paid = model.event_by_name("OrderPaid").expect("OrderPaid");
    let types = model.types_for(order, Some(paid));
    assert_eq!(types.self_type("amount"), &FieldType::Number);
    assert_eq!(types.event_type("amount"), &FieldType::Number);
    assert_eq!(types.event_type("missing"), &FieldType::Any);
    assert_eq!(model.event_types(paid).self_type("amount"), &FieldType::Any);
}

fn machine_with(transition: &str) -> String {
    format!(
        "machines:\n  M:\n    fields: {{ n: number, tier: \"enum [a, b]\", s: string, ok: bool }}\n    states: [x, y]\n    transitions:\n      - {transition}\nevents:\n  E: {{ payload: {{ n: number, s: string }} }}\n  F: {{ payload: [k] }}\n"
    )
}

#[test]
fn parse_errors_for_embedded_languages() {
    for (transition, what) in [
        ("{ from: x, to: y, on: go, guard: \"amount is positive\" }", "guard"),
        ("{ from: x, to: y, on: go, guard: \"?\" }", "empty choice"),
        ("{ from: x, to: y, on: go, guard: \"   \" }", "blank guard"),
        ("{ from: x, to: y, on: go, sets: { n: \"a b\" } }", "sets value"),
        ("{ from: x, to: y, on: go, emits: [{ event: E, with: { n: \"event.\" } }] }", "with value"),
        ("{ from: x, to: y, on: { event: E, where: \"n ==\" } }", "where"),
    ] {
        let err = load_err(&machine_with(transition));
        assert!(has(&err, |k| matches!(k, DiagnosticKind::InvalidExpression { .. })), "{what}: {err}");
    }
    let err = load_err(&machine_with("{ from: x, to: y, after: 30 }"));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::InvalidDuration { .. })), "{err}");
    let err = load_err(&machine_with("{ from: x, to: y, after: 0s }"));
    assert!(
        has(
            &err,
            |k| matches!(k, DiagnosticKind::InvalidDuration { message, .. } if message.contains("greater than zero"))
        ),
        "{err}"
    );
    let err = load_err(&machine_with("{ from: x, to: y, on: go, after: 1s }"));
    assert!(has(
        &err,
        |k| matches!(k, DiagnosticKind::ConflictingKeys { first, second, .. } if first == "on" && second == "after")
    ));
    let err = load_err(&machine_with("{ from: x, to: y }"));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::MissingKey { key, .. } if key == "on")));
    let err = load_err("machines:\n  M:\n    fields: { n: integer }\n    states: [x]\n");
    assert!(has(&err, |k| matches!(k, DiagnosticKind::InvalidFieldType { .. })), "{err}");
    let err = load_err(
        "machines:\n  M:\n    states: [x]\ncontrollers:\n  C:\n    on:\n      E: { fire: M.go, when: \"busy\" }\n",
    );
    assert!(has(&err, |k| matches!(k, DiagnosticKind::InvalidExpression { .. })), "{err}");
    let err = load_err(&machine_with("{ from: x, to: y, on: { event: E, filter: \"n == 1\" } }"));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::UnknownKey { key, .. } if key == "filter")), "{err}");
}

#[test]
fn diagnostics_carry_the_span_of_the_string() {
    let text = machine_with("{ from: x, to: y, on: go, guard: \"self.nope == 1\" }");
    let err = load_err(&text);
    let d = &err.diagnostics[0];
    assert!(matches!(&d.kind, DiagnosticKind::UnknownField { field, .. } if field == "nope"));
    assert_eq!(d.span.start.line, 6);
    let line = text.lines().nth(5).unwrap_or_default();
    let col = line.find("\"self.nope").map(|c| c + 1);
    assert_eq!(Some(d.span.start.col as usize), col);
}

#[test]
fn type_errors() {
    for (transition, check) in [
        ("{ from: x, to: y, on: go, guard: \"self.n == abc\" }", "number vs word"),
        ("{ from: x, to: y, on: go, guard: \"self.n > '5'\" }", "number vs quoted"),
        ("{ from: x, to: y, on: go, guard: \"self.ok == yes\" }", "bool vs word"),
        ("{ from: x, to: y, on: go, guard: \"self.s < b\" }", "ordering on string"),
        ("{ from: x, to: y, on: go, guard: \"self.tier > a\" }", "ordering on enum"),
        ("{ from: x, to: y, on: go, guard: \"self.n == self.s\" }", "number vs string fields"),
        ("{ from: x, to: y, on: { event: E }, sets: { n: event.s } }", "assignment"),
        ("{ from: x, to: y, on: go, sets: { ok: maybe } }", "bool assignment"),
        ("{ from: x, to: y, on: { event: E, where: \"n == event.s\" } }", "filter"),
        ("{ from: x, to: y, on: go, emits: [{ event: E, with: { n: self.s } }] }", "with"),
    ] {
        let err = load_err(&machine_with(transition));
        assert!(has(&err, |k| matches!(k, DiagnosticKind::TypeMismatch { .. })), "{check}: {err}");
    }
    for (transition, check) in [
        ("{ from: x, to: y, on: go, guard: \"self.tier == c\" }", "enum literal"),
        ("{ from: x, to: y, on: go, guard: \"self.tier in [a, c]\" }", "enum in list"),
        ("{ from: x, to: y, on: go, sets: { tier: c } }", "enum assignment"),
    ] {
        let err = load_err(&machine_with(transition));
        assert!(has(&err, |k| matches!(k, DiagnosticKind::NotAMember { value, .. } if value == "c")), "{check}: {err}");
    }
    // Compatible: any with anything, numbers with numbers, enum members.
    for transition in [
        "{ from: x, to: y, on: go, guard: \"self.n > 3 and self.tier in [a, b] and self.ok != false\" }",
        "{ from: x, to: y, on: go, guard: \"event.anything == whatever\" }",
        "{ from: x, to: y, on: { event: E, where: \"n == event.n and s == event.s\" } }",
        "{ from: x, to: y, on: go, sets: { n: 4, s: \"text\", tier: b, ok: true } }",
        "{ from: x, to: y, on: go, emits: [{ event: E, with: { n: self.n, s: anything } }] }",
        "{ from: x, to: y, on: go, emits: [{ event: F, with: { k: self.tier } }] }",
    ] {
        load(&machine_with(transition));
    }
}

#[test]
fn unknown_fields_and_payload_fields() {
    let err = load_err(&machine_with("{ from: x, to: y, on: go, sets: { nope: 1 } }"));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::UnknownField { field, .. } if field == "nope")));
    let err = load_err(&machine_with("{ from: x, to: y, on: { event: E, where: \"n == event.nope\" } }"));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::UnknownPayloadField { field, .. } if field == "nope")));
    let err = load_err(&machine_with("{ from: x, to: y, on: { event: E }, guard: \"event.nope == 1\" }"));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::UnknownPayloadField { field, .. } if field == "nope")));
    let err = load_err(&machine_with("{ from: x, to: y, on: go, emits: [{ event: E, with: { nope: 1 } }] }"));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::UnknownPayloadField { field, .. } if field == "nope")));
    // A command trigger's payload is open: `event.x` in its guard is `any`.
    load(&machine_with("{ from: x, to: y, on: go, guard: \"event.nope == 1\" }"));
    let err = load_err("machines:\n  M:\n    fields: [a, a]\n    states: [x]\n");
    assert!(has(&err, |k| matches!(k, DiagnosticKind::Duplicate { what: "field", .. })));
}

#[test]
fn self_is_not_allowed_in_conditions_or_spawns() {
    let base = "machines:\n  M:\n    fields: [id]\n    states: [x, y]\n    transitions: [{ from: x, to: y, on: go }]\ncontrollers:\n  C:\n    on:\n      E:\n";
    let err = load_err(&format!("{base}        - {{ fire: M.go, when: \"self.id == 1\" }}\n"));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::SelfNotAllowed { .. })), "{err}");
    let err = load_err(&format!("{base}        - {{ fire: M.go, target: \"new M with id = self.id\" }}\n"));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::SelfNotAllowed { .. })), "{err}");
    let err = load_err(&format!("{base}        - {{ fire: M.go, target: \"new M with nope = 1\" }}\n"));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::UnknownField { field, .. } if field == "nope")), "{err}");
    let model = load(&format!(
        "{base}        - {{ fire: M.go, when: \"event.id == 1\", target: \"all M where id == event.id or event.k in [a]\" }}\n"
    ));
    assert_eq!(model.rule_count(), 1);
}

#[test]
fn trigger_kind_conflicts() {
    // A command named like a subscribed event, in either order.
    let err = load_err(
        "machines:\n  M:\n    states: [x, y]\n    transitions:\n      - { from: x, to: y, on: { event: E } }\n      - { from: y, to: x, on: E }\n",
    );
    assert!(has(&err, |k| matches!(
        k,
        DiagnosticKind::TriggerKindConflict { trigger, first: "subscription", second: "command", .. } if trigger == "E"
    )));
    let err = load_err(
        "machines:\n  M:\n    states: [x, y]\n    transitions:\n      - { from: y, to: x, on: after-30m }\n      - { from: x, to: y, after: 30m }\n",
    );
    assert!(has(&err, |k| matches!(k, DiagnosticKind::TriggerKindConflict { first: "command", second: "timer", .. })));
    // Rules and external sources may only fire commands.
    let err = load_err(
        "machines:\n  M:\n    states: [x, y]\n    transitions:\n      - { from: x, to: y, after: 5s }\ncontrollers:\n  C:\n    on:\n      E: { fire: M.after-5s }\n",
    );
    assert!(has(&err, |k| matches!(k, DiagnosticKind::TriggerKindConflict { first: "timer", second: "command", .. })));
    let err = load_err(
        "machines:\n  M:\n    states: [x, y]\n    transitions:\n      - { from: x, to: y, on: { event: E } }\nexternal:\n  U: [M.E]\n",
    );
    assert!(has(&err, |k| matches!(k, DiagnosticKind::TriggerKindConflict { second: "command", .. })));
    // The same delay written two ways is one timer trigger.
    let model = load(
        "machines:\n  M:\n    states: [x, y]\n    transitions:\n      - { from: x, to: y, after: 120s }\n      - { from: y, to: x, after: 2m }\n",
    );
    let m = model.machine_by_name("M").expect("M");
    assert_eq!(model.machine(m).triggers.len(), 1);
    assert_eq!(model.trigger(model.machine(m).triggers[0]).name, "after-2m");
}

#[test]
fn strict_events_cover_subscriptions() {
    let err = load_err(
        "machines:\n  M:\n    states: [x, y]\n    transitions:\n      - { from: x, to: y, on: { event: Unknown }, emits: [E] }\nevents:\n  E: {}\n",
    );
    assert!(has(&err, |k| matches!(k, DiagnosticKind::UndeclaredEvent { event } if event == "Unknown")));
    // Lenient files create the event on first mention.
    let model = load(
        "machines:\n  M:\n    states: [x, y]\n    transitions:\n      - { from: x, to: y, on: { event: Ping } }\n",
    );
    let ping = model.event_by_name("Ping").expect("Ping");
    assert!(!model.event(ping).declared);
    assert_eq!(model.event(ping).subscribers.len(), 1);
}

#[test]
fn emits_accept_every_shape() {
    let model = load(
        "machines:\n  M:\n    states: [x, y]\n    transitions:\n      - { from: x, to: y, on: a, emits: E }\n      - { from: y, to: x, on: b, emits: { event: F, with: { k: v } } }\n      - { from: x, to: x, on: c, emits: [E, { event: F }, E] }\n",
    );
    let counts: Vec<usize> = model.transitions().map(|(_, t)| t.emits.len()).collect();
    assert_eq!(counts, [1, 1, 2], "duplicates of one event are dropped");
    let def =
        match parse_definition("machines:\n  M:\n    states: [x]\n    transitions: [{ from: x, to: x, after: 90s }]\n")
        {
            Ok(def) => def,
            Err(err) => panic!("{err}"),
        };
    assert_eq!(def.machines[0].transitions[0].trigger.name(), "after-90s");
    assert!(matches!(def.machines[0].transitions[0].trigger, TriggerSpec::After(_)));
    assert!(
        kinds(&load_err(
            "machines:\n  M:\n    states: [x]\n    transitions: [{ from: x, to: x, on: a, emits: [[E]] }]\n"
        ))
        .iter()
        .any(|k| matches!(k, DiagnosticKind::WrongType { .. }))
    );
}
