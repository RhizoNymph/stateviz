//! `patch_text` on the semantics-phase keys written in block style with
//! comments (`fixtures/patch/semantics.yaml`): typed `fields:`/`payload:`
//! (switching between the list and mapping forms), subscriptions
//! (`on: { event, where }`), timers (`after:`), guards, `sets:`, emits with
//! `with:`, and selector filters. Every edit must stay in place, touch only
//! the lines it changes, keep every unrelated comment, and parse to what
//! `edit::apply` produces.

mod patch_common;

use cascade_core::definition::{Definition, EmitDef, EventDef, FieldDef, TransitionDef, TriggerSpec};
use cascade_core::edit::{EditOp, apply, without_spans};
use cascade_core::expr::{Assignment, Duration, ExprContext, FieldType, Guard, parse_condition, parse_expr};
use cascade_core::span::SourceSpan;
use cascade_core::{Spanned, load_str};
use cascade_interop::patch_text;
use patch_common::{assert_comments_kept, fixture, line_diff, parse};

fn text() -> String {
    fixture("semantics.yaml")
}

/// Patch in place, check against `edit::apply`, the line diff and the
/// comments (all kept except those containing one of `removed`).
fn check(text: &str, op: &EditOp, expected_diff: &str, removed: &[&str]) -> String {
    let expected = match apply(&parse(text), op) {
        Ok(applied) => applied.definition,
        Err(err) => panic!("{op:?}: {err}"),
    };
    let out = match patch_text(text, op) {
        Ok(p) => p,
        Err(err) => panic!("{op:?}: {err}"),
    };
    assert!(!out.rewritten, "{op:?} rewrote the file:\n{}", out.text);
    let out = out.text;
    if let Err(err) = load_str(&out) {
        panic!("patched text does not resolve:\n{err}\n---\n{out}");
    }
    assert_eq!(without_spans(&parse(&out)), without_spans(&expected), "{op:?}\n{out}");
    assert_eq!(line_diff(text, &out).trim_end(), expected_diff.trim_end(), "--- patched ---\n{out}");
    assert_comments_kept(text, &out, removed);
    out
}

fn transition(def: &Definition, machine: &str, index: usize) -> TransitionDef {
    match def.machines.iter().find(|m| m.name.value == machine).and_then(|m| m.transitions.get(index)) {
        Some(t) => t.clone(),
        None => panic!("no transition {machine}#{index}"),
    }
}

fn update(machine: &str, index: usize, transition: TransitionDef) -> EditOp {
    EditOp::UpdateTransition { machine: machine.into(), index, transition }
}

fn assign(field: &str, value: &str) -> Spanned<Assignment> {
    let operand = match cascade_core::expr::parse_operand(value) {
        Ok(o) => o,
        Err(err) => panic!("{value}: {err}"),
    };
    Spanned::synthetic(Assignment::new(field, operand))
}

fn filter(text: &str) -> Spanned<cascade_core::expr::Expr> {
    match parse_expr(text, ExprContext::Filter) {
        Ok(e) => Spanned::synthetic(e),
        Err(err) => panic!("{text}: {err}"),
    }
}

fn enum_of(members: &[&str]) -> FieldType {
    FieldType::Enum(members.iter().map(|m| (*m).to_owned()).collect())
}

fn fields(def: &Definition, machine: &str) -> Vec<FieldDef> {
    match def.machines.iter().find(|m| m.name.value == machine) {
        Some(m) => m.fields.clone(),
        None => panic!("no machine {machine}"),
    }
}

fn set_fields(machine: &str, fields: Vec<FieldDef>) -> EditOp {
    EditOp::SetMachineFields { machine: machine.into(), fields }
}

