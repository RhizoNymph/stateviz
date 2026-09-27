//! Model diffing (`diff_models`) and the ghost union for diff mode
//! (`merge_for_display`).

use std::collections::BTreeSet;

use cascade_core::diff::{DiffStatus, ModelDiff, diff_models, merge_for_display};
use cascade_core::model::StateKind;
use cascade_core::{CausalGraph, ElementKey, ElementRef, Model, analyze, load_str};

use DiffStatus::{Added, Changed, Removed};

const BASE: &str = r#"
system: Shop
machines:
  Order:
    color: blue
    domain: sales
    initial: draft
    fields: [orderId]
    states:
      - draft
      - pending:
          initial: waiting
          states: [waiting, authorizing]
      - paid
      - cancelled: { kind: final }
    transitions:
      - { from: draft, to: pending, on: submit }
      - { from: waiting, to: authorizing, on: authorize }
      - { from: pending, to: paid, on: capture_ok, guard: amount > 0, emits: [OrderPaid] }
      - { from: pending, to: cancelled, on: timeout, emits: [OrderCancelled] }
      - { from: paid, to: paid, on: ping }
      - { from: paid, to: paid, on: ping, guard: again }

  Shipment:
    color: green
    states: [idle, picking, shipped]
    transitions:
      - { from: idle, to: picking, on: start }
      - { from: picking, to: shipped, on: handoff, emits: [Shipped] }

controllers:
  Fulfillment:
    on:
      OrderPaid:
        - fire: Shipment.start
          target: Shipment where orderId == event.orderId
        - fire: Order.ping
      OrderCancelled:
        - fire: Shipment.handoff
          when: never
  Audit:
    on:
      Shipped:
        - fire: Order.ping

external:
  Customer: [Order.submit]
  PaymentGateway: [Order.capture_ok, Order.authorize]
  Clock: [Order.timeout]
"#;

// --- Helpers -----------------------------------------------------------------

fn load(text: &str) -> Model {
    match load_str(text) {
        Ok(model) => model,
        Err(err) => panic!("expected the definition to load:\n{err}\n---\n{text}"),
    }
}

/// Replace the first occurrence of `from`, which must exist.
fn edit(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "pattern {from:?} not found");
    text.replacen(from, to, 1)
}

fn key(text: &str) -> ElementKey {
    match text.parse() {
        Ok(key) => key,
        Err(err) => panic!("bad key in test: {err}"),
    }
}

/// The diff's entries as sorted `(key string, status)` pairs.
fn changes(diff: &ModelDiff) -> Vec<(String, DiffStatus)> {
    let mut out: Vec<_> = diff.entries().map(|(k, s)| (k.to_string(), s)).collect();
    out.sort();
    out
}

fn expected(items: &[(&str, DiffStatus)]) -> Vec<(String, DiffStatus)> {
    let mut out: Vec<_> = items.iter().map(|(k, s)| ((*k).to_owned(), *s)).collect();
    out.sort();
    out
}

fn diff_texts(old: &str, new: &str) -> ModelDiff {
    diff_models(&load(old), &load(new))
}

fn key_set(model: &Model) -> BTreeSet<ElementKey> {
    model.all_elements().into_iter().map(|e| model.key_of(e)).collect()
}

fn merge(old: &str, new: &str) -> (Model, ModelDiff, Model) {
    let old = load(old);
    let new = load(new);
    match merge_for_display(&old, &new) {
        Ok((merged, diff)) => (merged, diff, new),
        Err(err) => panic!("expected the union to resolve:\n{err}"),
    }
}

fn find(model: &Model, text: &str) -> ElementRef {
    match model.resolve_key(&key(text)) {
        Some(element) => element,
        None => panic!("{text} is not in the model"),
    }
}

/// Every element of `new` is in `merged`, at the same source position.
fn assert_new_elements_keep_spans(merged: &Model, new: &Model) {
    for element in new.all_elements() {
        let k = new.key_of(element);
        let Some(found) = merged.resolve_key(&k) else { panic!("{k} is missing from the merged model") };
        assert_eq!(merged.span_of(found), new.span_of(element), "span of {k}");
    }
}

/// Every removed element is a ghost in `merged` with no source position,
/// and the merged model holds nothing but new elements and ghosts.
fn assert_ghosts(merged: &Model, diff: &ModelDiff, new: &Model) {
    for (k, status) in diff.entries() {
        if status == Removed {
            let Some(found) = merged.resolve_key(k) else { panic!("removed {k} is not a ghost in the merged model") };
            assert!(!merged.span_of(found).is_known(), "ghost {k} should have no span");
        }
    }
    for element in merged.all_elements() {
        let k = merged.key_of(element);
        assert!(new.resolve_key(&k).is_some() || diff.status(&k) == Removed, "{k} is neither new nor a removed ghost");
    }
}

// --- diff_models: no changes ---------------------------------------------------

#[test]
fn identical_models_have_an_empty_diff() {
    let model = load(BASE);
    assert!(diff_models(&model, &model).is_empty());
    assert!(diff_models(&model, &model.clone()).is_empty());
    assert!(diff_texts(BASE, BASE).is_empty());
}

