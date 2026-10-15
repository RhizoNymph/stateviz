//! The example's scenarios run and tell the story their comments describe.

mod common;

use common::{finals, lines, model, run, strs};

const DEFINITION: &str = include_str!("../../../examples/order-fulfillment/cascade.yaml");
const HAPPY: &str = include_str!("../../../examples/order-fulfillment/scenarios/happy-path.yaml");
const TIMEOUT_RACE: &str = include_str!("../../../examples/order-fulfillment/scenarios/timeout-race.yaml");
const TIMEOUT_FIRST: &str = include_str!("../../../examples/order-fulfillment/scenarios/timeout-first.yaml");

#[test]
fn happy_path() {
    let model = model(DEFINITION);
    let trace = run(&model, HAPPY);
    assert_eq!(trace.scenario, "happy path");
    assert_eq!(
        lines(&model, &trace),
        strs(&[
            "ext Customer -> o1 submit",
            "o1 draft -> pending",
            "ext PaymentGateway -> o1 capture_ok",
            "o1 pending -> paid",
            "o1 emit OrderPaid",
            "Fulfillment <- OrderPaid",
            "Fulfillment fire s1 start",
            "s1 idle -> picking",
        ])
    );
    assert_eq!(finals(&model, &trace), ["o1: paid", "s1: picking"]);
}

#[test]
fn timeout_races_the_payment() {
    let model = model(DEFINITION);
    let trace = run(&model, TIMEOUT_RACE);
    assert_eq!(
        lines(&model, &trace)[4..],
        strs(&[
            "o1 emit OrderPaid",
            "ext Clock -> o1 timeout",
            "o1 drop timeout @ paid",
            "Fulfillment <- OrderPaid",
            "Fulfillment fire s1 start",
            "s1 idle -> picking",
        ])
    );
}

#[test]
fn timeout_first() {
    let model = model(DEFINITION);
    let trace = run(&model, TIMEOUT_FIRST);
    assert_eq!(
        lines(&model, &trace),
        strs(&[
            "ext Customer -> o1 submit",
            "o1 draft -> pending",
            "ext Clock -> o1 timeout",
            "o1 pending -> cancelled",
            "o1 emit OrderCancelled",
            "ext PaymentGateway -> o1 capture_ok",
            "o1 drop capture_ok @ cancelled",
        ])
    );
    assert_eq!(finals(&model, &trace), ["o1: cancelled", "s1: idle"]);
}

const SAGA: &str = include_str!("../../../examples/checkout-saga/cascade.yaml");
const SAGA_HAPPY: &str = include_str!("../../../examples/checkout-saga/scenarios/happy-path.yaml");
const SAGA_DECLINED: &str = include_str!("../../../examples/checkout-saga/scenarios/declined.yaml");
const SAGA_TIMEOUT: &str = include_str!("../../../examples/checkout-saga/scenarios/timeout.yaml");

/// Batch runs and play sessions started from the scenario agree.
fn saga_trace(text: &str) -> (cascade_core::Model, cascade_sim::Trace) {
    let model = model(SAGA);
    let trace = run(&model, text);
    match cascade_sim::PlaySession::from_scenario(&model, &common::scenario(text)) {
        Ok(session) => assert_eq!(session.trace(), &trace),
        Err(err) => panic!("{err}"),
    }
    (model, trace)
}

#[test]
fn saga_happy_path() {
    let (model, trace) = saga_trace(SAGA_HAPPY);
    let lines = lines(&model, &trace);
    for expected in [
        "Saga spawn checkout1",
        "checkout1 opening -> waiting",
        "p1 choice [approved, card declined] approved",
        "checkout1 notify PaymentCaptured",
        "checkout1 waiting -> paid",
        r#"checkout1 sets {"paid": "50"}"#,
        "checkout1 notify StockReserved",
        "checkout1 paid -> done",
        "o1 notify CheckoutCompleted",
    ] {
        assert!(lines.contains(&expected.to_owned()), "missing `{expected}` in {lines:#?}");
    }
    assert_eq!(finals(&model, &trace), ["o1: confirmed", "p1: captured", "s1: reserved", "checkout1: done"]);
}

#[test]
fn saga_card_declined() {
    let (model, trace) = saga_trace(SAGA_DECLINED);
    assert!(lines(&model, &trace).contains(&"p1 choice [approved, card declined] card declined".to_owned()));
    assert_eq!(finals(&model, &trace), ["o1: cancelled", "p1: declined", "s1: idle", "checkout1: failed"]);
}

#[test]
fn saga_timeout() {
    let (model, trace) = saga_trace(SAGA_TIMEOUT);
    let lines = lines(&model, &trace);
    let wait = lines.iter().position(|l| l == "wait 30m -> 30m");
    assert!(wait.is_some(), "{lines:#?}");
    let after = &lines[wait.unwrap_or_default()..];
    assert_eq!(
        after[..6],
        strs(&[
            "wait 30m -> 30m",
            "checkout1 timer after-30m",
            "checkout1 waiting -> expired",
            "checkout1 emit CheckoutFailed",
            "o1 notify CheckoutFailed",
            "o1 placed -> cancelled",
        ])
    );
    // The late approval is ignored by the expired checkout.
    assert!(!lines.contains(&"checkout1 notify PaymentCaptured".to_owned()));
    assert_eq!(finals(&model, &trace), ["o1: cancelled", "p1: captured", "s1: reserved", "checkout1: expired"]);
}
