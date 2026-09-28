//! Edit ops on machines, transitions, events, controllers, external sources,
//! batches and `locate_transition`, on the examples. Every successful edit is
//! checked against the inverse law (undo restores the original, redo
//! reproduces the edit) by `ok`.

mod edit_support;

use cascade_core::definition::TargetMode;
use cascade_core::edit::{EditError, EditOp, apply, locate_transition};
use cascade_core::{DiagnosticKind, ElementKey, PaletteColor, load_str, resolve};
use edit_support::*;

fn invalid(err: &EditError, pred: impl Fn(&DiagnosticKind) -> bool) -> bool {
    match err {
        EditError::Invalid(load) => load.diagnostics.iter().any(|d| pred(&d.kind)),
        _ => false,
    }
}

fn machine_key(name: &str) -> ElementKey {
    ElementKey::Machine { machine: name.into() }
}

// --- Machines ------------------------------------------------------------------

#[test]
fn add_machine_appends_or_inserts() {
    let def = parse(ORDER_FULFILLMENT);
    let billing = machine_def("Billing", &["open", "closed"], vec![transition(&["open"], "closed", "close")]);
    let applied = ok(&def, EditOp::AddMachine { machine: billing.clone(), index: None });
    let names: Vec<_> = applied.definition.machines.iter().map(|m| m.name.value.as_str()).collect();
    assert_eq!(names, ["Order", "Shipment", "Billing"]);
    assert!(applied.touched.contains(&machine_key("Billing")));
    assert!(applied.touched.contains(&ElementKey::State { machine: "Billing".into(), path: "open".into() }));
    assert_eq!(applied.inverse, EditOp::RemoveMachine { machine: "Billing".into() });

    let applied = ok(&def, EditOp::AddMachine { machine: billing, index: Some(0) });
    assert_eq!(applied.definition.machines[0].name.value, "Billing");
}

#[test]
fn add_machine_rejections() {
    let def = parse(ORDER_FULFILLMENT);
    let taken = machine_def("Order", &["a"], Vec::new());
    assert_eq!(
        rejected(&def, EditOp::AddMachine { machine: taken, index: None }),
        EditError::NameTaken { what: "machine", name: "Order".into() }
    );
    let bad = machine_def("bad name", &["a"], Vec::new());
    assert_eq!(
        rejected(&def, EditOp::AddMachine { machine: bad, index: None }),
        EditError::InvalidName("bad name".into())
    );
    let bad_state = machine_def("Fine", &["a b"], Vec::new());
    assert_eq!(
        rejected(&def, EditOp::AddMachine { machine: bad_state, index: None }),
        EditError::InvalidName("a b".into())
    );
    let empty = machine_def("Empty", &[], Vec::new());
    let err = rejected(&def, EditOp::AddMachine { machine: empty, index: None });
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::EmptyMachine { .. })), "{err:?}");
    let dangling = machine_def("Dangling", &["a"], vec![transition(&["a"], "nowhere", "go")]);
    let err = rejected(&def, EditOp::AddMachine { machine: dangling, index: None });
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::UnknownState { .. })), "{err:?}");
    let far = machine_def("Far", &["a"], Vec::new());
    assert_eq!(
        rejected(&def, EditOp::AddMachine { machine: far, index: Some(3) }),
        EditError::IndexOutOfRange { what: "machine", index: 3, len: 2 }
    );
}

#[test]
fn remove_machine_cascades_to_rules_and_external_triggers() {
    let def = parse(ORDER_FULFILLMENT);
    let applied = ok(&def, EditOp::RemoveMachine { machine: "Shipment".into() });
    assert!(!has_machine(&applied.definition, "Shipment"));
    assert!(handler(&applied.definition, "Fulfillment", "OrderPaid").rules.is_empty());
    assert!(applied.touched.contains(&machine_key("Shipment")));
    assert!(applied.touched.contains(&ElementKey::Rule {
        controller: "Fulfillment".into(),
        event: "OrderPaid".into(),
        ordinal: 0
    }));

    let applied = ok(&def, EditOp::RemoveMachine { machine: "Order".into() });
    assert!(external_triggers(&applied.definition, "Customer").is_empty());
    assert!(external_triggers(&applied.definition, "Clock").is_empty());
    assert!(applied.touched.contains(&ElementKey::External { source: "Clock".into() }));
    // Fulfillment fires into Shipment, which stays.
    assert_eq!(handler(&applied.definition, "Fulfillment", "OrderPaid").rules.len(), 1);
}