#[test]
fn layout_comments_and_order_of_unordered_lists_do_not_count() {
    let reordered = edit(BASE, "  Customer: [Order.submit]\n", "");
    let reordered =
        edit(&reordered, "  Clock: [Order.timeout]\n", "  Clock: [Order.timeout]\n  Customer: [Order.submit]\n");
    let reordered = edit(
        &reordered,
        "PaymentGateway: [Order.capture_ok, Order.authorize]",
        "PaymentGateway: [Order.authorize, Order.capture_ok]",
    );
    let reordered = edit(&reordered, "machines:\n", "# a comment\n\nmachines:\n\n");
    assert!(diff_texts(BASE, &reordered).is_empty(), "{:?}", changes(&diff_texts(BASE, &reordered)));
}

// --- diff_models: machines -----------------------------------------------------

#[test]
fn machine_attribute_changes() {
    for (from, to) in [
        ("color: blue", "color: purple"),
        ("domain: sales", "domain: billing"),
        ("initial: draft", "initial: paid"),
        ("fields: [orderId]", "fields: [orderId, customerId]"),
        ("    domain: sales\n", ""),
    ] {
        let diff = diff_texts(BASE, &edit(BASE, from, to));
        assert_eq!(changes(&diff), expected(&[("machine:Order", Changed)]), "{from} -> {to}");
    }
}

#[test]
fn machine_added_and_removed() {
    let with_invoice = edit(
        BASE,
        "machines:\n",
        "machines:\n  Invoice:\n    states: [open, closed]\n    transitions:\n      - { from: open, to: closed, on: settle }\n",
    );
    let diff = diff_texts(BASE, &with_invoice);
    assert_eq!(
        changes(&diff),
        expected(&[
            ("machine:Invoice", Added),
            ("state:Invoice:open", Added),
            ("state:Invoice:closed", Added),
            ("trigger:Invoice.settle", Added),
            ("transition:Invoice:open->closed@settle", Added),
        ])
    );
    let back = diff_texts(&with_invoice, BASE);
    assert_eq!(back.count(Removed), 5);
    assert_eq!(back.count(Added), 0);
}

// --- diff_models: states -------------------------------------------------------

#[test]
fn state_added_and_removed() {
    let with_refunded = edit(BASE, "      - paid\n", "      - paid\n      - refunded\n");
    assert_eq!(changes(&diff_texts(BASE, &with_refunded)), expected(&[("state:Order:refunded", Added)]));
    assert_eq!(changes(&diff_texts(&with_refunded, BASE)), expected(&[("state:Order:refunded", Removed)]));
}

#[test]
fn state_kind_changes() {
    let normal = edit(BASE, "- cancelled: { kind: final }", "- cancelled");
    assert_eq!(changes(&diff_texts(BASE, &normal)), expected(&[("state:Order:cancelled", Changed)]));

    let shallow =
        edit(BASE, "states: [waiting, authorizing]", "states: [waiting, authorizing, { hist: { kind: history } }]");
    let deep = edit(
        BASE,
        "states: [waiting, authorizing]",
        "states: [waiting, authorizing, { hist: { kind: deep-history } }]",
    );
    assert_eq!(changes(&diff_texts(BASE, &shallow)), expected(&[("state:Order:pending.hist", Added)]));
    assert_eq!(changes(&diff_texts(&shallow, &deep)), expected(&[("state:Order:pending.hist", Changed)]));
}

#[test]
fn compound_initial_change() {
    let diff = diff_texts(BASE, &edit(BASE, "initial: waiting", "initial: authorizing"));
    assert_eq!(changes(&diff), expected(&[("state:Order:pending", Changed)]));

    // An explicit initial that names the default child is no change.
    let implicit = edit(BASE, "          initial: waiting\n", "");
    assert!(diff_texts(BASE, &implicit).is_empty());
}

#[test]
fn atomic_state_becoming_compound() {
    let compound = edit(BASE, "      - draft\n", "      - draft:\n          states: [editing, review]\n");
    assert_eq!(
        changes(&diff_texts(BASE, &compound)),
        expected(&[
            ("state:Order:draft", Changed),
            ("state:Order:draft.editing", Added),
            ("state:Order:draft.review", Added),
        ])
    );
}

// --- diff_models: transitions and triggers ---------------------------------------

const PING_AGAIN: &str = "      - { from: paid, to: paid, on: ping, guard: again }\n";

#[test]
fn transition_added_and_removed_with_its_trigger_and_event() {
    let refund = edit(
        BASE,
        PING_AGAIN,
        &format!("{PING_AGAIN}      - {{ from: paid, to: cancelled, on: refund, emits: [Refunded] }}\n"),
    );
    assert_eq!(
        changes(&diff_texts(BASE, &refund)),
        expected(&[
            ("transition:Order:paid->cancelled@refund", Added),
            ("trigger:Order.refund", Added),
            ("event:Refunded", Added),
        ])
    );
    assert_eq!(
        changes(&diff_texts(&refund, BASE)),
        expected(&[
            ("transition:Order:paid->cancelled@refund", Removed),
            ("trigger:Order.refund", Removed),
            ("event:Refunded", Removed),
        ])
    );
}

