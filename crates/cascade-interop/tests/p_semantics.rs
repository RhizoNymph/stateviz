//! P export of typed fields, expression guards and conditions, choices,
//! `sets`, subscriptions and timers.

mod common;

use cascade_interop::{ExportFormat, export};

const SAGA: &str = r#"
machines:
  Order:
    fields: { orderId: string, amount: number, tier: "enum [basic, gold]", gift: bool }
    states: [draft, paid, done]
    transitions:
      - { from: draft, to: paid, on: pay, guard: "self.amount > 0 and self.tier == gold", sets: { tier: basic, gift: "false" }, emits: [OrderPaid] }
      - { from: draft, to: done, on: pay, guard: "?card declined" }
      - { from: draft, to: done, on: pay, guard: else }
      - { from: paid, to: done, after: 30s, sets: { amount: event.amount } }
  Checkout:
    fields: [orderId]
    states: [waiting, paid]
    transitions:
      - { from: waiting, to: paid, on: { event: OrderPaid, where: "orderId == event.orderId" } }
controllers:
  Billing:
    on:
      OrderPaid:
        - fire: Order.pay
          when: "event.amount >= 100"
        - fire: Order.pay
          when: "?rarely"
events:
  OrderPaid: { payload: { orderId: string, amount: number, kind: "enum [sale, refund]" } }
external:
  Customer: [Order.pay]
"#;

fn p() -> String {
    match export(ExportFormat::P, &common::load(SAGA)) {
        Ok(text) => text,
        Err(err) => panic!("{err}"),
    }
}

fn has_line(text: &str, line: &str) -> bool {
    text.lines().any(|l| l.trim() == line)
}

#[test]
fn typed_fields_become_p_types_and_enums() {
    let out = p();
    assert!(has_line(&out, "enum tOrder_tier { Order_tier_basic, Order_tier_gold }"), "{out}");
    assert!(has_line(&out, "enum tOrderPaid_kind { OrderPaid_kind_sale, OrderPaid_kind_refund }"), "{out}");
    assert!(has_line(&out, "type tOrderPaid = (orderId: string, amount: float, kind: tOrderPaid_kind);"), "{out}");
    for var in ["var orderId: string;", "var amount: float;", "var tier: tOrder_tier;", "var gift: bool;"] {
        assert!(has_line(&out, var), "{var}\n{out}");
    }
    assert!(has_line(&out, "var orderId: any;"), "untyped field\n{out}");
}

#[test]
fn expression_guards_are_conditions_and_choices_are_dollar() {
    let out = p();
    assert!(has_line(&out, "if ((amount > 0.0) && (tier == Order_tier_gold)) {"), "{out}");
    assert!(has_line(&out, "} else if ($) {"), "{out}");
    assert!(has_line(&out, "// guard: ?card declined"), "{out}");
    // `else` is the fallback branch.
    assert!(has_line(&out, "} else {"), "{out}");
    assert!(has_line(&out, "// guard: else"), "{out}");
    assert!(has_line(&out, "tier = Order_tier_basic;"), "{out}");
    assert!(has_line(&out, "gift = false;"), "{out}");
    // Rule conditions read the handled event's payload.
    assert!(has_line(&out, "if (payload.amount >= 100.0) {"), "{out}");
    assert!(has_line(&out, "// when: ?rarely"), "{out}");
}

#[test]
fn subscriptions_receive_emits_and_timers_self_send() {
    let out = p();
    assert!(has_line(&out, r#"send registry["Checkout"], eCheckout_OrderPaid;"#), "{out}");
    assert!(has_line(&out, "// where: orderId == event.orderId"), "{out}");
    assert!(has_line(&out, "entry {"), "{out}");
    assert!(has_line(&out, "send this, eOrder_after_30s;"), "{out}");
    assert!(out.contains("// TODO sets amount = event.amount"), "{out}");
}