/// How the app changes a payload: remove the declaration and declare it
/// again at the same position.
fn set_payload(def: &Definition, event: &str, payload: Vec<FieldDef>) -> EditOp {
    let index = def.events.iter().position(|e| e.name.value == event);
    EditOp::Batch(vec![
        EditOp::RemoveEventDeclaration { event: event.into() },
        EditOp::DeclareEvent {
            event: EventDef { name: Spanned::synthetic(event.into()), payload, span: SourceSpan::unknown() },
            index,
        },
    ])
}

// --- Typed fields -------------------------------------------------------------------

#[test]
fn changing_one_field_type_touches_only_its_value() {
    let text = text();
    let mut f = fields(&parse(&text), "Order");
    f[2] = FieldDef::typed("tier", enum_of(&["basic", "gold", "vip"]));
    check(
        &text,
        &set_fields("Order", f),
        "@@ 8\n-      tier: enum [basic, gold]\n+      tier: enum [basic, gold, vip]",
        &[],
    );
}

#[test]
fn adding_and_removing_typed_fields_keeps_the_others_and_their_comments() {
    let text = text();
    let mut f = fields(&parse(&text), "Order");
    f.push(FieldDef::typed("note", FieldType::String));
    let out = check(&text, &set_fields("Order", f.clone()), "@@ 9\n+      note: string", &[]);

    f.pop();
    let back = check(&out, &set_fields("Order", f), "@@ 9\n-      note: string", &[]);
    assert_eq!(back, text);
}

#[test]
fn a_block_list_becomes_a_block_mapping_when_the_first_type_is_added() {
    let text = text();
    let f = vec![FieldDef::typed("orderId", FieldType::String), FieldDef::typed("total", FieldType::Any)];
    check(
        &text,
        &set_fields("Checkout", f),
        "@@ 36\n\
         -      - orderId   # the order\n\
         -      - total     # what to charge\n\
         +      orderId: string   # the order\n\
         +      total: any     # what to charge",
        &[],
    );
}

#[test]
fn a_block_mapping_becomes_a_block_list_when_the_last_type_goes() {
    let text = text();
    let f = vec![FieldDef::untyped("orderId"), FieldDef::untyped("amount"), FieldDef::untyped("tier")];
    check(
        &text,
        &set_fields("Order", f),
        "@@ 5\n\
         -      orderId: string      # the key\n\
         -      amount: number       # in cents\n\
         +      - orderId      # the key\n\
         +      - amount       # in cents\n\
         @@ 8\n\
         -      tier: enum [basic, gold]\n\
         +      - tier",
        &[],
    );
}

#[test]
fn switching_forms_and_changing_names_at_once() {
    let text = text();
    // Typed → untyped with a name added.
    let f = vec![
        FieldDef::untyped("orderId"),
        FieldDef::untyped("amount"),
        FieldDef::untyped("tier"),
        FieldDef::untyped("note"),
    ];
    let out = check(
        &text,
        &set_fields("Order", f),
        "@@ 5\n\
         -      orderId: string      # the key\n\
         -      amount: number       # in cents\n\
         +      - orderId      # the key\n\
         +      - amount       # in cents\n\
         @@ 8\n\
         -      tier: enum [basic, gold]\n\
         +      - tier\n\
         +      - note",
        &[],
    );
    // And back to typed without `note`.
    let f = vec![
        FieldDef::typed("orderId", FieldType::String),
        FieldDef::typed("amount", FieldType::Number),
        FieldDef::typed("tier", enum_of(&["basic", "gold"])),
    ];
    check(
        &out,
        &set_fields("Order", f),
        "@@ 5\n\
         -      - orderId      # the key\n\
         -      - amount       # in cents\n\
         +      orderId: string      # the key\n\
         +      amount: number       # in cents\n\
         @@ 8\n\
         -      - tier\n\
         -      - note\n\
         +      tier: enum [basic, gold]",
        &[],
    );
}

