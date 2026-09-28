//! `cascade check`: load diagnostics plus static analysis findings.
//!
//! Text output (standard output) is one line per finding in analysis order,
//! then a summary line:
//!
//! ```text
//! examples/shop/cascade.yaml:175:11: error[invalid-fire]: Fulfillment fires …
//! summary: 3 errors, 4 warnings, 12 info
//! ```
//!
//! JSON output is one object on one line:
//! `{file, diagnostics: [{line, col, message}], findings: [{check, severity,
//! message, line, col, primary, subjects}], summary: {errors, warnings,
//! info}}`, where `primary` and `subjects` are element keys.
//!
//! Exit codes: 0 when there are no errors (and no warnings under
//! `--deny-warnings`); 1 when there are; 2 when the definition is invalid
//! (its diagnostics go to standard error in text mode, into `diagnostics` in
//! JSON mode) or cannot be read (an `error: cannot read …` line on standard
//! error in both modes).

use std::fmt;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;
use cascade_core::{CausalGraph, Check, Finding, LoadError, LoadFileError, Model, Severity, analyze};
use serde::Serialize;

use crate::OutputFormat;

/// Findings fail the check.
const EXIT_FINDINGS: u8 = 1;
/// The definition is invalid.
const EXIT_INVALID: u8 = 2;

pub fn run(file: &Path, format: OutputFormat, deny_warnings: bool) -> anyhow::Result<ExitCode> {
    let model = match cascade_core::load_file(file) {
        Ok(model) => model,
        Err(LoadFileError::Invalid { path, source }) => {
            tracing::debug!(file = %path.display(), diagnostics = source.diagnostics.len(), "definition is invalid");
            match format {
                OutputFormat::Text => write_stderr(&render_diagnostics_text(&path, &source))?,
                OutputFormat::Json => write_stdout(&render_json(&path, JsonBody::Invalid(&source))?)?,
            }
            return Ok(ExitCode::from(EXIT_INVALID));
        }
        Err(err @ LoadFileError::Io { .. }) => {
            // The error's message already names its cause; printing the
            // source chain as well would repeat it.
            tracing::debug!(file = %file.display(), error = %err, "cannot read definition");
            write_stderr(&format!("error: {err}\n"))?;
            return Ok(ExitCode::from(EXIT_INVALID));
        }
    };

    let graph = CausalGraph::build(&model);
    let findings = analyze(&model, &graph);
    let summary = Summary::of(&findings);
    tracing::debug!(
        file = %file.display(),
        errors = summary.errors,
        warnings = summary.warnings,
        info = summary.info,
        "checked definition"
    );

    let report = match format {
        OutputFormat::Text => render_text(file, &model, &findings, summary),
        OutputFormat::Json => render_json(file, JsonBody::Checked { model: &model, findings: &findings, summary })?,
    };
    write_stdout(&report)?;
    Ok(if summary.fails(deny_warnings) { ExitCode::from(EXIT_FINDINGS) } else { ExitCode::SUCCESS })
}

/// Finding counts by severity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
struct Summary {
    errors: usize,
    warnings: usize,
    info: usize,
}

impl Summary {
    fn of(findings: &[Finding]) -> Self {
        findings.iter().fold(Self::default(), |mut s, f| {
            match f.severity {
                Severity::Error => s.errors += 1,
                Severity::Warning => s.warnings += 1,
                Severity::Info => s.info += 1,
            }
            s
        })
    }

    fn fails(self, deny_warnings: bool) -> bool {
        self.errors > 0 || (deny_warnings && self.warnings > 0)
    }
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == Self::default() {
            return f.write_str("no findings");
        }
        let plural = |n: usize, word: &str| if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") };
        write!(f, "{}, {}, {} info", plural(self.errors, "error"), plural(self.warnings, "warning"), self.info)
    }
}

