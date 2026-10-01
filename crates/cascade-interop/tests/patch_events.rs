//! `patch_text` for event declarations, controllers, handlers, rules and
//! external sources.

mod patch_common;

use cascade_core::Spanned;
use cascade_core::edit::{EditError, EditOp};
use cascade_interop::PatchError;
use patch_common::*;

// --- Events ------------------------------------------------------------------------

#[test]
fn declarations_are_one_line_entries() {
    let text = shop();
    let e = event("OrderRefunded", &["orderId"]);
    let out = check(
        &text,
        &EditOp::DeclareEvent { event: e.clone(), index: None },
        "@@ 42\n+  OrderRefunded: { payload: [orderId] }",
        |d| d.events.push(e),
    );
    assert_eq!(patched(&out, &EditOp::RemoveEventDeclaration { event: "OrderRefunded".into() }), text);
    let e = event("OrderRefunded", &[]);
    check(&text, &EditOp::DeclareEvent { event: e.clone(), index: Some(0) }, "@@ 25\n+  OrderRefunded: {}", |d| {
        d.events.insert(0, e)
    });
    let block = fixture("block.yaml");
    check(&block, &EditOp::DeclareEvent { event: event("Closed", &[]), index: None }, "@@ 25\n+    - Closed", |d| {
        d.events.push(event("Closed", &[]))
    });
    let flow = fixture("flow.yaml");
    check(
        &flow,
        &EditOp::DeclareEvent { event: event("Started", &[]), index: None },
        "@@ 5\n-events: { Stopped: {} }\n+events: { Stopped: {}, Started: {} }",
        |d| d.events.push(event("Started", &[])),
    );
}

#[test]
fn the_first_declaration_declares_every_event_in_use() {
    let text = order_fulfillment();
    let out = check(
        &text,
        &EditOp::DeclareEvent { event: event("Audit", &[]), index: Some(1) },
        "@@ 21\n+events:\n+  OrderPaid: {}\n+  Audit: {}\n+  OrderCancelled: {}\n+  Shipped: {}\n+",
        |d| {
            d.events =
                vec![event("OrderPaid", &[]), event("Audit", &[]), event("OrderCancelled", &[]), event("Shipped", &[])]
        },
    );
    assert!(out.contains("emits: [Shipped] }\n\nevents:\n  OrderPaid: {}\n"), "{out}");
    check(
        &text,
        &EditOp::DeclareEvent { event: event("OrderPaid", &["orderId"]), index: None },
        "@@ 21\n+events:\n+  OrderCancelled: {}\n+  Shipped: {}\n+  OrderPaid: { payload: [orderId] }\n+",
        |d| d.events = vec![event("OrderCancelled", &[]), event("Shipped", &[]), event("OrderPaid", &["orderId"])],
    );
}

#[test]
fn removing_the_last_declaration_removes_the_section() {
    let block = fixture("block.yaml");
    let out = check(
        &block,
        &EditOp::RemoveEventDeclaration { event: "Opened".into() },
        "@@ 23\n-events:\n-    - Opened\n-",
        |d| d.events.clear(),
    );
    assert_comments_kept(&block, &out, &[]);
    let flow = fixture("flow.yaml");
    check(&flow, &EditOp::RemoveEventDeclaration { event: "Stopped".into() }, "@@ 5\n-events: { Stopped: {} }", |d| {
        d.events.clear()
    });
}

#[test]
fn removing_a_declaration_still_in_use_is_invalid() {
    let text = shop();
    let err = try_patch(&text, &EditOp::RemoveEventDeclaration { event: "ReturnRequested".into() });
    assert!(matches!(err, Err(PatchError::Edit(EditError::Invalid(_)))), "{err:?}");
}

