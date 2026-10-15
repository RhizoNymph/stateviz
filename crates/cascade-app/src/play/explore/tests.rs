//! The Explore panel's logic on real sessions: the race fixture of
//! cascade-sim's tests and the checkout saga example.

use std::collections::BTreeMap;
use std::sync::Arc;

use cascade_core::definition::TriggerRef;
use cascade_core::{ElementKey, Model, Severity};
use cascade_sim::{ExploreOptions, OrderingModel, PlayAction, SimError};

use super::branch::{self, OpenError};
use super::rows::anomaly_severity;
use super::*;

const RACE: &str = include_str!("../../../../cascade-sim/tests/fixtures/race/cascade.yaml");
const SAGA: &str = include_str!("../../../../../examples/checkout-saga/cascade.yaml");

fn load(text: &str) -> Arc<Model> {
    Arc::new(cascade_core::load_str(text).unwrap_or_else(|e| panic!("loads: {e}")))
}

fn add(name: &str, machine: &str, fields: &[(&str, &str)]) -> PlayAction {
    PlayAction::AddInstance {
        name: Some(name.into()),
        machine: machine.into(),
        fields: fields.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
        state: None,
    }
}

fn fire(source: &str, machine: &str, trigger: &str, target: &str) -> PlayAction {
    PlayAction::Fire {
        source: source.into(),
        trigger: TriggerRef { machine: machine.into(), trigger: trigger.into() },
        target: target.into(),
        payload: BTreeMap::new(),
    }
}

fn act(state: &mut PlayState, model: &Model, action: PlayAction) {
    state.act(model, action).unwrap_or_else(|e| panic!("acts: {e}"));
}

/// The race fixture with the payment captured and `OrderPaid`'s cascade
/// still queued.
fn race_session(model: &Model) -> PlayState {
    let mut state = PlayState::new(model, 1);
    act(&mut state, model, add("o1", "Order", &[("orderId", "1")]));
    act(&mut state, model, add("s1", "Shipment", &[("orderId", "1")]));
    act(&mut state, model, fire("PaymentGateway", "Order", "capture_ok", "o1"));
    assert!(!state.session().pending().is_empty(), "the capture is queued");
    state
}

/// The saga with the order placed and its cascade queued.
fn saga_session(model: &Model) -> PlayState {
    let mut state = PlayState::new(model, 1);
    act(&mut state, model, add("o1", "Order", &[("orderId", "1"), ("amount", "50")]));
    act(&mut state, model, add("p1", "Payment", &[("orderId", "1"), ("amount", "50")]));
    act(&mut state, model, add("s1", "Stock", &[("orderId", "1")]));
    act(&mut state, model, fire("Customer", "Order", "place", "o1"));
    state
}

/// Start and run a job synchronously, finishing it into `ui`.
fn explore(ui: &mut ExploreUi, state: &PlayState, model: &Arc<Model>) -> Arc<ExploreResult> {
    let job = ui.start(state, Arc::clone(model));
    let progress = Arc::clone(&job.progress);
    let finished = ui.finish(&progress, job.run());
    assert!(matches!(finished, Finished::Done { .. }), "{finished:?}");
    Arc::clone(ui.result().expect("done"))
}

fn leaf(model: &Model, state: &PlayState, name: &str) -> String {
    let instance = state.session().instances().into_iter().find(|i| i.name == name).expect("instance exists");
    model.state(instance.state).path.clone()
}

// --- Controls -------------------------------------------------------------

#[test]
fn default_controls_explore_with_the_simulator_defaults() {
    let controls = ExploreControls::default();
    assert_eq!(controls.options(), ExploreOptions::default());
    assert_eq!(controls.label(), "per-consumer");
}

#[test]
fn every_control_reaches_the_options() {
    let controls = ExploreControls { ordering: OrderingModel::Unordered, choices: true, timers_anytime: true };
    let options = controls.options();
    assert_eq!(options.ordering, OrderingModel::Unordered);
    assert!(options.choices && options.timers_anytime);
    assert_eq!(options.max_states, cascade_sim::explore::DEFAULT_MAX_STATES);
    assert!(options.reduction);
    assert_eq!(controls.label(), "unordered, choices, timers anytime");
}