#[test]
fn flow_field_lists_switch_to_flow_mappings() {
    let text = text();
    let def = parse(&text);
    let op = set_payload(&def, "Shipped", vec![FieldDef::typed("orderId", FieldType::String)]);
    check(
        &text,
        &op,
        "@@ 48\n\
         -  Shipped: { payload: [orderId] }   # untyped\n\
         +  Shipped: { payload: { orderId: string } }   # untyped",
        &[],
    );
}

#[test]
fn payload_types_are_patched_entry_by_entry() {
    let text = text();
    let def = parse(&text);
    let payload = vec![
        FieldDef::typed("orderId", FieldType::Any),
        FieldDef::typed("amount", FieldType::Number),
        FieldDef::typed("kind", enum_of(&["sale", "refund"])),
    ];
    check(
        &text,
        &set_payload(&def, "OrderPaid", payload),
        "@@ 45\n\
         -      orderId: string   # key\n\
         +      orderId: any   # key\n\
         @@ 47\n\
         -      legacy: any       # unused\n\
         +      kind: enum [sale, refund]",
        &["unused"],
    );
    // Untyped again: a block list.
    let payload = vec![FieldDef::untyped("orderId"), FieldDef::untyped("amount")];
    check(
        &text,
        &set_payload(&def, "OrderPaid", payload),
        "@@ 45\n\
         -      orderId: string   # key\n\
         -      amount: number    # cents\n\
         -      legacy: any       # unused\n\
         +      - orderId   # key\n\
         +      - amount    # cents",
        &["unused"],
    );
}

// --- Triggers -------------------------------------------------------------------------

#[test]
fn a_subscription_filter_changes_in_place() {
    let text = text();
    let mut t = transition(&parse(&text), "Order", 1);
    t.trigger = TriggerSpec::Event {
        event: Spanned::synthetic("Shipped".into()),
        filter: Some(filter("orderId == event.orderId and event.orderId != none")),
    };
    check(
        &text,
        &update("Order", 1, t),
        "@@ 28\n\
         -          where: orderId == event.orderId   # match the order\n\
         +          where: orderId == event.orderId and event.orderId != none   # match the order",
        &[],
    );
}

#[test]
fn a_subscription_event_changes_in_place() {
    let text = text();
    let mut t = transition(&parse(&text), "Order", 1);
    if let TriggerSpec::Event { event, .. } = &mut t.trigger {
        *event = Spanned::synthetic("Cancelled".into());
    }
    check(&text, &update("Order", 1, t), "@@ 27\n-          event: Shipped\n+          event: Cancelled", &[]);
}

#[test]
fn filters_are_added_and_removed() {
    let text = text();
    let def = parse(&text);
    let mut t = transition(&def, "Order", 1);
    t.trigger = TriggerSpec::Event { event: Spanned::synthetic("Shipped".into()), filter: None };
    check(
        &text,
        &update("Order", 1, t),
        "@@ 28\n-          where: orderId == event.orderId   # match the order",
        &["match the order"],
    );

    let mut t = transition(&def, "Checkout", 0);
    t.trigger = TriggerSpec::Event {
        event: Spanned::synthetic("OrderPaid".into()),
        filter: Some(filter("orderId == event.orderId")),
    };
    check(
        &text,
        &update("Checkout", 0, t),
        "@@ 40\n\
         -      - { from: waiting, to: charged, on: { event: OrderPaid } }   # subscription\n\
         +      - { from: waiting, to: charged, on: { event: OrderPaid, where: orderId == event.orderId } }   # subscription",
        &[],
    );
}

#[test]
fn a_timer_delay_changes_in_place() {
    let text = text();
    let mut t = transition(&parse(&text), "Order", 2);
    t.trigger = TriggerSpec::After(Spanned::synthetic(Duration::from_secs(3600)));
    check(&text, &update("Order", 2, t), "@@ 31\n-        after: 30m   # give up\n+        after: 1h   # give up", &[]);
}