#[test]
fn transition_attribute_changes() {
    let capture = "transition:Order:pending->paid@capture_ok";
    let guard = diff_texts(BASE, &edit(BASE, "guard: amount > 0", "guard: amount >= 1"));
    assert_eq!(changes(&guard), expected(&[(capture, Changed)]));

    let no_guard = diff_texts(BASE, &edit(BASE, "guard: amount > 0, ", ""));
    assert_eq!(changes(&no_guard), expected(&[(capture, Changed)]));

    let two = edit(BASE, "emits: [OrderPaid] }", "emits: [OrderPaid, Receipt] }");
    assert_eq!(changes(&diff_texts(BASE, &two)), expected(&[(capture, Changed), ("event:Receipt", Added)]));

    let swapped = edit(BASE, "emits: [OrderPaid] }", "emits: [Receipt, OrderPaid] }");
    assert_eq!(changes(&diff_texts(&two, &swapped)), expected(&[(capture, Changed)]));

    let bounded = edit(BASE, "emits: [OrderCancelled] }", "emits: [OrderCancelled], bounded: true }");
    assert_eq!(
        changes(&diff_texts(BASE, &bounded)),
        expected(&[("transition:Order:pending->cancelled@timeout", Changed)])
    );
}

#[test]
fn duplicate_transitions_are_matched_by_ordinal() {
    let first_removed = edit(BASE, "      - { from: paid, to: paid, on: ping }\n", "");
    assert_eq!(
        changes(&diff_texts(BASE, &first_removed)),
        expected(&[("transition:Order:paid->paid@ping", Changed), ("transition:Order:paid->paid@ping#1", Removed)])
    );

    let second_removed = edit(BASE, PING_AGAIN, "");
    assert_eq!(
        changes(&diff_texts(BASE, &second_removed)),
        expected(&[("transition:Order:paid->paid@ping#1", Removed)])
    );

    // Moving both, keeping their relative order, changes nothing.
    let moved = edit(BASE, "      - { from: paid, to: paid, on: ping }\n", "");
    let moved = edit(&moved, PING_AGAIN, "");
    let moved = edit(
        &moved,
        "      - { from: draft, to: pending, on: submit }\n",
        &format!(
            "      - {{ from: paid, to: paid, on: ping }}\n{PING_AGAIN}      - {{ from: draft, to: pending, on: submit }}\n"
        ),
    );
    assert!(diff_texts(BASE, &moved).is_empty());
}

#[test]
fn trigger_acceptance_changes_do_not_count() {
    // A second transition on an existing trigger: only the transition is new.
    let second = edit(BASE, PING_AGAIN, &format!("{PING_AGAIN}      - {{ from: paid, to: draft, on: submit }}\n"));
    assert_eq!(changes(&diff_texts(BASE, &second)), expected(&[("transition:Order:paid->draft@submit", Added)]));

    // Losing an external source leaves the trigger it fired unchanged.
    let no_customer = edit(BASE, "  Customer: [Order.submit]\n", "");
    assert_eq!(changes(&diff_texts(BASE, &no_customer)), expected(&[("external:Customer", Removed)]));

    // Losing the only rule of a controller leaves the fired trigger unchanged.
    let no_audit = edit(BASE, "  Audit:\n    on:\n      Shipped:\n        - fire: Order.ping\n", "");
    assert_eq!(
        changes(&diff_texts(BASE, &no_audit)),
        expected(&[
            ("controller:Audit", Removed),
            ("handler:Audit/Shipped", Removed),
            ("rule:Audit/Shipped#0", Removed),
        ])
    );
}

#[test]
fn a_trigger_disappears_with_its_last_mention() {
    let no_timeout = edit(BASE, "      - { from: pending, to: cancelled, on: timeout, emits: [OrderCancelled] }\n", "");
    let no_timeout = edit(&no_timeout, "  Clock: [Order.timeout]\n", "");
    let diff = diff_texts(BASE, &no_timeout);
    assert_eq!(diff.status(&key("trigger:Order.timeout")), Removed);
    assert_eq!(diff.status(&key("external:Clock")), Removed);
    assert_eq!(diff.status(&key("transition:Order:pending->cancelled@timeout")), Removed);
    // Nothing emits OrderCancelled any more, but a controller still subscribes.
    assert_eq!(diff.status(&key("event:OrderCancelled")), DiffStatus::Unchanged);
}

// --- diff_models: events -------------------------------------------------------

const EVENTS: &str = "\nevents:\n  OrderPaid: { payload: [orderId, amount] }\n  OrderCancelled: {}\n  Shipped: {}\n";

#[test]
fn event_payload_and_declaration_changes() {
    let strict = format!("{BASE}{EVENTS}");
    let smaller = edit(&strict, "[orderId, amount]", "[orderId]");
    assert_eq!(changes(&diff_texts(&strict, &smaller)), expected(&[("event:OrderPaid", Changed)]));

    let reordered = edit(&strict, "[orderId, amount]", "[amount, orderId]");
    assert_eq!(changes(&diff_texts(&strict, &reordered)), expected(&[("event:OrderPaid", Changed)]));

    // Declaring the events: every event changes (undeclared → declared).
    assert_eq!(
        changes(&diff_texts(BASE, &strict)),
        expected(&[("event:OrderCancelled", Changed), ("event:OrderPaid", Changed), ("event:Shipped", Changed)])
    );
}

// --- diff_models: controllers, handlers and rules ----------------------------------

