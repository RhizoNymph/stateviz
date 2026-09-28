//! `cascade import --from xstate|scxml`: convert to a Cascade YAML
//! definition, validated by resolving it.
//!
//! Warnings (renamed names, approximations, dropped input) go to standard
//! error as `path: warning: …`; the YAML goes to standard output or `--out`.
//!
//! Owner: `feat/interop-and-diff`.

use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;

use cascade_interop::{ImportFormat, to_yaml};

use crate::commands::write_output;

pub fn run(file: &Path, from: &str, out: Option<&Path>) -> anyhow::Result<ExitCode> {
    let format: ImportFormat = from.parse()?;
    let text = std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let imported =
        cascade_interop::import(format, &text).with_context(|| format!("cannot import {}", file.display()))?;
    for warning in &imported.warnings {
        eprintln!("{}: warning: {warning}", file.display());
    }
    let yaml = to_yaml(&imported.definition);
    // Validate what we are about to write: an import that does not resolve
    // is a bug in the importer, but report it like any invalid definition.
    if let Err(err) = cascade_core::load_str(&yaml) {
        for d in &err.diagnostics {
            eprintln!("{}: error: imported definition is invalid: {}: {}", file.display(), d.span, d.kind);
        }
        return Ok(ExitCode::from(2));
    }
    write_output(out, &yaml)?;
    Ok(ExitCode::SUCCESS)
}
