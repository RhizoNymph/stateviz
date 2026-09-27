//! `cascade diff --base <rev> [--head <rev>] [--format text|json]`: what
//! changed in a definition between two git revisions (or a revision and the
//! working tree).
//!
//! Text output is one line per element that is not unchanged, grouped by
//! element kind (`+` added, `-` removed, `~` changed), then a summary line.
//! JSON output is `{ file, base, head, added, removed, changed }` with
//! element keys as strings (`head` is `null` for the working tree). A
//! definition that does not load at either side is reported like
//! `cascade check` does and exits with 2.
//!
//! Owner: `feat/interop-and-diff`.

use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;

use cascade_core::diff::{DiffStatus, ModelDiff, diff_models};
use cascade_core::{LoadError, Model};

use crate::OutputFormat;

pub fn run(file: &Path, base: &str, head: Option<&str>, format: OutputFormat) -> anyhow::Result<ExitCode> {
    let old_text = cascade_interop::read_at_rev(file, base)
        .with_context(|| format!("cannot read {} at `{base}`", file.display()))?;
    let new_text = match head {
        Some(rev) => cascade_interop::read_at_rev(file, rev)
            .with_context(|| format!("cannot read {} at `{rev}`", file.display()))?,
        None => std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?,
    };
    let head_label = head.unwrap_or("the working tree");
    let Some(old) = load_side(file, base, &old_text) else {
        return Ok(ExitCode::from(2));
    };
    let Some(new) = load_side(file, head_label, &new_text) else {
        return Ok(ExitCode::from(2));
    };
    let diff = diff_models(&old, &new);
    match format {
        OutputFormat::Text => print_text(&diff, base, head_label),
        OutputFormat::Json => print_json(&diff, file, base, head)?,
    }
    Ok(ExitCode::SUCCESS)
}

/// Load one side of the diff, printing its diagnostics on failure.
fn load_side(file: &Path, side: &str, text: &str) -> Option<Model> {
    match cascade_core::load_str(text) {
        Ok(model) => Some(model),
        Err(LoadError { diagnostics }) => {
            for d in &diagnostics {
                eprintln!("{} ({side}):{}: error: {}", file.display(), d.span, d.kind);
            }
            None
        }
    }
}

fn mark(status: DiffStatus) -> char {
    match status {
        DiffStatus::Added => '+',
        DiffStatus::Removed => '-',
        DiffStatus::Changed => '~',
        DiffStatus::Unchanged => ' ',
    }
}

fn print_text(diff: &ModelDiff, base: &str, head: &str) {
    if diff.is_empty() {
        println!("no changes between {base} and {head}");
        return;
    }
    // Keys order by kind first (machines, states, transitions, …), so the
    // listing is already grouped.
    for (key, status) in diff.entries() {
        println!("{} {key}", mark(status));
    }
    println!(
        "{} added, {} removed, {} changed",
        diff.count(DiffStatus::Added),
        diff.count(DiffStatus::Removed),
        diff.count(DiffStatus::Changed)
    );
}

fn print_json(diff: &ModelDiff, file: &Path, base: &str, head: Option<&str>) -> anyhow::Result<()> {
    let keys = |wanted: DiffStatus| -> Vec<String> {
        diff.entries().filter(|(_, s)| *s == wanted).map(|(k, _)| k.to_string()).collect()
    };
    let json = serde_json::json!({
        "file": file.display().to_string(),
        "base": base,
        "head": head,
        "added": keys(DiffStatus::Added),
        "removed": keys(DiffStatus::Removed),
        "changed": keys(DiffStatus::Changed),
    });
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}
