//! `cascade simulate` with the manual scenario steps, and the
//! `--interactive` prompt.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn race_definition() -> PathBuf {
    root().join("crates/cascade-sim/tests/fixtures/race/cascade.yaml")
}

fn race_scenario() -> PathBuf {
    root().join("crates/cascade-sim/tests/fixtures/race/scenario.yaml")
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

/// A fresh directory under the system temp dir (`$TMPDIR`).
fn temp_dir() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cascade-cli-play-{}-{n}", std::process::id()));
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

const RACE_START: &str = "scenario: manual\ninstances:\n  o1: { machine: Order, fields: { orderId: \"1\" } }\n  s1: { machine: Shipment, fields: { orderId: \"1\" } }\nsteps:\n  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }\n";

#[test]
fn manual_queue_steps_run_in_batch() {
    let scenario = temp_file("manual.yaml", &format!("{RACE_START}  - step\n  - {{ step: 1 }}\n"));
    let output = cascade(&["simulate", path_str(&race_definition()), path_str(&scenario)], "");
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Shipment s1: idle → on_hold (hold)"), "{text}");
    assert!(text.contains("Shipment s1: start dropped in on_hold"), "{text}");
}

#[test]
fn a_paused_scenario_leaves_the_queue() {
    let scenario = temp_file("paused.yaml", &format!("{RACE_START}  - step\nend: pause\n"));
    let output = cascade(&["simulate", path_str(&race_definition()), path_str(&scenario)], "");
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("   6          Billing fires hold at s1\nFinal states:"), "{text}");
    assert!(text.contains("  Shipment s1: idle\n"), "{text}");
}

#[test]
fn queue_step_problems_are_reported_with_positions() {
    let scenario = temp_file("bad-step.yaml", &format!("{RACE_START}  - step\n  - {{ step: 5 }}\n"));
    let output = cascade(&["simulate", path_str(&race_definition()), path_str(&scenario)], "");
    assert_eq!(output.status.code(), Some(2));
    let err = stderr(&output);
    assert!(err.contains(&format!("{}:8:", scenario.display())), "{err}");
    assert!(err.contains("no queue item at position 5"), "{err}");
}

#[test]
fn interactive_play_can_be_saved_and_replayed() {
    let saved = temp_dir().join("played.yaml");
    let input = format!(
        "add Order o1 orderId=1\nadd Shipment s1 orderId=1\nfires\nfire PaymentGateway Order.capture_ok o1\nstep\npending\nstep 1\nstep\ninstances\nsave {}\nquit\n",
        path_str(&saved)
    );
    let output = cascade(&["simulate", path_str(&race_definition()), "--interactive"], &input);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("PaymentGateway Order.capture_ok o1"), "{text}");
    assert!(text.contains("  0  Fulfillment → s1: start\n  1  Billing → s1: hold\n"), "{text}");
    assert!(text.contains("Shipment s1: idle → on_hold (hold)"), "{text}");
    assert!(text.contains("s1  Shipment  on_hold"), "{text}");
    assert!(text.contains(&format!("saved {}", saved.display())), "{text}");

    let yaml = match fs::read_to_string(&saved) {
        Ok(yaml) => yaml,
        Err(err) => panic!("cannot read {}: {err}", saved.display()),
    };
    assert!(yaml.contains("- { step: 1 }"), "{yaml}");
    let output = cascade(&["simulate", path_str(&race_definition()), path_str(&saved)], "");
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("Shipment s1: start dropped in on_hold"), "{}", stdout(&output));
}

#[test]
fn interactive_play_starts_from_a_scenario_and_rewinds() {
    let input = "instances\ntimeline\nseek 3\nstep\nrun\nbranches\nbranch 0\ntimeline\nquit\n";
    let output =
        cascade(&["simulate", path_str(&race_definition()), path_str(&race_scenario()), "--interactive"], input);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("o1  Order  paid"), "{text}");
    assert!(text.contains("  3  run until quiet\n"), "{text}");
    assert!(text.contains("  0  fork at 3: 4 actions\n"), "{text}");
}

#[test]
fn interactive_mistakes_are_reported_and_play_goes_on() {
    let input = "step\nfire Nobody Order.capture_ok o1\nadd Order\nfrobnicate\ninstances\n";
    let output = cascade(&["simulate", path_str(&race_definition()), "--interactive"], input);
    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    assert!(err.contains("error: nothing is queued to deliver"), "{err}");
    assert!(err.contains("error: unknown external source `Nobody`"), "{err}");
    assert!(err.contains("error: unknown command `frobnicate`"), "{err}");
    assert!(stdout(&output).contains("order1  Order  pending"), "{}", stdout(&output));
}

#[test]
fn a_scenario_is_required_without_interactive() {
    let output = cascade(&["simulate", path_str(&race_definition())], "");
    assert_eq!(output.status.code(), Some(2));
}
