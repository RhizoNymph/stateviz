//! Mermaid labels for guards, timers and subscriptions, and the causal
//! flowchart's notify and delay edges.

mod common;

use cascade_interop::{ExportFormat, export};

const SAGA: &str = r#"
machines:
  Order:
    states: [draft, paid, done]
    transitions:
      - { from: draft, to: paid, on: capture, guard: "event.amount > 0", emits: [OrderPaid] }
      - { from: draft, to: done, on: capture, guard: "?card declined" }
  Checkout:
    fields: [orderId]
    states: [waiting, paid, expired]
    transitions:
      - { from: waiting, to: paid, on: { event: OrderPaid, where: "orderId == event.orderId" } }
      - { from: paid, to: expired, after: 30s }
external:
  Customer: [Order.capture]
"#;

fn out(format: ExportFormat) -> String {
    match export(format, &common::load(SAGA)) {
        Ok(text) => text,
        Err(err) => panic!("{err}"),
    }
}

#[test]
fn structure_labels_name_guards_timers_and_subscriptions() {
    let text = out(ExportFormat::Mermaid);
    // `>` is written as a Mermaid entity code.
    assert!(text.contains("Order_draft --> Order_paid : capture [event.amount #62; 0]"), "{text}");
    assert!(text.contains("Order_draft --> Order_done : capture [?card declined]"), "{text}");
    assert!(text.contains("Checkout_paid --> Checkout_expired : after 30s"), "{text}");
    assert!(
        text.contains("Checkout_waiting --> Checkout_paid : on OrderPaid where orderId == event.orderId"),
        "{text}"
    );
}

#[test]
fn causal_flowchart_draws_notify_and_delay_edges_distinctly() {
    let text = out(ExportFormat::MermaidCausal);
    // OrderPaid (e0) notifies the subscription (t2); entering `paid` (t2)
    // arms the timer (t3).
    assert!(text.contains(r#"e0 ==>|"where orderId == event.orderId"| t2"#), "{text}");
    assert!(text.contains(r#"t2 -.->|"after 30s"| t3"#), "{text}");
    assert!(text.contains("stroke-dasharray:2 4"), "{text}");
}