#[test]
fn a_run_explores_with_the_controls_it_started_with() {
    let model = load(SAGA);
    let state = saga_session(&model);
    let mut ui = ExploreUi::default();
    ui.controls.timers_anytime = true;
    let job = ui.start(&state, Arc::clone(&model));
    ui.controls.timers_anytime = false; // changing the switches mid-run changes nothing
    assert!(job.controls.timers_anytime);
    let expected = cascade_sim::explore_from(state.session(), &model, &job.controls.options()).expect("explores");
    let progress = Arc::clone(&job.progress);
    ui.finish(&progress, job.run());
    let result = ui.result().expect("done");
    assert!(result.controls.timers_anytime);
    assert_eq!(result.exploration, expected);
}

// --- Lifecycle --------------------------------------------------------------

#[test]
fn starting_records_the_origin_and_runs_off_a_copy() {
    let model = load(RACE);
    let state = race_session(&model);
    let mut ui = ExploreUi::default();
    let job = ui.start(&state, Arc::clone(&model));
    let ExploreRun::Running { origin, controls, .. } = &ui.run else { panic!("running: {:?}", ui.run) };
    assert_eq!(origin, &Origin::of(&state));
    assert_eq!(origin.position(), 3);
    assert_eq!(origin.generation, 1);
    assert_eq!(*controls, ExploreControls::default());
    assert_eq!(ui.running_states(), Some(0));
    assert_eq!(job.session.timeline(), state.session().timeline());
}

#[test]
fn the_race_has_two_outcomes_and_a_complete_summary() {
    let model = load(RACE);
    let state = race_session(&model);
    let mut ui = ExploreUi::default();
    let result = explore(&mut ui, &state, &model);
    assert!(result.exploration.complete);
    assert_eq!(result.exploration.outcomes.len(), 2);
    assert_eq!(result.summary, format!("2 outcomes, {} states (complete)", result.exploration.states));
    assert_eq!(ui.running_states(), None);
}

#[test]
fn an_incomplete_run_names_its_bound() {
    let exploration = cascade_sim::Exploration { outcomes: Vec::new(), states: 5, complete: false, notes: Vec::new() };
    let options = ExploreOptions { max_states: 5, ..ExploreOptions::default() };
    assert_eq!(rows::summary_text(&exploration, &options), "0 outcomes, 5 states (incomplete: bound of 5 states hit)");
}

#[test]
fn cancelling_returns_to_idle_with_the_states_visited() {
    let model = load(RACE);
    let state = race_session(&model);
    let mut ui = ExploreUi::default();
    let job = ui.start(&state, Arc::clone(&model));
    ui.cancel();
    assert!(job.progress.is_cancelled());
    let progress = Arc::clone(&job.progress);
    let finished = ui.finish(&progress, job.run());
    assert_eq!(finished, Finished::Cancelled { states: 1 });
    assert!(matches!(ui.run, ExploreRun::Idle));
}

#[test]
fn a_restarted_run_ignores_the_old_jobs_result() {
    let model = load(RACE);
    let state = race_session(&model);
    let mut ui = ExploreUi::default();
    let first = ui.start(&state, Arc::clone(&model));
    let second = ui.start(&state, Arc::clone(&model));
    assert!(first.progress.is_cancelled(), "restarting cancels the running job");
    let first_progress = Arc::clone(&first.progress);
    assert_eq!(ui.finish(&first_progress, first.run()), Finished::Ignored);
    assert!(matches!(ui.run, ExploreRun::Running { .. }));
    let second_progress = Arc::clone(&second.progress);
    assert!(matches!(ui.finish(&second_progress, second.run()), Finished::Done { .. }));
}

#[test]
fn a_failed_run_keeps_its_error() {
    let model = load(RACE);
    let state = race_session(&model);
    let mut ui = ExploreUi::default();
    let job = ui.start(&state, Arc::clone(&model));
    let finished = ui.finish(&job.progress, Err(SimError::WitnessLost { at: 2 }));
    let Finished::Failed { error } = finished else { panic!("{finished:?}") };
    assert!(error.contains("witness"), "{error}");
    assert!(matches!(&ui.run, ExploreRun::Failed { origin, .. } if origin.position() == 3));
}

// --- Rows -------------------------------------------------------------------

