//! `patch_text` for state ops: states stay in the form they are written in
//! (flow list, block list, mapping), references follow renames, removals
//! cascade to transitions and initials.

mod patch_common;

use cascade_core::Spanned;
use cascade_core::definition::StateKindDef;
use cascade_core::edit::EditOp;
use patch_common::*;

fn add(machine: &str, parent: Option<&str>, state: cascade_core::definition::StateDef, index: Option<usize>) -> EditOp {
    EditOp::AddState { machine: machine.into(), parent: parent.map(str::to_owned), state, index }
}

fn remove(machine: &str, path: &str) -> EditOp {
    EditOp::RemoveState { machine: machine.into(), path: path.into() }
}

fn rename(machine: &str, path: &str, to: &str) -> EditOp {
    EditOp::RenameState { machine: machine.into(), path: path.into(), to: to.into() }
}

fn kind(machine: &str, path: &str, kind: StateKindDef) -> EditOp {
    EditOp::SetStateKind { machine: machine.into(), path: path.into(), kind }
}

#[test]
fn a_plain_state_joins_a_flow_list() {
    let text = order_fulfillment();
    let out = check(
        &text,
        &add("Order", None, state("refunded"), None),
        "@@ 7\n-    states: [draft, pending, paid, cancelled]\n+    states: [draft, pending, paid, cancelled, refunded]",
        |d| machine_mut(d, "Order").states.push(state("refunded")),
    );
    assert_eq!(patched(&out, &remove("Order", "refunded")), text);
    check(
        &text,
        &add("Order", None, state("review"), Some(1)),
        "@@ 7\n-    states: [draft, pending, paid, cancelled]\n+    states: [draft, review, pending, paid, cancelled]",
        |d| machine_mut(d, "Order").states.insert(1, state("review")),
    );
}

#[test]
fn a_state_with_a_body_turns_a_flow_list_into_a_block_list() {
    let text = order_fulfillment();
    check(
        &text,
        &add("Order", None, final_state("archived"), None),
        "@@ 7\n\
         -    states: [draft, pending, paid, cancelled]\n\
         +    states:\n\
         +      - draft\n\
         +      - pending\n\
         +      - paid\n\
         +      - cancelled\n\
         +      - archived: { kind: final }",
        |d| machine_mut(d, "Order").states.push(final_state("archived")),
    );
}

#[test]
fn block_lists_get_block_items() {
    let text = shop();
    let out = check(&text, &add("Order", None, state("on_hold"), Some(2)), "@@ 53\n+      - on_hold", |d| {
        machine_mut(d, "Order").states.insert(2, state("on_hold"))
    });
    assert_eq!(patched(&out, &remove("Order", "on_hold")), text);
    let out =
        check(&text, &add("Order", None, final_state("lost"), None), "@@ 60\n+      - lost: { kind: final }", |d| {
            machine_mut(d, "Order").states.push(final_state("lost"))
        });
    assert_comments_kept(&text, &out, &[]);
    let disputed = compound("disputed", Some("open"), vec![state("open"), final_state("resolved")]);
    check(
        &text,
        &add("Payment", None, disputed.clone(), None),
        "@@ 82\n\
         +      - disputed:\n\
         +          initial: open\n\
         +          states:\n\
         +            - open\n\
         +            - resolved: { kind: final }",
        |d| machine_mut(d, "Payment").states.push(disputed),
    );
    let block = fixture("block.yaml");
    check(
        &block,
        &add("Door", None, final_state("broken"), None),
        "@@ 12\n+            - broken: { kind: final }",
        |d| machine_mut(d, "Door").states.push(final_state("broken")),
    );
}

