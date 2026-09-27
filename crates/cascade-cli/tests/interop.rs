//! `cascade export`, `cascade import` and `cascade diff` end to end, through
//! the built binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_cascade");

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn example() -> PathBuf {
    root().join("examples/order-fulfillment/cascade.yaml")
}

fn cascade(args: &[&str]) -> Output {
    Command::new(BIN).args(args).output().expect("the cascade binary runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn code(output: &Output) -> Option<i32> {
    output.status.code()
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A fresh directory under the system temp directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "cascade-cli-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
            .args(["-c", "init.defaultBranch=main", "-c", "core.hooksPath=/dev/null"])
            .args(args)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?} failed");
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn arg(path: &Path) -> &str {
    path.to_str().expect("utf-8 temp path")
}

// --- export ------------------------------------------------------------------------

#[test]
fn export_every_format_of_the_example() {
    let file = example();
    for (format, starts_with) in [
        ("scxml", "<?xml version=\"1.0\""),
        ("mermaid", "stateDiagram-v2"),
        ("mermaid-causal", "flowchart LR"),
        ("p", "// P skeleton"),
        ("yaml", "machines:"),
    ] {
        let out = cascade(&["export", arg(&file), "--to", format]);
        assert_eq!(code(&out), Some(0), "{format}: {}", stderr(&out));
        assert!(stdout(&out).starts_with(starts_with), "{format}:\n{}", stdout(&out));
    }
}

#[test]
fn export_writes_to_a_file() {
    let dir = TempDir::new("export");
    let target = dir.path("out.scxml");
    let out = cascade(&["export", arg(&example()), "--to", "scxml", "-o", arg(&target)]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).is_empty());
    let text = std::fs::read_to_string(&target).expect("written");
    assert!(text.contains("<parallel id=\"system\">"));
}

#[test]
fn export_rejects_unknown_formats_and_invalid_definitions() {
    let out = cascade(&["export", arg(&example()), "--to", "dot"]);
    assert_eq!(code(&out), Some(2));
    assert!(stderr(&out).contains("mermaid-causal"), "{}", stderr(&out));

    let dir = TempDir::new("bad");
    let bad = dir.path("bad.yaml");
    std::fs::write(
        &bad,
        "machines:\n  A:\n    states: [a]\n    transitions:\n      - { from: a, to: nowhere, on: go }\n",
    )
    .expect("write");
    let out = cascade(&["export", arg(&bad), "--to", "mermaid"]);
    assert_eq!(code(&out), Some(2));
    assert!(stderr(&out).contains("nowhere"), "{}", stderr(&out));
}

// --- import ------------------------------------------------------------------------------

#[test]
fn import_xstate_fixtures_to_yaml() {
    for name in ["traffic-light.json", "fetch.json", "checkout.json"] {
        let file = root().join("examples/xstate").join(name);
        let out = cascade(&["import", arg(&file), "--from", "xstate"]);
        assert_eq!(code(&out), Some(0), "{name}: {}", stderr(&out));
        let yaml = stdout(&out);
        assert!(yaml.starts_with("machines:"), "{yaml}");
        assert!(cascade_core::load_str(&yaml).is_ok(), "{name} output loads");
    }
}

#[test]
fn import_reports_warnings_on_stderr() {
    let file = root().join("examples/xstate/traffic-light.json");
    let out = cascade(&["import", arg(&file), "--from", "xstate"]);
    assert_eq!(code(&out), Some(0));
    assert!(stderr(&out).contains("warning: "), "{}", stderr(&out));
    assert!(stderr(&out).contains("onDone"), "{}", stderr(&out));
}

