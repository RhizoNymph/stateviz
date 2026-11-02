//! Timers and the virtual clock: arming on entry (creation, transitions,
//! history), cancelling on exit, `Advance` / `wait`, `TimerExpired`, timer
//! queue items, and saving.

mod common;
mod support;

use cascade_core::Model;
use cascade_core::expr::Duration;
use cascade_sim::{
    PendingKind, PlayAction, PlaySession, ScenarioErrorKind, SimError, TraceStepKind, scenario_to_yaml, simulate,
};
use common::{causes, lines, model, run, run_err, scenario, strs};
use support::{add, add_in, apply, apply_err, fire, from_scenario, remove, session, step, unnamed};

const TIMED: &str = r#"
machines:
  Checkout:
    initial: waiting
    states:
      - waiting
      - paying:
          initial: authorizing
          states: [authorizing, capturing]
      - expired
    transitions:
      - { from: waiting, to: paying, on: start }
      - { from: paying.authorizing, to: paying.capturing, on: authorized }
      - { from: [waiting, paying], to: expired, after: 30m, emits: [Expired] }
      - { from: paying.capturing, to: paying.authorizing, after: 1m }
external:
  Shop: [Checkout.start, Checkout.authorized]
"#;

fn minutes(n: u64) -> Duration {
    Duration::from_secs(n * 60)
}

fn advance(by: Option<Duration>) -> PlayAction {
    PlayAction::Advance { by }
}

fn timers(model: &Model, s: &PlaySession) -> Vec<String> {
    s.timers().iter().map(|t| format!("{} {} {}", t.instance, model.trigger(t.trigger).name, t.deadline)).collect()
}

fn state_of(model: &Model, s: &PlaySession, name: &str) -> String {
    s.instances().iter().find(|i| i.name == name).map(|i| model.state(i.state).path.clone()).unwrap_or_default()
}

#[test]
fn creating_an_instance_arms_its_states_timers() {
    let model = model(TIMED);
    let s = session(&model, &[add("c1", "Checkout", &[])]);
    assert_eq!(s.now(), Duration::ZERO);
    assert_eq!(timers(&model, &s), ["c1 after-30m 30m"]);
}

#[test]
fn creating_in_a_nested_state_arms_every_entered_state_outermost_first() {
    let model = model(TIMED);
    let s = session(&model, &[add_in("c1", "Checkout", "paying.capturing")]);
    // Equal deadlines would be ordered by arming: outer state first.
    assert_eq!(timers(&model, &s), ["c1 after-1m 1m", "c1 after-30m 30m"]);
    let s = session(&model, &[add_in("c1", "Checkout", "paying")]);
    assert_eq!(timers(&model, &s), ["c1 after-30m 30m"]);
}

#[test]
fn the_clock_advances_and_transitions_rearm() {
    let model = model(TIMED);
    let mut s = session(&model, &[add("c1", "Checkout", &[])]);
    apply(&mut s, &model, advance(Some(minutes(10))));
    assert_eq!(s.now(), minutes(10));
    assert_eq!(lines(&model, s.trace()), strs(&["wait 10m -> 10m"]));
    assert_eq!(timers(&model, &s), ["c1 after-30m 30m"]);

    // Leaving `waiting` cancels its timer, entering `paying` arms one from now.
    apply(&mut s, &model, fire("Shop", "Checkout.start", "c1"));
    assert_eq!(timers(&model, &s), ["c1 after-30m 40m"]);
    // A move inside `paying` keeps paying's timer and arms capturing's.
    apply(&mut s, &model, fire("Shop", "Checkout.authorized", "c1"));
    assert_eq!(timers(&model, &s), ["c1 after-1m 11m", "c1 after-30m 40m"]);
}

#[test]
fn waiting_for_the_next_deadline_queues_the_timer() {
    let model = model(TIMED);
    let mut s = session(
        &model,
        &[add("c1", "Checkout", &[]), fire("Shop", "Checkout.start", "c1"), fire("Shop", "Checkout.authorized", "c1")],
    );
    let before = s.trace().steps.len();
    apply(&mut s, &model, advance(None));
    assert_eq!(s.now(), minutes(1));
    assert_eq!(lines(&model, s.trace())[before..], strs(&["wait 1m -> 1m", "c1 timer after-1m"]));
    assert_eq!(s.trace().steps[before + 1].cause.map(|c| c.index()), Some(before));
    // The expired timer left the armed list and sits in the queue.
    assert_eq!(timers(&model, &s), ["c1 after-30m 30m"]);
    let pending = s.pending();
    assert_eq!(pending.len(), 1);
    assert!(matches!(pending[0].kind, PendingKind::Timer { .. }));
    assert_eq!(pending[0].label, "timer → c1: after-1m");
    assert_eq!(pending[0].consumers_left, strs(&["c1"]));
    assert_eq!(pending[0].cause.index(), before + 1);

    apply(&mut s, &model, step());
    assert_eq!(lines(&model, s.trace()).last().map(String::as_str), Some("c1 paying.capturing -> paying.authorizing"));
    assert_eq!(s.trace().steps.last().and_then(|st| st.cause).map(|c| c.index()), Some(before + 1));
}