#[test]
fn children_go_into_the_parent_in_its_form() {
    let text = shop();
    check(
        &text,
        &add("Order", Some("placed"), state("refunding"), None),
        "@@ 52\n-          states: [awaiting_payment, paid]\n+          states: [awaiting_payment, paid, refunding]",
        |d| state_mut(d, "Order", "placed").states.push(state("refunding")),
    );
    let out = check(
        &text,
        &add("Order", Some("fulfilling"), state("picking"), None),
        "@@ 53\n-      - fulfilling\n+      - fulfilling:\n+          states: [picking]",
        |d| state_mut(d, "Order", "fulfilling").states.push(state("picking")),
    );
    assert_eq!(patched(&out, &remove("Order", "fulfilling.picking")), text);
    let of = order_fulfillment();
    check(
        &of,
        &add("Order", Some("pending"), state("waiting"), None),
        "@@ 7\n\
         -    states: [draft, pending, paid, cancelled]\n\
         +    states:\n\
         +      - draft\n\
         +      - pending:\n\
         +          states: [waiting]\n\
         +      - paid\n\
         +      - cancelled",
        |d| state_mut(d, "Order", "pending").states.push(state("waiting")),
    );
}

#[test]
fn mapping_form_states_get_mapping_entries() {
    let text = fixture("nested.yaml");
    check(&text, &add("Job", None, state("failed"), Some(2)), "@@ 18\n+      failed: {}", |d| {
        machine_mut(d, "Job").states.insert(2, state("failed"))
    });
    check(&text, &add("Job", None, final_state("failed"), None), "@@ 19\n+      failed: { kind: final }", |d| {
        machine_mut(d, "Job").states.push(final_state("failed"))
    });
    let out = check(
        &text,
        &add("Job", Some("queued"), state("waiting"), None),
        "@@ 10\n-      queued: {}\n+      queued:\n+        states: [waiting]",
        |d| state_mut(d, "Job", "queued").states.push(state("waiting")),
    );
    assert_eq!(patched(&out, &remove("Job", "queued.waiting")), text);
    let out = check(
        &text,
        &add("Job", Some("running"), final_state("aborted"), Some(1)),
        "@@ 15\n+          - aborted: { kind: final }",
        |d| state_mut(d, "Job", "running").states.insert(1, final_state("aborted")),
    );
    assert!(
        out.contains("          - aborted: { kind: final }\n          # Computing is where the work happens.\n"),
        "{out}"
    );
}

#[test]
fn a_new_state_that_captures_a_bare_reference_spells_it_out() {
    let text = fixture("nested.yaml");
    check(
        &text,
        &add("Job", None, state("computing"), None),
        "@@ 19\n\
         +      computing: {}\n\
         @@ 21\n\
         -      - { from: running.fetching, to: computing, on: fetched }\n\
         +      - { from: running.fetching, to: running.computing, on: fetched }",
        |d| {
            let job = machine_mut(d, "Job");
            job.states.push(state("computing"));
            job.transitions[1].to = s("running.computing");
        },
    );
}

#[test]
fn removing_a_state_removes_its_transitions_and_comments() {
    let text = shop();
    let out = check(
        &text,
        &remove("Order", "returned"),
        "@@ 57\n\
         -      # PLANTED unreachable-state: only `return_received` enters `returned`,\n\
         -      # and only the orphaned Returns controller fires it.\n\
         -      - returned: { kind: final }\n\
         @@ 66\n\
         -      - { from: delivered, to: returned, on: return_received }",
        |d| {
            let order = machine_mut(d, "Order");
            order.states.pop();
            order.transitions.remove(5);
        },
    );
    assert_comments_kept(&text, &out, &["PLANTED unreachable-state", "only the orphaned Returns"]);
}

