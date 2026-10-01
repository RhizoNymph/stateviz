//! `patch_text` for machine ops and the system name.

mod patch_common;

use cascade_core::edit::{EditError, EditOp};
use cascade_core::{PaletteColor, Spanned};
use cascade_interop::PatchError;
use patch_common::*;

fn refund_machine() -> cascade_core::definition::MachineDef {
    let mut m = machine(
        "Refund",
        vec![state("requested"), state("approved"), final_state("paid")],
        vec![
            transition(&["requested"], "approved", "approve"),
            emitting(transition(&["approved"], "paid", "pay"), &["OrderPaid"]),
        ],
    );
    m.color = Some(Spanned::synthetic(PaletteColor::Yellow));
    m.fields = vec![s("orderId")];
    m
}

const REFUND_LINES: &str = "\
+  Refund:
+    color: yellow
+    fields: [orderId]
+    states:
+      - requested
+      - approved
+      - paid: { kind: final }
+    transitions:
+      - { from: requested, to: approved, on: approve }
+      - { from: approved,  to: paid,     on: pay, emits: [OrderPaid] }";

#[test]
fn a_new_machine_is_appended_with_a_blank_line_like_its_siblings() {
    let text = shop();
    let m = refund_machine();
    let out = check(
        &text,
        &EditOp::AddMachine { machine: m.clone(), index: None },
        &format!("@@ 149\n{REFUND_LINES}\n+"),
        |d| d.machines.push(m),
    );
    assert!(out.contains("      - { from: sending, to: bounced, on: rejected }\n\n  Refund:\n"), "{out}");
    assert!(out.contains("emits: [OrderPaid] }\n\ncontrollers:\n"), "{out}");
    assert_comments_kept(&text, &out, &[]);
}

#[test]
fn a_new_first_machine_goes_before_the_first_one() {
    let text = order_fulfillment();
    let m = refund_machine();
    let out = check(
        &text,
        &EditOp::AddMachine { machine: m.clone(), index: Some(0) },
        &format!("@@ 4\n{REFUND_LINES}\n+"),
        |d| d.machines.insert(0, m),
    );
    assert!(out.starts_with("# The example from docs/spec.md: an order triggers a shipment through the\n# Fulfillment controller.\nmachines:\n  Refund:\n"));
}

#[test]
fn a_new_machine_follows_the_file_indentation() {
    let text = fixture("block.yaml");
    let m = machine("Bell", vec![state("quiet"), state("ringing")], vec![]);
    let out = check(
        &text,
        &EditOp::AddMachine { machine: m.clone(), index: None },
        "@@ 23\n+    Bell:\n+        states: [quiet, ringing]\n+",
        |d| d.machines.push(m),
    );
    assert!(
        out.contains("              guard: \"has key\"\n\n    Bell:\n        states: [quiet, ringing]\n\nevents:\n"),
        "{out}"
    );
}

#[test]
fn removing_a_machine_removes_rules_firing_into_it_and_its_external_triggers() {
    let text = order_fulfillment();
    check(
        &text,
        &EditOp::RemoveMachine { machine: "Shipment".into() },
        "@@ 13\n\
         -  Shipment:\n\
         -    color: green\n\
         -    initial: idle\n\
         -    states: [idle, picking, shipped]\n\
         -    transitions:\n\
         -      - { from: idle,    to: picking, on: start }\n\
         -      - { from: picking, to: shipped, on: handoff, emits: [Shipped] }\n\
         -\n\
         @@ 24\n\
         -      OrderPaid:\n\
         -        - fire: Shipment.start\n\
         -          target: Shipment where orderId == event.orderId\n\
         +      OrderPaid: []",
        |d| {
            d.machines.remove(1);
            handler_mut(d, "Fulfillment", "OrderPaid").rules.clear();
        },
    );
    let shop = shop();
    let out = patched(&shop, &EditOp::RemoveMachine { machine: "Payment".into() });
    expect_definition(&shop, &out, |d| {
        d.machines.remove(1);
        for c in &mut d.controllers {
            for h in &mut c.on {
                h.rules.retain(|r| r.fire.value.machine != "Payment");
            }
        }
        for x in &mut d.external {
            x.triggers.retain(|t| t.value.machine != "Payment");
        }
    });
    assert!(out.contains("  PaymentGateway: []\n"), "{out}");
    assert_comments_kept(&shop, &out, &["Two transitions on `capture`", "(`else` covers the rest)"]);
}