#[test]
fn a_payload_change_batch_is_patched_in_place() {
    let text = shop();
    let op = EditOp::Batch(vec![
        EditOp::RemoveEventDeclaration { event: "ShipmentLost".into() },
        EditOp::DeclareEvent { event: event("ShipmentLost", &["orderId", "carrier"]), index: Some(11) },
    ]);
    let out = check(
        &text,
        &op,
        "@@ 38\n-  ShipmentLost: { payload: [orderId] }\n+  ShipmentLost: { payload: [orderId, carrier] }",
        |d| d.events[11] = event("ShipmentLost", &["orderId", "carrier"]),
    );
    assert_comments_kept(&text, &out, &[]);
    let op = EditOp::Batch(vec![
        EditOp::RemoveEventDeclaration { event: "ReturnRequested".into() },
        EditOp::DeclareEvent { event: event("ReturnRequested", &[]), index: None },
    ]);
    check(&text, &op, "@@ 41\n-  ReturnRequested: { payload: [orderId] }\n+  ReturnRequested: {}", |d| {
        d.events[12] = event("ReturnRequested", &[])
    });
    // Moving the declaration is a real remove and re-declare.
    let op = EditOp::Batch(vec![
        EditOp::RemoveEventDeclaration { event: "OrderPaid".into() },
        EditOp::DeclareEvent { event: event("OrderPaid", &[]), index: Some(0) },
    ]);
    check(&text, &op, "@@ 25\n+  OrderPaid: {}\n@@ 26\n-  OrderPaid: { payload: [orderId] }", |d| {
        d.events.remove(1);
        d.events.insert(0, event("OrderPaid", &[]));
    });
}

#[test]
fn renaming_an_event_renames_declaration_emits_and_subscriptions() {
    let text = shop();
    let out = check(
        &text,
        &EditOp::RenameEvent { from: "OrderPaid".into(), to: "OrderSettled".into() },
        "@@ 26\n\
         -  OrderPaid: { payload: [orderId] }\n\
         +  OrderSettled: { payload: [orderId] }\n\
         @@ 62\n\
         -      - { from: placed.awaiting_payment, to: placed.paid, on: payment_captured, emits: [OrderPaid] }\n\
         +      - { from: placed.awaiting_payment, to: placed.paid, on: payment_captured, emits: [OrderSettled] }\n\
         @@ 198\n\
         -      OrderPaid:\n\
         +      OrderSettled:",
        |d| {
            d.events[1].name = s("OrderSettled");
            machine_mut(d, "Order").transitions[1].emits[0] = s("OrderSettled");
            handler_mut(d, "Fulfillment", "OrderPaid").event = s("OrderSettled");
        },
    );
    assert_eq!(patched(&out, &EditOp::RenameEvent { from: "OrderSettled".into(), to: "OrderPaid".into() }), text);
    let nested = fixture("nested.yaml");
    check(
        &nested,
        &EditOp::RenameEvent { from: "Finished".into(), to: "Done".into() },
        "@@ 25\n\
         -      - { from: [running.computing, queued], to: done, on: finish, emits: [Finished] }\n\
         +      - { from: [running.computing, queued], to: done, on: finish, emits: [Done] }\n\
         @@ 42\n\
         -      Finished: { fire: Worker.release, when: \"worker still busy\" }\n\
         +      Done: { fire: Worker.release, when: \"worker still busy\" }",
        |d| {
            machine_mut(d, "Job").transitions[4].emits[0] = s("Done");
            handler_mut(d, "Scheduler", "Finished").event = s("Done");
        },
    );
}

// --- Controllers ---------------------------------------------------------------------

#[test]
fn a_new_controller_is_a_block_like_its_siblings() {
    let text = order_fulfillment();
    let c = controller("Audit", vec![handler("OrderCancelled", vec![rule("Shipment.start", None, Some("never"))])]);
    let out = check(
        &text,
        &EditOp::AddController { controller: c.clone(), index: None },
        "@@ 28\n+  Audit:\n+    on:\n+      OrderCancelled:\n+        - fire: Shipment.start\n+          when: never\n+",
        |d| d.controllers.push(c),
    );
    assert!(out.contains("event.orderId\n\n  Audit:\n    on:\n"), "{out}");
    assert_eq!(patched(&out, &EditOp::RemoveController { controller: "Audit".into() }), text);
    let block = fixture("block.yaml");
    check(
        &block,
        &EditOp::AddController { controller: controller("Log", vec![]), index: None },
        "@@ 34\n+    Log:\n+        on: {}\n+",
        |d| d.controllers.push(controller("Log", vec![])),
    );
}