#[test]
fn removing_a_compound_state_removes_everything_touching_its_subtree() {
    let text = shop();
    let out = check(
        &text,
        &remove("Order", "placed"),
        "@@ 50\n\
         -      - placed:\n\
         -          initial: awaiting_payment\n\
         -          states: [awaiting_payment, paid]\n\
         @@ 61\n\
         -      - { from: cart, to: placed, on: checkout, emits: [OrderPlaced] }\n\
         -      - { from: placed.awaiting_payment, to: placed.paid, on: payment_captured, emits: [OrderPaid] }\n\
         -      - { from: placed.paid, to: fulfilling, on: stock_reserved }\n\
         @@ 67\n\
         -      # Declared on the compound `placed`, so it applies to both substates.\n\
         -      - { from: placed, to: cancelled, on: cancel, emits: [OrderCancelled] }",
        |d| {
            let order = machine_mut(d, "Order");
            order.states.remove(1);
            order.transitions.retain(|t| !t.from[0].value.starts_with("placed") && !t.to.value.starts_with("placed"));
        },
    );
    assert_comments_kept(&text, &out, &["Declared on the compound `placed`"]);
    check(
        &text,
        &remove("Order", "placed.paid"),
        "@@ 52\n\
         -          states: [awaiting_payment, paid]\n\
         +          states: [awaiting_payment]\n\
         @@ 62\n\
         -      - { from: placed.awaiting_payment, to: placed.paid, on: payment_captured, emits: [OrderPaid] }\n\
         -      - { from: placed.paid, to: fulfilling, on: stock_reserved }",
        |d| {
            state_mut(d, "Order", "placed").states.pop();
            let order = machine_mut(d, "Order");
            order.transitions.remove(2);
            order.transitions.remove(1);
        },
    );
}

#[test]
fn removing_a_state_trims_multi_source_lists_and_resets_initials() {
    let text = fixture("nested.yaml");
    check(
        &text,
        &remove("Job", "queued"),
        "@@ 7\n\
         -    initial: queued\n\
         @@ 10\n\
         -      queued: {}\n\
         @@ 20\n\
         -      - { from: queued, to: running, on: start }\n\
         @@ 22\n\
         -      # Pausing keeps the history.\n\
         -      - { from: running, to: queued, on: pause, emits: [Paused] }\n\
         -      - { from: queued, to: running.hist, on: resume }\n\
         -      - { from: [running.computing, queued], to: done, on: finish, emits: [Finished] }\n\
         +      - { from: [running.computing], to: done, on: finish, emits: [Finished] }",
        |d| {
            let job = machine_mut(d, "Job");
            job.initial = None;
            job.states.remove(0);
            job.transitions = vec![job.transitions[1].clone(), job.transitions[4].clone()];
            job.transitions[1].from.pop();
        },
    );
    let out = check(
        &text,
        &remove("Job", "running.fetching"),
        "@@ 12\n\
         -        initial: fetching\n\
         @@ 14\n\
         -          - fetching\n\
         @@ 21\n\
         -      - { from: running.fetching, to: computing, on: fetched }",
        |d| {
            let running = state_mut(d, "Job", "running");
            running.initial = None;
            running.states.remove(0);
            machine_mut(d, "Job").transitions.remove(1);
        },
    );
    assert_comments_kept(&text, &out, &[]);
    check(
        &text,
        &remove("Worker", "busy.warming"),
        "@@ 32\n-          states: [warming, working]\n+          states: [working]",
        |d| {
            state_mut(d, "Worker", "busy").states.remove(0);
        },
    );
}

#[test]
fn removing_the_last_child_makes_the_parent_plain_again() {
    let text = "machines:\n  A:\n    states:\n      - x:\n          states: [y]\n      - z\n    transitions:\n      - { from: z, to: x, on: go }\n";
    check(text, &remove("A", "x.y"), "@@ 4\n-      - x:\n-          states: [y]\n+      - x", |d| {
        state_mut(d, "A", "x").states.clear();
    });
}