#[test]
fn remove_machine_in_shop_restores_every_rule_on_undo() {
    let def = parse(SHOP);
    let applied = ok(&def, EditOp::RemoveMachine { machine: "Payment".into() });
    let d = &applied.definition;
    assert!(handler(d, "Checkout", "OrderPlaced").rules.is_empty());
    assert!(handler(d, "Billing", "PaymentAuthorized").rules.is_empty());
    assert!(handler(d, "Refunds", "OrderCancelled").rules.is_empty());
    assert!(external_triggers(d, "PaymentGateway").is_empty());
    assert_eq!(external_triggers(d, "Customer").len(), 3);
    // Orders fires into Order, untouched.
    assert_eq!(handler(d, "Orders", "PaymentCaptured").rules.len(), 1);

    assert_eq!(rejected(&def, EditOp::RemoveMachine { machine: "Nope".into() }), not_found("machine", "Nope"));
}

fn not_found(what: &'static str, name: &str) -> EditError {
    EditError::NotFound { what, name: name.into() }
}

#[test]
fn rename_machine_propagates_to_fires_targets_and_externals() {
    let def = parse(SHOP);
    let applied = ok(&def, EditOp::RenameMachine { from: "Shipment".into(), to: "Parcel".into() });
    let d = &applied.definition;
    assert!(has_machine(d, "Parcel") && !has_machine(d, "Shipment"));
    let rule = &handler(d, "Fulfillment", "StockReserved").rules[0];
    assert_eq!(rule.fire.value.machine, "Parcel");
    assert_eq!(rule.target.as_ref().map(|t| t.value.machine.as_str()), Some("Parcel"));
    assert_eq!(
        external_triggers(d, "Carrier"),
        ["Parcel.handed_over", "Parcel.delivered", "Parcel.lost"].map(String::from)
    );
    assert!(applied.touched.contains(&machine_key("Parcel")));
    assert!(applied.touched.contains(&ElementKey::External { source: "Carrier".into() }));
    assert_eq!(applied.inverse, EditOp::RenameMachine { from: "Parcel".into(), to: "Shipment".into() });
    // The renamed model has the same shape.
    let before = load_str(SHOP).expect("shop loads");
    let after = resolve(d.clone()).expect("renamed shop resolves");
    assert_eq!(before.rule_count(), after.rule_count());
    assert_eq!(before.trigger_count(), after.trigger_count());
}

#[test]
fn rename_machine_rejections() {
    let def = parse(SHOP);
    assert_eq!(
        rejected(&def, EditOp::RenameMachine { from: "Shipment".into(), to: "Order".into() }),
        EditError::NameTaken { what: "machine", name: "Order".into() }
    );
    assert_eq!(
        rejected(&def, EditOp::RenameMachine { from: "Shipment".into(), to: "9lives".into() }),
        EditError::InvalidName("9lives".into())
    );
    assert_eq!(
        rejected(&def, EditOp::RenameMachine { from: "Nope".into(), to: "X".into() }),
        not_found("machine", "Nope")
    );
    // Renaming to itself is a no-op.
    let same = ok(&def, EditOp::RenameMachine { from: "Order".into(), to: "Order".into() });
    assert_same(&same.definition, &def, "no-op rename");
}

#[test]
fn machine_setters_round_trip() {
    let def = parse(SHOP);
    let applied = ok(&def, EditOp::SetMachineColor { machine: "Order".into(), color: Some(PaletteColor::Yellow) });
    assert_eq!(machine(&applied.definition, "Order").color.as_ref().map(|c| c.value), Some(PaletteColor::Yellow));
    assert_eq!(applied.touched, [machine_key("Order")]);
    let applied = ok(&def, EditOp::SetMachineColor { machine: "Order".into(), color: None });
    assert!(machine(&applied.definition, "Order").color.is_none());

    let applied = ok(&def, EditOp::SetMachineDomain { machine: "Order".into(), domain: Some("sales".into()) });
    assert_eq!(machine(&applied.definition, "Order").domain.as_ref().map(|d| d.value.as_str()), Some("sales"));

    let applied = ok(&def, EditOp::SetMachineInitial { machine: "Order".into(), initial: Some("placed.paid".into()) });
    assert_eq!(initial(&applied.definition, "Order").as_deref(), Some("placed.paid"));
    let applied = ok(&def, EditOp::SetMachineInitial { machine: "Order".into(), initial: None });
    assert_eq!(initial(&applied.definition, "Order"), None);

    let applied =
        ok(&def, EditOp::SetMachineFields { machine: "Order".into(), fields: vec!["orderId".into(), "region".into()] });
    let fields: Vec<_> = machine(&applied.definition, "Order").fields.iter().map(|f| f.value.as_str()).collect();
    assert_eq!(fields, ["orderId", "region"]);
}

#[test]
fn machine_setter_rejections() {
    let def = parse(SHOP);
    let err = rejected(&def, EditOp::SetMachineInitial { machine: "Order".into(), initial: Some("limbo".into()) });
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::UnknownState { .. })), "{err:?}");
    // Dropping `orderId` breaks every selector on Order.
    let err = rejected(&def, EditOp::SetMachineFields { machine: "Order".into(), fields: vec!["customerId".into()] });
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::UnknownField { .. })), "{err:?}");
    assert_eq!(
        rejected(&def, EditOp::SetMachineFields { machine: "Order".into(), fields: vec!["order id".into()] }),
        EditError::InvalidName("order id".into())
    );
    assert_eq!(
        rejected(&def, EditOp::SetMachineColor { machine: "Nope".into(), color: None }),
        not_found("machine", "Nope")
    );
    let nested = parse(NESTED);
    let err =
        rejected(&nested, EditOp::SetMachineInitial { machine: "Job".into(), initial: Some("running.hist".into()) });
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::InitialIsHistory { .. })), "{err:?}");
}

