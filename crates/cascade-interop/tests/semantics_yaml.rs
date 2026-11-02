//! The semantics-phase keys through `to_yaml` and `patch_text`: typed
//! fields and payloads, subscriptions (`on: { event, where }`), timers
//! (`after:`), guards (expressions, `?choices`, `else`), `sets:`, emits
//! `with:`, rule `when:` and filters/spawns in target selectors.

use cascade_core::definition::{Definition, EmitDef, FieldDef, TargetSpec, TransitionDef, TriggerSpec};
use cascade_core::edit::{EditOp, apply, without_spans};
use cascade_core::expr::{
    Assignment, Condition, Duration, ExprContext, FieldType, Guard, Literal, Operand, parse_expr,
};
use cascade_core::{Spanned, parse_definition};
use cascade_interop::{patch_text, to_yaml};

const SAGA: &str = r#"
# A saga with every new key.
machines:
  Order:
    fields: { orderId: string, amount: number, tier: "enum [basic, gold]" }
    states: [draft, paid, done]
    transitions:
      - { from: draft, to: paid, on: pay, guard: "event.amount > 0 and self.tier == gold", sets: { amount: event.amount, tier: basic }, emits: [OrderPaid, { event: Receipt, with: { total: self.amount, note: "'two words'" } }] }
      - { from: draft, to: done, on: pay, guard: "?card declined" }
      - { from: draft, to: done, on: pay, guard: else }
  Checkout:
    fields: [orderId]
    states: [waiting, paid, expired]
    transitions:
      - { from: waiting, to: paid, on: { event: OrderPaid, where: "orderId == event.orderId" } }
      - { from: [waiting, paid], to: expired, after: 30m }
controllers:
  Billing:
    on:
      OrderPaid:
        - fire: Order.pay
          target: Order where orderId == event.orderId and event.kind == refund
          when: "event.amount >= 100"
        - fire: Order.pay
          target: new Order with orderId = event.orderId, tier = gold
          when: "?not a gift card"
events:
  OrderPaid: { payload: { orderId: string, amount: number, kind: any } }
  Receipt: { payload: [total, note] }
external:
  Customer: [Order.pay]
"#;

fn parse(text: &str) -> Definition {
    match parse_definition(text) {
        Ok(def) => def,
        Err(err) => panic!("{err}\n---\n{text}"),
    }
}