#[test]
fn renaming_a_state_rewrites_every_reference() {
    let text = shop();
    let out = check(
        &text,
        &rename("Order", "placed", "ordered"),
        "@@ 50\n\
         -      - placed:\n\
         +      - ordered:\n\
         @@ 61\n\
         -      - { from: cart, to: placed, on: checkout, emits: [OrderPlaced] }\n\
         -      - { from: placed.awaiting_payment, to: placed.paid, on: payment_captured, emits: [OrderPaid] }\n\
         -      - { from: placed.paid, to: fulfilling, on: stock_reserved }\n\
         +      - { from: cart, to: ordered, on: checkout, emits: [OrderPlaced] }\n\
         +      - { from: ordered.awaiting_payment, to: ordered.paid, on: payment_captured, emits: [OrderPaid] }\n\
         +      - { from: ordered.paid, to: fulfilling, on: stock_reserved }\n\
         @@ 68\n\
         -      - { from: placed, to: cancelled, on: cancel, emits: [OrderCancelled] }\n\
         +      - { from: ordered, to: cancelled, on: cancel, emits: [OrderCancelled] }",
        |d| {
            let order = machine_mut(d, "Order");
            order.states[1].name = s("ordered");
            for t in &mut order.transitions {
                for r in t.from.iter_mut().chain(std::iter::once(&mut t.to)) {
                    if let Some(rest) = r.value.strip_prefix("placed") {
                        r.value = format!("ordered{rest}");
                    }
                }
            }
        },
    );
    assert_comments_kept(&text, &out, &[]);
    assert_eq!(patched(&out, &rename("Order", "ordered", "placed")), text);
    check(
        &text,
        &rename("Order", "placed.awaiting_payment", "unpaid"),
        "@@ 51\n\
         -          initial: awaiting_payment\n\
         -          states: [awaiting_payment, paid]\n\
         +          initial: unpaid\n\
         +          states: [unpaid, paid]\n\
         @@ 62\n\
         -      - { from: placed.awaiting_payment, to: placed.paid, on: payment_captured, emits: [OrderPaid] }\n\
         +      - { from: placed.unpaid, to: placed.paid, on: payment_captured, emits: [OrderPaid] }",
        |d| {
            let placed = state_mut(d, "Order", "placed");
            placed.initial = Some(s("unpaid"));
            placed.states[0].name = s("unpaid");
            machine_mut(d, "Order").transitions[1].from[0] = s("placed.unpaid");
        },
    );
}

#[test]
fn renames_keep_aligned_rows_aligned() {
    let text = order_fulfillment();
    check(
        &text,
        &rename("Order", "paid", "settled"),
        "@@ 7\n\
         -    states: [draft, pending, paid, cancelled]\n\
         +    states: [draft, pending, settled, cancelled]\n\
         @@ 10\n\
         -      - { from: pending, to: paid,      on: capture_ok, emits: [OrderPaid] }\n\
         +      - { from: pending, to: settled,   on: capture_ok, emits: [OrderPaid] }",
        |d| {
            let order = machine_mut(d, "Order");
            order.states[2].name = s("settled");
            order.transitions[1].to = s("settled");
        },
    );
    check(
        &text,
        &rename("Order", "draft", "new"),
        "@@ 6\n\
         -    initial: draft\n\
         -    states: [draft, pending, paid, cancelled]\n\
         +    initial: new\n\
         +    states: [new, pending, paid, cancelled]\n\
         @@ 9\n\
         -      - { from: draft,   to: pending,   on: submit }\n\
         +      - { from: new,     to: pending,   on: submit }",
        |d| {
            let order = machine_mut(d, "Order");
            order.initial = Some(s("new"));
            order.states[0].name = s("new");
            order.transitions[0].from[0] = s("new");
        },
    );
}