#[test]
fn a_missing_or_empty_controllers_section_is_created_in_canonical_order() {
    let text = "machines:\n  A:\n    states: [x, y]\n    transitions:\n      - { from: x, to: y, on: go, emits: [Went] }\n\nexternal:\n  U: [A.go]\n";
    let c = controller("C", vec![handler("Went", vec![rule("A.go", None, None)])]);
    let out = check(
        text,
        &EditOp::AddController { controller: c.clone(), index: None },
        "@@ 7\n+controllers:\n+  C:\n+    on:\n+      Went:\n+        - fire: A.go\n+",
        |d| d.controllers.push(c.clone()),
    );
    assert_eq!(patched(&out, &EditOp::RemoveController { controller: "C".into() }), text);
    let flow = fixture("flow.yaml");
    let sync = controller("Sync", vec![handler("Stopped", vec![rule("Light.go", None, None)])]);
    check(
        &flow,
        &EditOp::AddController { controller: sync.clone(), index: None },
        "@@ 6\n-controllers: {}\n+controllers:\n+  Sync:\n+    on:\n+      Stopped:\n+        - fire: Light.go",
        |d| d.controllers.push(sync),
    );
}

#[test]
fn removing_a_controller_takes_its_comments() {
    let text = shop();
    let out = check(
        &text,
        &EditOp::RemoveController { controller: "FraudCheck".into() },
        "@@ 162\n\
         -  # PLANTED race-candidate: FraudCheck and Billing both react to\n\
         -  # PaymentAuthorized by firing into the same Payment, so whether it ends up\n\
         -  # captured or voided depends on which rule runs first.\n\
         -  FraudCheck:\n\
         -    on:\n\
         -      PaymentAuthorized:\n\
         -        - fire: Payment.void\n\
         -          target: Payment where orderId == event.orderId\n\
         -          when: \"risk score above threshold\"\n\
         -",
        |d| {
            d.controllers.remove(2);
        },
    );
    assert_comments_kept(&text, &out, &["PLANTED race-candidate", "PaymentAuthorized by firing", "captured or voided"]);
    let of = order_fulfillment();
    check(
        &of,
        &EditOp::RemoveController { controller: "Fulfillment".into() },
        "@@ 21\n-controllers:\n-  Fulfillment:\n-    on:\n-      OrderPaid:\n-        - fire: Shipment.start\n-          target: Shipment where orderId == event.orderId\n-",
        |d| d.controllers.clear(),
    );
}

#[test]
fn renaming_a_controller_changes_only_its_key() {
    let text = shop();
    check(
        &text,
        &EditOp::RenameController { from: "Billing".into(), to: "Payments".into() },
        "@@ 156\n-  Billing:\n+  Payments:",
        |d| controller_mut(d, "Billing").name = s("Payments"),
    );
}

#[test]
fn handlers_are_added_and_removed_with_their_rules() {
    let text = shop();
    let h = handler("PaymentRefunded", vec![rule("Order.cancel", Some("Order where orderId == event.orderId"), None)]);
    let out = check(
        &text,
        &EditOp::AddHandler { controller: "Billing".into(), handler: h.clone(), index: None },
        "@@ 161\n+      PaymentRefunded:\n+        - fire: Order.cancel\n+          target: Order where orderId == event.orderId",
        |d| controller_mut(d, "Billing").on.push(h),
    );
    assert_eq!(
        patched(&out, &EditOp::RemoveHandler { controller: "Billing".into(), event: "PaymentRefunded".into() }),
        text
    );
    check(
        &text,
        &EditOp::AddHandler { controller: "Orders".into(), handler: handler("ShipmentLost", vec![]), index: Some(0) },
        "@@ 174\n+      ShipmentLost: []",
        |d| controller_mut(d, "Orders").on.insert(0, handler("ShipmentLost", vec![])),
    );
    let out = check(
        &text,
        &EditOp::RemoveHandler { controller: "Orders".into(), event: "PaymentRefunded".into() },
        "@@ 183\n\
         -      # PLANTED cascade-cycle (with Refunds below): cancelling an order\n\
         -      # refunds its payment, and a refunded payment cancels its order. Nobody\n\
         -      # marked the loop bounded, and nothing but the target's state stops it.\n\
         -      PaymentRefunded:\n\
         -        - fire: Order.cancel\n\
         -          target: Order where orderId == event.orderId",
        |d| {
            controller_mut(d, "Orders").on.pop();
        },
    );
    assert_comments_kept(&text, &out, &["PLANTED cascade-cycle", "refunds its payment", "marked the loop bounded"]);
    check(
        &text,
        &EditOp::RemoveHandler { controller: "Refunds".into(), event: "OrderCancelled".into() },
        "@@ 191\n\
         -    on:\n\
         -      OrderCancelled:\n\
         -        - fire: Payment.refund\n\
         -          target: Payment where orderId == event.orderId\n\
         +    on: {}",
        |d| controller_mut(d, "Refunds").on.clear(),
    );
}

