//! `cascade explore` end to end.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn race() -> (PathBuf, PathBuf) {
    let dir = root().join("crates/cascade-sim/tests/fixtures/race");
    (dir.join("cascade.yaml"), dir.join("scenario.yaml"))
}

fn orders(scenario: &str) -> (PathBuf, PathBuf) {
    let dir = root().join("examples/order-fulfillment");
    (dir.join("cascade.yaml"), dir.join("scenarios").join(scenario))
}

fn saga(scenario: &str) -> (PathBuf, PathBuf) {
    let dir = root().join("examples/checkout-saga");
    (dir.join("cascade.yaml"), dir.join("scenarios").join(scenario))
}

fn path_str(path: &Path) -> &str {
    match path.to_str() {
        Some(s) => s,
        None => panic!("non-UTF-8 path {}", path.display()),
    }
}

fn cascade(args: &[&str]) -> Output {
    match Command::new(env!("CARGO_BIN_EXE_cascade")).args(args).output() {
        Ok(output) => output,
        Err(err) => panic!("cannot run cascade: {err}"),
    }
}

fn explore((definition, scenario): &(PathBuf, PathBuf), extra: &[&str]) -> Output {
    let mut args = vec!["explore", path_str(definition), path_str(scenario)];
    args.extend_from_slice(extra);
    cascade(&args)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn temp_dir() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cascade-cli-explore-{}-{n}", std::process::id()));
    if let Err(err) = fs::create_dir_all(&dir) {
        panic!("cannot create {}: {err}", dir.display());
    }
    dir
}

#[test]
fn the_race_has_two_outcomes_and_exits_1() {
    let output = explore(&race(), &[]);
    assert_eq!(output.status.code(), Some(1), "{}", String::from_utf8_lossy(&output.stderr));
    let text = stdout(&output);
    assert!(text.starts_with("Exploration: payment captured\n"), "{text}");
    assert!(text.contains("2 outcomes, "), "{text}");
    assert!(text.contains("(complete)"), "{text}");
    assert!(text.contains("Outcome 1 ("), "{text}");
    assert!(text.contains("Instances (differing from outcome 1):\n    s1 (Shipment): on_hold {orderId: 1}"), "{text}");
    assert!(text.contains("s1: start dropped in on_hold"), "{text}");
    assert!(text.contains("    - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }"), "{text}");
}

#[test]
fn one_clean_outcome_exits_0() {
    let output = explore(&orders("happy-path.yaml"), &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(stdout(&output).contains("1 outcome, "));
    assert!(stdout(&output).contains("Anomalies: none"));
}

#[test]
fn the_state_bound_exits_3() {
    let output = explore(&race(), &["--max-states", "2"]);
    assert_eq!(output.status.code(), Some(3));
    assert!(stdout(&output).contains("INCOMPLETE: bound of 2 hit"), "{}", stdout(&output));
}

#[test]
fn notes_are_printed() {
    let output = explore(&orders("second-order-first.yaml"), &[]);
    let text = stdout(&output);
    assert!(text.contains("Note: 2 manual queue directives were ignored"), "{text}");
}

#[test]
fn flags_change_the_options() {
    let output = explore(&saga("happy-path.yaml"), &["--unordered", "--choices", "--timers", "--no-reduction"]);
    let text = stdout(&output);
    assert!(
        text.contains("Options: ordering: unordered, choices: every option, timers: any time, no reduction"),
        "{text}"
    );
    assert!(text.contains("Note: 1 choose decision was ignored"), "{text}");
    assert!(text.contains("- { choose: \"card declined\" }"), "{text}");
    assert!(text.contains("- { wait: ~, timing: immediate }"), "{text}");
}

#[test]
fn json_lists_outcomes() {
    let output = explore(&race(), &["--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    let value: Value = match serde_json::from_slice(&output.stdout) {
        Ok(v) => v,
        Err(err) => panic!("not JSON: {err}\n{}", stdout(&output)),
    };
    assert_eq!(value["scenario"], "payment captured");
    assert_eq!(value["complete"], true);
    assert_eq!(value["options"]["ordering"], "per-consumer");
    let outcomes = value["outcomes"].as_array().cloned().unwrap_or_default();
    assert_eq!(outcomes.len(), 2);
    let first = &outcomes[0];
    assert_eq!(first["index"], 0);
    assert!(first["paths"].as_u64().is_some_and(|p| p >= 1));
    assert_eq!(first["instances"][0]["name"], "o1");
    assert_eq!(first["instances"][0]["machine"], "Order");
    assert_eq!(first["instances"][0]["state"], "paid");
    assert_eq!(first["anomalies"].as_array().map(Vec::len), Some(1));
    assert_eq!(first["witness"][0], "{ source: PaymentGateway, fire: Order.capture_ok, target: o1 }");
}

#[test]
fn saved_witnesses_simulate_to_their_outcome() {
    let dir = temp_dir();
    let output = explore(&race(), &["--save-witness", path_str(&dir)]);
    assert_eq!(output.status.code(), Some(1));
    let (definition, _) = race();
    let mut finals = Vec::new();
    for n in 1..=2 {
        let path = dir.join(format!("scenario-outcome-{n}.yaml"));
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(text.contains(&format!("scenario: \"payment captured (outcome {n})\"")), "{text}");
        let simulated = cascade(&["simulate", path_str(&definition), path_str(&path)]);
        assert_eq!(simulated.status.code(), Some(0), "{}", String::from_utf8_lossy(&simulated.stderr));
        let trace = stdout(&simulated);
        let line = trace.lines().find(|l| l.starts_with("  Shipment s1:")).map(str::to_owned);
        finals.push(line.unwrap_or_default());
    }
    finals.sort();
    assert_eq!(finals, ["  Shipment s1: on_hold", "  Shipment s1: picking"]);
}

#[test]
fn scenario_problems_exit_2() {
    let dir = temp_dir();
    let bad = dir.join("bad.yaml");
    if let Err(err) = fs::write(&bad, "scenario: bad\ninstances:\n  o1: { machine: Nope }\nsteps: []\n") {
        panic!("{err}");
    }
    let (definition, _) = race();
    let output = explore(&(definition, bad), &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("bad.yaml:3:"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
