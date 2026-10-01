//! Sessions and scenarios: a session driven by a scenario is the batch run,
//! and a session saved as a scenario replays to the same trace.

mod common;
mod support;

use cascade_core::Model;
use cascade_sim::{
    PlayAction, PlaySession, ScenarioEnd, ScenarioErrorKind, SimError, parse_scenario, scenario_to_yaml, simulate_run,
};
use common::{has, lines, model, scenario};
use support::{
    HAPPY, ORDERS, RACE, RACE_SCENARIO, SECOND_FIRST, SPAWNING, TIMEOUT_FIRST, TIMEOUT_RACE, add, add_auto, add_in,
    choose, fire, fire_with, from_scenario, remove, run_all, session, step,
};

const PLAYER: &str = r#"
machines:
  Player:
    initial: stopped
    states:
      - stopped
      - playing:
          initial: normal
          states:
            - normal
            - fast
            - deep: { kind: deep-history }
      - paused
    transitions:
      - { from: stopped, to: playing, on: play, emits: [Started] }
      - { from: playing.normal, to: playing.fast, on: ff }
      - { from: playing, to: paused, on: pause }
      - { from: paused, to: playing.deep, on: resume }
  Log:
    states: [ready]
    transitions:
      - { from: ready, to: ready, on: record }
controllers:
  Audit:
    on:
      Started: { fire: Log.record, target: all Log }
external:
  User: [Player.play, Player.ff, Player.pause, Player.resume]
"#;

const PLAYER_SCENARIO: &str = r#"scenario: history
instances:
  p1: Player
  l1: Log
  l2: Log
steps:
  - { source: User, fire: Player.play, target: p1 }
  - { source: User, fire: Player.ff, target: p1, timing: immediate }
  - { source: User, fire: Player.pause }
  - { source: User, fire: Player.resume }
"#;

const SPAWN_SCENARIO: &str = r#"scenario: spawn
instances:
  o1: { machine: Order, fields: { orderId: 1 } }
  o2: { machine: Order, fields: { orderId: 2 } }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1, payload: { note: "first one" } }
  - { source: PaymentGateway, fire: Order.capture_ok, target: o2, timing: immediate }
  - { source: Warehouse, fire: Shipment.start, target: shipment2 }
"#;

const MANUAL_RACE: &str = r#"scenario: manual race
instances:
  o1: { machine: Order, fields: { orderId: "1" } }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
  - { create: s1, machine: Shipment, fields: { orderId: "1" } }
  - step
  - { step: 1 }
  - run
  - { create: s2, machine: Shipment, state: on_hold }
  - { remove: s2 }
"#;

const PAUSED_RACE: &str = r#"scenario: paused race
instances:
  o1: { machine: Order, fields: { orderId: "1" } }
  s1: { machine: Shipment, fields: { orderId: "1" } }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
  - step
end: pause
"#;

/// Every example and fixture scenario, plus a few that exercise history,
/// spawning, payloads and the extended step kinds.
fn cases() -> Vec<(&'static str, &'static str)> {
    vec![
        (ORDERS, HAPPY),
        (ORDERS, TIMEOUT_RACE),
        (ORDERS, TIMEOUT_FIRST),
        (ORDERS, SECOND_FIRST),
        (RACE, RACE_SCENARIO),
        (PLAYER, PLAYER_SCENARIO),
        (SPAWNING, SPAWN_SCENARIO),
        (RACE, MANUAL_RACE),
        (RACE, PAUSED_RACE),
    ]
}

#[test]
fn a_session_driven_by_a_scenario_is_the_batch_run() {
    for (definition, text) in cases() {
        let model = model(definition);
        let scenario = scenario(text);
        let batch = match simulate_run(&model, &scenario) {
            Ok(run) => run,
            Err(err) => panic!("{}: {err}", scenario.name),
        };
        let session = from_scenario(&model, &scenario);
        assert_eq!(session.trace(), &batch.trace, "{}", scenario.name);
        assert_eq!(session.payloads(), &batch.payloads, "{}", scenario.name);
        assert_eq!(session.timeline().position, session.timeline().actions.len());
    }
}