#[test]
fn rules_follow_their_neighbours() {
    let text = shop();
    let r = rule("Inventory.reserve", Some("new Inventory with orderId = event.orderId"), Some("in stock"));
    let out = check(
        &text,
        &EditOp::AddRule {
            controller: "Billing".into(),
            event: "PaymentAuthorized".into(),
            rule: r.clone(),
            index: None,
        },
        "@@ 161\n\
         +        - fire: Inventory.reserve\n\
         +          target: new Inventory with orderId = event.orderId\n\
         +          when: \"in stock\"",
        |d| handler_mut(d, "Billing", "PaymentAuthorized").rules.push(r),
    );
    assert_eq!(
        patched(
            &out,
            &EditOp::RemoveRule { controller: "Billing".into(), event: "PaymentAuthorized".into(), index: 1 }
        ),
        text
    );
    let block = fixture("block.yaml");
    let r = rule("Door.open_door", None, Some("never"));
    check(
        &block,
        &EditOp::AddRule { controller: "Alarm".into(), event: "Opened".into(), rule: r.clone(), index: None },
        "@@ 33\n+                - fire: Door.open_door\n+                  when: never",
        |d| handler_mut(d, "Alarm", "Opened").rules.push(r),
    );
}

#[test]
fn a_single_rule_mapping_becomes_a_list_when_a_rule_is_added() {
    let text = fixture("nested.yaml");
    let r = rule("Worker.claim", None, None);
    check(
        &text,
        &EditOp::AddRule { controller: "Scheduler".into(), event: "Finished".into(), rule: r.clone(), index: None },
        "@@ 42\n\
         -      Finished: { fire: Worker.release, when: \"worker still busy\" }\n\
         +      Finished:\n\
         +        - { fire: Worker.release, when: \"worker still busy\" }\n\
         +        - { fire: Worker.claim }",
        |d| handler_mut(d, "Scheduler", "Finished").rules.push(r),
    );
}

#[test]
fn rule_updates_touch_only_changed_fields() {
    let text = shop();
    let mut r = rule("Payment.void", Some("all Payment where orderId == event.orderId"), None);
    r.bounded = true;
    let out = check(
        &text,
        &EditOp::UpdateRule {
            controller: "FraudCheck".into(),
            event: "PaymentAuthorized".into(),
            index: 0,
            rule: r.clone(),
        },
        "@@ 169\n\
         -          target: Payment where orderId == event.orderId\n\
         -          when: \"risk score above threshold\"\n\
         +          target: all Payment where orderId == event.orderId\n\
         +          bounded: true",
        |d| handler_mut(d, "FraudCheck", "PaymentAuthorized").rules[0] = r,
    );
    assert_comments_kept(&text, &out, &[]);
    let nested = fixture("nested.yaml");
    let r = rule("Worker.claim", None, Some("always"));
    check(
        &nested,
        &EditOp::UpdateRule { controller: "Scheduler".into(), event: "Finished".into(), index: 0, rule: r.clone() },
        "@@ 42\n\
         -      Finished: { fire: Worker.release, when: \"worker still busy\" }\n\
         +      Finished: { fire: Worker.claim, when: \"always\" }",
        |d| handler_mut(d, "Scheduler", "Finished").rules[0] = r,
    );
}

