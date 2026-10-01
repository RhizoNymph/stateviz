//! `patch_text` for transition ops: new rows written like their
//! neighbours (aligned flow rows, block mappings, flow lists), updates that
//! touch only the changed fields, removals that take their comments along.

mod patch_common;

use cascade_core::edit::EditOp;
use patch_common::*;

fn add(machine: &str, transition: cascade_core::definition::TransitionDef, index: Option<usize>) -> EditOp {
    EditOp::AddTransition { machine: machine.into(), transition, index }
}

#[test]
fn a_new_row_is_padded_to_its_aligned_neighbours() {
    let text = order_fulfillment();
    let t = transition(&["paid"], "draft", "refund");
    check(&text, &add("Order", t.clone(), None), "@@ 12\n+      - { from: paid,    to: draft,     on: refund }", |d| {
        machine_mut(d, "Order").transitions.push(t)
    });
}

#[test]
fn a_row_at_the_front_keeps_the_columns_it_fits() {
    let text = order_fulfillment();
    let t = emitting(transition(&["shipped"], "idle", "reset"), &["Reset"]);
    check(
        &text,
        &add("Shipment", t.clone(), Some(0)),
        "@@ 18\n+      - { from: shipped, to: idle,    on: reset, emits: [Reset] }",
        |d| machine_mut(d, "Shipment").transitions.insert(0, t),
    );
}

#[test]
fn unaligned_rows_get_single_spaces() {
    let text = shop();
    let t = transition(&["delivered"], "cart", "reopen");
    check(&text, &add("Order", t.clone(), None), "@@ 69\n+      - { from: delivered, to: cart, on: reopen }", |d| {
        machine_mut(d, "Order").transitions.push(t)
    });
}

#[test]
fn inserting_before_an_entry_goes_above_its_comments() {
    let text = shop();
    let t = transition(&["requested"], "committed", "skip");
    let out = check(
        &text,
        &add("Inventory", t.clone(), Some(0)),
        "@@ 107\n+      - { from: requested, to: committed, on: skip }",
        |d| machine_mut(d, "Inventory").transitions.insert(0, t),
    );
    assert_comments_kept(&text, &out, &[]);
}

#[test]
fn block_mapping_transitions_get_a_block_mapping() {
    let text = fixture("block.yaml");
    let t = transition(&["open"], "closed", "close_door");
    check(
        &text,
        &add("Door", t.clone(), None),
        "@@ 22\n+            - from: open\n+              to: closed\n+              on: close_door",
        |d| machine_mut(d, "Door").transitions.push(t),
    );
}

#[test]
fn flow_lists_stay_flow() {
    let text = fixture("flow.yaml");
    let t = transition(&["zero"], "some", "inc");
    check(
        &text,
        &add("Counter", t.clone(), None),
        "@@ 4\n\
         -  Counter: { states: [zero, some], transitions: [] }   # no transitions yet\n\
         +  Counter: { states: [zero, some], transitions: [{ from: zero, to: some, on: inc }] }   # no transitions yet",
        |d| machine_mut(d, "Counter").transitions.push(t),
    );
    let t = transition(&["red"], "yellow", "warn");
    let out = patched(&text, &add("Light", t, Some(1)));
    assert!(
        out.contains(
            "transitions: [{ from: red, to: green, on: go }, { from: red, to: yellow, on: warn }, { from: green"
        ),
        "{out}"
    );
}

#[test]
fn a_machine_without_transitions_gets_the_key() {
    let text = "machines:\n  A:\n    states: [x, y]\n";
    let t = transition(&["x"], "y", "go");
    let out =
        check(text, &add("A", t.clone(), None), "@@ 4\n+    transitions:\n+      - { from: x, to: y, on: go }", |d| {
            machine_mut(d, "A").transitions.push(t)
        });
    // Removing the only transition takes the key away again.
    assert_eq!(patched(&out, &EditOp::RemoveTransition { machine: "A".into(), index: 0 }), text);
}

#[test]
fn an_update_touches_only_changed_fields_and_keeps_alignment() {
    let text = order_fulfillment();
    let t = emitting(transition(&["pending"], "cancelled", "capture_ok"), &["OrderPaid"]);
    check(
        &text,
        &EditOp::UpdateTransition { machine: "Order".into(), index: 1, transition: t.clone() },
        "@@ 10\n\
         -      - { from: pending, to: paid,      on: capture_ok, emits: [OrderPaid] }\n\
         +      - { from: pending, to: cancelled, on: capture_ok, emits: [OrderPaid] }",
        |d| machine_mut(d, "Order").transitions[1] = t,
    );
}

