//! Choice points: `?choice` guards and conditions, pending decisions, the
//! `choose` directive and `PlayAction::Choose`, unused decisions, and the
//! choice-point preview.

mod common;
mod support;

use cascade_sim::{
    ChoicePoint, PlayAction, PlaySession, ScenarioErrorKind, SimError, TraceStepKind, scenario_to_yaml, simulate,
};
use common::{causes, has, lines, model, run, run_err, scenario, strs};
use support::{add, apply, apply_err, fire, from_scenario, session, unnamed};

const PAYMENT: &str = r#"
machines:
  Payment:
    initial: authorizing
    states: [authorizing, captured, declined, review]
    transitions:
      - { from: authorizing, to: captured, on: respond, guard: "?approved" }
      - { from: authorizing, to: declined, on: respond, guard: "?card declined" }
      - { from: authorizing, to: review,   on: respond, guard: else }
external:
  Bank: [Payment.respond]
"#;

fn respond(before: &str) -> String {
    format!("scenario: c\ninstances:\n  p: Payment\nsteps:\n{before}  - {{ source: Bank, fire: Payment.respond }}\n")
}

fn choose(option: &str) -> PlayAction {
    PlayAction::Choose { option: option.to_owned() }
}

#[test]
fn without_a_decision_the_first_option_is_taken_and_marked_default() {
    let model = model(PAYMENT);
    let trace = run(&model, &respond(""));
    assert_eq!(
        lines(&model, &trace),
        strs(&[
            "ext Bank -> p respond",
            "p choice [approved, card declined, else] approved default",
            "p authorizing -> captured",
        ])
    );
    // The choice is caused by the delivering step and causes the transition.
    assert_eq!(causes(&trace), [None, Some(0), Some(1)]);
}

#[test]
fn a_choose_directive_decides_the_next_choice_point() {
    let model = model(PAYMENT);
    let trace = run(&model, &respond("  - { choose: \"card declined\" }\n"));
    assert_eq!(
        lines(&model, &trace)[1..],
        strs(&["p choice [approved, card declined, else] card declined", "p authorizing -> declined"])
    );
    match &trace.steps[1].kind {
        TraceStepKind::Choice { chosen, defaulted, .. } => {
            assert_eq!(chosen, "card declined");
            assert!(!defaulted);
        }
        other => panic!("{other:?}"),
    }
    let trace = run(&model, &respond("  - { choose: else }\n"));
    assert_eq!(lines(&model, &trace)[2], "p authorizing -> review");
}

#[test]
fn the_first_pending_decision_that_is_an_option_is_taken() {
    let model = model(PAYMENT);
    // `elsewhere` is not an option here, so `card declined` is taken and
    // `elsewhere` stays pending; at the end of a batch run that is an error at
    // its directive.
    let text = respond("  - { choose: elsewhere }\n  - { choose: \"card declined\" }\n");
    let err = run_err(&model, &text);
    let SimError::Scenario(err) = err else { panic!("{err}") };
    assert_eq!(err.diagnostics.len(), 1, "{err}");
    assert!(matches!(&err.diagnostics[0].kind, ScenarioErrorKind::UnusedChoice { option } if option == "elsewhere"));
    assert_eq!(err.diagnostics[0].span.start.line, 5);

    // A session keeps it pending.
    let session = from_scenario(&model, &scenario(&text));
    assert_eq!(session.pending_decisions(), ["elsewhere"]);
    assert_eq!(common::finals(&model, session.trace()), ["p: declined"]);
}

#[test]
fn decisions_are_taken_in_push_order() {
    let model = model(PAYMENT);
    let text = "scenario: c\ninstances:\n  p: Payment\n  q: Payment\nsteps:\n  - { choose: else }\n  - { choose: approved }\n  - { source: Bank, fire: Payment.respond, target: p }\n  - { source: Bank, fire: Payment.respond, target: q }\n";
    let trace = run(&model, text);
    assert_eq!(common::finals(&model, &trace), ["p: review", "q: captured"]);
}

#[test]
fn a_single_remaining_candidate_is_not_a_choice_point() {
    let model = model(
        r#"
machines:
  Payment:
    initial: a
    states: [a, b]
    transitions:
      - { from: a, to: b, on: go, guard: "?lucky" }
external:
  Bank: [Payment.go]
"#,
    );
    let trace = run(&model, "scenario: c\ninstances:\n  p: Payment\nsteps:\n  - { source: Bank, fire: Payment.go }\n");
    assert_eq!(lines(&model, &trace), strs(&["ext Bank -> p go", "p a -> b"]));
}