#[test]
fn a_long_wait_expires_everything_due_in_deadline_order() {
    let model = model(TIMED);
    let mut s = session(&model, &[add("c1", "Checkout", &[]), add("c2", "Checkout", &[])]);
    apply(&mut s, &model, advance(Some(minutes(60))));
    assert_eq!(lines(&model, s.trace()), strs(&["wait 1h -> 1h", "c1 timer after-30m", "c2 timer after-30m"]));
    assert!(s.timers().is_empty());
    apply(&mut s, &model, PlayAction::RunUntilQuiet);
    assert_eq!(state_of(&model, &s, "c1"), "expired");
    assert_eq!(state_of(&model, &s, "c2"), "expired");
}

#[test]
fn waiting_with_nothing_armed_is_an_error() {
    let model = model(TIMED);
    let mut s = session(&model, &[]);
    assert!(matches!(apply_err(&mut s, &model, advance(None)), SimError::Action(ScenarioErrorKind::NoTimerArmed)));
    // A fixed wait is fine.
    apply(&mut s, &model, advance(Some(minutes(5))));
    assert_eq!(s.now(), minutes(5));
}

#[test]
fn a_self_transition_rearms() {
    let model = model(
        r#"
machines:
  Light:
    initial: on
    states: [on, off]
    transitions:
      - { from: on, to: on, on: poke }
      - { from: on, to: off, after: 10s }
external:
  User: [Light.poke]
"#,
    );
    let text = "scenario: s\ninstances:\n  l: Light\nsteps:\n  - { wait: 5s }\n  - { source: User, fire: Light.poke }\n  - { wait: 5s }\n";
    let s = from_scenario(&model, &scenario(text));
    assert_eq!(timers(&model, &s), ["l after-10s 15s"]);
    assert_eq!(state_of(&model, &s, "l"), "on");
    let trace = run(&model, &format!("{text}  - wait\n"));
    assert_eq!(common::finals(&model, &trace), ["l: off"]);
}

#[test]
fn a_queued_timer_whose_state_was_left_is_dropped() {
    let model = model(
        r#"
machines:
  Door:
    initial: open
    states: [open, closed, locked]
    transitions:
      - { from: open, to: closed, after: 10s }
      - { from: open, to: locked, on: lock }
external:
  User: [Door.lock]
"#,
    );
    let text = "scenario: s\ninstances:\n  d: Door\nsteps:\n  - { wait: 10s }\n  - { source: User, fire: Door.lock, timing: immediate }\n";
    let trace = run(&model, text);
    assert_eq!(
        lines(&model, &trace),
        strs(&[
            "wait 10s -> 10s",
            "d timer after-10s",
            "ext User -> d lock",
            "d open -> locked",
            "d drop after-10s @ locked"
        ])
    );
    assert_eq!(causes(&trace)[4], Some(1));
}

#[test]
fn history_restores_rearm_the_restored_states() {
    let model = model(
        r#"
machines:
  Player:
    initial: active
    states:
      - active:
          initial: a
          states:
            - a
            - b
            - h: { kind: history }
      - paused
    transitions:
      - { from: active.a, to: active.b, on: next }
      - { from: active.b, to: active.a, after: 5s }
      - { from: active, to: paused, on: pause }
      - { from: paused, to: active.h, on: resume }
external:
  User: [Player.next, Player.pause, Player.resume]
"#,
    );
    let mut s = session(&model, &[add("p", "Player", &[]), fire("User", "Player.next", "p")]);
    assert_eq!(timers(&model, &s), ["p after-5s 5s"]);
    apply(&mut s, &model, fire("User", "Player.pause", "p"));
    assert!(s.timers().is_empty());
    apply(&mut s, &model, advance(Some(Duration::from_secs(60))));
    apply(&mut s, &model, fire("User", "Player.resume", "p"));
    assert_eq!(state_of(&model, &s, "p"), "active.b");
    assert_eq!(timers(&model, &s), ["p after-5s 65s"]);
}

#[test]
fn removing_an_instance_cancels_its_timers_and_queued_expiries() {
    let model = model(TIMED);
    let mut s = session(&model, &[add("c1", "Checkout", &[]), add("c2", "Checkout", &[])]);
    apply(&mut s, &model, advance(Some(minutes(30))));
    assert_eq!(s.pending().len(), 2);
    apply(&mut s, &model, remove("c1"));
    assert_eq!(s.pending().iter().map(|p| p.label.clone()).collect::<Vec<_>>(), ["timer → c2: after-30m"]);
    let mut s = session(&model, &[add("c1", "Checkout", &[])]);
    apply(&mut s, &model, remove("c1"));
    assert!(s.timers().is_empty());
}