#[test]
fn renaming_a_machine_renames_every_reference() {
    let text = shop();
    let out = check(
        &text,
        &EditOp::RenameMachine { from: "Payment".into(), to: "Charge".into() },
        "@@ 70\n\
         -  Payment:\n\
         +  Charge:\n\
         @@ 153\n\
         -        - fire: Payment.authorize\n\
         -          target: new Payment with orderId = event.orderId\n\
         +        - fire: Charge.authorize\n\
         +          target: new Charge with orderId = event.orderId\n\
         @@ 159\n\
         -        - fire: Payment.capture\n\
         -          target: Payment where orderId == event.orderId\n\
         +        - fire: Charge.capture\n\
         +          target: Charge where orderId == event.orderId\n\
         @@ 168\n\
         -        - fire: Payment.void\n\
         -          target: Payment where orderId == event.orderId\n\
         +        - fire: Charge.void\n\
         +          target: Charge where orderId == event.orderId\n\
         @@ 193\n\
         -        - fire: Payment.refund\n\
         -          target: Payment where orderId == event.orderId\n\
         +        - fire: Charge.refund\n\
         +          target: Charge where orderId == event.orderId\n\
         @@ 243\n\
         -  PaymentGateway: [Payment.approved, Payment.declined]\n\
         +  PaymentGateway: [Charge.approved, Charge.declined]",
        |d| {
            machine_mut(d, "Payment").name = s("Charge");
            for c in &mut d.controllers {
                for h in &mut c.on {
                    for r in &mut h.rules {
                        if r.fire.value.machine == "Payment" {
                            r.fire.value.machine = "Charge".into();
                        }
                        if let Some(t) = &mut r.target
                            && t.value.machine == "Payment"
                        {
                            t.value.machine = "Charge".into();
                        }
                    }
                }
            }
            for x in &mut d.external {
                for t in &mut x.triggers {
                    if t.value.machine == "Payment" {
                        t.value.machine = "Charge".into();
                    }
                }
            }
        },
    );
    assert_comments_kept(&text, &out, &[]);
    // And back again, byte for byte.
    assert_eq!(patched(&out, &EditOp::RenameMachine { from: "Charge".into(), to: "Payment".into() }), text);
}

#[test]
fn renaming_to_a_taken_or_invalid_name_is_rejected() {
    let text = shop();
    let taken = try_patch(&text, &EditOp::RenameMachine { from: "Payment".into(), to: "Order".into() });
    assert!(matches!(taken, Err(PatchError::Edit(EditError::NameTaken { .. }))), "{taken:?}");
    let invalid = try_patch(&text, &EditOp::RenameMachine { from: "Payment".into(), to: "bad name".into() });
    assert!(matches!(invalid, Err(PatchError::Edit(EditError::InvalidName(_)))), "{invalid:?}");
}

#[test]
fn color_is_replaced_added_and_removed_in_place() {
    let text = order_fulfillment();
    check(
        &text,
        &EditOp::SetMachineColor { machine: "Order".into(), color: Some(PaletteColor::Purple) },
        "@@ 5\n-    color: blue\n+    color: purple",
        |d| machine_mut(d, "Order").color = Some(Spanned::synthetic(PaletteColor::Purple)),
    );
    check(&text, &EditOp::SetMachineColor { machine: "Order".into(), color: None }, "@@ 5\n-    color: blue", |d| {
        machine_mut(d, "Order").color = None
    });
    let flow = fixture("flow.yaml");
    let out = check(
        &flow,
        &EditOp::SetMachineColor { machine: "Light".into(), color: Some(PaletteColor::Green) },
        "@@ 3\n\
         -  Light: { initial: red, states: [red, green, yellow], transitions: [{ from: red, to: green, on: go }, { from: green, to: yellow, on: slow }, { from: yellow, to: red, on: stop, emits: [Stopped] }] }\n\
         +  Light: { color: green, initial: red, states: [red, green, yellow], transitions: [{ from: red, to: green, on: go }, { from: green, to: yellow, on: slow }, { from: yellow, to: red, on: stop, emits: [Stopped] }] }",
        |d| machine_mut(d, "Light").color = Some(Spanned::synthetic(PaletteColor::Green)),
    );
    assert_eq!(patched(&out, &EditOp::SetMachineColor { machine: "Light".into(), color: None }), flow);
}