#[test]
fn a_timer_becomes_a_subscription_and_back() {
    let text = text();
    let def = parse(&text);
    let mut t = transition(&def, "Order", 2);
    t.trigger = TriggerSpec::Event {
        event: Spanned::synthetic("Cancelled".into()),
        filter: Some(filter("orderId == event.orderId")),
    };
    let out = check(
        &text,
        &update("Order", 2, t),
        "@@ 31\n\
         -        after: 30m   # give up\n\
         +        on: { event: Cancelled, where: orderId == event.orderId }",
        &["give up"],
    );
    let back = transition(&def, "Order", 2);
    check(
        &out,
        &update("Order", 2, back),
        "@@ 31\n\
         -        on: { event: Cancelled, where: orderId == event.orderId }\n\
         +        after: 30m",
        &[],
    );
}

// --- Guards ---------------------------------------------------------------------------

#[test]
fn guards_change_in_place_between_forms() {
    let text = text();
    let def = parse(&text);
    let mut t = transition(&def, "Order", 0);
    t.guard = Some(Spanned::synthetic(Guard::Choice("big spender".into())));
    let out = check(
        &text,
        &update("Order", 0, t.clone()),
        "@@ 15\n-        guard: event.amount > 0\n+        guard: \"?big spender\"",
        &[],
    );
    // The file double-quotes its free text where it has a choice (`when:`),
    // so `else` is quoted too.
    t.guard = Some(Spanned::synthetic(Guard::Else));
    let out = check(
        &out,
        &update("Order", 0, t.clone()),
        "@@ 15\n-        guard: \"?big spender\"\n+        guard: \"else\"",
        &[],
    );
    t.guard = None;
    // The comment above the guard belongs to it.
    check(
        &out,
        &update("Order", 0, t),
        "@@ 14\n-        # only real money\n-        guard: \"else\"",
        &["only real money"],
    );

    // In a file that writes free text plain, a forced quote goes away.
    let plain = text.replace("when: \"event.amount >= 100\"  ", "when: event.amount >= 100    ");
    let mut t = transition(&parse(&plain), "Order", 0);
    t.guard = Some(Spanned::synthetic(Guard::Choice("big spender".into())));
    let out = check(
        &plain,
        &update("Order", 0, t.clone()),
        "@@ 15\n-        guard: event.amount > 0\n+        guard: \"?big spender\"",
        &[],
    );
    t.guard = Some(Spanned::synthetic(Guard::Else));
    check(&out, &update("Order", 0, t), "@@ 15\n-        guard: \"?big spender\"\n+        guard: else", &[]);
}

// --- sets: ---------------------------------------------------------------------------------

#[test]
fn sets_entries_are_patched_one_by_one() {
    let text = text();
    let def = parse(&text);
    let mut t = transition(&def, "Order", 0);
    t.sets[0] = assign("amount", "self.amount");
    let out = check(
        &text,
        &update("Order", 0, t.clone()),
        "@@ 17\n\
         -          amount: event.amount   # copied from the payment\n\
         +          amount: self.amount   # copied from the payment",
        &[],
    );
    t.sets.remove(1);
    t.sets.push(assign("orderId", "event.orderId"));
    check(
        &out,
        &update("Order", 0, t),
        "@@ 18\n\
         -          tier: basic            # reset\n\
         +          orderId: event.orderId",
        &["reset"],
    );
}

#[test]
fn sets_in_a_flow_row_stay_flow() {
    let text = text();
    let def = parse(&text);
    let mut t = transition(&def, "Order", 3);
    t.sets = vec![assign("tier", "gold"), assign("amount", "0")];
    check(
        &text,
        &update("Order", 3, t.clone()),
        "@@ 32\n\
         -      - { from: paid, to: done, on: { event: Cancelled, where: \"orderId == event.orderId\" }, sets: { tier: basic } }\n\
         +      - { from: paid, to: done, on: { event: Cancelled, where: \"orderId == event.orderId\" }, sets: { tier: gold, amount: \"0\" } }",
        &[],
    );
    t.sets.clear();
    check(
        &text,
        &update("Order", 3, t),
        "@@ 32\n\
         -      - { from: paid, to: done, on: { event: Cancelled, where: \"orderId == event.orderId\" }, sets: { tier: basic } }\n\
         +      - { from: paid, to: done, on: { event: Cancelled, where: \"orderId == event.orderId\" } }",
        &[],
    );
}

