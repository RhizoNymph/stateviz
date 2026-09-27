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
