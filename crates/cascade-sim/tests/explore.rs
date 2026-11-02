//! Exploration: orderings, choices, timers, bounds, witnesses and notes.

mod explore_support;

use cascade_sim::{
    Anomaly, ExploreOptions, OrderingModel, PlayAction, PlaySession, explore, explore_from, replay_witness,
};
use explore_support::*;

fn per_consumer() -> ExploreOptions {
    ExploreOptions::default()
}

#[test]
fn defaults_match_the_contract() {
    let options = ExploreOptions::default();
    assert_eq!(options.ordering, OrderingModel::PerConsumer);
    assert!(!options.choices);
    assert!(!options.timers_anytime);
    assert_eq!(options.max_states, 100_000);
    assert!(options.reduction);
}

#[test]
fn the_race_fixture_has_two_outcomes() {
    let model = model(RACE);
    let exploration = run(&model, RACE_SCENARIO, &per_consumer());
    assert!(exploration.complete);
    assert_eq!(exploration.outcomes.len(), 2, "{exploration:#?}");
    let finals: Vec<Vec<String>> = exploration.outcomes.iter().map(|o| states(&model, o)).collect();
    assert!(finals.contains(&vec!["o1: paid".to_owned(), "s1: picking".to_owned()]));
    assert!(finals.contains(&vec!["o1: paid".to_owned(), "s1: on_hold".to_owned()]));
    for outcome in &exploration.outcomes {
        // The fire that lost is dropped.
        assert_eq!(outcome.anomalies.len(), 1, "{:?}", outcome.anomalies);
        assert!(matches!(outcome.anomalies.first(), Some(Anomaly::Dropped { instance, .. }) if instance == "s1"));
        assert!(outcome.paths >= 1);
    }
    check_witnesses(&model, &PlaySession::new(&model), &exploration, "race");
}

#[test]
fn fifo_is_one_of_the_outcomes() {
    let model = model(RACE);
    let trace = cascade_sim::simulate(&model, &scenario(RACE_SCENARIO)).unwrap_or_else(|e| panic!("{e}"));
    let exploration = run(&model, RACE_SCENARIO, &per_consumer());
    let fifo: Vec<String> = trace
        .final_states
        .iter()
        .map(|(ix, state)| match &trace.lifelines[ix.index()] {
            cascade_sim::Lifeline::Instance { name, .. } => format!("{name}: {}", model.state(*state).path),
            other => panic!("{other:?}"),
        })
        .collect();
    let mut fifo = fifo;
    fifo.sort();
    assert!(exploration.outcomes.iter().any(|o| states(&model, o) == fifo));
}

/// #30: one event, two controllers; one spawns the shipment the other's
/// rule targets.
const HANDLER_ORDER: &str = r#"
machines:
  Order:
    initial: pending
    fields: [orderId]
    states: [pending, paid]
    transitions:
      - { from: pending, to: paid, on: capture_ok, emits: [OrderPaid] }
  Shipment:
    initial: idle
    fields: [orderId]
    states: [idle, picking]
    transitions:
      - { from: idle, to: picking, on: start }
      - { from: idle, to: idle, on: flag }
      - { from: picking, to: picking, on: flag }
controllers:
  Fulfillment:
    on:
      OrderPaid:
        - fire: Shipment.start
          target: new Shipment with orderId = event.orderId
  Audit:
    on:
      OrderPaid:
        - fire: Shipment.flag
          target: Shipment where orderId == event.orderId
external:
  PaymentGateway: [Order.capture_ok]
"#;

const HANDLER_ORDER_SCENARIO: &str = r#"
scenario: paid
instances:
  o1: { machine: Order, fields: { orderId: "1" } }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
"#;