#[test]
fn controller_handler_and_rule_added_and_removed() {
    let billing =
        edit(BASE, "\nexternal:", "  Billing:\n    on:\n      OrderPaid:\n        - fire: Order.ping\n\nexternal:");
    assert_eq!(
        changes(&diff_texts(BASE, &billing)),
        expected(&[
            ("controller:Billing", Added),
            ("handler:Billing/OrderPaid", Added),
            ("rule:Billing/OrderPaid#0", Added),
        ])
    );

    let handler = edit(BASE, "  Audit:\n    on:\n", "  Audit:\n    on:\n      OrderPaid: { fire: Order.ping }\n");
    assert_eq!(
        changes(&diff_texts(BASE, &handler)),
        expected(&[("handler:Audit/OrderPaid", Added), ("rule:Audit/OrderPaid#0", Added)])
    );

    let rule = edit(
        BASE,
        "        - fire: Order.ping\n      OrderCancelled:",
        "        - fire: Order.ping\n        - fire: Shipment.start\n      OrderCancelled:",
    );
    assert_eq!(changes(&diff_texts(BASE, &rule)), expected(&[("rule:Fulfillment/OrderPaid#2", Added)]));
    assert_eq!(changes(&diff_texts(&rule, BASE)), expected(&[("rule:Fulfillment/OrderPaid#2", Removed)]));

    // Removing the first rule shifts the second into its place.
    let first_gone =
        edit(BASE, "        - fire: Shipment.start\n          target: Shipment where orderId == event.orderId\n", "");
    assert_eq!(
        changes(&diff_texts(BASE, &first_gone)),
        expected(&[("rule:Fulfillment/OrderPaid#0", Changed), ("rule:Fulfillment/OrderPaid#1", Removed)])
    );
}

#[test]
fn rule_attribute_changes() {
    let cases = [
        ("Shipped:\n        - fire: Order.ping", "Shipped:\n        - fire: Order.submit", "rule:Audit/Shipped#0"),
        ("target: Shipment where", "target: all Shipment where", "rule:Fulfillment/OrderPaid#0"),
        ("== event.orderId", "== event.id", "rule:Fulfillment/OrderPaid#0"),
        (
            "target: Shipment where orderId == event.orderId",
            "target: new Shipment with orderId = event.orderId",
            "rule:Fulfillment/OrderPaid#0",
        ),
        ("\n          target: Shipment where orderId == event.orderId", "", "rule:Fulfillment/OrderPaid#0"),
        ("when: never", "when: sometimes", "rule:Fulfillment/OrderCancelled#0"),
        ("\n          when: never", "", "rule:Fulfillment/OrderCancelled#0"),
        ("when: never", "when: never\n          bounded: true", "rule:Fulfillment/OrderCancelled#0"),
    ];
    for (from, to, changed) in cases {
        let diff = diff_texts(BASE, &edit(BASE, from, to));
        assert_eq!(changes(&diff), expected(&[(changed, Changed)]), "{from:?} -> {to:?}");
    }
}

// --- diff_models: external sources ------------------------------------------------

#[test]
fn external_source_changes() {
    let admin = edit(BASE, "  Clock: [Order.timeout]\n", "  Clock: [Order.timeout]\n  Admin: [Order.timeout]\n");
    assert_eq!(changes(&diff_texts(BASE, &admin)), expected(&[("external:Admin", Added)]));
    assert_eq!(changes(&diff_texts(&admin, BASE)), expected(&[("external:Admin", Removed)]));

    let more = edit(BASE, "Clock: [Order.timeout]", "Clock: [Order.timeout, Order.submit]");
    assert_eq!(changes(&diff_texts(BASE, &more)), expected(&[("external:Clock", Changed)]));
}

// --- diff_models: renames ------------------------------------------------------------

#[test]
fn renaming_a_machine_is_a_removal_plus_an_addition() {
    let renamed = BASE.replace("Shipment", "Parcel");
    let diff = diff_texts(BASE, &renamed);
    for removed in
        ["machine:Shipment", "state:Shipment:idle", "trigger:Shipment.start", "transition:Shipment:idle->picking@start"]
    {
        assert_eq!(diff.status(&key(removed)), Removed, "{removed}");
    }
    for added in
        ["machine:Parcel", "state:Parcel:idle", "trigger:Parcel.start", "transition:Parcel:idle->picking@start"]
    {
        assert_eq!(diff.status(&key(added)), Added, "{added}");
    }
    // Rules keep their keys but fire into the renamed machine.
    assert_eq!(diff.status(&key("rule:Fulfillment/OrderPaid#0")), Changed);
    assert_eq!(diff.status(&key("rule:Fulfillment/OrderCancelled#0")), Changed);
    assert_eq!(diff.status(&key("event:Shipped")), DiffStatus::Unchanged);
    assert_eq!(diff.status(&key("machine:Order")), DiffStatus::Unchanged);
}

#[test]
fn renaming_a_state_an_event_or_a_controller() {
    let state = diff_texts(BASE, &BASE.replace("picking", "packing"));
    assert_eq!(
        changes(&state),
        expected(&[
            ("state:Shipment:picking", Removed),
            ("state:Shipment:packing", Added),
            ("transition:Shipment:idle->picking@start", Removed),
            ("transition:Shipment:idle->packing@start", Added),
            ("transition:Shipment:picking->shipped@handoff", Removed),
            ("transition:Shipment:packing->shipped@handoff", Added),
        ])
    );

    let event = diff_texts(BASE, &BASE.replace("OrderCancelled", "OrderVoided"));
    assert_eq!(
        changes(&event),
        expected(&[
            ("event:OrderCancelled", Removed),
            ("event:OrderVoided", Added),
            ("handler:Fulfillment/OrderCancelled", Removed),
            ("handler:Fulfillment/OrderVoided", Added),
            ("rule:Fulfillment/OrderCancelled#0", Removed),
            ("rule:Fulfillment/OrderVoided#0", Added),
            ("transition:Order:pending->cancelled@timeout", Changed),
        ])
    );

    let controller = diff_texts(BASE, &BASE.replace("Audit", "Monitor"));
    assert_eq!(
        changes(&controller),
        expected(&[
            ("controller:Audit", Removed),
            ("controller:Monitor", Added),
            ("handler:Audit/Shipped", Removed),
            ("handler:Monitor/Shipped", Added),
            ("rule:Audit/Shipped#0", Removed),
            ("rule:Monitor/Shipped#0", Added),
        ])
    );
}

