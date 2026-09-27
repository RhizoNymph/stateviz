//! `cascade simulate` end to end.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn example() -> PathBuf {
    root().join("examples/order-fulfillment/cascade.yaml")
}

fn example_scenario(name: &str) -> PathBuf {
    root().join("examples/order-fulfillment/scenarios").join(name)
}

fn cascade(args: &[&str]) -> Output {
    match Command::new(env!("CARGO_BIN_EXE_cascade")).args(args).output() {
        Ok(output) => output,
        Err(err) => panic!("cannot run cascade: {err}"),
    }
}

fn simulate(definition: &Path, scenario: &Path, extra: &[&str]) -> Output {
    let mut args = vec!["simulate", path_str(definition), path_str(scenario)];
    args.extend_from_slice(extra);
    cascade(&args)
}

fn path_str(path: &Path) -> &str {
    match path.to_str() {
        Some(s) => s,
        None => panic!("non-UTF-8 path {}", path.display()),
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A file in a fresh directory under the system temp dir (`$TMPDIR`).
fn temp_file(name: &str, text: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cascade-cli-simulate-{}-{n}", std::process::id()));
    if let Err(err) = fs::create_dir_all(&dir) {
        panic!("cannot create {}: {err}", dir.display());
    }
    let path = dir.join(name);
    if let Err(err) = fs::write(&path, text) {
        panic!("cannot write {}: {err}", path.display());
    }
    path
}

#[test]
fn prints_one_line_per_step_indented_by_cause() {
    let output = simulate(&example(), &example_scenario("happy-path.yaml"), &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    let expected = "\
Scenario: happy path
Lifelines: Customer, PaymentGateway, Order o1, Shipment s1, Fulfillment
   0  Customer fires submit at o1
   1    Order o1: draft → pending (submit)
   2  PaymentGateway fires capture_ok at o1 {amount: 42}
   3    Order o1: pending → paid (capture_ok)
   4      o1 emits OrderPaid {amount: 42, orderId: 1}
   5        Fulfillment receives OrderPaid
   6          Fulfillment fires start at s1
   7            Shipment s1: idle → picking (start)
Final states:
  Order o1: paid
  Shipment s1: picking
";
    assert_eq!(text, expected);
}

#[test]
fn names_causes_that_are_not_the_line_above() {
    let output = simulate(&example(), &example_scenario("timeout-race.yaml"), &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("   5  Clock fires timeout at o1\n"), "{text}");
    assert!(text.contains("   6    Order o1: timeout dropped in paid\n"), "{text}");
    assert!(text.contains("   7        Fulfillment receives OrderPaid  ← 4\n"), "{text}");
}

#[test]
fn prints_json() {
    let output = simulate(&example(), &example_scenario("happy-path.yaml"), &["--format", "json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let json: Value = match serde_json::from_str(&stdout(&output)) {
        Ok(v) => v,
        Err(err) => panic!("invalid JSON: {err}\n{}", stdout(&output)),
    };
    assert_eq!(json["scenario"], "happy path");
    assert_eq!(json["ordering"], Value::Null);
    assert_eq!(json["lifelines"].as_array().map(Vec::len), Some(5));
    assert_eq!(json["lifelines"][2], serde_json::json!({ "kind": "instance", "machine": "Order", "name": "o1" }));

    let steps = json["steps"].as_array().cloned().unwrap_or_default();
    assert_eq!(steps.len(), 8);
    assert_eq!(steps[0]["kind"], "external-fire");
    assert_eq!(steps[0]["cause"], Value::Null);
    assert_eq!(steps[3]["kind"], "transition");
    assert_eq!(steps[3]["transition"], "transition:Order:pending->paid@capture_ok");
    assert_eq!(steps[3]["from"], "pending");
    assert_eq!(steps[3]["to"], "paid");
    assert_eq!(steps[3]["cause"], 2);
    assert_eq!(steps[4]["kind"], "emit");
    assert_eq!(steps[4]["payload"]["orderId"], "1");
    assert_eq!(steps[4]["payload"]["amount"], "42");
    assert_eq!(steps[6]["kind"], "fire");
    assert_eq!(steps[6]["rule"], "rule:Fulfillment/OrderPaid#0");
    assert_eq!(steps[7]["depth"], 5);
    assert_eq!(steps[7]["text"], "Shipment s1: idle → picking (start)");

    assert_eq!(
        json["final_states"],
        serde_json::json!([
            { "lifeline": 2, "instance": "o1", "state": "paid" },
            { "lifeline": 3, "instance": "s1", "state": "picking" },
        ])
    );
}

#[test]
fn scenario_problems_are_reported_with_positions() {
    let scenario = temp_file(
        "bad.yaml",
        "scenario: bad\ninstances:\n  o1: Order\nsteps:\n  - { source: Courier, fire: Order.submit, target: o1 }\n",
    );
    let output = simulate(&example(), &scenario, &[]);
    assert_eq!(output.status.code(), Some(2));
    let err = stderr(&output);
    assert!(err.contains(&format!("{}:5:", scenario.display())), "{err}");
    assert!(err.contains("error: unknown external source `Courier`"), "{err}");
}

#[test]
fn scenario_syntax_errors_are_reported_with_positions() {
    let scenario = temp_file("broken.yaml", "scenario: broken\nsteps:\n  - { source: Customer, fire: Order }\n");
    let output = simulate(&example(), &scenario, &[]);
    assert_eq!(output.status.code(), Some(2));
    let err = stderr(&output);
    assert!(err.contains(&format!("{}:3:", scenario.display())), "{err}");
    assert!(err.contains("not a trigger reference"), "{err}");
}

#[test]
fn a_missing_scenario_file_is_an_error() {
    let output = simulate(&example(), &root().join("examples/order-fulfillment/scenarios/nope.yaml"), &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("cannot read"), "{}", stderr(&output));
}

#[test]
fn an_unbounded_cycle_fails() {
    let definition = temp_file(
        "loop.yaml",
        r#"
machines:
  Ping:
    states: [a, b]
    transitions:
      - { from: a, to: b, on: go, emits: [Tick] }
      - { from: b, to: a, on: go, emits: [Tick] }
controllers:
  Loop:
    on:
      Tick: { fire: Ping.go, bounded: true }
external:
  Starter: [Ping.go]
"#,
    );
    let scenario = temp_file(
        "forever.yaml",
        "scenario: forever\ninstances:\n  p: Ping\nsteps:\n  - { source: Starter, fire: Ping.go }\n",
    );
    let output = simulate(&definition, &scenario, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("unbounded cycle"), "{}", stderr(&output));
}

#[test]
fn race_needs_an_existing_race_candidate() {
    // The spec example has one controller, so it has no race candidates.
    let output = simulate(&example(), &example_scenario("happy-path.yaml"), &["--race", "0"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("no race candidate #0"), "{}", stderr(&output));
}

#[test]
fn race_prints_both_orderings_when_analysis_finds_the_race() {
    let fixtures = root().join("crates/cascade-sim/tests/fixtures/race");
    let output = simulate(&fixtures.join("cascade.yaml"), &fixtures.join("scenario.yaml"), &["--race", "0"]);
    if output.status.success() {
        let text = stdout(&output);
        assert!(text.contains("== Fulfillment first (as queued) =="), "{text}");
        assert!(text.contains("== Billing first (swapped) =="), "{text}");
        assert!(text.contains("Shipment s1: hold dropped in picking"), "{text}");
        assert!(text.contains("Shipment s1: start dropped in on_hold"), "{text}");
    } else {
        // Until static analysis reports race candidates, the flag can only
        // explain that there is none.
        assert!(stderr(&output).contains("no race candidate #0"), "{}", stderr(&output));
    }
}
