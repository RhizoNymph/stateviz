//! `cascade check` end to end: the binary run against the examples and
//! temporary definitions, covering text output, JSON output and exit codes.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const SHOP: &str = "examples/shop/cascade.yaml";
const SPEC_EXAMPLE: &str = "examples/order-fulfillment/cascade.yaml";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Run `cascade` from the workspace root, so example paths print as given.
fn cascade(args: &[&str]) -> Output {
    match Command::new(env!("CARGO_BIN_EXE_cascade"))
        .args(args)
        .current_dir(workspace_root())
        .env_remove("RUST_LOG")
        .output()
    {
        Ok(output) => output,
        Err(err) => panic!("cannot run cascade: {err}"),
    }
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap_or_else(|| panic!("cascade was killed by a signal: {output:?}"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn json(output: &Output) -> Value {
    match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(err) => panic!("stdout is not JSON ({err}):\n{}", stdout(output)),
    }
}

/// A definition written to the temp directory, removed on drop.
struct TempDefinition {
    path: PathBuf,
}

impl TempDefinition {
    fn new(name: &str, text: &str) -> Self {
        let path = std::env::temp_dir().join(format!("cascade-check-{}-{name}.yaml", std::process::id()));
        if let Err(err) = std::fs::write(&path, text) {
            panic!("cannot write {}: {err}", path.display());
        }
        Self { path }
    }

    fn arg(&self) -> &str {
        self.path.to_str().unwrap_or_else(|| panic!("non-UTF-8 temp path {}", self.path.display()))
    }
}

impl Drop for TempDefinition {
    fn drop(&mut self) {
        // Best effort: a leftover temp file does not affect other tests.
        let _ = std::fs::remove_file(&self.path);
    }
}

const CLEAN: &str = r#"
machines:
  Lamp:
    states: [off, on]
    transitions:
      - { from: off, to: on, on: toggle }
      - { from: on, to: off, on: toggle }
external:
  User: [Lamp.toggle]
"#;

/// Only an info note: `start` is dropped once the job is running.
const INFO_ONLY: &str = r#"
machines:
  Trigger:
    states: [armed, fired]
    transitions:
      - { from: armed, to: fired, on: pull, emits: [Pulled] }
  Job:
    states: [idle, running]
    transitions:
      - { from: idle, to: running, on: start }
controllers:
  Runner:
    on:
      Pulled: [{ fire: Job.start }]
external:
  User: [Trigger.pull]
"#;

const INVALID: &str = r#"
machines:
  Order:
    states: [draft]
    transitions:
      - { from: draft, to: nowhere, on: submit }
"#;

#[test]
fn shop_text_output_matches_the_expected_findings_file() {
    let output = cascade(&["check", SHOP]);
    let expected = match std::fs::read_to_string(workspace_root().join("examples/shop/expected-findings.txt")) {
        Ok(text) => text,
        Err(err) => panic!("cannot read expected-findings.txt: {err}"),
    };
    assert_eq!(stdout(&output), expected);
    assert_eq!(stderr(&output), "");
    assert_eq!(code(&output), 1, "errors fail the check");
}

#[test]
fn text_lines_have_location_severity_and_check_code() {
    let output = cascade(&["check", SHOP]);
    let text = stdout(&output);
    let lines: Vec<&str> = text.lines().collect();
    let (summary, findings) = lines.split_last().unwrap_or_else(|| panic!("no output"));
    assert_eq!(*summary, "summary: 3 errors, 4 warnings, 12 info");
    for line in findings {
        let rest = line.strip_prefix(&format!("{SHOP}:")).unwrap_or_else(|| panic!("no file prefix: {line}"));
        let mut parts = rest.splitn(4, ':');
        let (line_no, col, label) = (parts.next(), parts.next(), parts.next());
        assert!(line_no.is_some_and(|l| l.parse::<u32>().is_ok_and(|n| n > 0)), "line number: {line}");
        assert!(col.is_some_and(|c| c.parse::<u32>().is_ok()), "column: {line}");
        let label = label.unwrap_or_default().trim();
        let (severity, check) = label.split_once('[').unwrap_or_else(|| panic!("no check code: {line}"));
        assert!(["error", "warning", "info"].contains(&severity), "severity: {line}");
        assert!(check.ends_with(']'), "check code: {line}");
        assert!(parts.next().is_some_and(|m| !m.trim().is_empty()), "message: {line}");
    }
}

#[test]
fn shop_json_report_is_structured() {
    let output = cascade(&["check", "--format", "json", SHOP]);
    assert_eq!(code(&output), 1);
    let report = json(&output);
    assert_eq!(report["file"], SHOP);
    assert_eq!(report["diagnostics"], Value::Array(Vec::new()));
    assert_eq!(report["summary"], serde_json::json!({ "errors": 3, "warnings": 4, "info": 12 }));

    let findings = report["findings"].as_array().unwrap_or_else(|| panic!("findings is not an array"));
    assert_eq!(findings.len(), 19);
    assert_eq!(
        findings[0],
        serde_json::json!({
            "check": "invalid-fire",
            "severity": "error",
            "message": "Fulfillment fires Shipment.cancel on OrderCancelled, but no Shipment transition accepts `cancel`",
            "line": findings[0]["line"],
            "col": findings[0]["col"],
            "primary": "rule:Fulfillment/OrderCancelled#0",
            "subjects": ["rule:Fulfillment/OrderCancelled#0", "trigger:Shipment.cancel"],
        })
    );
    let codes: Vec<&str> = findings.iter().filter_map(|f| f["check"].as_str()).collect();
    for check in [
        "invalid-fire",
        "nondeterminism",
        "cascade-cycle",
        "unhandled-event",
        "orphan-controller",
        "unreachable-state",
        "race-candidate",
        "state-dependent-fire",
    ] {
        assert!(codes.contains(&check), "no {check} finding in {codes:?}");
    }
    for f in findings {
        assert!(f["line"].as_u64().is_some_and(|l| l > 0), "{f}");
        let primary = f["primary"].as_str().unwrap_or_default();
        let subjects = f["subjects"].as_array().unwrap_or_else(|| panic!("{f}"));
        assert!(subjects.iter().any(|s| s == primary), "primary is a subject: {f}");
    }
}

#[test]
fn json_and_text_list_the_same_findings_in_the_same_order() {
    let text = stdout(&cascade(&["check", SHOP]));
    let report = json(&cascade(&["check", "--format", "json", SHOP]));
    let json_lines: Vec<String> = report["findings"]
        .as_array()
        .unwrap_or_else(|| panic!("no findings"))
        .iter()
        .map(|f| {
            format!(
                "{SHOP}:{}:{}: {}[{}]: {}",
                f["line"],
                f["col"],
                f["severity"].as_str().unwrap_or_default(),
                f["check"].as_str().unwrap_or_default(),
                f["message"].as_str().unwrap_or_default()
            )
        })
        .collect();
    let text_lines: Vec<&str> = text.lines().filter(|l| !l.starts_with("summary:")).collect();
    assert_eq!(text_lines, json_lines);
}

#[test]
fn json_output_is_stable_across_runs() {
    let first = stdout(&cascade(&["check", "--format", "json", SHOP]));
    let second = stdout(&cascade(&["check", "--format", "json", SHOP]));
    assert_eq!(first, second);
    assert_eq!(first.lines().count(), 1, "one JSON object on one line");
}

#[test]
fn warnings_pass_unless_denied() {
    let output = cascade(&["check", SPEC_EXAMPLE]);
    assert_eq!(code(&output), 0, "{}", stdout(&output));
    assert!(stdout(&output).ends_with("summary: 0 errors, 3 warnings, 1 info\n"), "{}", stdout(&output));
    assert!(stdout(&output).contains(&format!("{SPEC_EXAMPLE}:16:")), "unreachable Shipment.shipped on line 16");

    let denied = cascade(&["check", "--deny-warnings", SPEC_EXAMPLE]);
    assert_eq!(code(&denied), 1);
    assert_eq!(stdout(&denied), stdout(&output), "the flag changes only the exit code");
}

#[test]
fn info_notes_never_fail_the_check() {
    let def = TempDefinition::new("info-only", INFO_ONLY);
    let output = cascade(&["check", "--deny-warnings", def.arg()]);
    assert_eq!(code(&output), 0, "{}", stdout(&output));
    let text = stdout(&output);
    assert!(
        text.contains(": info[state-dependent-fire]: Runner fires Job.start on Pulled, which Job drops in running"),
        "{text}"
    );
    assert!(text.ends_with("summary: 0 errors, 0 warnings, 1 info\n"), "{text}");
}

#[test]
fn a_clean_definition_has_no_findings() {
    let def = TempDefinition::new("clean", CLEAN);
    let output = cascade(&["check", "--deny-warnings", def.arg()]);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout(&output), "summary: no findings\n");

    let report = json(&cascade(&["check", "--format", "json", def.arg()]));
    assert_eq!(report["findings"], Value::Array(Vec::new()));
    assert_eq!(report["summary"], serde_json::json!({ "errors": 0, "warnings": 0, "info": 0 }));
}

#[test]
fn an_invalid_definition_exits_2_with_diagnostics() {
    let def = TempDefinition::new("invalid", INVALID);
    let output = cascade(&["check", def.arg()]);
    assert_eq!(code(&output), 2);
    assert_eq!(stdout(&output), "");
    let err = stderr(&output);
    assert!(err.contains(&format!("{}:6:", def.arg())), "{err}");
    assert!(err.contains(": error: machine `Order` has no state `nowhere`"), "{err}");
    assert!(err.ends_with("summary: invalid definition (1 error); checks did not run\n"), "{err}");
}

#[test]
fn an_invalid_definition_in_json_lists_diagnostics() {
    let def = TempDefinition::new("invalid-json", INVALID);
    let output = cascade(&["check", "--format", "json", def.arg()]);
    assert_eq!(code(&output), 2);
    let report = json(&output);
    assert_eq!(report["file"], def.arg());
    assert_eq!(report["findings"], Value::Array(Vec::new()));
    let diagnostics = report["diagnostics"].as_array().unwrap_or_else(|| panic!("no diagnostics"));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0]["line"], 6);
    assert_eq!(diagnostics[0]["message"], "machine `Order` has no state `nowhere`");
}

#[test]
fn a_missing_file_exits_2() {
    let missing = std::env::temp_dir().join(format!("cascade-check-{}-missing.yaml", std::process::id()));
    let arg = missing.to_str().unwrap_or_else(|| panic!("non-UTF-8 temp path"));
    for format in ["text", "json"] {
        let output = cascade(&["check", "--format", format, arg]);
        assert_eq!(code(&output), 2);
        assert_eq!(stdout(&output), "");
        let err = stderr(&output);
        assert!(err.starts_with(&format!("error: cannot read {arg}: ")), "{err}");
        assert_eq!(err.lines().count(), 1, "{err}");
    }
}