// --- diff_models: properties -----------------------------------------------------------

#[test]
fn diff_is_antisymmetric() {
    let edits = [
        BASE.replace("Shipment", "Parcel"),
        edit(BASE, "color: blue", "color: purple"),
        edit(BASE, "      - paid\n", "      - paid\n      - refunded\n"),
        edit(BASE, PING_AGAIN, ""),
        edit(BASE, "Clock: [Order.timeout]", "Clock: [Order.timeout, Order.submit]"),
        format!("{BASE}{EVENTS}"),
    ];
    for other in &edits {
        let forward = diff_texts(BASE, other);
        let backward = diff_texts(other, BASE);
        let flipped: Vec<_> = changes(&forward)
            .into_iter()
            .map(|(k, s)| {
                let s = match s {
                    Added => Removed,
                    Removed => Added,
                    other => other,
                };
                (k, s)
            })
            .collect();
        assert_eq!(flipped, changes(&backward));
        assert!(!forward.is_empty());
    }
}

#[test]
fn counts_entries_and_serde() {
    let refund = edit(
        BASE,
        PING_AGAIN,
        &format!("{PING_AGAIN}      - {{ from: paid, to: cancelled, on: refund, emits: [Refunded] }}\n"),
    );
    let refund = edit(&refund, "color: blue", "color: purple");
    let diff = diff_texts(BASE, &refund);
    assert_eq!(diff.count(Added), 3);
    assert_eq!(diff.count(Changed), 1);
    assert_eq!(diff.count(Removed), 0);
    assert_eq!(diff.count(DiffStatus::Unchanged), 0);
    assert_eq!(diff.entries().count(), 4);
    let added: Vec<String> = diff.with_status(Added).map(ToString::to_string).collect();
    assert_eq!(added.len(), 3);
    assert!(added.contains(&"event:Refunded".to_owned()));

    let json = serde_json::to_string(&diff).expect("serializes");
    assert!(json.contains("\"machine:Order\":\"changed\""), "{json}");
    let back: ModelDiff = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back, diff);
}

// --- merge_for_display ---------------------------------------------------------------------

#[test]
fn merging_a_model_with_itself_is_the_model() {
    let model = load(BASE);
    let (merged, diff) = match merge_for_display(&model, &model) {
        Ok(pair) => pair,
        Err(err) => panic!("{err}"),
    };
    assert!(diff.is_empty());
    assert_eq!(key_set(&merged), key_set(&model));
    assert_new_elements_keep_spans(&merged, &model);
}

#[test]
fn removed_elements_appear_as_ghosts() {
    let old = edit(BASE, "      - paid\n", "      - paid\n      - refunded\n");
    let old = edit(
        &old,
        PING_AGAIN,
        &format!("{PING_AGAIN}      - {{ from: paid, to: refunded, on: refund, emits: [Refunded] }}\n"),
    );
    let old =
        edit(&old, "\nexternal:", "  Billing:\n    on:\n      Refunded:\n        - fire: Order.ping\n\nexternal:");
    let old = edit(&old, "  Audit:\n    on:\n", "  Audit:\n    on:\n      OrderPaid: { fire: Order.ping }\n");
    let old = edit(
        &old,
        "        - fire: Order.ping\n      OrderCancelled:",
        "        - fire: Order.ping\n        - fire: Shipment.start\n          when: rarely\n      OrderCancelled:",
    );
    let old = edit(&old, "  Clock: [Order.timeout]\n", "  Clock: [Order.timeout]\n  Admin: [Order.refund]\n");

    let (merged, diff, new) = merge(&old, BASE);
    for removed in [
        "state:Order:refunded",
        "transition:Order:paid->refunded@refund",
        "trigger:Order.refund",
        "event:Refunded",
        "controller:Billing",
        "handler:Billing/Refunded",
        "rule:Billing/Refunded#0",
        "handler:Audit/OrderPaid",
        "rule:Audit/OrderPaid#0",
        "rule:Fulfillment/OrderPaid#2",
        "external:Admin",
    ] {
        assert_eq!(diff.status(&key(removed)), Removed, "{removed}");
    }
    assert_eq!(diff.count(Removed), 11);
    assert_eq!(diff.count(Added) + diff.count(Changed), 0);
    assert_ghosts(&merged, &diff, &new);
    assert_new_elements_keep_spans(&merged, &new);

    // The ghost rule keeps its attributes.
    let ElementRef::Rule(rule) = find(&merged, "rule:Fulfillment/OrderPaid#2") else { panic!("not a rule") };
    assert_eq!(merged.rule(rule).condition.as_deref(), Some("rarely"));
    // A transition keeps the line it has in the new file.
    let capture = find(&merged, "transition:Order:pending->paid@capture_ok");
    assert_eq!(merged.span_of(capture).line(), Some(19));

    // The merged model feeds the causal graph and the checks.
    let graph = CausalGraph::build(&merged);
    let _ = analyze(&merged, &graph);
    assert!(graph.node_count() > CausalGraph::build(&new).node_count());
}