// --- Transitions ---------------------------------------------------------------

#[test]
fn add_transition_appends_or_inserts_and_reports_its_keys() {
    let def = parse(ORDER_FULFILLMENT);
    let t = transition(&["paid"], "cancelled", "refund");
    let applied = ok(&def, EditOp::AddTransition { machine: "Order".into(), transition: t.clone(), index: None });
    assert_eq!(
        transitions(&applied.definition, "Order").last().map(String::as_str),
        Some("paid -> cancelled @ refund")
    );
    assert_eq!(
        applied.touched,
        [ElementKey::Transition {
            machine: "Order".into(),
            from: "paid".into(),
            to: "cancelled".into(),
            trigger: "refund".into(),
            ordinal: 0
        }]
    );
    assert_eq!(applied.inverse, EditOp::RemoveTransition { machine: "Order".into(), index: 3 });

    let applied = ok(&def, EditOp::AddTransition { machine: "Order".into(), transition: t, index: Some(0) });
    assert_eq!(transitions(&applied.definition, "Order")[0], "paid -> cancelled @ refund");
}

#[test]
fn add_multi_source_transition_touches_every_expansion() {
    let def = parse(NESTED);
    let t = transition(&["queued", "failed"], "done", "abort");
    let applied = ok(&def, EditOp::AddTransition { machine: "Job".into(), transition: t, index: None });
    let key = |from: &str| ElementKey::Transition {
        machine: "Job".into(),
        from: from.into(),
        to: "done".into(),
        trigger: "abort".into(),
        ordinal: 0,
    };
    assert_eq!(applied.touched, [key("queued"), key("failed")]);
    let model = resolve(applied.definition.clone()).expect("resolves");
    for key in &applied.touched {
        assert!(model.resolve_key(key).is_some(), "{key}");
    }
}

#[test]
fn add_duplicate_transition_gets_the_next_ordinal() {
    let def = parse(NESTED);
    let t = transition(&["fetching"], "fetching", "retry");
    let applied = ok(&def, EditOp::AddTransition { machine: "Job".into(), transition: t, index: None });
    assert_eq!(
        applied.touched,
        [ElementKey::Transition {
            machine: "Job".into(),
            from: "running.fetching".into(),
            to: "running.fetching".into(),
            trigger: "retry".into(),
            ordinal: 2
        }]
    );
}

#[test]
fn add_transition_rejections() {
    let def = parse(SHOP);
    let err = rejected(
        &def,
        EditOp::AddTransition {
            machine: "Order".into(),
            transition: transition(&["cart"], "limbo", "go"),
            index: None,
        },
    );
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::UnknownState { .. })), "{err:?}");
    let err = rejected(
        &def,
        EditOp::AddTransition {
            machine: "Order".into(),
            transition: transition(&["closed"], "cart", "reopen"),
            index: None,
        },
    );
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::TransitionFromFinal { .. })), "{err:?}");
    // Strict events: emitting an undeclared event is rejected.
    let err = rejected(
        &def,
        EditOp::AddTransition {
            machine: "Order".into(),
            transition: emitting(transition(&["cart"], "cancelled", "abandon"), &["CartAbandoned"]),
            index: None,
        },
    );
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::UndeclaredEvent { .. })), "{err:?}");
    assert_eq!(
        rejected(
            &def,
            EditOp::AddTransition {
                machine: "Order".into(),
                transition: transition(&["cart"], "cancelled", "not valid"),
                index: None
            }
        ),
        EditError::InvalidName("not valid".into())
    );
    assert_eq!(
        rejected(
            &def,
            EditOp::AddTransition {
                machine: "Order".into(),
                transition: transition(&["cart..x"], "cancelled", "go"),
                index: None
            }
        ),
        EditError::InvalidName("cart..x".into())
    );
    let err = rejected(
        &def,
        EditOp::AddTransition { machine: "Order".into(), transition: transition(&[], "cancelled", "go"), index: None },
    );
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::MissingKey { key, .. } if key == "from")), "{err:?}");
    assert_eq!(
        rejected(
            &def,
            EditOp::AddTransition {
                machine: "Order".into(),
                transition: transition(&["cart"], "cancelled", "go"),
                index: Some(99)
            }
        ),
        EditError::IndexOutOfRange { what: "transition", index: 99, len: 7 }
    );
}