#[test]
fn a_holding_expression_next_to_choices_is_one_of_the_options() {
    let model = model(
        r#"
machines:
  Payment:
    initial: a
    states: [a, b, c, d]
    transitions:
      - { from: a, to: b, on: go, guard: "event.vip == true" }
      - { from: a, to: c, on: go, guard: "?lucky" }
      - { from: a, to: d, on: go }
external:
  Bank: [Payment.go]
"#,
    );
    let text = |vip: &str, before: &str| {
        format!(
            "scenario: c\ninstances:\n  p: Payment\nsteps:\n{before}  - {{ source: Bank, fire: Payment.go, payload: {{ vip: {vip} }} }}\n"
        )
    };
    let trace = run(&model, &text("true", ""));
    assert_eq!(lines(&model, &trace)[1], "p choice [event.vip == true, lucky, unguarded] event.vip == true default");
    // A failing expression is not an option.
    let trace = run(&model, &text("false", "  - { choose: unguarded }\n"));
    assert_eq!(lines(&model, &trace)[1..], strs(&["p choice [lucky, unguarded] unguarded", "p a -> d"]));
}

const GIFT: &str = r#"
machines:
  Order:
    initial: open
    states: [open, paid]
    transitions:
      - { from: open, to: paid, on: pay, emits: [Paid] }
  Shipment:
    initial: idle
    states: [idle, picking]
    transitions:
      - { from: idle, to: picking, on: start }
controllers:
  Fulfillment:
    on:
      Paid: { fire: Shipment.start, target: Shipment, when: "?not a gift card" }
external:
  Api: [Order.pay]
"#;

#[test]
fn a_choice_condition_is_decided_at_the_controller() {
    let model = model(GIFT);
    let base = "scenario: g\ninstances:\n  o: Order\n  s: Shipment\nsteps:\n";
    let trace = run(&model, &format!("{base}  - {{ source: Api, fire: Order.pay }}\n"));
    assert_eq!(
        lines(&model, &trace),
        strs(&[
            "ext Api -> o pay",
            "o open -> paid",
            "o emit Paid",
            "Fulfillment <- Paid",
            "Fulfillment choice [not a gift card, not not a gift card] not a gift card default",
            "Fulfillment fire s start",
            "s idle -> picking",
        ])
    );
    assert_eq!(causes(&trace)[4..6], [Some(3), Some(4)]);

    let trace = run(
        &model,
        &format!("{base}  - {{ choose: \"not not a gift card\" }}\n  - {{ source: Api, fire: Order.pay }}\n"),
    );
    assert_eq!(
        lines(&model, &trace)[3..],
        strs(&["Fulfillment <- Paid", "Fulfillment choice [not a gift card, not not a gift card] not not a gift card"])
    );
}

#[test]
fn choose_actions_push_pending_decisions_in_a_session() {
    let model = model(PAYMENT);
    let mut s = session(&model, &[add("p", "Payment", &[]), choose("card declined"), choose("else")]);
    assert_eq!(s.pending_decisions(), ["card declined", "else"]);
    // No trace step for a decision.
    assert!(s.trace().steps.is_empty());
    apply(&mut s, &model, fire("Bank", "Payment.respond", "p"));
    assert_eq!(s.pending_decisions(), ["else"]);
    assert_eq!(common::finals(&model, s.trace()), ["p: declined"]);
}

#[test]
fn choice_points_preview_a_step_without_changing_anything() {
    let model = model(GIFT);
    let mut s = session(&model, &[add("o", "Order", &[]), add("s", "Shipment", &[])]);
    // Nothing queued: the preview fails like the step would.
    assert!(matches!(s.choice_points(&model, None, None), Err(SimError::Action(ScenarioErrorKind::QueueEmpty))));
    apply(&mut s, &model, fire("Api", "Order.pay", "o"));
    apply(&mut s, &model, choose("not not a gift card"));
    let before = (s.trace().clone(), s.timeline().clone(), s.state_key());
    let points = match s.choice_points(&model, None, None) {
        Ok(points) => points,
        Err(err) => panic!("{err}"),
    };
    assert_eq!(
        points,
        [ChoicePoint { at: "Fulfillment".into(), options: strs(&["not a gift card", "not not a gift card"]) }]
    );
    assert_eq!((s.trace().clone(), s.timeline().clone(), s.state_key()), before);
    assert_eq!(s.pending_decisions(), ["not not a gift card"]);
    // Per consumer too.
    assert_eq!(s.choice_points(&model, Some(0), Some("Fulfillment")).map(|p| p.len()), Ok(1));
    assert!(s.choice_points(&model, Some(3), None).is_err());
}