#[test]
fn an_update_of_a_block_transition_edits_its_lines() {
    let text = shop();
    let mut t = guarded(transition(&["authorized"], "captured", "capture"), "amount < limit");
    t.bounded = true;
    let out = check(
        &text,
        &EditOp::UpdateTransition { machine: "Payment".into(), index: 3, transition: t.clone() },
        "@@ 91\n\
         -        guard: \"amount <= authorized_amount\"\n\
         -        emits: [PaymentCaptured]\n\
         +        guard: \"amount < limit\"\n\
         +        bounded: true",
        |d| machine_mut(d, "Payment").transitions[3] = t,
    );
    assert_comments_kept(&text, &out, &[]);
}

/// The shop quotes its free text, so a new guard is quoted too.
#[test]
fn an_update_can_add_fields_and_sources_to_a_flow_row() {
    let text = shop();
    let t = guarded(
        emitting(transition(&["created", "authorized"], "authorizing", "authorize"), &["PaymentAuthorized"]),
        "x > 1",
    );
    check(
        &text,
        &EditOp::UpdateTransition { machine: "Payment".into(), index: 0, transition: t.clone() },
        "@@ 83\n\
         -      - { from: created, to: authorizing, on: authorize }\n\
         +      - { from: [created, authorized], to: authorizing, on: authorize, guard: \"x > 1\", emits: [PaymentAuthorized] }",
        |d| machine_mut(d, "Payment").transitions[0] = t,
    );
}

#[test]
fn removing_takes_the_entry_and_its_own_comments() {
    let text = shop();
    let out = check(
        &text,
        &EditOp::RemoveTransition { machine: "Inventory".into(), index: 0 },
        "@@ 107\n\
         -      # PLANTED nondeterminism: the backorder branch was added later, but the\n\
         -      # happy path never got the opposite guard, so with low stock both\n\
         -      # transitions on `reserve` apply.\n\
         -      - { from: requested, to: reserved, on: reserve, emits: [StockReserved] }",
        |d| {
            machine_mut(d, "Inventory").transitions.remove(0);
        },
    );
    assert_comments_kept(&text, &out, &["PLANTED nondeterminism", "happy path never", "transitions on `reserve`"]);
}

#[test]
fn removing_a_block_transition_takes_all_its_lines() {
    let text = shop();
    check(
        &text,
        &EditOp::RemoveTransition { machine: "Payment".into(), index: 3 },
        "@@ 86\n\
         -      # Two transitions on `capture`, but the guards are mutually exclusive\n\
         -      # (`else` covers the rest), so this is not nondeterminism.\n\
         -      - from: authorized\n\
         -        to: captured\n\
         -        on: capture\n\
         -        guard: \"amount <= authorized_amount\"\n\
         -        emits: [PaymentCaptured]",
        |d| {
            machine_mut(d, "Payment").transitions.remove(3);
        },
    );
    let block = fixture("block.yaml");
    check(
        &block,
        &EditOp::RemoveTransition { machine: "Door".into(), index: 1 },
        "@@ 17\n\
         -            # Locking only works when closed.\n\
         -            - from: closed\n\
         -              to: locked\n\
         -              on: lock\n\
         -              guard: \"has key\"",
        |d| {
            machine_mut(d, "Door").transitions.remove(1);
        },
    );
}

#[test]
fn removing_from_a_flow_list_keeps_the_rest_of_the_line() {
    let text = fixture("flow.yaml");
    check(
        &text,
        &EditOp::RemoveTransition { machine: "Light".into(), index: 1 },
        "@@ 3\n\
         -  Light: { initial: red, states: [red, green, yellow], transitions: [{ from: red, to: green, on: go }, { from: green, to: yellow, on: slow }, { from: yellow, to: red, on: stop, emits: [Stopped] }] }\n\
         +  Light: { initial: red, states: [red, green, yellow], transitions: [{ from: red, to: green, on: go }, { from: yellow, to: red, on: stop, emits: [Stopped] }] }",
        |d| {
            machine_mut(d, "Light").transitions.remove(1);
        },
    );
}

#[test]
fn bad_positions_are_rejected() {
    let text = order_fulfillment();
    let err = try_patch(&text, &EditOp::RemoveTransition { machine: "Order".into(), index: 9 });
    assert!(matches!(
        err,
        Err(cascade_interop::PatchError::Edit(cascade_core::edit::EditError::IndexOutOfRange { .. }))
    ));
    let err = try_patch(&text, &add("Nope", transition(&["a"], "b", "c"), None));
    assert!(matches!(err, Err(cascade_interop::PatchError::Edit(cascade_core::edit::EditError::NotFound { .. }))));
}