#[test]
fn handler_order_dependence_shows_a_missing_target() {
    let model = model(HANDLER_ORDER);
    let exploration = run(&model, HANDLER_ORDER_SCENARIO, &per_consumer());
    assert!(exploration.complete);
    assert_eq!(exploration.outcomes.len(), 2, "{exploration:#?}");
    let no_target: Vec<_> = exploration
        .outcomes
        .iter()
        .filter(|o| o.anomalies.iter().any(|a| matches!(a, Anomaly::NoTarget { rule } if rule.starts_with("Audit"))))
        .collect();
    assert_eq!(no_target.len(), 1);
    for outcome in &exploration.outcomes {
        assert_eq!(states(&model, outcome), ["o1: paid", "shipment1: picking"]);
    }
    check_witnesses(&model, &PlaySession::new(&model), &exploration, "handler order");
}

/// A payment races the order's 30-minute timeout.
const TIMEOUT: &str = r#"
machines:
  Order:
    initial: draft
    fields: [orderId]
    states: [draft, placed, confirmed, expired]
    transitions:
      - { from: draft, to: placed, on: place }
      - { from: placed, to: confirmed, on: confirm }
      - { from: placed, to: expired, after: 30m }
  Payment:
    initial: idle
    fields: [orderId]
    states: [idle, paid]
    transitions:
      - { from: idle, to: paid, on: pay, emits: [Paid] }
controllers:
  Billing:
    on:
      Paid:
        - fire: Order.confirm
          target: Order where orderId == event.orderId
external:
  Customer: [Order.place]
  Bank: [Payment.pay]
"#;

const TIMEOUT_SCENARIO: &str = r#"
scenario: pay in time
instances:
  o1: { machine: Order, fields: { orderId: "1" } }
  p1: { machine: Payment, fields: { orderId: "1" } }
steps:
  - { source: Customer, fire: Order.place, target: o1 }
  - { source: Bank, fire: Payment.pay, target: p1 }
"#;

#[test]
fn timers_only_expire_when_the_scenario_waits() {
    let model = model(TIMEOUT);
    let exploration = run(&model, TIMEOUT_SCENARIO, &per_consumer());
    assert_eq!(exploration.outcomes.len(), 1, "{exploration:#?}");
    assert_eq!(states(&model, &exploration.outcomes[0]), ["o1: confirmed", "p1: paid"]);
    assert!(exploration.outcomes[0].timers.is_empty());
}

#[test]
fn a_timer_can_race_a_cascade() {
    let model = model(TIMEOUT);
    let options = ExploreOptions { timers_anytime: true, ..ExploreOptions::default() };
    let exploration = run(&model, TIMEOUT_SCENARIO, &options);
    assert!(exploration.complete);
    let finals: std::collections::BTreeSet<Vec<String>> =
        exploration.outcomes.iter().map(|o| states(&model, o)).collect();
    assert_eq!(finals.len(), 2, "{exploration:#?}");
    // Confirmed before the deadline, the timer is cancelled; confirmed after
    // it expired but before its delivery, the late timer is dropped.
    assert_eq!(exploration.outcomes.len(), 3, "{exploration:#?}");
    assert!(finals.contains(&vec!["o1: confirmed".to_owned(), "p1: paid".to_owned()]));
    assert!(finals.contains(&vec!["o1: expired".to_owned(), "p1: paid".to_owned()]));
    let expired = exploration.outcomes.iter().find(|o| states(&model, o)[0] == "o1: expired");
    let expired = expired.unwrap_or_else(|| panic!("no expired outcome"));
    assert!(expired.anomalies.contains(&Anomaly::Dropped {
        instance: "o1".into(),
        trigger: "confirm".into(),
        state: "expired".into()
    }));
    assert!(expired.witness.iter().any(|a| matches!(a, PlayAction::Advance { by: None })));
    check_witnesses(&model, &PlaySession::new(&model), &exploration, "timer race");
}