#[test]
fn race_rows_list_every_instance_first_then_only_what_differs() {
    let model = load(RACE);
    let state = race_session(&model);
    let mut ui = ExploreUi::default();
    let result = explore(&mut ui, &state, &model);
    let [first, second] = result.rows.as_slice() else { panic!("{:?}", result.rows) };
    assert_eq!(first.number, 1);
    assert_eq!(second.number, 2);
    assert!(first.title.starts_with("Outcome 1 · "), "{}", first.title);
    assert!(first.title.ends_with("path") || first.title.ends_with("paths"), "{}", first.title);
    assert_eq!(first.instances.len(), 2, "{:?}", first.instances);
    assert!(first.instances[0].starts_with("o1 (Order): paid"), "{:?}", first.instances);
    assert!(first.instances[1].starts_with("s1 (Shipment): "), "{:?}", first.instances);
    assert_eq!(second.instances.len(), 1, "only the shipment differs: {:?}", second.instances);
    assert!(second.instances[0].starts_with("s1 (Shipment): "));
    assert_ne!(first.instances[1], second.instances[0]);
    assert!(!second.same_instances);
    assert_eq!(first.differs, ["s1"]);
    assert_eq!(second.differs, ["s1"]);
    for row in [first, second] {
        assert!(row.timers.is_empty());
        let [anomaly] = row.anomalies.as_slice() else { panic!("{:?}", row.anomalies) };
        assert_eq!(anomaly.severity, Severity::Warning);
        assert!(anomaly.text.starts_with("s1: ") && anomaly.text.contains("dropped"), "{}", anomaly.text);
        assert!(row.paths >= 1);
    }
}

#[test]
fn saga_rows_show_timers_left_armed() {
    let model = load(SAGA);
    let state = saga_session(&model);
    let mut ui = ExploreUi::default();
    let result = explore(&mut ui, &state, &model);
    // Without timers anytime the checkout's 30 minute timer is still armed.
    let row = &result.rows[0];
    assert!(row.timers.iter().any(|t| t.contains("after-30m due in 30m")), "{:?}", row.timers);
    assert!(row.instances.iter().any(|i| i.starts_with("o1 (Order): placed")), "{:?}", row.instances);
}

#[test]
fn anomaly_severities_follow_the_findings_scale() {
    use cascade_sim::Anomaly;
    assert_eq!(anomaly_severity(&Anomaly::StepLimit), Severity::Error);
    assert_eq!(anomaly_severity(&Anomaly::Ambiguous { rule: "r".into(), candidates: vec![] }), Severity::Error);
    assert_eq!(anomaly_severity(&Anomaly::NoTarget { rule: "r".into() }), Severity::Warning);
    assert_eq!(
        anomaly_severity(&Anomaly::Dropped { instance: "a".into(), trigger: "t".into(), state: "s".into() }),
        Severity::Warning
    );
    assert_eq!(anomaly_severity(&Anomaly::InputFailed { input: "i".into(), error: "e".into() }), Severity::Warning);
    assert_eq!(anomaly_severity(&Anomaly::UnusedChoice { option: "o".into() }), Severity::Info);
}

// --- Open as branch ---------------------------------------------------------

#[test]
fn opening_an_outcome_lands_on_its_state_and_steps_back_through_it() {
    let model = load(RACE);
    let mut state = race_session(&model);
    let mut ui = ExploreUi::default();
    let result = explore(&mut ui, &state, &model);
    for index in [1, 0] {
        branch::open(&result, index, &mut state, &model).unwrap_or_else(|e| panic!("{e}"));
        let outcome = &result.exploration.outcomes[index];
        let timeline = state.session().timeline();
        assert_eq!(timeline.position, 3 + outcome.witness.len());
        assert_eq!(timeline.actions[3..], outcome.witness[..]);
        assert!(state.session().pending().is_empty(), "the outcome is settled");
        for (name, (leaf_id, fields)) in &outcome.instances {
            assert_eq!(leaf(&model, &state, name), model.state(*leaf_id).path);
            let instance = state.session().instances().into_iter().find(|i| &i.name == name).expect("exists");
            assert_eq!(&instance.fields, fields);
        }
        assert!(!state.last_steps(&model).is_empty(), "the witness's steps are the last action");
        assert_eq!(result.freshness(&state), Freshness::Moved);
    }
    // Opening the second outcome after the first forked the timeline: the
    // first outcome's line is a branch.
    let branches = &state.session().timeline().branches;
    assert_eq!(branches.len(), 1);
    // The two witnesses may share their first moves; the line forks where
    // they part.
    assert!(branches[0].fork >= 3, "{}", branches[0].fork);
    assert_eq!(branches[0].actions[3..], result.exploration.outcomes[1].witness[..]);
    // Stepping back through the ordering returns to the origin.
    state.seek(&model, 3).expect("seeks back");
    assert_eq!(result.freshness(&state), Freshness::Current);
    assert_eq!(state.session().pending().len(), 1);
}

