//! `cascade simulate` with subscriptions, timers, choices and per-consumer
//! steps: text and JSON output, scenario diagnostics, and the interactive
//! prompt's `wait`, `choose`, `step n to c`, `timers` and `now`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn saga() -> PathBuf {
    root().join("examples/checkout-saga/cascade.yaml")
}

fn saga_scenario(name: &str) -> PathBuf {
    root().join("examples/checkout-saga/scenarios").join(name)
}

fn path_str(path: &Path) -> &str {
    match path.to_str() {
        Some(s) => s,
        None => panic!("non-UTF-8 path {}", path.display()),
    }
}

fn cascade(args: &[&str], input: &str) -> Output {
    let mut child = match Command::new(env!("CARGO_BIN_EXE_cascade"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => panic!("cannot run cascade: {err}"),
    };
    if let Some(mut stdin) = child.stdin.take()
        && let Err(err) = stdin.write_all(input.as_bytes())
    {
        panic!("cannot write to cascade: {err}");
    }
    match child.wait_with_output() {
        Ok(output) => output,
        Err(err) => panic!("cannot wait for cascade: {err}"),
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn temp_dir() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cascade-cli-semantics-{}-{n}", std::process::id()));
    if let Err(err) = fs::create_dir_all(&dir) {
        panic!("cannot create {}: {err}", dir.display());
    }
    dir
}

fn temp_file(name: &str, text: &str) -> PathBuf {
    let path = temp_dir().join(name);
    if let Err(err) = fs::write(&path, text) {
        panic!("cannot write {}: {err}", path.display());
    }
    path
}

#[test]
fn text_output_describes_timers_choices_notifies_and_sets() {
    let output = cascade(&["simulate", path_str(&saga()), path_str(&saga_scenario("timeout.yaml"))], "");
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    for expected in [
        "   9  waited 30m (now 30m)\n",
        "  10    Checkout checkout1: timer after-30m expired\n",
        "  11      Checkout checkout1: waiting → expired (after-30m)\n",
        "o1 receives CheckoutFailed\n",
        "p1 chose approved (default)\n",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }

    let output = cascade(&["simulate", path_str(&saga()), path_str(&saga_scenario("happy-path.yaml"))], "");
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("p1 chose approved\n"), "{text}");
    assert!(text.contains("checkout1 receives PaymentCaptured\n"), "{text}");
    assert!(text.contains("checkout1 sets {paid: 50}\n"), "{text}");
    assert!(text.contains("  Checkout checkout1: done\n"), "{text}");
}

#[test]
fn json_output_has_the_new_step_kinds() {
    let output =
        cascade(&["simulate", path_str(&saga()), path_str(&saga_scenario("timeout.yaml")), "--format", "json"], "");
    assert!(output.status.success(), "{}", stderr(&output));
    let json: Value = match serde_json::from_str(&stdout(&output)) {
        Ok(json) => json,
        Err(err) => panic!("not JSON: {err}\n{}", stdout(&output)),
    };
    let steps = json["steps"].as_array().cloned().unwrap_or_default();
    let of_kind = |kind: &str| steps.iter().find(|s| s["kind"] == kind).cloned().unwrap_or(Value::Null);
    let wait = of_kind("wait");
    assert_eq!(wait["by"], "30m");
    assert_eq!(wait["now"], "30m");
    assert_eq!(wait["cause"], Value::Null);
    let expired = of_kind("timer-expired");
    assert_eq!(expired["cause"], wait["index"]);
    assert!(expired["trigger"].as_str().is_some_and(|t| t.contains("after-30m")), "{expired}");
    let choice = of_kind("choice");
    assert_eq!(choice["chosen"], "approved");
    assert_eq!(choice["defaulted"], true);
    assert_eq!(choice["options"], serde_json::json!(["approved", "card declined"]));
    assert_eq!(of_kind("notify")["event"], "CheckoutFailed");

    let output =
        cascade(&["simulate", path_str(&saga()), path_str(&saga_scenario("happy-path.yaml")), "--format", "json"], "");
    let json: Value = serde_json::from_str(&stdout(&output)).unwrap_or(Value::Null);
    let steps = json["steps"].as_array().cloned().unwrap_or_default();
    let sets = steps.iter().find(|s| s["kind"] == "set-fields").cloned().unwrap_or(Value::Null);
    assert_eq!(sets["fields"], serde_json::json!({ "paid": "50" }));
    assert_eq!(steps.iter().find(|s| s["kind"] == "choice").map(|c| c["defaulted"].clone()), Some(Value::Bool(false)));
}

#[test]
fn unused_choices_and_bad_values_are_scenario_errors() {
    let scenario = temp_file(
        "unused.yaml",
        "scenario: unused\ninstances:\n  o1: { machine: Order, fields: { orderId: \"1\", amount: \"50\" } }\nsteps:\n  - { choose: maybe }\n",
    );
    let output = cascade(&["simulate", path_str(&saga()), path_str(&scenario)], "");
    assert_eq!(output.status.code(), Some(2));
    let err = stderr(&output);
    assert!(err.contains(&format!("{}:5:", scenario.display())), "{err}");
    assert!(err.contains("no choice point took the decision `maybe`"), "{err}");

    let definition =
        temp_file("typed-definition.yaml", "machines:\n  Order:\n    fields: { amount: number }\n    states: [open]\n");
    let scenario = temp_file(
        "typed.yaml",
        "scenario: typed\ninstances:\n  o1: { machine: Order, fields: { amount: lots } }\nsteps: []\n",
    );
    let output = cascade(&["simulate", path_str(&definition), path_str(&scenario)], "");
    assert_eq!(output.status.code(), Some(2), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("`lots` is not a valid `amount`: expected a decimal number"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn interactive_wait_choose_consumer_steps_timers_and_now() {
    let saved = temp_dir().join("saga-played.yaml");
    let input = format!(
        "add Order o1 orderId=1 amount=50\n\
         add Payment p1 orderId=1 amount=50\n\
         add Stock s1 orderId=1\n\
         now\n\
         fire Customer Order.place o1\n\
         pending\n\
         step 0 to Saga\n\
         run\n\
         timers\n\
         wait 10m\n\
         now\n\
         choose card declined\n\
         timers\n\
         fire Bank Payment.respond p1\n\
         run\n\
         wait\n\
         wait soon\n\
         save {}\n\
         quit\n",
        path_str(&saved)
    );
    let output = cascade(&["simulate", path_str(&saga()), "--interactive"], &input);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    for expected in [
        "  now 0s\n",
        "  0  OrderPlaced from o1  (to Saga)\n",
        "Saga spawns Checkout checkout1",
        "  checkout1  after-30m  at 30m (in 30m)\n",
        "waited 10m (now 10m)",
        "  now 10m\n",
        "  decision pending: card declined\n",
        "  decision: card declined\n",
        "p1 chose card declined\n",
        "Order o1: placed → cancelled (CheckoutFailed)",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
    let err = stderr(&output);
    assert!(err.contains("error: no timer is armed"), "{err}");
    assert!(err.contains("error: `soon` is not a duration"), "{err}");

    let yaml = match fs::read_to_string(&saved) {
        Ok(yaml) => yaml,
        Err(err) => panic!("cannot read {}: {err}", saved.display()),
    };
    assert!(yaml.contains("- { step: 0, consumer: Saga }"), "{yaml}");
    assert!(yaml.contains("- { wait: 10m }"), "{yaml}");
    assert!(yaml.contains("- { choose: \"card declined\" }"), "{yaml}");
    let output = cascade(&["simulate", path_str(&saga()), path_str(&saved)], "");
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("  Order o1: cancelled\n"), "{}", stdout(&output));
}

#[test]
fn interactive_step_to_an_unknown_consumer_is_an_error() {
    let input =
        "add Order o1 orderId=1 amount=50\nfire Customer Order.place o1\nstep 0 to Nobody\nstep to Saga\npending\n";
    let output = cascade(&["simulate", path_str(&saga()), "--interactive"], input);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("error: `Nobody` is not a remaining consumer of `OrderPlaced from o1`"),
        "{}",
        stderr(&output)
    );
    assert!(stdout(&output).contains("Saga spawns Checkout checkout1"), "{}", stdout(&output));
}

#[test]
fn a_subscription_race_is_shown_as_queued_with_a_note() {
    let happy = saga_scenario("happy-path.yaml");
    let output = cascade(&["simulate", path_str(&saga()), path_str(&happy), "--race", "0"], "");
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.starts_with("Race candidate #0: PaymentCaptured leads Order"), "{text}");
    assert!(text.contains("note: not replayed in both orders: it involves a subscription"), "{text}");
    assert!(text.contains("Final states:"), "the as-queued trace follows: {text}");
    assert!(!text.contains("(swapped)"), "{text}");

    let output = cascade(&["simulate", path_str(&saga()), path_str(&happy), "--race", "0", "--format", "json"], "");
    assert!(output.status.success(), "{}", stderr(&output));
    let json: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(json["replayed"], Value::Bool(false));
    assert!(json["note"].as_str().is_some_and(|n| n.contains("subscription")), "{json}");
    assert!(json["as_queued"].is_object(), "{json}");
    assert!(json.get("swapped").is_none(), "{json}");
}

#[test]
fn rendering_a_subscription_race_renders_the_scenario_as_queued() {
    let out = temp_dir().join("saga-race.svg");
    let (saga, happy) = (saga(), saga_scenario("happy-path.yaml"));
    let args = [
        "render",
        path_str(&saga),
        "--view",
        "trace",
        "--scenario",
        path_str(&happy),
        "--state",
        "cascade://trace?race=0",
        "--out",
        path_str(&out),
    ];
    let output = cascade(&args, "");
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("race candidate #0 is not replayed in both orders"), "{}", stderr(&output));
    let svg = fs::read_to_string(&out).expect("written");
    assert!(svg.contains("<svg"));
}