#[test]
fn added_and_changed_elements_come_from_the_new_model() {
    let new = edit(BASE, "      - paid\n", "      - paid\n      - refunded\n");
    let new = edit(&new, "guard: amount > 0", "guard: amount >= 1");
    let (merged, diff, new_model) = merge(BASE, &new);
    assert_eq!(diff.status(&key("state:Order:refunded")), Added);
    assert_eq!(diff.status(&key("transition:Order:pending->paid@capture_ok")), Changed);
    let ElementRef::Transition(t) = find(&merged, "transition:Order:pending->paid@capture_ok") else {
        panic!("not a transition")
    };
    assert_eq!(merged.transition(t).guard.as_deref(), Some("amount >= 1"));
    assert_eq!(key_set(&merged), key_set(&new_model));
}

#[test]
fn removed_machines_are_placed_where_they_were() {
    let invoice = "  Invoice:\n    states: [open, closed]\n    transitions:\n      - { from: open, to: closed, on: settle, emits: [OrderPaid] }\n\n";
    let first = edit(BASE, "machines:\n", &format!("machines:\n{invoice}"));
    let (merged, diff, new) = merge(&first, BASE);
    let names: Vec<&str> = merged.machines().map(|(_, m)| m.name.as_str()).collect();
    assert_eq!(names, ["Invoice", "Order", "Shipment"]);
    assert_eq!(diff.status(&key("machine:Invoice")), Removed);
    assert!(merged.resolve_key(&key("transition:Invoice:open->closed@settle")).is_some());
    assert_ghosts(&merged, &diff, &new);
    // The ghost machine emits OrderPaid before Order does in the union, but
    // the event keeps the position of its first mention in the new file.
    assert_new_elements_keep_spans(&merged, &new);

    let middle = edit(BASE, "  Shipment:\n", &format!("{invoice}  Shipment:\n"));
    let (merged, _, _) = merge(&middle, BASE);
    let names: Vec<&str> = merged.machines().map(|(_, m)| m.name.as_str()).collect();
    assert_eq!(names, ["Order", "Invoice", "Shipment"]);
}

#[test]
fn ghost_states_do_not_change_initial_states() {
    let old = r#"
machines:
  Job:
    states:
      - intro
      - queued
      - running:
          states: [prep, fetching, computing]
      - done: { kind: final }
    transitions:
      - { from: intro, to: queued, on: begin }
      - { from: queued, to: running, on: go }
      - { from: prep, to: fetching, on: ready }
      - { from: running, to: done, on: finish }
"#;
    let new = r#"
machines:
  Job:
    states:
      - queued
      - running:
          states: [fetching, computing]
      - done: { kind: final }
    transitions:
      - { from: queued, to: running, on: go }
      - { from: running, to: done, on: finish }
"#;
    let (merged, diff, new_model) = merge(old, new);
    let ElementRef::Machine(job) = find(&merged, "machine:Job") else { panic!("not a machine") };
    assert_eq!(merged.state(merged.machine(job).initial).path, "queued");
    let ElementRef::State(running) = find(&merged, "state:Job:running") else { panic!("not a state") };
    match &merged.state(running).kind {
        StateKind::Compound { initial, children } => {
            assert_eq!(merged.state(*initial).path, "running.fetching");
            let names: Vec<&str> = children.iter().map(|&c| merged.state(c).name.as_str()).collect();
            assert_eq!(names, ["prep", "fetching", "computing"]);
        }
        other => panic!("running should be compound, got {other:?}"),
    }
    let top: Vec<&str> = merged.machine(job).top_states.iter().map(|&s| merged.state(s).name.as_str()).collect();
    assert_eq!(top, ["intro", "queued", "running", "done"]);
    // The old machine's initial state changed from intro to queued.
    assert_eq!(diff.status(&key("machine:Job")), Changed);
    assert_eq!(diff.status(&key("state:Job:intro")), Removed);
    assert_ghosts(&merged, &diff, &new_model);
    assert_new_elements_keep_spans(&merged, &new_model);
}

#[test]
fn ghost_states_do_not_make_new_references_ambiguous() {
    let old = r#"
machines:
  Order:
    states:
      - pending:
          states: [waiting, authorizing]
      - review:
          states: [waiting, approved]
      - done
    transitions:
      - { from: pending.waiting, to: pending.authorizing, on: authorize }
      - { from: review.waiting, to: review.approved, on: approve }
      - { from: [pending, review], to: done, on: finish }
"#;
    let new = r#"
machines:
  Order:
    initial: waiting
    states:
      - pending:
          states: [waiting, authorizing]
      - done
    transitions:
      - { from: waiting, to: authorizing, on: authorize }
      - { from: [pending], to: done, on: finish }
"#;
    let (merged, diff, new_model) = merge(old, new);
    assert!(merged.resolve_key(&key("state:Order:review.waiting")).is_some());
    assert_eq!(
        diff.status(&key("transition:Order:pending.waiting->pending.authorizing@authorize")),
        DiffStatus::Unchanged
    );
    assert_eq!(diff.status(&key("transition:Order:review->done@finish")), Removed);
    let ElementRef::Machine(order) = find(&merged, "machine:Order") else { panic!("not a machine") };
    assert_eq!(merged.state(merged.machine(order).initial).path, "pending.waiting");
    assert_ghosts(&merged, &diff, &new_model);
    assert_new_elements_keep_spans(&merged, &new_model);
}