#[test]
fn choice_point_preview_of_a_fire_into_choice_guards() {
    let model = model(
        r#"
machines:
  Order:
    initial: open
    states: [open, paid]
    transitions:
      - { from: open, to: paid, on: pay, emits: [Paid] }
  Payment:
    initial: authorizing
    states: [authorizing, captured, declined]
    transitions:
      - { from: authorizing, to: captured, on: respond, guard: "?approved" }
      - { from: authorizing, to: declined, on: respond, guard: "?card declined" }
controllers:
  Bank:
    on:
      Paid: { fire: Payment.respond, target: Payment }
external:
  Api: [Order.pay]
"#,
    );
    let mut s = session(&model, &[add("o", "Order", &[]), add("p", "Payment", &[]), fire("Api", "Order.pay", "o")]);
    // The event: the controller fires, no choice yet.
    assert_eq!(s.choice_points(&model, None, None), Ok(vec![]));
    apply(&mut s, &model, support::step());
    assert_eq!(
        s.choice_points(&model, None, None),
        Ok(vec![ChoicePoint { at: "p".into(), options: strs(&["approved", "card declined"]) }])
    );
}

#[test]
fn sessions_save_choose_and_replay_the_same_trace() {
    let model = model(PAYMENT);
    let s = session(&model, &[add("p", "Payment", &[]), choose("card declined"), fire("Bank", "Payment.respond", "p")]);
    let saved = match s.to_scenario("saved") {
        Ok(saved) => saved,
        Err(err) => panic!("{err}"),
    };
    let text = scenario_to_yaml(&saved);
    assert!(text.contains("  - { choose: \"card declined\" }\n"), "{text}");
    let again = scenario(&text);
    let replayed = from_scenario(&model, &again);
    assert_eq!(unnamed(replayed.trace()), unnamed(s.trace()));
    assert_eq!(replayed.timeline().actions, s.timeline().actions);
    let batch = match simulate(&model, &again) {
        Ok(trace) => trace,
        Err(err) => panic!("{err}"),
    };
    assert_eq!(unnamed(&batch), unnamed(s.trace()));
}

#[test]
fn a_rejected_step_keeps_pending_decisions() {
    let model = model(PAYMENT);
    let mut s = session(&model, &[add("p", "Payment", &[]), choose("approved")]);
    let err = apply_err(&mut s, &model, support::step());
    assert!(matches!(err, SimError::Action(ScenarioErrorKind::QueueEmpty)));
    assert_eq!(s.pending_decisions(), ["approved"]);
}

#[test]
fn unused_choices_are_reported_for_every_leftover_directive() {
    let model = model(PAYMENT);
    let text = "scenario: c\ninstances:\n  p: Payment\nsteps:\n  - { choose: x }\n  - { choose: y }\n";
    let err = run_err(&model, text);
    let SimError::Scenario(err) = err else { panic!("{err}") };
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnusedChoice { option } if option == "x")));
    let lines: Vec<u32> = err.diagnostics.iter().map(|d| d.span.start.line).collect();
    assert_eq!(lines, [5, 6]);
}

#[test]
fn a_session_started_from_a_scenario_with_unused_choices_keeps_them() {
    let model = model(PAYMENT);
    let text = "scenario: c\ninstances:\n  p: Payment\nsteps:\n  - { choose: x }\n";
    let s = match PlaySession::from_scenario(&model, &scenario(text)) {
        Ok(s) => s,
        Err(err) => panic!("{err}"),
    };
    assert_eq!(s.pending_decisions(), ["x"]);
}

#[test]
fn choice_points_of_an_external_fire_preview_its_guards() {
    let model = model(
        r#"
machines:
  Payment:
    initial: authorizing
    states: [authorizing, captured, declined]
    transitions:
      - { from: authorizing, to: captured, on: respond, guard: "?approved" }
      - { from: authorizing, to: declined, on: respond, guard: "?card declined" }
external:
  Bank: [Payment.respond]
"#,
    );
    let mut s = session(&model, &[add("p", "Payment", &[])]);
    let respond = fire("Bank", "Payment.respond", "p");
    let before = (s.trace().clone(), s.timeline().clone(), s.state_key());
    assert_eq!(
        s.choice_points_of(&model, &respond),
        Ok(vec![ChoicePoint { at: "p".into(), options: strs(&["approved", "card declined"]) }])
    );
    assert_eq!((s.trace().clone(), s.timeline().clone(), s.state_key()), before, "a preview changes nothing");
    // A pending decision still shows the point (the caller matches it).
    apply(&mut s, &model, choose("card declined"));
    assert_eq!(s.choice_points_of(&model, &respond).map(|p| p.len()), Ok(1));
    // Fails like the fire would.
    assert!(s.choice_points_of(&model, &fire("Bank", "Payment.respond", "nobody")).is_err());
    // A step is the same as `choice_points`.
    let step = PlayAction::Step { choice: None, consumer: None };
    assert_eq!(s.choice_points_of(&model, &step), s.choice_points(&model, None, None));
}
