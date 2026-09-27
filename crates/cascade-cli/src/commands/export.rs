//! `cascade export --to scxml|mermaid|p|yaml`.
//!
//! Owner: `feat/interop-and-diff`.

use std::path::Path;
use std::process::ExitCode;

use cascade_interop::ExportFormat;

use crate::commands::{load_or_report, write_output};

pub fn run(file: &Path, to: &str, out: Option<&Path>) -> anyhow::Result<ExitCode> {
    let format: ExportFormat = to.parse()?;
    let model = match load_or_report(file)? {
        Ok(model) => model,
        Err(code) => return Ok(code),
    };
    let text = cascade_interop::export(format, &model)?;
    write_output(out, &text)?;
    Ok(ExitCode::SUCCESS)
}