#[test]
fn update_and_remove_transition() {
    let def = parse(SHOP);
    let mut t = machine(&def, "Payment").transitions[3].clone();
    t.guard = None;
    t.to = s("voided");
    let applied = ok(&def, EditOp::UpdateTransition { machine: "Payment".into(), index: 3, transition: t });
    assert_eq!(transitions(&applied.definition, "Payment")[3], "authorized -> voided @ capture");
    assert!(machine(&applied.definition, "Payment").transitions[3].guard.is_none());

    let applied = ok(&def, EditOp::RemoveTransition { machine: "Payment".into(), index: 0 });
    assert_eq!(transitions(&applied.definition, "Payment").len(), 6);
    assert_eq!(
        applied.touched,
        [ElementKey::Transition {
            machine: "Payment".into(),
            from: "created".into(),
            to: "authorizing".into(),
            trigger: "authorize".into(),
            ordinal: 0
        }]
    );

    assert_eq!(
        rejected(&def, EditOp::RemoveTransition { machine: "Payment".into(), index: 7 }),
        EditError::IndexOutOfRange { what: "transition", index: 7, len: 7 }
    );
    assert_eq!(
        rejected(
            &def,
            EditOp::UpdateTransition {
                machine: "Payment".into(),
                index: 7,
                transition: transition(&["created"], "failed", "x")
            }
        ),
        EditError::IndexOutOfRange { what: "transition", index: 7, len: 7 }
    );
    assert_eq!(
        rejected(&def, EditOp::RemoveTransition { machine: "Nope".into(), index: 0 }),
        not_found("machine", "Nope")
    );
}

// --- Events ----------------------------------------------------------------------

#[test]
fn declaring_the_first_event_declares_every_used_event() {
    let def = parse(ORDER_FULFILLMENT);
    let applied = ok(&def, EditOp::DeclareEvent { event: event_def("OrderPaid", &["orderId", "amount"]), index: None });
    assert_eq!(event_names(&applied.definition), ["OrderCancelled", "Shipped", "OrderPaid"]);
    let paid = &applied.definition.events[2];
    assert_eq!(paid.payload.iter().map(|p| p.value.as_str()).collect::<Vec<_>>(), ["orderId", "amount"]);
    assert!(applied.touched.contains(&ElementKey::Event { event: "OrderPaid".into() }));
    assert!(applied.touched.contains(&ElementKey::Event { event: "Shipped".into() }));

    let applied = ok(&def, EditOp::DeclareEvent { event: event_def("Brand-new", &[]), index: Some(0) });
    assert_eq!(event_names(&applied.definition), ["Brand-new", "OrderPaid", "OrderCancelled", "Shipped"]);
}

#[test]
fn declare_event_in_strict_mode() {
    let def = parse(SHOP);
    let applied = ok(&def, EditOp::DeclareEvent { event: event_def("Audit", &["orderId"]), index: Some(1) });
    assert_eq!(event_names(&applied.definition)[1], "Audit");
    assert_eq!(applied.definition.events.len(), def.events.len() + 1);
    assert_eq!(applied.inverse, EditOp::RemoveEventDeclaration { event: "Audit".into() });
    assert_eq!(
        rejected(&def, EditOp::DeclareEvent { event: event_def("OrderPaid", &[]), index: None }),
        EditError::NameTaken { what: "event", name: "OrderPaid".into() }
    );
    assert_eq!(
        rejected(&def, EditOp::DeclareEvent { event: event_def("Bad event", &[]), index: None }),
        EditError::InvalidName("Bad event".into())
    );
    assert_eq!(
        rejected(&def, EditOp::DeclareEvent { event: event_def("Audit", &[]), index: Some(99) }),
        EditError::IndexOutOfRange { what: "event", index: 99, len: 13 }
    );
}

#[test]
fn remove_event_declaration_is_rejected_while_referenced_in_strict_mode() {
    let def = parse(SHOP);
    let err = rejected(&def, EditOp::RemoveEventDeclaration { event: "OrderPaid".into() });
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::UndeclaredEvent { event } if event == "OrderPaid")));
    assert_eq!(
        rejected(&def, EditOp::RemoveEventDeclaration { event: "Nope".into() }),
        not_found("event declaration", "Nope")
    );

    // An unreferenced declaration can go.
    let declared = ok(&def, EditOp::DeclareEvent { event: event_def("Audit", &[]), index: None }).definition;
    let applied = ok(&declared, EditOp::RemoveEventDeclaration { event: "Audit".into() });
    assert_same(&applied.definition, &def, "declare then remove");

    // The last declaration can go even though it is referenced: the file
    // leaves strict mode.
    let one = parse(ONE_EVENT);
    let applied = ok(&one, EditOp::RemoveEventDeclaration { event: "Ping".into() });
    assert!(applied.definition.events.is_empty());
}

