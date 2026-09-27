//! `cascade check`: load diagnostics plus static analysis findings.
//!
//! Owner: `feat/static-analysis` refines the output; the load and exit-code
//! contract is fixed.

use std::path::Path;
use std::process::ExitCode;

use cascade_core::{CausalGraph, Finding, LoadFileError, Model, Severity, analyze};

use crate::OutputFormat;

pub fn run(file: &Path, format: OutputFormat, deny_warnings: bool) -> anyhow::Result<ExitCode> {
    let model = match cascade_core::load_file(file) {
        Ok(model) => model,
        Err(LoadFileError::Invalid { path, source }) => {
            match format {
                OutputFormat::Text => {
                    for d in &source.diagnostics {
                        eprintln!("{}:{}: error: {}", path.display(), d.span, d.kind);
                    }
                }
                OutputFormat::Json => {
                    let diagnostics: Vec<_> = source
                        .diagnostics
                        .iter()
                        .map(|d| {
                            serde_json::json!({
                                "line": d.span.start.line,
                                "col": d.span.start.col,
                                "message": d.kind.to_string(),
                            })
                        })
                        .collect();
                    println!(
                        "{}",
                        serde_json::json!({ "file": path.display().to_string(), "diagnostics": diagnostics, "findings": [] })
                    );
                }
            }
            return Ok(ExitCode::from(2));
        }
        Err(err @ LoadFileError::Io { .. }) => return Err(err.into()),
    };

    let graph = CausalGraph::build(&model);
    let findings = analyze(&model, &graph);
    match format {
        OutputFormat::Text => print_text(file, &model, &findings),
        OutputFormat::Json => print_json(file, &model, &findings),
    }

    let fails = findings.iter().any(|f| match f.severity {
        Severity::Error => true,
        Severity::Warning => deny_warnings,
        Severity::Info => false,
    });
    Ok(if fails { ExitCode::from(1) } else { ExitCode::SUCCESS })
}

fn print_text(file: &Path, model: &Model, findings: &[Finding]) {
    for f in findings {
        let span = model.span_of(f.detail.primary());
        println!("{}:{}: {}[{}]: {}", file.display(), span, f.severity, f.check(), f.message);
    }
    let count = |s: Severity| findings.iter().filter(|f| f.severity == s).count();
    eprintln!(
        "{} error(s), {} warning(s), {} info",
        count(Severity::Error),
        count(Severity::Warning),
        count(Severity::Info)
    );
}

fn print_json(file: &Path, model: &Model, findings: &[Finding]) {
    let items: Vec<_> = findings
        .iter()
        .map(|f| {
            let span = model.span_of(f.detail.primary());
            let subjects: Vec<String> = f.detail.subjects().into_iter().map(|e| model.key_of(e).to_string()).collect();
            serde_json::json!({
                "check": f.check().code(),
                "severity": f.severity.to_string(),
                "message": f.message,
                "line": span.start.line,
                "col": span.start.col,
                "subjects": subjects,
            })
        })
        .collect();
    println!("{}", serde_json::json!({ "file": file.display().to_string(), "diagnostics": [], "findings": items }));
}
