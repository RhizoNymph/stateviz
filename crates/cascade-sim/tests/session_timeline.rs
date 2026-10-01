//! The session timeline: seeking, branching and replaying after an edit.

mod common;
mod support;

use cascade_core::Model;
use cascade_sim::{Branch, PlayAction, PlaySession, ScenarioErrorKind, SimError, Trace};
use common::{finals, lines, model, strs};
use support::{ORDERS, RACE, add, apply, apply_err, choose, fire, run_all, session, step};

fn race_line() -> Vec<PlayAction> {
    vec![
        add("o1", "Order", &[("orderId", "1")]),
        add("s1", "Shipment", &[("orderId", "1")]),
        fire("PaymentGateway", "Order.capture_ok", "o1"),
        step(),
        step(),
        step(),
    ]
}

/// The trace after each prefix of `actions`, from a fresh session each time.
fn traces_by_position(model: &Model, actions: &[PlayAction]) -> Vec<Trace> {
    (0..=actions.len()).map(|n| session(model, &actions[..n]).trace().clone()).collect()
}

fn seek(session: &mut PlaySession, model: &Model, position: usize) {
    if let Err(err) = session.seek(model, position) {
        panic!("expected to seek to {position}:\n{err}");
    }
}

fn switch(session: &mut PlaySession, model: &Model, index: usize) {
    if let Err(err) = session.switch_branch(model, index) {
        panic!("expected to switch to branch {index}:\n{err}");
    }
}

#[test]
fn seeking_rewinds_and_fast_forwards() {
    let model = model(RACE);
    let actions = race_line();
    let expected = traces_by_position(&model, &actions);
    let mut session = session(&model, &actions);
    for position in [3, 0, 6, 4, 4, 1, 5, 2, 6] {
        seek(&mut session, &model, position);
        assert_eq!(session.timeline().position, position);
        assert_eq!(session.trace(), &expected[position], "at {position}");
        // Seeking never loses the future.
        assert_eq!(session.timeline().actions, actions);
        assert!(session.timeline().branches.is_empty());
    }
    seek(&mut session, &model, 4);
    // OrderPaid was delivered; both controllers' fires are queued.
    assert_eq!(session.pending().len(), 2);
    assert_eq!(session.instances().len(), 2);
}

#[test]
fn seeking_past_the_end_is_an_error() {
    let model = model(RACE);
    let mut session = session(&model, &race_line());
    seek(&mut session, &model, 2);
    assert_eq!(session.seek(&model, 7), Err(SimError::NoSuchPosition { position: 7, len: 6 }));
    assert_eq!(session.timeline().position, 2);
}

#[test]
fn acting_after_seeking_back_saves_the_old_future_as_a_branch() {
    let model = model(RACE);
    let old = race_line();
    let mut session = session(&model, &old);
    let old_trace = session.trace().clone();

    seek(&mut session, &model, 4);
    apply(&mut session, &model, choose(1));
    let timeline = session.timeline();
    assert_eq!(timeline.position, 5);
    assert_eq!(timeline.actions.len(), 5);
    assert_eq!(timeline.actions[..4], old[..4]);
    assert_eq!(timeline.actions[4], choose(1));
    assert_eq!(timeline.branches, [Branch { fork: 4, actions: old.clone() }]);
    assert_eq!(finals(&model, session.trace()), ["o1: paid", "s1: on_hold"]);

    // Back to the saved future, positioned at its end.
    switch(&mut session, &model, 0);
    assert_eq!(session.timeline().actions, old);
    assert_eq!(session.timeline().position, old.len());
    assert_eq!(session.trace(), &old_trace);
    let mut new_line = old[..4].to_vec();
    new_line.push(choose(1));
    assert_eq!(session.timeline().branches, [Branch { fork: 4, actions: new_line.clone() }]);

    // And back again.
    switch(&mut session, &model, 0);
    assert_eq!(session.timeline().actions, new_line);
    assert_eq!(finals(&model, session.trace()), ["o1: paid", "s1: on_hold"]);
}

#[test]
fn repeating_the_next_action_moves_forward_without_branching() {
    let model = model(RACE);
    let actions = race_line();
    let mut session = session(&model, &actions);
    seek(&mut session, &model, 3);
    apply(&mut session, &model, step());
    assert_eq!(session.timeline().position, 4);
    assert_eq!(session.timeline().actions, actions);
    assert!(session.timeline().branches.is_empty());
}

#[test]
fn a_rejected_action_after_seeking_keeps_the_future() {
    let model = model(RACE);
    let actions = race_line();
    let mut session = session(&model, &actions);
    seek(&mut session, &model, 2);
    // The queue is empty at position 2.
    assert!(matches!(apply_err(&mut session, &model, step()), SimError::Action(ScenarioErrorKind::QueueEmpty)));
    assert_eq!(session.timeline().actions, actions);
    assert!(session.timeline().branches.is_empty());
    assert_eq!(session.timeline().position, 2);
}