#[test]
fn rename_event_everywhere() {
    let def = parse(SHOP);
    let applied = ok(&def, EditOp::RenameEvent { from: "OrderPaid".into(), to: "OrderSettled".into() });
    let d = &applied.definition;
    assert!(event_names(d).contains(&"OrderSettled".to_owned()));
    assert!(!event_names(d).contains(&"OrderPaid".to_owned()));
    assert_eq!(machine(d, "Order").transitions[1].emits[0].value, "OrderSettled");
    assert_eq!(handler(d, "Fulfillment", "OrderSettled").rules.len(), 1);
    assert!(applied.touched.contains(&ElementKey::Event { event: "OrderSettled".into() }));
    assert!(
        applied
            .touched
            .contains(&ElementKey::Handler { controller: "Fulfillment".into(), event: "OrderSettled".into() })
    );

    // Undeclared events rename the same way.
    let order = parse(ORDER_FULFILLMENT);
    let applied = ok(&order, EditOp::RenameEvent { from: "OrderPaid".into(), to: "Paid".into() });
    assert_eq!(machine(&applied.definition, "Order").transitions[1].emits[0].value, "Paid");
    assert_eq!(controller(&applied.definition, "Fulfillment").on[0].event.value, "Paid");

    assert_eq!(
        rejected(&def, EditOp::RenameEvent { from: "OrderPaid".into(), to: "OrderPlaced".into() }),
        EditError::NameTaken { what: "event", name: "OrderPlaced".into() }
    );
    assert_eq!(rejected(&def, EditOp::RenameEvent { from: "Nope".into(), to: "X".into() }), not_found("event", "Nope"));
    assert_eq!(
        rejected(&def, EditOp::RenameEvent { from: "OrderPaid".into(), to: "a.b".into() }),
        EditError::InvalidName("a.b".into())
    );
}

// --- Controllers -----------------------------------------------------------------

#[test]
fn controller_lifecycle() {
    let def = parse(ORDER_FULFILLMENT);
    let audit = controller_def("Audit", vec![handler_def("Shipped", vec![rule("Order", "submit")])]);
    let applied = ok(&def, EditOp::AddController { controller: audit, index: Some(0) });
    assert_eq!(applied.definition.controllers[0].name.value, "Audit");
    assert!(applied.touched.contains(&ElementKey::Controller { controller: "Audit".into() }));
    assert!(applied.touched.contains(&ElementKey::Rule {
        controller: "Audit".into(),
        event: "Shipped".into(),
        ordinal: 0
    }));

    let applied = ok(&def, EditOp::RemoveController { controller: "Fulfillment".into() });
    assert!(applied.definition.controllers.is_empty());

    let applied = ok(&def, EditOp::RenameController { from: "Fulfillment".into(), to: "Logistics".into() });
    assert_eq!(applied.definition.controllers[0].name.value, "Logistics");
    assert!(applied.touched.contains(&ElementKey::Controller { controller: "Logistics".into() }));

    assert_eq!(
        rejected(&def, EditOp::AddController { controller: controller_def("Fulfillment", vec![]), index: None }),
        EditError::NameTaken { what: "controller", name: "Fulfillment".into() }
    );
    assert_eq!(rejected(&def, EditOp::RemoveController { controller: "Nope".into() }), not_found("controller", "Nope"));
    let err = rejected(
        &def,
        EditOp::AddController {
            controller: controller_def("Ghost", vec![handler_def("OrderPaid", vec![rule("Nobody", "go")])]),
            index: None,
        },
    );
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::UnknownMachine { .. })), "{err:?}");
}

#[test]
fn handlers_and_rules() {
    let def = parse(ORDER_FULFILLMENT);
    let applied = ok(
        &def,
        EditOp::AddHandler {
            controller: "Fulfillment".into(),
            handler: handler_def("OrderCancelled", vec![targeted(rule("Shipment", "handoff"), TargetMode::All)]),
            index: None,
        },
    );
    assert_eq!(handler(&applied.definition, "Fulfillment", "OrderCancelled").rules.len(), 1);
    assert_eq!(
        applied.inverse,
        EditOp::RemoveHandler { controller: "Fulfillment".into(), event: "OrderCancelled".into() }
    );

    let applied = ok(&def, EditOp::RemoveHandler { controller: "Fulfillment".into(), event: "OrderPaid".into() });
    assert!(controller(&applied.definition, "Fulfillment").on.is_empty());
    assert!(applied.touched.contains(&ElementKey::Rule {
        controller: "Fulfillment".into(),
        event: "OrderPaid".into(),
        ordinal: 0
    }));

    let applied = ok(
        &def,
        EditOp::AddRule {
            controller: "Fulfillment".into(),
            event: "OrderPaid".into(),
            rule: targeted(rule("Shipment", "handoff"), TargetMode::Spawn),
            index: Some(0),
        },
    );
    let rules = &handler(&applied.definition, "Fulfillment", "OrderPaid").rules;
    assert_eq!(rules[0].fire.value.trigger, "handoff");
    assert_eq!(rules[1].fire.value.trigger, "start");
    assert_eq!(
        applied.touched,
        [ElementKey::Rule { controller: "Fulfillment".into(), event: "OrderPaid".into(), ordinal: 0 }]
    );

    let applied = ok(
        &def,
        EditOp::UpdateRule {
            controller: "Fulfillment".into(),
            event: "OrderPaid".into(),
            index: 0,
            rule: rule("Shipment", "handoff"),
        },
    );
    assert!(handler(&applied.definition, "Fulfillment", "OrderPaid").rules[0].target.is_none());

    let applied =
        ok(&def, EditOp::RemoveRule { controller: "Fulfillment".into(), event: "OrderPaid".into(), index: 0 });
    assert!(handler(&applied.definition, "Fulfillment", "OrderPaid").rules.is_empty());
}