/// `path:line:col: severity[check]: message` per finding, then the summary.
fn render_text(file: &Path, model: &Model, findings: &[Finding], summary: Summary) -> String {
    let mut out = String::new();
    for f in findings {
        let span = model.span_of(f.detail.primary());
        out.push_str(&format!("{}:{}: {}[{}]: {}\n", file.display(), span, f.severity, f.check(), f.message));
    }
    out.push_str(&format!("summary: {summary}\n"));
    out
}

fn render_diagnostics_text(path: &Path, error: &LoadError) -> String {
    let mut out = String::new();
    for d in &error.diagnostics {
        out.push_str(&format!("{}:{}: error: {}\n", path.display(), d.span, d.kind));
    }
    let n = error.diagnostics.len();
    let noun = if n == 1 { "error" } else { "errors" };
    out.push_str(&format!("summary: invalid definition ({n} {noun}); checks did not run\n"));
    out
}

/// What the JSON report describes: a definition that failed to load, or the
/// findings of one that loaded.
enum JsonBody<'a> {
    Invalid(&'a LoadError),
    Checked { model: &'a Model, findings: &'a [Finding], summary: Summary },
}

#[derive(Serialize)]
struct JsonReport<'a> {
    file: String,
    diagnostics: Vec<JsonDiagnostic>,
    findings: Vec<JsonFinding<'a>>,
    summary: Summary,
}

#[derive(Serialize)]
struct JsonDiagnostic {
    line: u32,
    col: u32,
    message: String,
}

#[derive(Serialize)]
struct JsonFinding<'a> {
    check: Check,
    severity: Severity,
    message: &'a str,
    line: u32,
    col: u32,
    /// Element key of the element to focus.
    primary: String,
    /// Element keys of every element to badge.
    subjects: Vec<String>,
}

fn render_json(file: &Path, body: JsonBody<'_>) -> anyhow::Result<String> {
    let report = match body {
        JsonBody::Invalid(error) => JsonReport {
            file: file.display().to_string(),
            diagnostics: error
                .diagnostics
                .iter()
                .map(|d| JsonDiagnostic { line: d.span.start.line, col: d.span.start.col, message: d.kind.to_string() })
                .collect(),
            findings: Vec::new(),
            summary: Summary::default(),
        },
        JsonBody::Checked { model, findings, summary } => JsonReport {
            file: file.display().to_string(),
            diagnostics: Vec::new(),
            findings: findings
                .iter()
                .map(|f| {
                    let primary = f.detail.primary();
                    let span = model.span_of(primary);
                    JsonFinding {
                        check: f.check(),
                        severity: f.severity,
                        message: &f.message,
                        line: span.start.line,
                        col: span.start.col,
                        primary: model.key_of(primary).to_string(),
                        subjects: f.detail.subjects().into_iter().map(|e| model.key_of(e).to_string()).collect(),
                    }
                })
                .collect(),
            summary,
        },
    };
    let mut json = serde_json::to_string(&report).context("cannot serialize the check report")?;
    json.push('\n');
    Ok(json)
}

fn write_stdout(text: &str) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_all(text.as_bytes()).and_then(|()| out.flush()).context("cannot write to standard output")
}

fn write_stderr(text: &str) -> anyhow::Result<()> {
    let mut err = std::io::stderr().lock();
    err.write_all(text.as_bytes()).and_then(|()| err.flush()).context("cannot write to standard error")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_reads_naturally() {
        assert_eq!(Summary::default().to_string(), "no findings");
        assert_eq!(Summary { errors: 1, warnings: 0, info: 0 }.to_string(), "1 error, 0 warnings, 0 info");
        assert_eq!(Summary { errors: 3, warnings: 1, info: 12 }.to_string(), "3 errors, 1 warning, 12 info");
    }

    #[test]
    fn warnings_fail_only_when_denied() {
        let warnings = Summary { errors: 0, warnings: 2, info: 5 };
        assert!(!warnings.fails(false));
        assert!(warnings.fails(true));
        let info = Summary { errors: 0, warnings: 0, info: 5 };
        assert!(!info.fails(true));
        let errors = Summary { errors: 1, warnings: 0, info: 0 };
        assert!(errors.fails(false));
    }
}