#[test]
fn timers_left_armed_are_part_of_the_outcome() {
    let model = model(TIMEOUT);
    let only_place = r#"
scenario: placed
instances:
  o1: { machine: Order, fields: { orderId: "1" } }
steps:
  - { source: Customer, fire: Order.place, target: o1 }
"#;
    let exploration = run(&model, only_place, &per_consumer());
    assert_eq!(exploration.outcomes.len(), 1);
    let timers = &exploration.outcomes[0].timers;
    assert_eq!(timers.len(), 1);
    assert_eq!(timers[0].instance, "o1");
    assert_eq!(timers[0].remaining.to_string(), "30m");
    // With timers expiring at any time, the timer must run out.
    let anytime = ExploreOptions { timers_anytime: true, ..ExploreOptions::default() };
    let exploration = run(&model, only_place, &anytime);
    assert_eq!(exploration.outcomes.len(), 1);
    assert_eq!(states(&model, &exploration.outcomes[0]), ["o1: expired"]);
    assert!(exploration.outcomes[0].timers.is_empty());
}

#[test]
fn choices_branch_on_every_option() {
    let model = model(SAGA);
    let happy = SAGA_SCENARIOS[0];
    let plain = run(&model, happy, &per_consumer());
    let with_choices = run(&model, happy, &ExploreOptions { choices: true, ..ExploreOptions::default() });
    assert!(with_choices.outcomes.len() > plain.outcomes.len(), "{with_choices:#?}");
    assert!(with_choices.notes.iter().any(|n| n.contains("1 choose decision was ignored")), "{:?}", with_choices.notes);
    let payment_states: std::collections::BTreeSet<String> = with_choices
        .outcomes
        .iter()
        .filter_map(|o| o.instances.get("p1").map(|(s, _)| model.state(*s).path.clone()))
        .collect();
    assert!(payment_states.contains("captured") && payment_states.contains("declined"), "{payment_states:?}");
    // The branch is taken by pushing the option right before the step.
    assert!(
        with_choices
            .outcomes
            .iter()
            .any(|o| o.witness.iter().any(|a| matches!(a, PlayAction::Choose { option } if option == "card declined")))
    );
    check_witnesses(&model, &PlaySession::new(&model), &with_choices, "saga choices");
}

#[test]
fn scenario_decisions_are_honoured_without_choices() {
    let model = model(SAGA);
    let declined = run(&model, SAGA_SCENARIOS[1], &per_consumer());
    assert!(!declined.outcomes.is_empty());
    for outcome in &declined.outcomes {
        assert_eq!(outcome.instances.get("p1").map(|(s, _)| model.state(*s).path.as_str()), Some("declined"));
    }
}

#[test]
fn unused_decisions_are_anomalies() {
    let model = model(RACE);
    let text = r#"
scenario: decided for nothing
instances:
  o1: { machine: Order, fields: { orderId: "1" } }
  s1: { machine: Shipment, fields: { orderId: "1" } }
steps:
  - { choose: never offered }
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
"#;
    let exploration = run(&model, text, &per_consumer());
    for outcome in &exploration.outcomes {
        assert!(outcome.anomalies.contains(&Anomaly::UnusedChoice { option: "never offered".into() }));
    }
    check_witnesses(&model, &PlaySession::new(&model), &exploration, "unused");
}

#[test]
fn the_state_bound_makes_it_incomplete() {
    let model = model(SHOP);
    let options = ExploreOptions { max_states: 5, ..ExploreOptions::default() };
    let exploration = run(&model, SHOP_SCENARIO, &options);
    assert!(!exploration.complete);
    assert!(exploration.states <= 5);
    check_witnesses(&model, &PlaySession::new(&model), &exploration, "bounded");
}

#[test]
fn output_is_deterministic_and_sorted() {
    for (definition, scenario_text) in corpus() {
        let model = model(definition);
        for options in option_grid() {
            let a = run(&model, scenario_text, &options);
            let b = run(&model, scenario_text, &options);
            assert_eq!(a, b);
            let keys: Vec<(usize, String)> = a.outcomes.iter().map(|o| (o.witness.len(), o.sort_text())).collect();
            let mut sorted = keys.clone();
            sorted.sort();
            assert_eq!(keys, sorted);
        }
    }
}

