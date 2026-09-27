//! `cascade diff --base <rev> [--head <rev>]`: what changed between two
//! versions of a definition.
//!
//! Owner: `feat/interop-and-diff`.

use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;

use cascade_core::diff::{DiffStatus, diff_models};

use crate::OutputFormat;

pub fn run(file: &Path, base: &str, head: Option<&str>, format: OutputFormat) -> anyhow::Result<ExitCode> {
    let old_text = cascade_interop::read_at_rev(file, base)?;
    let new_text = match head {
        Some(rev) => cascade_interop::read_at_rev(file, rev)?,
        None => std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?,
    };
    let old = cascade_core::load_str(&old_text).with_context(|| format!("definition at {base} is invalid"))?;
    let new = cascade_core::load_str(&new_text).context("new definition is invalid")?;
    let diff = diff_models(&old, &new);
    match format {
        OutputFormat::Text => {
            for (key, status) in diff.entries() {
                let mark = match status {
                    DiffStatus::Added => '+',
                    DiffStatus::Removed => '-',
                    DiffStatus::Changed => '~',
                    DiffStatus::Unchanged => ' ',
                };
                println!("{mark} {key}");
            }
        }
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&diff)?),
    }
    Ok(ExitCode::SUCCESS)
}