#[test]
fn removing_the_last_rule_leaves_an_empty_list() {
    let text = shop();
    check(
        &text,
        &EditOp::RemoveRule { controller: "Tracking".into(), event: "TrackingPolled".into(), index: 0 },
        "@@ 221\n\
         -      TrackingPolled:\n\
         -        - fire: Shipment.poll_tracking\n\
         -          target: Shipment where orderId == event.orderId\n\
         -          when: \"parcel not yet delivered\"\n\
         -          bounded: true\n\
         +      TrackingPolled: []",
        |d| handler_mut(d, "Tracking", "TrackingPolled").rules.clear(),
    );
    let nested = fixture("nested.yaml");
    check(
        &nested,
        &EditOp::RemoveRule { controller: "Scheduler".into(), event: "Finished".into(), index: 0 },
        "@@ 42\n-      Finished: { fire: Worker.release, when: \"worker still busy\" }\n+      Finished: []",
        |d| handler_mut(d, "Scheduler", "Finished").rules.clear(),
    );
}

// --- External sources ----------------------------------------------------------------------

#[test]
fn external_sources_are_one_line_entries() {
    let text = shop();
    let x = external("Admin", &["Order.cancel"]);
    let out = check(
        &text,
        &EditOp::AddExternal { external: x.clone(), index: None },
        "@@ 248\n+  Admin: [Order.cancel]",
        |d| d.external.push(x.clone()),
    );
    assert_eq!(patched(&out, &EditOp::RemoveExternal { external: "Admin".into() }), text);
    check(
        &text,
        &EditOp::AddExternal { external: x.clone(), index: Some(0) },
        "@@ 240\n+  Admin: [Order.cancel]",
        |d| d.external.insert(0, x.clone()),
    );
    let flow = fixture("flow.yaml");
    let y = external("Admin", &["Light.stop"]);
    check(
        &flow,
        &EditOp::AddExternal { external: y.clone(), index: None },
        "@@ 7\n\
         -external: { Timer: [Light.go, Light.slow, Light.stop] }\n\
         +external: { Timer: [Light.go, Light.slow, Light.stop], Admin: [Light.stop] }",
        |d| d.external.push(y),
    );
    let block = fixture("block.yaml");
    let z = external("Admin", &["Door.lock"]);
    check(&block, &EditOp::AddExternal { external: z.clone(), index: None }, "@@ 37\n+    Admin: [Door.lock]", |d| {
        d.external.push(z)
    });
}

#[test]
fn removing_and_renaming_external_sources() {
    let text = shop();
    let out = check(
        &text,
        &EditOp::RemoveExternal { external: "Customer".into() },
        "@@ 240\n\
         -  # PLANTED invalid-fire (a dead command): customers can request a return,\n\
         -  # but no Order transition accepts `request_return`.\n\
         -  Customer: [Order.checkout, Order.cancel, Order.request_return]",
        |d| {
            d.external.remove(0);
        },
    );
    assert_comments_kept(&text, &out, &["PLANTED invalid-fire (a dead command)", "no Order transition accepts"]);
    check(
        &text,
        &EditOp::RenameExternal { from: "Clock".into(), to: "Timer".into() },
        "@@ 247\n-  Clock: [Order.return_window_expired]\n+  Timer: [Order.return_window_expired]",
        |d| external_mut(d, "Clock").name = s("Timer"),
    );
}

#[test]
fn trigger_lists_change_item_by_item_in_any_form() {
    let text = shop();
    check(
        &text,
        &EditOp::SetExternalTriggers {
            external: "Customer".into(),
            triggers: vec![trigger("Order.checkout"), trigger("Order.cancel")],
        },
        "@@ 242\n\
         -  Customer: [Order.checkout, Order.cancel, Order.request_return]\n\
         +  Customer: [Order.checkout, Order.cancel]",
        |d| {
            external_mut(d, "Customer").triggers.pop();
        },
    );
    let nested = fixture("nested.yaml");
    check(
        &nested,
        &EditOp::SetExternalTriggers {
            external: "Queue".into(),
            triggers: vec![trigger("Worker.claim"), trigger("Worker.release")],
        },
        "@@ 46\n-  Queue: Worker.claim\n+  Queue: [Worker.claim, Worker.release]",
        |d| external_mut(d, "Queue").triggers.push(Spanned::synthetic(trigger("Worker.release"))),
    );
    let block = fixture("block.yaml");
    check(
        &block,
        &EditOp::SetExternalTriggers {
            external: "Resident".into(),
            triggers: vec![trigger("Door.open_door"), trigger("Door.lock")],
        },
        "@@ 37\n+        - Door.lock",
        |d| external_mut(d, "Resident").triggers.push(Spanned::synthetic(trigger("Door.lock"))),
    );
}