#[test]
fn renames_keep_bare_names_bare_unless_they_would_be_captured() {
    let text = fixture("nested.yaml");
    check(
        &text,
        &rename("Job", "running.computing", "crunching"),
        "@@ 16\n\
         -          - computing: { kind: normal }\n\
         +          - crunching: { kind: normal }\n\
         @@ 21\n\
         -      - { from: running.fetching, to: computing, on: fetched }\n\
         +      - { from: running.fetching, to: crunching, on: fetched }\n\
         @@ 25\n\
         -      - { from: [running.computing, queued], to: done, on: finish, emits: [Finished] }\n\
         +      - { from: [running.crunching, queued], to: done, on: finish, emits: [Finished] }",
        |d| {
            state_mut(d, "Job", "running.computing").name = s("crunching");
            let job = machine_mut(d, "Job");
            job.transitions[1].to = s("crunching");
            job.transitions[4].from[0] = s("running.crunching");
        },
    );
    // `done` → `computing`: the bare `computing` would now mean the
    // top-level state, so the reference to `running.computing` is spelled
    // out.
    check(
        &text,
        &rename("Job", "done", "computing"),
        "@@ 18\n\
         -      done: { kind: final }\n\
         +      computing: { kind: final }\n\
         @@ 21\n\
         -      - { from: running.fetching, to: computing, on: fetched }\n\
         +      - { from: running.fetching, to: running.computing, on: fetched }\n\
         @@ 25\n\
         -      - { from: [running.computing, queued], to: done, on: finish, emits: [Finished] }\n\
         +      - { from: [running.computing, queued], to: computing, on: finish, emits: [Finished] }",
        |d| {
            let job = machine_mut(d, "Job");
            job.states[2].name = s("computing");
            job.transitions[1].to = s("running.computing");
            job.transitions[4].to = s("computing");
        },
    );
}

#[test]
fn kinds_add_and_collapse_bodies() {
    let text = shop();
    let out = check(
        &text,
        &kind("Order", "closed", StateKindDef::Normal),
        "@@ 55\n-      - closed: { kind: final }\n+      - closed",
        |d| state_mut(d, "Order", "closed").kind = Spanned::synthetic(StateKindDef::Normal),
    );
    assert_eq!(patched(&out, &kind("Order", "closed", StateKindDef::Final)), text);
    let of = order_fulfillment();
    check(
        &of,
        &kind("Order", "cancelled", StateKindDef::Final),
        "@@ 7\n\
         -    states: [draft, pending, paid, cancelled]\n\
         +    states:\n\
         +      - draft\n\
         +      - pending\n\
         +      - paid\n\
         +      - cancelled: { kind: final }",
        |d| state_mut(d, "Order", "cancelled").kind = Spanned::synthetic(StateKindDef::Final),
    );
    let nested = fixture("nested.yaml");
    check(
        &nested,
        &kind("Job", "running.computing", StateKindDef::Normal),
        "@@ 16\n-          - computing: { kind: normal }\n+          - computing",
        |_| {},
    );
    check(
        &nested,
        &kind("Job", "done", StateKindDef::Normal),
        "@@ 18\n-      done: { kind: final }\n+      done: {}",
        |d| state_mut(d, "Job", "done").kind = Spanned::synthetic(StateKindDef::Normal),
    );
    let block = fixture("block.yaml");
    check(
        &block,
        &kind("Door", "locked", StateKindDef::Normal),
        "@@ 10\n-            - locked:\n-                  kind: final\n+            - locked",
        |d| state_mut(d, "Door", "locked").kind = Spanned::synthetic(StateKindDef::Normal),
    );
}

#[test]
fn compound_initials_are_set_in_the_body() {
    let text = shop();
    check(
        &text,
        &EditOp::SetStateInitial { machine: "Order".into(), path: "placed".into(), initial: Some("paid".into()) },
        "@@ 51\n-          initial: awaiting_payment\n+          initial: paid",
        |d| state_mut(d, "Order", "placed").initial = Some(s("paid")),
    );
    let out = check(
        &text,
        &EditOp::SetStateInitial { machine: "Order".into(), path: "placed".into(), initial: None },
        "@@ 51\n-          initial: awaiting_payment",
        |d| state_mut(d, "Order", "placed").initial = None,
    );
    assert_eq!(
        patched(
            &out,
            &EditOp::SetStateInitial {
                machine: "Order".into(),
                path: "placed".into(),
                initial: Some("awaiting_payment".into())
            }
        ),
        text
    );
    let nested = fixture("nested.yaml");
    check(
        &nested,
        &EditOp::SetStateInitial { machine: "Worker".into(), path: "busy".into(), initial: Some("working".into()) },
        "@@ 32\n+          initial: working",
        |d| state_mut(d, "Worker", "busy").initial = Some(s("working")),
    );
}