#[test]
fn duplicate_transition_ghosts_keep_their_ordinals() {
    let (merged, diff, new) = merge(BASE, &edit(BASE, PING_AGAIN, ""));
    assert_eq!(diff.status(&key("transition:Order:paid->paid@ping#1")), Removed);
    let ElementRef::Transition(ghost) = find(&merged, "transition:Order:paid->paid@ping#1") else {
        panic!("not a transition")
    };
    assert_eq!(merged.transition(ghost).guard.as_deref(), Some("again"));
    assert_ghosts(&merged, &diff, &new);

    let (merged, diff, new) = merge(BASE, &edit(BASE, "      - { from: paid, to: paid, on: ping }\n", ""));
    assert_eq!(diff.status(&key("transition:Order:paid->paid@ping")), Changed);
    assert_eq!(diff.status(&key("transition:Order:paid->paid@ping#1")), Removed);
    let ElementRef::Transition(kept) = find(&merged, "transition:Order:paid->paid@ping") else {
        panic!("not a transition")
    };
    assert_eq!(merged.transition(kept).guard.as_deref(), Some("again"));
    assert!(merged.resolve_key(&key("transition:Order:paid->paid@ping#1")).is_some());
    assert_ghosts(&merged, &diff, &new);
    assert_new_elements_keep_spans(&merged, &new);
}

#[test]
fn strict_events_union_declares_ghost_events_and_widens_checked_lists() {
    let old = edit(
        BASE,
        PING_AGAIN,
        &format!("{PING_AGAIN}      - {{ from: paid, to: cancelled, on: refund, emits: [Refunded] }}\n"),
    );
    let old = edit(
        &old,
        "\nexternal:",
        "  Billing:\n    on:\n      Refunded:\n        - fire: Order.ping\n          target: Order where orderId == event.refundId\n\nexternal:",
    );
    let old = edit(
        &old,
        "        - fire: Order.ping\n      OrderCancelled:",
        "        - fire: Order.ping\n        - fire: Shipment.handoff\n          target: Shipment where amount == event.amount\n      OrderCancelled:",
    );
    let old = format!(
        "{old}\nevents:\n  OrderPaid: {{ payload: [orderId, amount] }}\n  OrderCancelled: {{}}\n  Shipped: {{}}\n  Refunded: {{ payload: [refundId] }}\n"
    );
    let new = edit(BASE, "    color: green\n", "    color: green\n    fields: [orderId]\n");
    let new =
        format!("{new}\nevents:\n  OrderPaid: {{ payload: [orderId] }}\n  OrderCancelled: {{}}\n  Shipped: {{}}\n");

    let (merged, diff, new_model) = merge(&old, &new);
    assert_eq!(diff.status(&key("event:Refunded")), Removed);
    assert_eq!(diff.status(&key("event:OrderPaid")), Changed);
    assert_eq!(diff.status(&key("rule:Fulfillment/OrderPaid#2")), Removed);
    let ElementRef::Event(refunded) = find(&merged, "event:Refunded") else { panic!("not an event") };
    assert!(merged.event(refunded).declared);
    assert_eq!(merged.event(refunded).payload, ["refundId"]);
    // The ghost rule matches on a payload field and a machine field the new
    // version no longer declares; the union declares them so it resolves.
    let ElementRef::Event(paid) = find(&merged, "event:OrderPaid") else { panic!("not an event") };
    assert!(merged.event(paid).payload.contains(&"amount".to_owned()));
    let ElementRef::Machine(shipment) = find(&merged, "machine:Shipment") else { panic!("not a machine") };
    assert!(merged.machine(shipment).fields.contains(&"amount".to_owned()));
    assert_ghosts(&merged, &diff, &new_model);
    assert_new_elements_keep_spans(&merged, &new_model);
}

#[test]
fn strict_new_version_declares_events_the_old_one_left_undeclared() {
    let old = edit(
        BASE,
        PING_AGAIN,
        &format!("{PING_AGAIN}      - {{ from: paid, to: cancelled, on: refund, emits: [Refunded] }}\n"),
    );
    let new = format!("{BASE}{EVENTS}");
    let (merged, diff, new_model) = merge(&old, &new);
    let ElementRef::Event(refunded) = find(&merged, "event:Refunded") else { panic!("not an event") };
    assert!(merged.event(refunded).declared);
    assert!(merged.event(refunded).payload.is_empty());
    assert_ghosts(&merged, &diff, &new_model);
}

#[test]
fn non_strict_new_version_stays_non_strict() {
    let old = edit(
        BASE,
        PING_AGAIN,
        &format!("{PING_AGAIN}      - {{ from: paid, to: cancelled, on: refund, emits: [Refunded] }}\n"),
    );
    let old = format!(
        "{old}\nevents:\n  OrderPaid: {{ payload: [orderId, amount] }}\n  OrderCancelled: {{}}\n  Shipped: {{}}\n  Refunded: {{ payload: [refundId] }}\n  Unused: {{}}\n"
    );
    let (merged, diff, new_model) = merge(&old, BASE);
    assert!(merged.events().all(|(_, e)| !e.declared));
    assert!(merged.resolve_key(&key("event:Refunded")).is_some());
    // A declared-only event has nothing to hang a ghost on in a non-strict union.
    assert_eq!(diff.status(&key("event:Unused")), Removed);
    assert!(merged.resolve_key(&key("event:Unused")).is_none());
    assert_new_elements_keep_spans(&merged, &new_model);
}

