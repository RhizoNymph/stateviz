use super::*;

/// Paid is handled by Billing (capture) and Fraud (void), both selecting the
/// order's Payment.
const CAPTURE_VS_VOID: &str = r#"
machines:
  Order:
    states: [placed, paid]
    transitions:
      - { from: placed, to: paid, on: pay, emits: [Paid] }
  Payment:
    fields: [orderId]
    states: [authorized, captured, voided]
    transitions:
      - { from: authorized, to: captured, on: capture }
      - { from: authorized, to: voided, on: void }
controllers:
  Billing:
    on:
      Paid: [{ fire: Payment.capture, target: Payment where orderId == event.orderId }]
  Fraud:
    on:
      Paid: [{ fire: Payment.void, target: Payment where orderId == event.orderId }]
external:
  Customer: [Order.pay]
"#;

/// `(origin, machine, first, second)` keys of every race candidate.
fn races(text: &str) -> Vec<[String; 4]> {
    let (model, findings) = run(text);
    of(&findings, Check::RaceCandidate)
        .iter()
        .map(|f| match f.detail {
            FindingDetail::RaceCandidate { origin, machine, first, second } => [
                key(&model, ElementRef::Event(origin)),
                key(&model, ElementRef::Machine(machine)),
                key(&model, ElementRef::Rule(first)),
                key(&model, ElementRef::Rule(second)),
            ],
            ref other => panic!("{other:?}"),
        })
        .collect()
}

fn race(origin: &str, machine: &str, first: &str, second: &str) -> [String; 4] {
    [origin.to_owned(), machine.to_owned(), first.to_owned(), second.to_owned()]
}

#[test]
fn two_controllers_firing_into_one_instance_race() {
    let (model, findings) = run(CAPTURE_VS_VOID);
    let f = single(&findings, Check::RaceCandidate);
    assert_eq!(f.severity, Severity::Info);
    assert_eq!(keys(&model, &f.detail.subjects()), ["rule:Billing/Paid#0", "rule:Fraud/Paid#0"]);
    assert_eq!(key(&model, f.detail.primary()), "rule:Billing/Paid#0");
    assert_eq!(
        f.message,
        "Paid leads Billing to fire Payment.capture and Fraud to fire Payment.void, which may hit the same Payment instance in either order"
    );
    assert_eq!(
        races(CAPTURE_VS_VOID),
        [race("event:Paid", "machine:Payment", "rule:Billing/Paid#0", "rule:Fraud/Paid#0")]
    );
}

#[test]
fn rules_of_one_controller_do_not_race() {
    let text = r#"
machines:
  Order:
    states: [placed, paid]
    transitions:
      - { from: placed, to: paid, on: pay, emits: [Paid] }
  Payment:
    states: [authorized, captured, voided]
    transitions:
      - { from: authorized, to: captured, on: capture }
      - { from: authorized, to: voided, on: void }
controllers:
  Billing:
    on:
      Paid: [{ fire: Payment.capture }, { fire: Payment.void }]
external:
  Customer: [Order.pay]
"#;
    assert!(races(text).is_empty());
}

#[test]
fn spawned_instances_never_alias() {
    let text = CAPTURE_VS_VOID.replace(
        "fire: Payment.void, target: Payment where orderId == event.orderId",
        "fire: Payment.void, target: new Payment with orderId = event.orderId",
    );
    assert!(races(&text).is_empty());
}

#[test]
fn singletons_alias_any_selector() {
    let text = CAPTURE_VS_VOID
        .replace("fire: Payment.void, target: Payment where orderId == event.orderId", "fire: Payment.void");
    assert_eq!(races(&text), [race("event:Paid", "machine:Payment", "rule:Billing/Paid#0", "rule:Fraud/Paid#0")]);
}

#[test]
fn fan_out_selectors_may_alias() {
    let text = CAPTURE_VS_VOID.replace("target: Payment where", "target: all Payment where");
    assert_eq!(races(&text).len(), 1);
}

#[test]
fn fires_into_different_machines_do_not_race() {
    let text = r#"
machines:
  Order:
    states: [placed, paid]
    transitions:
      - { from: placed, to: paid, on: pay, emits: [Paid] }
  Payment:
    states: [authorized, captured]
    transitions:
      - { from: authorized, to: captured, on: capture }
  Shipment:
    states: [idle, packing]
    transitions:
      - { from: idle, to: packing, on: pack }
controllers:
  Billing:
    on:
      Paid: [{ fire: Payment.capture }]
  Shipping:
    on:
      Paid: [{ fire: Shipment.pack }]
external:
  Customer: [Order.pay]
"#;
    assert!(races(text).is_empty());
}