#[test]
fn manual_queue_directives_are_ignored_with_a_note() {
    let model = model(ORDERS);
    let second_first = ORDERS_SCENARIOS[1];
    let exploration = run(&model, second_first, &per_consumer());
    assert_eq!(exploration.notes.len(), 1, "{:?}", exploration.notes);
    assert!(exploration.notes[0].contains("2 manual queue directives were ignored"), "{:?}", exploration.notes);
    // Both orders still get their shipments started in every ordering.
    for outcome in &exploration.outcomes {
        assert_eq!(states(&model, outcome), ["o1: paid", "o2: paid", "s1: picking", "s2: picking"]);
    }
    let happy = run(&model, ORDERS_SCENARIOS[0], &per_consumer());
    assert!(happy.notes.is_empty());
}

#[test]
fn an_input_that_cannot_apply_is_skipped() {
    let model = model(HANDLER_ORDER);
    // shipment1 only exists once Fulfillment has handled OrderPaid; the
    // immediate fire may come before that.
    let def = HANDLER_ORDER.replace(
        "PaymentGateway: [Order.capture_ok]",
        "PaymentGateway: [Order.capture_ok]\n  Warehouse: [Shipment.start]",
    );
    let model2 = explore_support::model(&def);
    let text = r#"
scenario: early start
instances:
  o1: { machine: Order, fields: { orderId: "1" } }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
  - { source: Warehouse, fire: Shipment.start, target: shipment1, timing: immediate }
"#;
    let _ = model;
    let exploration = run(&model2, text, &per_consumer());
    assert!(exploration.outcomes.iter().any(|o| {
        o.anomalies.iter().any(
            |a| matches!(a, Anomaly::InputFailed { input, .. } if input.contains("Warehouse fires Shipment.start")),
        )
    }));
    assert!(exploration.outcomes.iter().any(|o| !o.anomalies.iter().any(|a| matches!(a, Anomaly::InputFailed { .. }))));
    check_witnesses(&model2, &PlaySession::new(&model2), &exploration, "skipped input");
}

#[test]
fn unordered_finds_at_least_what_per_consumer_finds() {
    for (definition, scenario_text) in corpus() {
        let model = model(definition);
        let per = run(&model, scenario_text, &per_consumer());
        let any = run(&model, scenario_text, &ExploreOptions { ordering: OrderingModel::Unordered, ..per_consumer() });
        if per.complete && any.complete {
            assert!(identities(&per).is_subset(&identities(&any)), "{}", scenario(scenario_text).name);
        }
    }
}

#[test]
fn unordered_lets_fires_overtake_each_other() {
    // One controller fires twice at one instance: in order under
    // per-consumer delivery, either way when unordered.
    let def = r#"
machines:
  Order:
    initial: pending
    states: [pending, paid]
    transitions:
      - { from: pending, to: paid, on: pay, emits: [Paid] }
  Box:
    initial: a
    states: [a, b, c]
    transitions:
      - { from: a, to: b, on: first }
      - { from: a, to: c, on: second }
controllers:
  Packer:
    on:
      Paid:
        - { fire: Box.first, target: Box }
        - { fire: Box.second, target: Box }
external:
  Shop: [Order.pay]
"#;
    let text = r#"
scenario: pay
instances:
  o1: Order
  b1: Box
steps:
  - { source: Shop, fire: Order.pay, target: o1 }
"#;
    let model = model(def);
    let per = run(&model, text, &per_consumer());
    assert_eq!(per.outcomes.len(), 1);
    let any = run(&model, text, &ExploreOptions { ordering: OrderingModel::Unordered, ..per_consumer() });
    assert_eq!(any.outcomes.len(), 2);
    check_witnesses(&model, &PlaySession::new(&model), &any, "unordered");
}