#[test]
fn to_yaml_round_trips_every_new_key() {
    let def = parse(SAGA);
    let yaml = to_yaml(&def);
    let back = parse(&yaml);
    assert_eq!(without_spans(&back), without_spans(&def), "\n{yaml}");
    assert!(yaml.contains(r#"fields: { orderId: string, amount: number, tier: "enum [basic, gold]" }"#), "{yaml}");
    assert!(yaml.contains("on: { event: OrderPaid, where: orderId == event.orderId }"), "{yaml}");
    assert!(yaml.contains("after: 30m"), "{yaml}");
    assert!(yaml.contains(r#"guard: "?card declined""#), "{yaml}");
    assert!(yaml.contains("guard: else"), "{yaml}");
    assert!(yaml.contains(r#"with: { total: self.amount, note: "\"two words\"" }"#), "{yaml}");
    assert!(yaml.contains(r#"when: "?not a gift card""#), "{yaml}");
    assert!(yaml.contains("payload: [total, note]"), "{yaml}");
}

/// Apply `op` with `patch_text`: it must patch in place (no rewrite) and
/// the result must parse to what `edit::apply` produces.
fn patched(text: &str, op: &EditOp) -> String {
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
    assert!(out.text.contains("# A saga with every new key."), "comments survive");
    out.text
}

fn transition(def: &Definition, machine: &str, index: usize) -> TransitionDef {
    match def.machines.iter().find(|m| m.name.value == machine).and_then(|m| m.transitions.get(index)) {
        Some(t) => t.clone(),
        None => panic!("no transition {machine}#{index}"),
    }
}

#[test]
fn patching_guards_sets_and_emits() {
    let def = parse(SAGA);
    let mut t = transition(&def, "Order", 0);
    t.guard = Some(Spanned::synthetic(Guard::Choice("big order".into())));
    t.sets = vec![Spanned::synthetic(Assignment::new("tier", Operand::Lit(Literal::new("gold"))))];
    t.emits = vec![EmitDef::plain("OrderPaid")];
    let out = patched(SAGA, &EditOp::UpdateTransition { machine: "Order".into(), index: 0, transition: t.clone() });
    assert!(out.contains(r#"guard: "?big order""#), "{out}");
    assert!(out.contains("sets: { tier: gold }"), "{out}");

    // Back to an expression guard, no sets, emits with `with`.
    let mut t2 = t.clone();
    let expr = match parse_expr("self.amount >= 10", ExprContext::Plain) {
        Ok(e) => e,
        Err(err) => panic!("{err}"),
    };
    t2.guard = Some(Spanned::synthetic(Guard::Expr(expr)));
    t2.sets.clear();
    t2.emits = vec![EmitDef {
        event: Spanned::synthetic("Receipt".into()),
        with: vec![Spanned::synthetic(Assignment::new("total", Operand::Event("amount".into())))],
    }];
    let out = patched(&out, &EditOp::UpdateTransition { machine: "Order".into(), index: 0, transition: t2 });
    assert!(out.contains("emits: [{ event: Receipt, with: { total: event.amount } }]"), "{out}");
    assert!(!out.contains("sets:"), "{out}");

    let mut t3 = transition(&def, "Order", 2);
    t3.guard = None;
    let out = patched(SAGA, &EditOp::UpdateTransition { machine: "Order".into(), index: 2, transition: t3 });
    assert!(!out.contains("guard: else"), "{out}");
}

#[test]
fn patching_triggers_between_kinds() {
    let def = parse(SAGA);
    let mut t = transition(&def, "Checkout", 1);
    t.trigger = TriggerSpec::After(Spanned::synthetic(Duration::from_secs(90)));
    let out = patched(SAGA, &EditOp::UpdateTransition { machine: "Checkout".into(), index: 1, transition: t.clone() });
    assert!(out.contains("after: 90s"), "{out}");

    t.trigger = TriggerSpec::Command(Spanned::synthetic("expire".into()));
    let out = patched(SAGA, &EditOp::UpdateTransition { machine: "Checkout".into(), index: 1, transition: t });
    assert!(out.contains("on: expire") && !out.contains("after:"), "{out}");

    let mut sub = transition(&def, "Checkout", 0);
    sub.trigger = TriggerSpec::Event { event: Spanned::synthetic("OrderPaid".into()), filter: None };
    let out = patched(SAGA, &EditOp::UpdateTransition { machine: "Checkout".into(), index: 0, transition: sub });
    assert!(out.contains("on: { event: OrderPaid }"), "{out}");

    let mut new = transition(&def, "Checkout", 0);
    new.from = vec![Spanned::synthetic("paid".into())];
    new.to = Spanned::synthetic("waiting".into());
    patched(SAGA, &EditOp::AddTransition { machine: "Checkout".into(), transition: new, index: None });
}

#[test]
fn patching_rules_and_fields() {
    let def = parse(SAGA);
    let mut r = def.controllers[0].on[0].rules[0].clone();
    r.when = Some(Spanned::synthetic(Condition::Choice("rarely".into())));
    let filter = match parse_expr("orderId == event.orderId or event.kind in [a, b]", ExprContext::Filter) {
        Ok(e) => e,
        Err(err) => panic!("{err}"),
    };
    r.target = Some(Spanned::synthetic(TargetSpec::All { machine: "Order".into(), filter: Some(filter) }));
    let out = patched(
        SAGA,
        &EditOp::UpdateRule { controller: "Billing".into(), event: "OrderPaid".into(), index: 0, rule: r },
    );
    assert!(out.contains(r#"when: "?rarely""#), "{out}");
    assert!(out.contains("target: all Order where orderId == event.orderId or event.kind in [a, b]"), "{out}");

    // Typed → untyped, untyped → typed.
    let fields = vec![FieldDef::untyped("orderId"), FieldDef::untyped("amount"), FieldDef::untyped("tier")];
    let out = patched(SAGA, &EditOp::SetMachineFields { machine: "Order".into(), fields });
    assert!(out.contains("fields: [orderId, amount, tier]"), "{out}");
    let fields = vec![FieldDef::typed("orderId", FieldType::String)];
    let out = patched(SAGA, &EditOp::SetMachineFields { machine: "Checkout".into(), fields });
    assert!(out.contains("fields: { orderId: string }"), "{out}");
}

#[test]
fn renaming_an_event_renames_subscriptions_and_emit_mappings() {
    let out = patched(SAGA, &EditOp::RenameEvent { from: "Receipt".into(), to: "Slip".into() });
    assert!(out.contains("{ event: Slip, with:"), "{out}");
    let out = patched(SAGA, &EditOp::RenameEvent { from: "OrderPaid".into(), to: "Paid".into() });
    assert!(out.contains("on: { event: Paid, where:"), "{out}");
    assert!(out.contains("emits: [Paid,"), "{out}");
}
