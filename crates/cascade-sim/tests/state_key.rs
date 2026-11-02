//! The state key covers armed timers (relative to now), pending decisions,
//! partially delivered events and timer queue items.

mod common;
mod support;

use cascade_core::expr::Duration;
use cascade_sim::PlayAction;
use common::model;
use support::{add, fire, session};

const TIMED: &str = r#"
machines:
  Checkout:
    initial: waiting
    states: [waiting, expired]
    transitions:
      - { from: waiting, to: expired, after: 30m }
      - { from: expired, to: waiting, on: reset }
external:
  Shop: [Checkout.reset]
"#;

fn advance(minutes: u64) -> PlayAction {
    PlayAction::Advance { by: Some(Duration::from_secs(minutes * 60)) }
}

#[test]
fn timers_count_relative_to_now() {
    let model = model(TIMED);
    // Same remaining time on the timer, different clocks: same key.
    let a = session(&model, &[add("c1", "Checkout", &[])]);
    let b = session(&model, &[advance(5), add("c1", "Checkout", &[])]);
    assert_ne!(a.now(), b.now());
    assert_eq!(a.state_key(), b.state_key());
    // Different remaining time: different keys.
    let c = session(&model, &[add("c1", "Checkout", &[]), advance(5)]);
    assert_ne!(a.state_key(), c.state_key());
}

#[test]
fn queued_timers_are_part_of_the_key() {
    let model = model(TIMED);
    let due = session(&model, &[add("c1", "Checkout", &[]), advance(30)]);
    let delivered = session(&model, &[add("c1", "Checkout", &[]), advance(30), PlayAction::RunUntilQuiet]);
    assert_ne!(due.state_key(), delivered.state_key());
}

#[test]
fn pending_decisions_are_part_of_the_key() {
    let model = model(TIMED);
    let plain = session(&model, &[add("c1", "Checkout", &[])]);
    let chosen = session(&model, &[add("c1", "Checkout", &[]), PlayAction::Choose { option: "x".into() }]);
    let other = session(&model, &[add("c1", "Checkout", &[]), PlayAction::Choose { option: "y".into() }]);
    assert_ne!(plain.state_key(), chosen.state_key());
    assert_ne!(chosen.state_key(), other.state_key());
    let again = session(&model, &[add("c1", "Checkout", &[]), PlayAction::Choose { option: "x".into() }]);
    assert_eq!(chosen.state_key(), again.state_key());
}

#[test]
fn a_partially_delivered_event_differs_from_an_undelivered_one() {
    let model = model(support::RACE);
    let start = [
        add("o1", "Order", &[("orderId", "1")]),
        add("s1", "Shipment", &[("orderId", "1")]),
        fire("PaymentGateway", "Order.capture_ok", "o1"),
    ];
    let whole = session(&model, &start);
    let mut actions = start.to_vec();
    actions.push(PlayAction::Step { choice: None, consumer: Some("Billing".into()) });
    let partial = session(&model, &actions);
    assert_ne!(whole.state_key(), partial.state_key());
    // Billing then Fulfillment ends where Fulfillment then Billing's swapped
    // order does, not where FIFO does.
    actions.push(PlayAction::Step { choice: None, consumer: Some("Fulfillment".into()) });
    actions.push(PlayAction::RunUntilQuiet);
    let swapped = session(&model, &actions);
    let fifo = session(&model, &[start.to_vec(), vec![PlayAction::RunUntilQuiet]].concat());
    assert_ne!(swapped.state_key(), fifo.state_key());
}

#[test]
fn a_reset_cycle_returns_to_an_equal_key() {
    let model = model(TIMED);
    let start = session(&model, &[add("c1", "Checkout", &[])]);
    // Expire, reset: back in `waiting` with a fresh 30m timer. The history
    // record now holds `expired`, so the key differs from the start…
    let cycled = session(
        &model,
        &[add("c1", "Checkout", &[]), advance(30), PlayAction::RunUntilQuiet, fire("Shop", "Checkout.reset", "c1")],
    );
    assert_eq!(cycled.timers().len(), 1);
    assert_ne!(start.state_key(), cycled.state_key());
    // …but a second cycle lands on the first cycle's key, an hour later.
    let twice = session(
        &model,
        &[
            add("c1", "Checkout", &[]),
            advance(30),
            PlayAction::RunUntilQuiet,
            fire("Shop", "Checkout.reset", "c1"),
            advance(30),
            PlayAction::RunUntilQuiet,
            fire("Shop", "Checkout.reset", "c1"),
        ],
    );
    assert_ne!(cycled.now(), twice.now());
    assert_eq!(cycled.state_key(), twice.state_key());
}