#[test]
fn a_timer_item_can_be_stepped_to_its_instance() {
    let model = model(TIMED);
    let mut s = session(&model, &[add("c1", "Checkout", &[]), advance(Some(minutes(30)))]);
    apply(&mut s, &model, PlayAction::Step { choice: None, consumer: Some("c1".to_owned()) });
    assert_eq!(state_of(&model, &s, "c1"), "expired");
}

#[test]
fn draining_never_advances_time() {
    let model = model(TIMED);
    let trace = run(&model, "scenario: s\ninstances:\n  c1: Checkout\nsteps: []\n");
    assert!(trace.steps.is_empty());
    let s = from_scenario(&model, &scenario("scenario: s\ninstances:\n  c1: Checkout\nsteps: []\n"));
    assert_eq!(s.now(), Duration::ZERO);
    assert_eq!(timers(&model, &s), ["c1 after-30m 30m"]);
}

#[test]
fn scenario_waits_run_after_quiescence_unless_immediate() {
    let model = model(
        r#"
machines:
  A:
    initial: x
    states: [x, y]
    transitions:
      - { from: x, to: y, on: go, emits: [Went] }
  B:
    initial: idle
    states: [idle, busy, done]
    transitions:
      - { from: idle, to: busy, on: { event: Went } }
      - { from: busy, to: done, after: 1s }
external:
  Api: [A.go]
"#,
    );
    let base = "scenario: s\ninstances:\n  a: A\n  b: B\nsteps:\n  - { source: Api, fire: A.go }\n";
    // After quiescence: Went is delivered first, so b's timer is armed.
    let trace = run(&model, &format!("{base}  - {{ wait: 1s }}\n"));
    assert_eq!(common::finals(&model, &trace), ["a: y", "b: done"]);
    // Immediate: the clock moves before Went is delivered; nothing is due.
    let trace = run(&model, &format!("{base}  - {{ wait: 1s, timing: immediate }}\n"));
    assert_eq!(common::finals(&model, &trace), ["a: y", "b: busy"]);
    assert!(lines(&model, &trace).contains(&"wait 1s -> 1s".to_owned()));
}

#[test]
fn a_bare_wait_with_nothing_armed_is_reported_at_its_line() {
    let model = model(TIMED);
    let err = run_err(&model, "scenario: s\nsteps:\n  - wait\n");
    let SimError::Scenario(err) = err else { panic!("{err}") };
    assert!(matches!(err.diagnostics[0].kind, ScenarioErrorKind::NoTimerArmed));
    assert_eq!(err.diagnostics[0].span.start.line, 3);
}

#[test]
fn wait_steps_have_no_cause() {
    let model = model(TIMED);
    let trace = run(&model, "scenario: s\ninstances:\n  c1: Checkout\nsteps:\n  - wait\n");
    assert!(matches!(trace.steps[0].kind, TraceStepKind::Wait { .. }));
    assert_eq!(trace.steps[0].cause, None);
    assert_eq!(common::finals(&model, &trace), ["c1: expired"]);
}

#[test]
fn sessions_with_waits_save_and_replay_exactly() {
    let model = model(TIMED);
    let mut s = session(&model, &[add("c1", "Checkout", &[]), fire("Shop", "Checkout.start", "c1")]);
    apply(&mut s, &model, advance(Some(minutes(5))));
    apply(&mut s, &model, fire("Shop", "Checkout.authorized", "c1"));
    apply(&mut s, &model, advance(None));
    // An advance while items are queued saves as immediate.
    apply(&mut s, &model, advance(Some(minutes(1))));
    apply(&mut s, &model, PlayAction::RunUntilQuiet);
    let saved = match s.to_scenario("timers") {
        Ok(saved) => saved,
        Err(err) => panic!("{err}"),
    };
    let text = scenario_to_yaml(&saved);
    assert!(text.contains("  - { wait: 5m }\n"), "{text}");
    assert!(text.contains("  - wait\n"), "{text}");
    assert!(text.contains("  - { wait: 1m, timing: immediate }\n"), "{text}");
    let again = scenario(&text);
    let replayed = from_scenario(&model, &again);
    assert_eq!(replayed.timeline().actions, s.timeline().actions);
    assert_eq!(unnamed(replayed.trace()), unnamed(s.trace()));
    assert_eq!(replayed.now(), s.now());
    match simulate(&model, &again) {
        Ok(trace) => assert_eq!(unnamed(&trace), unnamed(s.trace())),
        Err(err) => panic!("{err}"),
    }
}

#[test]
fn seeking_back_restores_the_clock() {
    let model = model(TIMED);
    let mut s = session(&model, &[add("c1", "Checkout", &[]), advance(Some(minutes(10)))]);
    s.seek(&model, 1).unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(s.now(), Duration::ZERO);
    assert_eq!(timers(&model, &s), ["c1 after-30m 30m"]);
}