#[test]
fn opening_the_outcome_already_on_the_line_does_not_fork() {
    let model = load(RACE);
    let mut state = race_session(&model);
    let mut ui = ExploreUi::default();
    let result = explore(&mut ui, &state, &model);
    branch::open(&result, 0, &mut state, &model).expect("opens");
    branch::open(&result, 0, &mut state, &model).expect("opens again");
    assert!(state.session().timeline().branches.is_empty());
}

#[test]
fn opening_from_a_session_with_a_future_keeps_it_as_a_branch() {
    let model = load(RACE);
    let mut state = race_session(&model);
    act(&mut state, &model, PlayAction::RunUntilQuiet);
    state.seek(&model, 3).expect("seeks back");
    let mut ui = ExploreUi::default();
    let result = explore(&mut ui, &state, &model);
    let differs = result
        .exploration
        .outcomes
        .iter()
        .position(|o| o.witness != state.session().timeline().actions[3..])
        .expect("some outcome is not the FIFO run's");
    branch::open(&result, differs, &mut state, &model).expect("opens");
    let timeline = state.session().timeline();
    assert_eq!(timeline.branches.len(), 1);
    assert_eq!(timeline.branches[0].actions.last(), Some(&PlayAction::RunUntilQuiet));
}

#[test]
fn saga_timers_anytime_opens_the_expired_checkout() {
    let model = load(SAGA);
    let mut state = saga_session(&model);
    let mut ui = ExploreUi::default();
    ui.controls.timers_anytime = true;
    let result = explore(&mut ui, &state, &model);
    let expired = result
        .exploration
        .outcomes
        .iter()
        .position(|o| o.instances.values().any(|(leaf_id, _)| model.state(*leaf_id).path == "expired"))
        .expect("the checkout can expire");
    branch::open(&result, expired, &mut state, &model).expect("opens");
    let checkout = state
        .session()
        .instances()
        .into_iter()
        .find(|i| model.machine(i.machine).name == "Checkout")
        .expect("the saga spawned a checkout");
    assert_eq!(model.state(checkout.state).path, "expired");
    assert_eq!(leaf(&model, &state, "o1"), "cancelled");
    assert!(state.session().timers().is_empty());
}

/// A bank answer delivered by a controller, so its `?approved` choice is
/// made while the cascade runs (the saga's bank is external: the app asks
/// for that choice before the fire).
const QUEUED_CHOICE: &str = r#"
machines:
  Order:
    fields: [orderId]
    initial: draft
    states: [draft, placed]
    transitions:
      - { from: draft, to: placed, on: place, emits: [OrderPlaced] }
  Payment:
    fields: [orderId]
    initial: authorizing
    states: [authorizing, captured, declined]
    transitions:
      - { from: authorizing, to: captured, on: respond, guard: "?approved" }
      - { from: authorizing, to: declined, on: respond, guard: "?card declined" }
controllers:
  Bank:
    on:
      OrderPlaced:
        - fire: Payment.respond
          target: Payment where orderId == event.orderId
external:
  Customer: [Order.place]
"#;

#[test]
fn branching_on_choices_reaches_every_option() {
    let model = load(QUEUED_CHOICE);
    let mut state = PlayState::new(&model, 1);
    act(&mut state, &model, add("o1", "Order", &[("orderId", "1")]));
    act(&mut state, &model, add("p1", "Payment", &[("orderId", "1")]));
    act(&mut state, &model, fire("Customer", "Order", "place", "o1"));
    let payments = |result: &ExploreResult| -> Vec<String> {
        result.exploration.outcomes.iter().map(|o| model.state(o.instances["p1"].0).path.clone()).collect()
    };
    let mut ui = ExploreUi::default();
    let defaults = explore(&mut ui, &state, &model);
    assert_eq!(payments(&defaults), ["captured"], "without choices the default is taken");
    ui.controls.choices = true;
    let result = explore(&mut ui, &state, &model);
    let found = payments(&result);
    assert!(found.contains(&"captured".to_owned()) && found.contains(&"declined".to_owned()), "{found:?}");
    let declined = found.iter().position(|p| p == "declined").expect("declined");
    branch::open(&result, declined, &mut state, &model).expect("opens");
    assert_eq!(leaf(&model, &state, "p1"), "declined");
    assert!(
        state.session().timeline().actions.contains(&PlayAction::Choose { option: "card declined".into() }),
        "the witness records the decision: {:?}",
        state.session().timeline().actions
    );
}

#[test]
fn opening_a_missing_outcome_is_an_error_and_changes_nothing() {
    let model = load(RACE);
    let mut state = race_session(&model);
    let mut ui = ExploreUi::default();
    let result = explore(&mut ui, &state, &model);
    let before = state.session().timeline().clone();
    let error = branch::open(&result, 7, &mut state, &model).expect_err("no outcome 8");
    assert!(matches!(error, OpenError::NoSuchOutcome(8)), "{error}");
    assert_eq!(state.session().timeline(), &before);
}