#[test]
fn a_state_that_became_final_drops_its_ghost_children_and_transitions() {
    let old = r#"
machines:
  Order:
    states:
      - open
      - cancelled:
          states: [pendingRefund, refunded]
    transitions:
      - { from: open, to: cancelled, on: cancel }
      - { from: open, to: cancelled.refunded, on: void }
      - { from: cancelled.pendingRefund, to: cancelled.refunded, on: refund }
      - { from: cancelled, to: open, on: reopen }
"#;
    let new = r#"
machines:
  Order:
    states:
      - open
      - cancelled: { kind: final }
    transitions:
      - { from: open, to: cancelled, on: cancel }
"#;
    let (merged, diff, new_model) = merge(old, new);
    assert_eq!(diff.status(&key("state:Order:cancelled")), Changed);
    for gone in [
        "state:Order:cancelled.refunded",
        "transition:Order:cancelled->open@reopen",
        "transition:Order:cancelled.pendingRefund->cancelled.refunded@refund",
        "transition:Order:open->cancelled.refunded@void",
    ] {
        assert_eq!(diff.status(&key(gone)), Removed, "{gone}");
        assert!(merged.resolve_key(&key(gone)).is_none(), "{gone} cannot be drawn under a final state");
    }
    assert_new_elements_keep_spans(&merged, &new_model);
}

#[test]
fn ghost_children_of_a_now_atomic_state_keep_the_old_initial_child() {
    let old = r#"
machines:
  Order:
    states:
      - draft
      - pending:
          initial: waiting
          states:
            - hist: { kind: history }
            - waiting
            - authorizing
    transitions:
      - { from: draft, to: pending, on: submit }
      - { from: pending.waiting, to: pending.authorizing, on: authorize }
      - { from: draft, to: pending.hist, on: resume }
"#;
    let new = r#"
machines:
  Order:
    states: [draft, pending]
    transitions:
      - { from: draft, to: pending, on: submit }
"#;
    let (merged, diff, new_model) = merge(old, new);
    let ElementRef::State(pending) = find(&merged, "state:Order:pending") else { panic!("not a state") };
    match &merged.state(pending).kind {
        StateKind::Compound { initial, .. } => assert_eq!(merged.state(*initial).path, "pending.waiting"),
        other => panic!("pending should be compound in the union, got {other:?}"),
    }
    assert!(merged.resolve_key(&key("transition:Order:draft->pending.hist@resume")).is_some());
    assert_ghosts(&merged, &diff, &new_model);
    assert_new_elements_keep_spans(&merged, &new_model);
}

#[test]
fn removed_controllers_and_sources_keep_their_position_and_selectors() {
    let old = edit(
        BASE,
        "controllers:\n",
        "controllers:\n  Intake:\n    on:\n      Shipped:\n        - fire: Shipment.start\n          target: all Shipment where orderId == event.orderId\n          bounded: true\n",
    );
    let old = edit(&old, "external:\n", "external:\n  Warehouse: [Shipment.handoff, Shipment.start]\n");
    let (merged, diff, new) = merge(&old, BASE);
    let controllers: Vec<&str> = merged.controllers().map(|(_, c)| c.name.as_str()).collect();
    assert_eq!(controllers, ["Intake", "Fulfillment", "Audit"]);
    let sources: Vec<&str> = merged.externals().map(|(_, x)| x.name.as_str()).collect();
    assert_eq!(sources, ["Warehouse", "Customer", "PaymentGateway", "Clock"]);

    let ElementRef::Rule(rule) = find(&merged, "rule:Intake/Shipped#0") else { panic!("not a rule") };
    let ElementRef::Rule(old_rule) = find(&load(&old), "rule:Intake/Shipped#0") else { panic!("not a rule") };
    assert_eq!(merged.rule(rule).target, load(&old).rule(old_rule).target);
    assert!(merged.rule(rule).bounded);
    let ElementRef::External(warehouse) = find(&merged, "external:Warehouse") else { panic!("not a source") };
    assert_eq!(merged.external(warehouse).triggers.len(), 2);
    assert_ghosts(&merged, &diff, &new);
    assert_new_elements_keep_spans(&merged, &new);
}

#[test]
fn sources_in_both_versions_keep_only_their_new_triggers() {
    let old = edit(BASE, "Clock: [Order.timeout]", "Clock: [Order.timeout, Order.submit]");
    let (merged, diff, _) = merge(&old, BASE);
    assert_eq!(diff.status(&key("external:Clock")), Changed);
    let ElementRef::External(clock) = find(&merged, "external:Clock") else { panic!("not a source") };
    assert_eq!(merged.external(clock).triggers.len(), 1);
}

#[test]
fn merge_of_renamed_machine_shows_both() {
    let renamed = BASE.replace("Shipment", "Parcel");
    let (merged, diff, new) = merge(BASE, &renamed);
    let names: Vec<&str> = merged.machines().map(|(_, m)| m.name.as_str()).collect();
    assert_eq!(names, ["Order", "Shipment", "Parcel"]);
    assert_ghosts(&merged, &diff, &new);
    assert_new_elements_keep_spans(&merged, &new);
}