#[test]
fn adding_sets_to_a_block_transition() {
    let text = text();
    let def = parse(&text);
    let mut t = transition(&def, "Order", 1);
    t.sets = vec![assign("tier", "gold")];
    check(&text, &update("Order", 1, t), "@@ 29\n+        sets: { tier: gold }", &[]);
}

// --- emits with `with:` ------------------------------------------------------------------

#[test]
fn a_with_value_changes_in_place() {
    let text = text();
    let mut t = transition(&parse(&text), "Order", 0);
    t.emits[1].with[0] = assign("total", "event.amount");
    check(
        &text,
        &update("Order", 0, t),
        "@@ 23\n\
         -              total: self.amount # after sets\n\
         +              total: event.amount # after sets",
        &[],
    );
}

#[test]
fn a_plain_emit_gains_a_with_mapping() {
    let text = text();
    let mut t = transition(&parse(&text), "Order", 0);
    t.emits[0] =
        EmitDef { event: Spanned::synthetic("OrderPaid".into()), with: vec![assign("orderId", "self.orderId")] };
    check(
        &text,
        &update("Order", 0, t),
        "@@ 20\n\
         -          - OrderPaid            # plain\n\
         +          - { event: OrderPaid, with: { orderId: self.orderId } }            # plain",
        &[],
    );
}

#[test]
fn a_shaped_emit_loses_its_with_mapping() {
    let text = text();
    let mut t = transition(&parse(&text), "Order", 0);
    t.emits[1] = EmitDef::plain("Receipt");
    check(
        &text,
        &update("Order", 0, t),
        "@@ 21\n\
         -          - event: Receipt       # shaped\n\
         -            with:\n\
         -              total: self.amount # after sets\n\
         +          - Receipt       # shaped",
        &["after sets"],
    );
}

#[test]
fn emits_are_added_and_removed_one_by_one() {
    let text = text();
    let def = parse(&text);
    let mut t = transition(&def, "Order", 0);
    t.emits.remove(0);
    let out =
        check(&text, &update("Order", 0, t.clone()), "@@ 20\n-          - OrderPaid            # plain", &["# plain"]);
    t.emits
        .push(EmitDef { event: Spanned::synthetic("Shipped".into()), with: vec![assign("orderId", "self.orderId")] });
    check(&out, &update("Order", 0, t), "@@ 23\n+          - { event: Shipped, with: { orderId: self.orderId } }", &[]);
}

#[test]
fn with_entries_are_added_to_a_block_mapping() {
    let text = text();
    let def = parse(&text);
    let mut t = transition(&def, "Order", 0);
    t.emits[1].with.insert(0, assign("total", "0"));
    t.emits[1].with.remove(1);
    check(
        &text,
        &update("Order", 0, t),
        "@@ 23\n\
         -              total: self.amount # after sets\n\
         +              total: \"0\" # after sets",
        &[],
    );
}

// --- Rules ---------------------------------------------------------------------------------

#[test]
fn rule_filters_and_conditions_change_in_place() {
    let text = text();
    let def = parse(&text);
    let mut r = def.controllers[0].on[0].rules[0].clone();
    r.target = Some(Spanned::synthetic(cascade_core::definition::TargetSpec::One {
        machine: "Order".into(),
        filter: Some(filter("orderId == event.orderId and tier == gold").value),
    }));
    r.when = match parse_condition("event.amount >= 200") {
        Ok(c) => Some(Spanned::synthetic(c)),
        Err(err) => panic!("{err}"),
    };
    let op = EditOp::UpdateRule { controller: "Billing".into(), event: "OrderPaid".into(), index: 0, rule: r };
    check(
        &text,
        &op,
        "@@ 58\n\
         -          target: Order where orderId == event.orderId   # one order\n\
         -          when: \"event.amount >= 100\"                     # big ones only\n\
         +          target: Order where orderId == event.orderId and tier == gold   # one order\n\
         +          when: \"event.amount >= 200\"                     # big ones only",
        &[],
    );
}