// --- Staleness ----------------------------------------------------------------

#[test]
fn moving_on_the_timeline_marks_results_moved_and_coming_back_current() {
    let model = load(RACE);
    let mut state = race_session(&model);
    let mut ui = ExploreUi::default();
    let result = explore(&mut ui, &state, &model);
    assert_eq!(result.freshness(&state), Freshness::Current);
    act(&mut state, &model, PlayAction::Step { choice: None, consumer: None });
    assert_eq!(result.freshness(&state), Freshness::Moved);
    state.seek(&model, 1).expect("seeks back before the origin");
    assert_eq!(result.freshness(&state), Freshness::Moved, "the origin is still on the line");
    state.seek(&model, 3).expect("seeks to the origin");
    assert_eq!(result.freshness(&state), Freshness::Current);
}

#[test]
fn leaving_the_explored_line_makes_results_stale() {
    let model = load(RACE);
    let mut state = race_session(&model);
    let mut ui = ExploreUi::default();
    let result = explore(&mut ui, &state, &model);
    state.seek(&model, 2).expect("seeks back");
    act(&mut state, &model, PlayAction::RemoveInstance { name: "s1".into() });
    assert_eq!(result.freshness(&state), Freshness::Stale);
    assert!(matches!(branch::open(&result, 0, &mut state, &model), Err(OpenError::Stale)));
    assert!(matches!(ui.toggle_select(0, &state, &model), Err(OpenError::Stale)));
    assert_eq!(ui.selected, None);
}

#[test]
fn an_edit_makes_results_stale() {
    let model = load(RACE);
    let mut state = race_session(&model);
    let mut ui = ExploreUi::default();
    let result = explore(&mut ui, &state, &model);
    let edited = load(&RACE.replace("on_hold]", "on_hold, cancelled]"));
    assert_eq!(state.rebase(&edited, 2), None, "the timeline still replays");
    assert_eq!(state.session().timeline().position, 3);
    assert_eq!(result.freshness(&state), Freshness::Stale);
    assert!(ui.preview_for(&state).is_none());
    // The rows were built with the explored model and still read.
    assert_eq!(result.rows.len(), 2);
}

// --- Preview ------------------------------------------------------------------

#[test]
fn selecting_an_outcome_previews_it_and_highlights_what_differs() {
    let model = load(RACE);
    let mut state = race_session(&model);
    let mut ui = ExploreUi { open: true, ..ExploreUi::default() };
    let result = explore(&mut ui, &state, &model);
    let before = state.session().timeline().clone();
    ui.toggle_select(1, &state, &model).expect("previews");
    assert_eq!(ui.selected, Some(1));
    assert_eq!(state.session().timeline(), &before, "previewing changes nothing");
    let preview = ui.preview_for(&state).expect("a preview").clone();
    let outcome = &result.exploration.outcomes[1];
    let s1 = model.state(outcome.instances["s1"].0);
    let s1_key = ElementKey::State { machine: "Shipment".into(), path: s1.path.clone() };
    assert_eq!(preview.active, std::slice::from_ref(&s1_key));
    assert!(preview.pending.is_empty());
    assert_eq!(preview.markers.len(), 2);
    let marker = preview.markers.iter().find(|m| m.instance == "s1").expect("s1's marker");
    assert_eq!(marker.state, s1_key);
    // The preview is what opening shows.
    branch::open(&result, 1, &mut state, &model).expect("opens");
    assert_eq!(state.overlay(&model).markers, preview.markers);
    // Selecting again deselects.
    ui.toggle_select(1, &state, &model).expect("deselects");
    assert_eq!(ui.selected, None);
    assert!(ui.preview_for(&state).is_none());
}

#[test]
fn a_closed_panel_or_a_new_run_shows_no_preview() {
    let model = load(RACE);
    let state = race_session(&model);
    let mut ui = ExploreUi { open: true, ..ExploreUi::default() };
    explore(&mut ui, &state, &model);
    ui.toggle_select(0, &state, &model).expect("previews");
    ui.open = false;
    assert!(ui.preview_for(&state).is_none());
    ui.open = true;
    assert!(ui.preview_for(&state).is_some());
    let _job = ui.start(&state, Arc::clone(&model));
    assert_eq!(ui.selected, None);
    assert!(ui.preview_for(&state).is_none());
}
