//! `sets:` on transitions and `with:` on emits.

mod common;

use cascade_sim::{StepIx, TraceStepKind, simulate_run};
use common::{causes, lines, model, scenario, strs};

const CHANNEL: &str = r#"
machines:
  Channel:
    fields: [channelId, status, fromAgent, prev, note]
    initial: awaiting
    states: [awaiting, confirmed]
    transitions:
      - from: awaiting
        to: confirmed
        on: match
        sets: { fromAgent: event.originAgent, prev: self.status, status: open, note: event.nope }
        emits:
          - Confirmed
          - event: Superseded
            with: { channelId: self.channelId, by: event.channelId, now: self.status, gone: event.nope }
external:
  Net: [Channel.match]
"#;

const RUN: &str = r#"scenario: sets
instances:
  c1: { machine: Channel, fields: { channelId: "c-1", status: waiting } }
steps:
  - { source: Net, fire: Channel.match, target: c1, payload: { originAgent: a7, channelId: "c-2" } }
"#;

fn map(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

#[test]
fn sets_records_the_assigned_fields_after_the_transition() {
    let model = model(CHANNEL);
    let run = match simulate_run(&model, &scenario(RUN)) {
        Ok(run) => run,
        Err(err) => panic!("{err}"),
    };
    let trace = &run.trace;
    assert_eq!(
        lines(&model, trace),
        strs(&[
            "ext Net -> c1 match",
            "c1 awaiting -> confirmed",
            // `prev` reads `self.status` before the transition; `note` reads
            // a missing payload value and stays unset.
            r#"c1 sets {"fromAgent": "a7", "prev": "waiting", "status": "open"}"#,
            "c1 emit Confirmed",
            "c1 emit Superseded",
        ])
    );
    assert_eq!(causes(trace), [None, Some(0), Some(1), Some(1), Some(1)]);
    match &trace.steps[2].kind {
        TraceStepKind::SetFields { fields, .. } => {
            assert_eq!(fields, &map(&[("fromAgent", "a7"), ("prev", "waiting"), ("status", "open")]));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn emitted_payloads_are_fields_after_sets_overlaid_with_the_trigger_then_with() {
    let model = model(CHANNEL);
    let run = match simulate_run(&model, &scenario(RUN)) {
        Ok(run) => run,
        Err(err) => panic!("{err}"),
    };
    // Fields after `sets`, then the step payload on top.
    let confirmed = map(&[
        ("channelId", "c-2"),
        ("fromAgent", "a7"),
        ("originAgent", "a7"),
        ("prev", "waiting"),
        ("status", "open"),
    ]);
    assert_eq!(run.payloads.get(&StepIx(3)), Some(&confirmed));
    // Then each `with` entry: `self` is the instance after `sets`, `event` the
    // trigger's payload; a missing value adds nothing.
    let mut superseded = confirmed.clone();
    superseded.insert("channelId".into(), "c-1".into());
    superseded.insert("by".into(), "c-2".into());
    superseded.insert("now".into(), "open".into());
    assert_eq!(run.payloads.get(&StepIx(4)), Some(&superseded));
}

#[test]
fn sets_change_the_instance_for_later_guards_and_selectors() {
    let model = model(
        r#"
machines:
  Counter:
    fields: [level]
    initial: a
    states: [a, b, c, d]
    transitions:
      - { from: a, to: b, on: go, sets: { level: high } }
      - { from: b, to: c, on: go, guard: "self.level == high" }
      - { from: b, to: d, on: go, guard: else }
external:
  Api: [Counter.go]
"#,
    );
    let trace = common::run(
        &model,
        "scenario: s\ninstances:\n  k: Counter\nsteps:\n  - { source: Api, fire: Counter.go }\n  - { source: Api, fire: Counter.go }\n",
    );
    assert_eq!(common::finals(&model, &trace), ["k: c"]);
}

#[test]
fn sets_with_only_missing_values_record_nothing() {
    let model = model(
        r#"
machines:
  M:
    fields: [x]
    initial: a
    states: [a, b]
    transitions:
      - { from: a, to: b, on: go, sets: { x: event.missing } }
external:
  Api: [M.go]
"#,
    );
    let trace = common::run(&model, "scenario: s\ninstances:\n  m: M\nsteps:\n  - { source: Api, fire: M.go }\n");
    assert_eq!(lines(&model, &trace), strs(&["ext Api -> m go", "m a -> b"]));
}

#[test]
fn the_session_shows_fields_after_sets() {
    let model = model(CHANNEL);
    let session = match cascade_sim::PlaySession::from_scenario(&model, &scenario(RUN)) {
        Ok(session) => session,
        Err(err) => panic!("{err}"),
    };
    let fields = &session.instances()[0].fields;
    assert_eq!(fields.get("status").map(String::as_str), Some("open"));
    assert_eq!(fields.get("fromAgent").map(String::as_str), Some("a7"));
    assert_eq!(fields.get("note"), None);
}