// --- Adding a transition with every key ---------------------------------------------------

#[test]
fn a_new_block_transition_carries_the_new_keys() {
    let text = text();
    let def = parse(&text);
    let mut t = transition(&def, "Order", 0);
    t.from = vec![Spanned::synthetic("paid".into())];
    t.guard = Some(Spanned::synthetic(Guard::Choice("again".into())));
    let op = EditOp::AddTransition { machine: "Order".into(), transition: t, index: Some(1) };
    let out = check(
        &text,
        &op,
        "@@ 25\n\
         +        to: paid\n\
         +        on: pay\n\
         +        guard: \"?again\"\n\
         +        sets: { amount: event.amount, tier: basic }\n\
         +        emits: [OrderPaid, { event: Receipt, with: { total: self.amount } }]\n\
         +      - from: paid",
        &[],
    );
    assert!(out.contains("# give up"));
}

// --- Fallbacks and chains -------------------------------------------------------------

#[test]
fn reordered_sets_are_replaced_as_a_block_in_place() {
    let text = text();
    let mut t = transition(&parse(&text), "Order", 0);
    t.sets.reverse();
    check(
        &text,
        &update("Order", 0, t),
        "@@ 17\n\
         -          amount: event.amount   # copied from the payment\n\
         -          tier: basic            # reset\n\
         +          tier: basic\n\
         +          amount: event.amount",
        &["copied from the payment", "reset"],
    );
}

/// Every transition of the fixture through a series of edits, each one
/// patched in place and checked against `edit::apply`, ending where it
/// started: the text comes back byte for byte.
#[test]
fn chains_of_edits_come_back_to_the_original_text() {
    // Free text written plain, so a guard that had to be quoted on the
    // way goes back to plain.
    let original = text().replace("when: \"event.amount >= 100\"  ", "when: event.amount >= 100    ");
    let def = parse(&original);
    for (machine, transitions) in def.machines.iter().map(|m| (m.name.value.clone(), m.transitions.clone())) {
        for (index, start) in transitions.iter().enumerate() {
            let mut steps: Vec<TransitionDef> = Vec::new();
            let mut t = start.clone();
            t.guard = Some(Spanned::synthetic(Guard::Choice(format!("step {index}"))));
            steps.push(t.clone());
            if machine == "Order" {
                t.sets.push(assign("tier", "gold"));
                t.sets.dedup_by(|a, b| a.value.field == b.value.field);
                steps.push(t.clone());
            }
            t.emits.push(EmitDef {
                event: Spanned::synthetic("Receipt".into()),
                with: vec![assign("total", "self.orderId")],
            });
            steps.push(t.clone());
            steps.push(start.clone());
            let mut text = original.clone();
            for step in steps {
                text = check_quiet(&text, &update(&machine, index, step));
            }
            assert_eq!(text, original, "{machine}#{index} did not come back");
        }
    }
}

/// [`check`] without a diff expectation.
fn check_quiet(text: &str, op: &EditOp) -> String {
    let expected = match apply(&parse(text), op) {
        Ok(applied) => applied.definition,
        Err(err) => panic!("{op:?}: {err}"),
    };
    let out = match patch_text(text, op) {
        Ok(p) => p,
        Err(err) => panic!("{op:?}: {err}"),
    };
    assert!(!out.rewritten, "{op:?} rewrote the file:\n{}", out.text);
    assert_eq!(without_spans(&parse(&out.text)), without_spans(&expected), "{op:?}\n{}", out.text);
    out.text
}