#[test]
fn a_new_key_goes_after_its_canonical_predecessor() {
    let text = shop();
    let out = check(
        &text,
        &EditOp::SetMachineDomain { machine: "Order".into(), domain: Some("sales".into()) },
        "@@ 46\n+    domain: sales",
        |d| machine_mut(d, "Order").domain = Some(s("sales")),
    );
    assert!(out.contains("    color: blue\n    domain: sales\n    fields: [orderId, customerId]\n"));
    assert_eq!(patched(&out, &EditOp::SetMachineDomain { machine: "Order".into(), domain: None }), text);
}

#[test]
fn initial_is_replaced_and_removed() {
    let text = order_fulfillment();
    check(
        &text,
        &EditOp::SetMachineInitial { machine: "Order".into(), initial: Some("pending".into()) },
        "@@ 6\n-    initial: draft\n+    initial: pending",
        |d| machine_mut(d, "Order").initial = Some(s("pending")),
    );
    check(
        &text,
        &EditOp::SetMachineInitial { machine: "Order".into(), initial: None },
        "@@ 6\n-    initial: draft",
        |d| machine_mut(d, "Order").initial = None,
    );
    let flow = fixture("flow.yaml");
    let out = patched(&flow, &EditOp::SetMachineInitial { machine: "Light".into(), initial: None });
    assert!(out.contains("  Light: { states: [red, green, yellow], transitions:"), "{out}");
}

#[test]
fn fields_change_item_by_item() {
    let text = shop();
    check(
        &text,
        &EditOp::SetMachineFields { machine: "Order".into(), fields: vec!["orderId".into(), "cartId".into()] },
        "@@ 46\n-    fields: [orderId, customerId]\n+    fields: [orderId, cartId]",
        |d| machine_mut(d, "Order").fields = vec![s("orderId"), s("cartId")],
    );
    check(
        &text,
        &EditOp::SetMachineFields { machine: "Order".into(), fields: vec![] },
        "@@ 46\n-    fields: [orderId, customerId]",
        |d| machine_mut(d, "Order").fields.clear(),
    );
    let of = order_fulfillment();
    check(
        &of,
        &EditOp::SetMachineFields { machine: "Order".into(), fields: vec!["orderId".into()] },
        "@@ 7\n+    fields: [orderId]",
        |d| machine_mut(d, "Order").fields = vec![s("orderId")],
    );
}

#[test]
fn the_system_name_keeps_its_quoting_and_place() {
    let of = order_fulfillment();
    // Sections here are separated by blank lines, so the new line is too.
    let out = check(&of, &EditOp::SetSystemName { name: Some("Orders".into()) }, "@@ 3\n+system: Orders\n+", |d| {
        d.system = Some(s("Orders"))
    });
    assert_eq!(patched(&out, &EditOp::SetSystemName { name: None }), of);
    let shop = shop();
    check(
        &shop,
        &EditOp::SetSystemName { name: Some("Shop: v2".into()) },
        "@@ 22\n-system: Shop\n+system: \"Shop: v2\"",
        |d| d.system = Some(s("Shop: v2")),
    );
    let out = check(&shop, &EditOp::SetSystemName { name: None }, "@@ 22\n-system: Shop\n-", |d| d.system = None);
    assert_comments_kept(&shop, &out, &[]);
    assert_eq!(patched(&out, &EditOp::SetSystemName { name: Some("Shop".into()) }), shop);
    let block = fixture("block.yaml");
    check(
        &block,
        &EditOp::SetSystemName { name: Some("Door's".into()) },
        "@@ 2\n-system: 'Door: block style'\n+system: 'Door''s'",
        |d| d.system = Some(s("Door's")),
    );
}