#[test]
fn explore_from_a_session_mid_cascade() {
    let model = model(RACE);
    let mut session = PlaySession::from_scenario(
        &model,
        &scenario(
            r#"
scenario: paused
instances:
  o1: { machine: Order, fields: { orderId: "1" } }
  s1: { machine: Shipment, fields: { orderId: "1" } }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
end: pause
"#,
        ),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(session.pending().len(), 1);
    let exploration = explore_from(&session, &model, &per_consumer()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(exploration.outcomes.len(), 2);
    check_witnesses(&model, &session, &exploration, "from a session");
    // A host opens a witness as a branch: seek back, apply it.
    let position = session.timeline().position;
    let witness = &exploration.outcomes[1].witness;
    let branched = replay_witness(&session, &model, witness).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(branched.timeline().actions.len(), position + witness.len());
    for action in witness {
        session.apply(&model, action.clone()).unwrap_or_else(|e| panic!("{e}"));
    }
    assert_eq!(session.instances(), branched.instances());
}

#[test]
fn explore_from_an_empty_session_has_one_empty_outcome() {
    let model = model(RACE);
    let session = PlaySession::new(&model);
    let exploration = explore_from(&session, &model, &per_consumer()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(exploration.outcomes.len(), 1);
    assert!(exploration.outcomes[0].witness.is_empty());
    assert!(exploration.outcomes[0].instances.is_empty());
    assert_eq!(exploration.states, 1);
}

#[test]
fn scenario_errors_are_reported() {
    let model = model(RACE);
    let text = r#"
scenario: bad
instances:
  o1: { machine: Nope }
steps: []
"#;
    assert!(matches!(explore(&model, &scenario(text), &per_consumer()), Err(cascade_sim::SimError::Scenario(_))));
}

#[test]
fn witnesses_save_as_scenarios_that_simulate_to_the_outcome() {
    let model = model(RACE);
    let exploration = run(&model, RACE_SCENARIO, &per_consumer());
    for outcome in &exploration.outcomes {
        let session =
            replay_witness(&PlaySession::new(&model), &model, &outcome.witness).unwrap_or_else(|e| panic!("{e}"));
        let saved = session.to_scenario("witness").unwrap_or_else(|e| panic!("{e}"));
        let yaml = cascade_sim::scenario_to_yaml(&saved);
        let trace = cascade_sim::simulate(&model, &scenario(&yaml)).unwrap_or_else(|e| panic!("{e}\n{yaml}"));
        let finals: Vec<String> = trace
            .final_states
            .iter()
            .map(|(ix, state)| match &trace.lifelines[ix.index()] {
                cascade_sim::Lifeline::Instance { name, .. } => format!("{name}: {}", model.state(*state).path),
                other => panic!("{other:?}"),
            })
            .collect();
        let mut finals = finals;
        finals.sort();
        assert_eq!(finals, states(&model, outcome), "{yaml}");
    }
}

#[test]
fn a_cascade_that_never_settles_has_no_outcome_and_a_note() {
    let def = r#"
machines:
  Ping:
    states: [a, b]
    transitions:
      - { from: a, to: b, on: go, emits: [Tick] }
      - { from: b, to: a, on: go, emits: [Tick] }
controllers:
  Loop:
    on:
      Tick: { fire: Ping.go, target: Ping }
external:
  Starter: [Ping.go]
"#;
    let text = r#"
scenario: forever
instances:
  p1: Ping
steps:
  - { source: Starter, fire: Ping.go, target: p1 }
"#;
    let model = model(def);
    let exploration = run(&model, text, &per_consumer());
    assert!(exploration.complete);
    assert!(exploration.outcomes.is_empty());
    assert!(exploration.notes.iter().any(|n| n.starts_with("no ordering settles")), "{:?}", exploration.notes);
    assert!(exploration.states < 10);
}