#[test]
fn handler_and_rule_rejections() {
    let def = parse(ORDER_FULFILLMENT);
    assert_eq!(
        rejected(
            &def,
            EditOp::AddHandler {
                controller: "Fulfillment".into(),
                handler: handler_def("OrderPaid", vec![]),
                index: None
            }
        ),
        EditError::NameTaken { what: "handler", name: "Fulfillment/OrderPaid".into() }
    );
    assert_eq!(
        rejected(&def, EditOp::RemoveHandler { controller: "Fulfillment".into(), event: "Shipped".into() }),
        not_found("handler", "Fulfillment/Shipped")
    );
    assert_eq!(
        rejected(
            &def,
            EditOp::AddRule {
                controller: "Fulfillment".into(),
                event: "Shipped".into(),
                rule: rule("Order", "submit"),
                index: None
            }
        ),
        not_found("handler", "Fulfillment/Shipped")
    );
    assert_eq!(
        rejected(&def, EditOp::RemoveRule { controller: "Fulfillment".into(), event: "OrderPaid".into(), index: 1 }),
        EditError::IndexOutOfRange { what: "rule", index: 1, len: 1 }
    );
    // A target selector must name the fired machine.
    let mut mismatched = targeted(rule("Shipment", "start"), TargetMode::One);
    if let Some(target) = mismatched.target.as_mut() {
        target.value.machine = "Order".into();
    }
    let err = rejected(
        &def,
        EditOp::UpdateRule { controller: "Fulfillment".into(), event: "OrderPaid".into(), index: 0, rule: mismatched },
    );
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::TargetMachineMismatch { .. })), "{err:?}");
    // Strict events: subscribing to an undeclared event is rejected.
    let shop = parse(SHOP);
    let err = rejected(
        &shop,
        EditOp::AddHandler { controller: "Billing".into(), handler: handler_def("Mystery", vec![]), index: None },
    );
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::UndeclaredEvent { .. })), "{err:?}");
}

// --- External sources ------------------------------------------------------------

#[test]
fn external_sources() {
    let def = parse(ORDER_FULFILLMENT);
    let applied =
        ok(&def, EditOp::AddExternal { external: external_def("Warehouse", &[("Shipment", "handoff")]), index: None });
    assert_eq!(external_triggers(&applied.definition, "Warehouse"), ["Shipment.handoff"]);
    assert_eq!(applied.touched, [ElementKey::External { source: "Warehouse".into() }]);

    let applied = ok(&def, EditOp::RemoveExternal { external: "Clock".into() });
    assert_eq!(applied.definition.external.len(), 2);
    assert_eq!(applied.inverse, EditOp::AddExternal { external: def.external[2].clone(), index: Some(2) });

    let applied = ok(&def, EditOp::RenameExternal { from: "Clock".into(), to: "Timer".into() });
    assert_eq!(external_triggers(&applied.definition, "Timer"), ["Order.timeout"]);

    let applied = ok(
        &def,
        EditOp::SetExternalTriggers {
            external: "Customer".into(),
            triggers: vec![trigger_ref("Order", "submit"), trigger_ref("Order", "timeout")],
        },
    );
    assert_eq!(external_triggers(&applied.definition, "Customer"), ["Order.submit", "Order.timeout"]);

    assert_eq!(
        rejected(&def, EditOp::RenameExternal { from: "Clock".into(), to: "Customer".into() }),
        EditError::NameTaken { what: "external source", name: "Customer".into() }
    );
    assert_eq!(
        rejected(&def, EditOp::RemoveExternal { external: "Nope".into() }),
        not_found("external source", "Nope")
    );
    assert_eq!(
        rejected(
            &def,
            EditOp::SetExternalTriggers { external: "Customer".into(), triggers: vec![trigger_ref("Order", "no way")] }
        ),
        EditError::InvalidName("no way".into())
    );
    let err = rejected(
        &def,
        EditOp::SetExternalTriggers { external: "Customer".into(), triggers: vec![trigger_ref("Nobody", "go")] },
    );
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::UnknownMachine { .. })), "{err:?}");
}

