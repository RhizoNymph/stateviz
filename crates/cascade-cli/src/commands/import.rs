//! `cascade import --from xstate|scxml`: convert to a Cascade YAML
//! definition, validated by resolving it.
//!
//! Owner: `feat/interop-and-diff`.

use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;

use cascade_interop::{ExportFormat, ImportFormat};

use crate::commands::write_output;

pub fn run(file: &Path, from: &str, out: Option<&Path>) -> anyhow::Result<ExitCode> {
    let format: ImportFormat = from.parse()?;
    let text = std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let definition = cascade_interop::import(format, &text)?;
    let model = cascade_core::resolve(definition).context("the imported definition does not resolve")?;
    let yaml = cascade_interop::export(ExportFormat::Yaml, &model)?;
    write_output(out, &yaml)?;
    Ok(ExitCode::SUCCESS)
}