#[test]
fn import_scxml_and_write_the_result_then_check_it() {
    let dir = TempDir::new("import");
    let target = dir.path("cascade.yaml");
    let file = root().join("examples/scxml/order-fulfillment.scxml");
    let out = cascade(&["import", arg(&file), "--from", "scxml", "-o", arg(&target)]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    let yaml = std::fs::read_to_string(&target).expect("written");
    let original = std::fs::read_to_string(example()).expect("example");
    // The spec example survives YAML → SCXML → YAML, comments aside.
    let strip = |t: &str| t.lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>().join("\n");
    assert_eq!(strip(&yaml).trim(), strip(&original).trim());
    // The written file is a valid definition for the other commands.
    let out = cascade(&["export", arg(&target), "--to", "yaml"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
}

#[test]
fn import_errors_exit_with_2() {
    let dir = TempDir::new("import-bad");
    let bad = dir.path("bad.json");
    std::fs::write(&bad, r#"{ "id": "m", "states": { "a": { "always": "b" }, "b": {} } }"#).expect("write");
    let out = cascade(&["import", arg(&bad), "--from", "xstate"]);
    assert_eq!(code(&out), Some(2));
    assert!(stderr(&out).contains("always"), "{}", stderr(&out));

    let out = cascade(&["import", arg(&bad), "--from", "bpmn"]);
    assert_eq!(code(&out), Some(2));
    assert!(stderr(&out).contains("unknown format"), "{}", stderr(&out));
}

// --- diff ----------------------------------------------------------------------------------

const V1: &str = "\
machines:
  Order:
    color: blue
    states: [draft, pending, paid]
    transitions:
      - { from: draft, to: pending, on: submit }
      - { from: pending, to: paid, on: pay, emits: [Paid] }
controllers:
  Billing:
    on:
      Paid:
        - fire: Order.submit
external:
  Customer: [Order.submit]
";

const V2: &str = "\
machines:
  Order:
    color: green
    states: [draft, pending, paid, refunded]
    transitions:
      - { from: draft, to: pending, on: submit }
      - { from: paid, to: refunded, on: refund }
external:
  Customer: [Order.submit, Order.refund]
";

fn diff_repo() -> (TempDir, PathBuf) {
    let dir = TempDir::new("diff");
    dir.git(&["init", "-q"]);
    let file = dir.path("cascade.yaml");
    std::fs::write(&file, V1).expect("write v1");
    dir.git(&["add", "cascade.yaml"]);
    dir.git(&["commit", "-q", "-m", "v1"]);
    std::fs::write(&file, V2).expect("write v2");
    (dir, file)
}

#[test]
fn diff_against_the_working_tree_as_text() {
    let (_dir, file) = diff_repo();
    let out = cascade(&["diff", arg(&file), "--base", "HEAD"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    for line in [
        "~ machine:Order",
        "+ state:Order:refunded",
        "- transition:Order:pending->paid@pay",
        "+ transition:Order:paid->refunded@refund",
        "- event:Paid",
        "- controller:Billing",
        "- rule:Billing/Paid#0",
        "~ external:Customer",
    ] {
        assert!(text.lines().any(|l| l == line), "missing {line:?} in\n{text}");
    }
    assert!(!text.contains("state:Order:draft"), "unchanged elements are not listed");
    assert!(text.trim_end().ends_with("changed"), "{text}");
}

#[test]
fn diff_between_revisions_as_json() {
    let (dir, file) = diff_repo();
    dir.git(&["commit", "-q", "-am", "v2"]);
    let out = cascade(&["diff", arg(&file), "--base", "HEAD~1", "--head", "HEAD", "--format", "json"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json output");
    assert_eq!(json["base"], "HEAD~1");
    assert_eq!(json["head"], "HEAD");
    let list = |k: &str| -> Vec<String> {
        json[k].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default()
    };
    assert!(list("added").contains(&"state:Order:refunded".to_owned()));
    assert!(list("removed").contains(&"controller:Billing".to_owned()));
    assert!(list("changed").contains(&"machine:Order".to_owned()));

    // No changes between a revision and itself.
    let out = cascade(&["diff", arg(&file), "--base", "HEAD", "--head", "HEAD"]);
    assert_eq!(code(&out), Some(0));
    assert!(stdout(&out).starts_with("no changes"), "{}", stdout(&out));
}

#[test]
fn diff_errors_exit_with_2() {
    let (dir, file) = diff_repo();
    let out = cascade(&["diff", arg(&file), "--base", "no-such-rev"]);
    assert_eq!(code(&out), Some(2));
    assert!(stderr(&out).contains("no-such-rev"), "{}", stderr(&out));

    // An invalid working copy is reported with diagnostics.
    std::fs::write(&file, "machines:\n  A: { states: [] }\n").expect("write");
    let out = cascade(&["diff", arg(&file), "--base", "HEAD"]);
    assert_eq!(code(&out), Some(2));
    assert!(stderr(&out).contains("error:"), "{}", stderr(&out));
    drop(dir);
}