#[test]
fn system_name() {
    let def = parse(ORDER_FULFILLMENT);
    let applied = ok(&def, EditOp::SetSystemName { name: Some("Orders".into()) });
    assert_eq!(applied.definition.system.as_ref().map(|s| s.value.as_str()), Some("Orders"));
    assert!(applied.touched.is_empty());
    let shop = parse(SHOP);
    let applied = ok(&shop, EditOp::SetSystemName { name: None });
    assert!(applied.definition.system.is_none());
}

// --- Batches -----------------------------------------------------------------------

#[test]
fn batch_applies_in_order_and_undoes_in_reverse() {
    let def = parse(ORDER_FULFILLMENT);
    let ops = vec![
        EditOp::AddMachine {
            machine: machine_def("Billing", &["open", "closed"], vec![transition(&["open"], "closed", "close")]),
            index: None,
        },
        EditOp::AddRule {
            controller: "Fulfillment".into(),
            event: "OrderPaid".into(),
            rule: rule("Billing", "close"),
            index: None,
        },
        EditOp::RenameMachine { from: "Billing".into(), to: "Invoice".into() },
    ];
    let applied = ok(&def, EditOp::Batch(ops));
    let rules = &handler(&applied.definition, "Fulfillment", "OrderPaid").rules;
    assert_eq!(rules[1].fire.value.machine, "Invoice");
    let EditOp::Batch(inverses) = &applied.inverse else { panic!("batch inverse should be a batch") };
    assert_eq!(inverses[0], EditOp::RenameMachine { from: "Invoice".into(), to: "Billing".into() });
    assert_eq!(inverses[2], EditOp::RemoveMachine { machine: "Billing".into() });
    assert!(applied.touched.contains(&machine_key("Billing")));
    assert!(applied.touched.contains(&machine_key("Invoice")));
}

#[test]
fn batch_is_atomic() {
    let def = parse(ORDER_FULFILLMENT);
    let before = def.clone();
    let err = rejected(
        &def,
        EditOp::Batch(vec![
            EditOp::RenameMachine { from: "Order".into(), to: "Purchase".into() },
            EditOp::RemoveMachine { machine: "Order".into() },
        ]),
    );
    assert_eq!(err, not_found("machine", "Order"));
    assert_eq!(def, before);

    // Each step may be fine on its own but the end result invalid.
    let err = rejected(
        &def,
        EditOp::Batch(vec![
            EditOp::SetSystemName { name: Some("X".into()) },
            EditOp::RemoveState { machine: "Order".into(), path: "draft".into() },
            EditOp::SetMachineInitial { machine: "Order".into(), initial: Some("draft".into()) },
        ]),
    );
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::UnknownState { .. })), "{err:?}");
}

#[test]
fn batch_intermediate_states_need_not_resolve() {
    // Swap two machine names through a temporary.
    let def = parse(ORDER_FULFILLMENT);
    let applied = ok(
        &def,
        EditOp::Batch(vec![
            EditOp::RenameMachine { from: "Order".into(), to: "Tmp".into() },
            EditOp::RenameMachine { from: "Shipment".into(), to: "Order".into() },
            EditOp::RenameMachine { from: "Tmp".into(), to: "Shipment".into() },
        ]),
    );
    assert_eq!(handler(&applied.definition, "Fulfillment", "OrderPaid").rules[0].fire.value.machine, "Order");

    // Removing both declarations of a strict file: the first step alone is
    // invalid (Ping is still emitted), the end result is not.
    let two = parse(TWO_EVENTS);
    let applied = ok(
        &two,
        EditOp::Batch(vec![
            EditOp::RemoveEventDeclaration { event: "Ping".into() },
            EditOp::RemoveEventDeclaration { event: "Pong".into() },
        ]),
    );
    assert!(applied.definition.events.is_empty());
    let err = rejected(&two, EditOp::RemoveEventDeclaration { event: "Ping".into() });
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::UndeclaredEvent { .. })), "{err:?}");
}

#[test]
fn empty_batch_is_a_no_op() {
    let def = parse(SHOP);
    let applied = ok(&def, EditOp::Batch(Vec::new()));
    assert_eq!(applied.definition, def);
    assert_eq!(applied.inverse, EditOp::Batch(Vec::new()));
    assert!(applied.touched.is_empty());
}

#[test]
fn an_invalid_definition_accepts_only_edits_that_fix_it() {
    // A definition that does not resolve stays uneditable until an edit
    // makes it resolve.
    let mut def = parse(ORDER_FULFILLMENT);
    def.machines[0].initial = Some(s("limbo"));
    let err = rejected(&def, EditOp::SetSystemName { name: Some("X".into()) });
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::UnknownState { .. })), "{err:?}");
    let fixed = apply(&def, &EditOp::SetMachineInitial { machine: "Order".into(), initial: None });
    assert!(fixed.is_ok(), "{fixed:?}");
}

// --- locate_transition -----------------------------------------------------------