#[test]
fn scenario_steps_become_actions() {
    let model = model(ORDERS);
    let session = from_scenario(&model, &scenario(TIMEOUT_RACE));
    assert_eq!(
        session.timeline().actions,
        [
            add("o1", "Order", &[("orderId", "1")]),
            add("s1", "Shipment", &[("orderId", "1")]),
            fire("Customer", "Order.submit", "o1"),
            fire("PaymentGateway", "Order.capture_ok", "o1"),
            fire("Clock", "Order.timeout", "o1"),
            // The final drain.
            PlayAction::RunUntilQuiet,
        ]
    );

    // After-quiescence steps drain first, but only when something is queued.
    let player = common::model(PLAYER);
    let session = from_scenario(&player, &scenario(PLAYER_SCENARIO));
    let kinds: Vec<&str> = session
        .timeline()
        .actions
        .iter()
        .map(|a| match a {
            PlayAction::AddInstance { .. } => "add",
            PlayAction::RemoveInstance { .. } => "remove",
            PlayAction::Fire { .. } => "fire",
            PlayAction::Step { .. } => "step",
            PlayAction::RunUntilQuiet => "run",
        })
        .collect();
    assert_eq!(kinds, ["add", "add", "add", "fire", "fire", "run", "fire", "fire"]);

    // A targetless step names the instance it found.
    assert!(session.timeline().actions.contains(&fire("User", "Player.resume", "p1")));
}

#[test]
fn from_scenario_reports_problems_with_their_positions() {
    let model = model(RACE);
    let text = "scenario: x\ninstances:\n  o1: Order\nsteps:\n  - { source: Courier, fire: Order.capture_ok }\n";
    match PlaySession::from_scenario(&model, &scenario(text)) {
        Err(SimError::Scenario(err)) => {
            assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownSource { .. })));
            assert_eq!(err.diagnostics[0].span.start.line, 5);
        }
        other => panic!("expected a scenario error, got {other:?}"),
    }
    let text = "scenario: x\ninstances:\n  o1: Order\nsteps:\n  - { step: 3 }\n";
    match PlaySession::from_scenario(&model, &scenario(text)) {
        Err(SimError::Scenario(err)) => {
            assert!(has(&err, |k| matches!(k, ScenarioErrorKind::QueueEmpty)));
            assert_eq!(err.diagnostics[0].span.start.line, 5);
        }
        other => panic!("expected a scenario error, got {other:?}"),
    }
}

/// `to_scenario` → YAML → `parse_scenario` → `from_scenario` gives the same
/// trace, payloads and actions.
fn round_trip(model: &Model, session: &PlaySession) -> String {
    let saved = match session.to_scenario("saved") {
        Ok(s) => s,
        Err(err) => panic!("expected the session to save:\n{err}"),
    };
    let yaml = scenario_to_yaml(&saved);
    let parsed = match parse_scenario(&yaml) {
        Ok(s) => s,
        Err(err) => panic!("expected the saved scenario to parse:\n{err}\n---\n{yaml}"),
    };
    let restored = match PlaySession::from_scenario(model, &parsed) {
        Ok(s) => s,
        Err(err) => panic!("expected the saved scenario to run:\n{err}\n---\n{yaml}"),
    };
    let mut expected = session.trace().clone();
    expected.scenario = "saved".to_owned();
    assert_eq!(restored.trace(), &expected, "{yaml}");
    assert_eq!(restored.payloads(), session.payloads(), "{yaml}");
    let position = session.timeline().position;
    assert_eq!(restored.timeline().actions, session.timeline().actions[..position], "{yaml}");
    assert_eq!(restored.pending(), session.pending(), "{yaml}");

    // The batch simulator agrees.
    match simulate_run(model, &parsed) {
        Ok(run) => assert_eq!(run.trace, expected, "{yaml}"),
        Err(err) => panic!("expected the saved scenario to simulate:\n{err}\n---\n{yaml}"),
    }
    yaml
}

#[test]
fn manual_queue_choices_survive_saving() {
    let model = model(RACE);
    let session = session(
        &model,
        &[
            add("o1", "Order", &[("orderId", "1")]),
            add("s1", "Shipment", &[("orderId", "1")]),
            fire("PaymentGateway", "Order.capture_ok", "o1"),
            step(),
            choose(1),
            step(),
        ],
    );
    let yaml = round_trip(&model, &session);
    assert!(yaml.contains("- { step: 1 }"), "{yaml}");
    assert!(yaml.contains("\n  - step\n"), "{yaml}");
    let replayed = match simulate_run(&model, &scenario(&yaml)) {
        Ok(run) => run.trace,
        Err(err) => panic!("{err}"),
    };
    assert_eq!(lines(&model, &replayed)[7..], ["s1 idle -> on_hold", "s1 drop start @ on_hold"]);
}

#[test]
fn a_session_paused_mid_cascade_saves_as_paused() {
    let model = model(RACE);
    let session = session(
        &model,
        &[
            add("o1", "Order", &[("orderId", "1")]),
            add("s1", "Shipment", &[("orderId", "1")]),
            fire("PaymentGateway", "Order.capture_ok", "o1"),
            step(),
        ],
    );
    let yaml = round_trip(&model, &session);
    assert!(yaml.contains("end: pause"), "{yaml}");
    let saved = match session.to_scenario("saved") {
        Ok(s) => s,
        Err(err) => panic!("{err}"),
    };
    assert_eq!(saved.end, ScenarioEnd::Pause);

    // A quiet session needs no end marker.
    let quiet = support::session(&model, &[add("o1", "Order", &[])]);
    let yaml = round_trip(&model, &quiet);
    assert!(!yaml.contains("end:"), "{yaml}");
}