#[test]
fn switching_to_a_missing_branch_is_an_error() {
    let model = model(RACE);
    let mut session = session(&model, &race_line());
    assert_eq!(session.switch_branch(&model, 0), Err(SimError::NoSuchBranch { index: 0, count: 0 }));
}

#[test]
fn several_branches_are_kept() {
    let model = model(RACE);
    let mut session = session(&model, &race_line());
    seek(&mut session, &model, 4);
    apply(&mut session, &model, choose(1));
    seek(&mut session, &model, 2);
    apply(&mut session, &model, run_all());
    let branches = &session.timeline().branches;
    assert_eq!(branches.len(), 2);
    assert_eq!(branches[0].fork, 4);
    assert_eq!(branches[1].fork, 2);
    assert_eq!(branches[1].actions.len(), 5);
    assert_eq!(session.timeline().actions.len(), 3);

    switch(&mut session, &model, 0);
    assert_eq!(session.timeline().actions, race_line());
    // The line we left takes the branch's place.
    assert_eq!(session.timeline().branches[0].actions.len(), 3);
    assert_eq!(session.timeline().branches[0].fork, 2);
}

// --- Replaying after an edit ---------------------------------------------------

fn order_line() -> Vec<PlayAction> {
    vec![
        add("o1", "Order", &[("orderId", "1")]),
        add("s1", "Shipment", &[("orderId", "1")]),
        fire("Customer", "Order.submit", "o1"),
        fire("PaymentGateway", "Order.capture_ok", "o1"),
        step(),
        step(),
    ]
}

#[test]
fn replaying_against_the_same_model_reproduces_the_session() {
    let model = model(ORDERS);
    let mut session = session(&model, &order_line());
    seek(&mut session, &model, 5);
    let (replayed, failure) = session.replay(&model);
    assert_eq!(failure, None);
    assert_eq!(replayed.trace(), session.trace());
    assert_eq!(replayed.timeline(), session.timeline());
    assert_eq!(replayed.pending(), session.pending());
}

#[test]
fn replay_follows_an_edited_transition() {
    let model = model(ORDERS);
    let session = session(&model, &order_line());
    // Capturing the payment now cancels the order (and still emits OrderPaid).
    let edited = common::model(&ORDERS.replace("to: paid,      on: capture_ok", "to: cancelled, on: capture_ok"));
    let (replayed, failure) = session.replay(&edited);
    assert_eq!(failure, None);
    assert_eq!(finals(&edited, replayed.trace()), ["o1: cancelled", "s1: picking"]);
}

#[test]
fn replay_stops_at_the_first_action_that_no_longer_applies() {
    let model = model(ORDERS);
    let session = session(&model, &order_line());
    // Without the capture transition the capture is dropped, nothing is
    // queued, and the first `step` has nothing to deliver.
    let edited = common::model(
        &ORDERS.replace("      - { from: pending, to: paid,      on: capture_ok, emits: [OrderPaid] }\n", ""),
    );
    let (replayed, failure) = session.replay(&edited);
    assert_eq!(failure, Some((4, SimError::Action(ScenarioErrorKind::QueueEmpty))));
    assert_eq!(replayed.timeline().position, 4);
    // The rest of the line is kept, to seek into once it applies again.
    assert_eq!(replayed.timeline().actions, order_line());
    assert_eq!(
        lines(&edited, replayed.trace()),
        strs(&[
            "ext Customer -> o1 submit",
            "o1 draft -> pending",
            "ext PaymentGateway -> o1 capture_ok",
            "o1 drop capture_ok @ pending",
        ])
    );
    // The original session is untouched.
    assert_eq!(session.timeline().position, 6);
}

#[test]
fn replay_reports_names_the_edit_removed() {
    let model = model(ORDERS);
    let session = session(&model, &order_line());
    let edited = common::model(&ORDERS.replace("  Customer: [Order.submit]\n", ""));
    let (replayed, failure) = session.replay(&edited);
    assert!(
        matches!(
            &failure,
            Some((2, SimError::Action(ScenarioErrorKind::UnknownSource { name }))) if name == "Customer"
        ),
        "{failure:?}"
    );
    assert_eq!(replayed.timeline().position, 2);
    assert_eq!(replayed.instances().len(), 2);
}

#[test]
fn replay_only_goes_up_to_the_position() {
    let model = model(ORDERS);
    let mut session = session(&model, &order_line());
    seek(&mut session, &model, 3);
    // The capture (action 3) would fail, but it is in the future.
    let edited = common::model(&ORDERS.replace("  PaymentGateway: [Order.capture_ok]\n", ""));
    let (replayed, failure) = session.replay(&edited);
    assert_eq!(failure, None);
    assert_eq!(replayed.timeline().position, 3);
    assert_eq!(replayed.timeline().actions, order_line());
}