#[test]
fn locate_transition_handles_lists_and_ordinals() {
    let def = parse(NESTED);
    let key = |from: &str, to: &str, trigger: &str, ordinal: u32| ElementKey::Transition {
        machine: "Job".into(),
        from: from.into(),
        to: to.into(),
        trigger: trigger.into(),
        ordinal,
    };
    assert_eq!(locate_transition(&def, &key("queued", "running", "start", 0)), Some(("Job".into(), 0)));
    assert_eq!(locate_transition(&def, &key("running.computing", "done", "finish", 0)), Some(("Job".into(), 3)));
    assert_eq!(locate_transition(&def, &key("paused.waiting", "done", "finish", 0)), Some(("Job".into(), 3)));
    assert_eq!(
        locate_transition(&def, &key("running.fetching", "running.fetching", "retry", 0)),
        Some(("Job".into(), 5))
    );
    assert_eq!(
        locate_transition(&def, &key("running.fetching", "running.fetching", "retry", 1)),
        Some(("Job".into(), 6))
    );
    assert_eq!(locate_transition(&def, &key("running.fetching", "running.fetching", "retry", 2)), None);
    assert_eq!(locate_transition(&def, &key("failed", "running.hist", "resume", 0)), Some(("Job".into(), 4)));
    assert_eq!(locate_transition(&def, &key("queued", "done", "start", 0)), None);
    assert_eq!(locate_transition(&def, &machine_key("Job")), None);
    let other = ElementKey::Transition {
        machine: "Nope".into(),
        from: "queued".into(),
        to: "running".into(),
        trigger: "start".into(),
        ordinal: 0,
    };
    assert_eq!(locate_transition(&def, &other), None);
}

#[test]
fn locate_transition_agrees_with_the_model_on_every_example() {
    for (name, def) in fixtures() {
        let model = resolve(def.clone()).expect("fixture resolves");
        for (id, t) in model.transitions() {
            let key = model.key_of(cascade_core::ElementRef::Transition(id));
            let located = locate_transition(&def, &key);
            let machine_name = model.machine(t.machine).name.clone();
            let entry = located.as_ref().map(|(m, i)| {
                (m.clone(), &def.machines.iter().find(|d| d.name.value == *m).expect("machine").transitions[*i])
            });
            assert!(
                matches!(&entry, Some((m, entry)) if *m == machine_name && entry.span == t.span),
                "{name}: {key} located at {located:?}"
            );
        }
    }
}

#[test]
fn new_elements_have_unknown_spans_after_synthesis() {
    // Auto-declared events are synthesized by the edit.
    let def = parse(ORDER_FULFILLMENT);
    let applied = ok(&def, EditOp::DeclareEvent { event: event_def("OrderPaid", &[]), index: None });
    for event in &applied.definition.events {
        assert!(!event.span.is_known(), "{}", event.name.value);
    }
}

/// How the app changes a payload: remove the declaration and re-declare it
/// at the same index. In strict mode the step in between does not resolve.
fn change_payload(def: &cascade_core::Definition, event: &str, payload: &[&str]) -> cascade_core::edit::Applied {
    let index = def.events.iter().position(|e| e.name.value == event).expect("declared");
    if def.events.len() > 1 {
        // On its own, removing a used declaration of a strict file is rejected.
        let err = rejected(def, EditOp::RemoveEventDeclaration { event: event.into() });
        assert!(invalid(&err, |k| matches!(k, DiagnosticKind::UndeclaredEvent { .. })), "{err:?}");
    }
    ok(
        def,
        EditOp::Batch(vec![
            EditOp::RemoveEventDeclaration { event: event.into() },
            EditOp::DeclareEvent { event: event_def(event, payload), index: Some(index) },
        ]),
    )
}

#[test]
fn batch_changes_an_event_payload_in_strict_mode() {
    let def = parse(SHOP);
    let applied = change_payload(&def, "OrderPaid", &["orderId", "amount"]);
    assert_eq!(event_names(&applied.definition), event_names(&def));
    let paid = &applied.definition.events[1];
    assert_eq!(paid.name.value, "OrderPaid");
    assert_eq!(paid.payload.iter().map(|p| p.value.as_str()).collect::<Vec<_>>(), ["orderId", "amount"]);
    assert!(applied.touched.contains(&ElementKey::Event { event: "OrderPaid".into() }));
    // The inverse restores the old payload at the same place (also checked by `ok`).
    let undone = apply(&applied.definition, &applied.inverse).expect("undo applies");
    assert_eq!(undone.definition.events[1].payload.iter().map(|p| p.value.as_str()).collect::<Vec<_>>(), ["orderId"]);

    // The only declaration: the list is empty in between (lenient), and the
    // re-declaration makes it strict again with just this event.
    let one = parse(ONE_EVENT);
    let applied = change_payload(&one, "Ping", &[]);
    assert_eq!(event_names(&applied.definition), ["Ping"]);
    assert!(applied.definition.events[0].payload.is_empty());
}