#[test]
fn fires_during_a_cascade_save_as_immediate() {
    let model = model(ORDERS);
    let session = session(
        &model,
        &[
            add("o1", "Order", &[("orderId", "1")]),
            add("s1", "Shipment", &[("orderId", "1")]),
            fire("Customer", "Order.submit", "o1"),
            fire_with("PaymentGateway", "Order.capture_ok", "o1", &[("amount", "42"), ("note", "a: b # c")]),
            fire("Clock", "Order.timeout", "o1"),
            run_all(),
        ],
    );
    let yaml = round_trip(&model, &session);
    assert_eq!(yaml.matches("timing: immediate").count(), 1, "{yaml}");
}

#[test]
fn instances_added_and_removed_mid_session_survive_saving() {
    let model = model(RACE);
    let session = session(
        &model,
        &[
            add("o1", "Order", &[("orderId", "1")]),
            fire("PaymentGateway", "Order.capture_ok", "o1"),
            add("s1", "Shipment", &[("orderId", "1")]),
            add_auto("Shipment"),
            step(),
            remove("shipment1"),
            add_in("s9", "Shipment", "on_hold"),
            run_all(),
        ],
    );
    let yaml = round_trip(&model, &session);
    assert!(yaml.contains("create: s1"), "{yaml}");
    assert!(yaml.contains("create: shipment1"), "{yaml}");
    assert!(yaml.contains("remove: shipment1"), "{yaml}");
    assert!(yaml.contains("state: on_hold"), "{yaml}");
}

#[test]
fn only_the_timeline_up_to_the_position_is_saved() {
    let model = model(RACE);
    let mut session = session(
        &model,
        &[
            add("o1", "Order", &[("orderId", "1")]),
            add("s1", "Shipment", &[("orderId", "1")]),
            fire("PaymentGateway", "Order.capture_ok", "o1"),
            run_all(),
        ],
    );
    if let Err(err) = session.seek(&model, 3) {
        panic!("{err}");
    }
    let yaml = round_trip(&model, &session);
    assert!(!yaml.contains("- run"), "{yaml}");
    assert!(yaml.contains("end: pause"), "{yaml}");
}

#[test]
fn every_scenario_round_trips_through_yaml() {
    for (definition, text) in cases() {
        let model = model(definition);
        let original = scenario(text);
        let yaml = scenario_to_yaml(&original);
        let parsed = match parse_scenario(&yaml) {
            Ok(s) => s,
            Err(err) => panic!("{}: {err}\n---\n{yaml}", original.name),
        };
        // Writing is stable...
        assert_eq!(scenario_to_yaml(&parsed), yaml);
        // ...and keeps the meaning.
        let before = simulate_run(&model, &original).map(|r| r.trace);
        let after = simulate_run(&model, &parsed).map(|r| r.trace);
        assert_eq!(before, after, "{yaml}");
        // Sessions round-trip too.
        round_trip(&model, &from_scenario(&model, &original));
    }
}

#[test]
fn awkward_text_is_quoted() {
    let model = model(RACE);
    let session = session(
        &model,
        &[
            add("o1", "Order", &[("orderId", "")]),
            add("true", "Order", &[("orderId", "null")]),
            fire_with(
                "PaymentGateway",
                "Order.capture_ok",
                "o1",
                &[("a", "yes"), ("b", "1.50"), ("c", "\"quoted\" \\ back"), ("d", "line\nbreak\ttab"), ("e", "~")],
            ),
        ],
    );
    let saved = match session.to_scenario("name: with # awkward \"text\"") {
        Ok(s) => s,
        Err(err) => panic!("{err}"),
    };
    let yaml = scenario_to_yaml(&saved);
    let parsed = match parse_scenario(&yaml) {
        Ok(s) => s,
        Err(err) => panic!("{err}\n---\n{yaml}"),
    };
    assert_eq!(parsed.name, "name: with # awkward \"text\"");
    assert_eq!(parsed.instances[1].name.value, "true");
    assert_eq!(parsed.instances[1].fields.get("orderId"), Some("null"));
    assert_eq!(parsed.instances[0].fields.get("orderId"), Some(""));
    let payload = parsed.steps[0].payload.to_payload();
    assert_eq!(
        payload,
        support::map(&[
            ("a", "yes"),
            ("b", "1.50"),
            ("c", "\"quoted\" \\ back"),
            ("d", "line\nbreak\ttab"),
            ("e", "~")
        ])
    );
    round_trip(&model, &session);
}