#[test]
fn causally_ordered_fires_are_not_races() {
    // Fraud reacts to Captured, which Billing's capture emits: Fraud always
    // runs after Billing.
    let text = r#"
machines:
  Order:
    states: [placed, paid]
    transitions:
      - { from: placed, to: paid, on: pay, emits: [Paid] }
  Payment:
    states: [authorized, captured, voided]
    transitions:
      - { from: authorized, to: captured, on: capture, emits: [Captured] }
      - { from: captured, to: voided, on: void }
controllers:
  Billing:
    on:
      Paid: [{ fire: Payment.capture }]
  Fraud:
    on:
      Captured: [{ fire: Payment.void }]
external:
  Customer: [Order.pay]
"#;
    assert!(races(text).is_empty());
}

#[test]
fn the_closest_origin_event_is_reported() {
    // Placed → Relay fires Order.pay → Paid → both rules. Paid is closer.
    let text = r#"
machines:
  Order:
    states: [cart, placed, paid]
    transitions:
      - { from: cart, to: placed, on: place, emits: [Placed] }
      - { from: placed, to: paid, on: pay, emits: [Paid] }
  Payment:
    states: [authorized, captured, voided]
    transitions:
      - { from: authorized, to: captured, on: capture }
      - { from: authorized, to: voided, on: void }
controllers:
  Relay:
    on:
      Placed: [{ fire: Order.pay }]
  Billing:
    on:
      Paid: [{ fire: Payment.capture }]
  Fraud:
    on:
      Paid: [{ fire: Payment.void }]
external:
  Customer: [Order.place]
"#;
    assert_eq!(races(text), [race("event:Paid", "machine:Payment", "rule:Billing/Paid#0", "rule:Fraud/Paid#0")]);
}

#[test]
fn equally_close_origins_go_to_the_first_event_in_model_order() {
    // Two upstream events relay into `Order.pay`, which emits both the
    // events the racing rules handle. Declaring `Second` first makes it
    // first in model order.
    let text = r#"
events: [Second, First, ToBilling, ToFraud]
machines:
  Up:
    states: [u0, u1, u2]
    transitions:
      - { from: u0, to: u1, on: first, emits: [First] }
      - { from: u1, to: u2, on: second, emits: [Second] }
  Order:
    states: [placed, paid]
    transitions:
      - { from: placed, to: paid, on: pay, emits: [ToBilling, ToFraud] }
  Payment:
    states: [authorized, captured, voided]
    transitions:
      - { from: authorized, to: captured, on: capture }
      - { from: authorized, to: voided, on: void }
controllers:
  Relay:
    on:
      First: [{ fire: Order.pay }]
      Second: [{ fire: Order.pay }]
  Billing:
    on:
      ToBilling: [{ fire: Payment.capture }]
  Fraud:
    on:
      ToFraud: [{ fire: Payment.void }]
external:
  User: [Up.first, Up.second]
"#;
    assert_eq!(
        races(text),
        [race("event:Second", "machine:Payment", "rule:Billing/ToBilling#0", "rule:Fraud/ToFraud#0")]
    );
}

#[test]
fn invalid_fires_do_not_race() {
    let text = CAPTURE_VS_VOID.replace("fire: Payment.void,", "fire: Payment.refund,");
    assert!(races(&text).is_empty());
}

#[test]
fn first_and_second_follow_rule_order() {
    // Fraud is declared before Billing, so its rule has the smaller id.
    let text = r#"
machines:
  Order:
    states: [placed, paid]
    transitions:
      - { from: placed, to: paid, on: pay, emits: [Paid] }
  Payment:
    states: [authorized, captured, voided]
    transitions:
      - { from: authorized, to: captured, on: capture }
      - { from: authorized, to: voided, on: void }
controllers:
  Fraud:
    on:
      Paid: [{ fire: Payment.void }]
  Billing:
    on:
      Paid: [{ fire: Payment.capture }]
external:
  Customer: [Order.pay]
"#;
    assert_eq!(races(text), [race("event:Paid", "machine:Payment", "rule:Fraud/Paid#0", "rule:Billing/Paid#0")]);
}

#[test]
fn each_pair_is_reported_once_across_origins() {
    // Both Placed and Paid reach both rules; one finding.
    let text = r#"
machines:
  Order:
    states: [cart, placed, paid]
    transitions:
      - { from: cart, to: placed, on: place, emits: [Placed] }
      - { from: placed, to: paid, on: pay, emits: [Paid] }
  Payment:
    states: [authorized, captured, voided]
    transitions:
      - { from: authorized, to: captured, on: capture }
      - { from: authorized, to: voided, on: void }
controllers:
  Relay:
    on:
      Placed: [{ fire: Order.pay }]
  Billing:
    on:
      Paid: [{ fire: Payment.capture }]
      Placed: [{ fire: Payment.capture }]
  Fraud:
    on:
      Paid: [{ fire: Payment.void }]
external:
  Customer: [Order.place]
"#;
    assert_eq!(
        races(text),
        [
            race("event:Paid", "machine:Payment", "rule:Billing/Paid#0", "rule:Fraud/Paid#0"),
            race("event:Placed", "machine:Payment", "rule:Billing/Placed#0", "rule:Fraud/Paid#0"),
        ]
    );
}
