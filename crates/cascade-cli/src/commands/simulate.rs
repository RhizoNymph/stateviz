//! `cascade simulate <file> <scenario>`: print a scenario's trace.
//!
//! Owner: `feat/simulator`.

use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;

use crate::commands::load_or_report;

pub fn run(file: &Path, scenario_path: &Path) -> anyhow::Result<ExitCode> {
    let model = match load_or_report(file)? {
        Ok(model) => model,
        Err(code) => return Ok(code),
    };
    let text =
        std::fs::read_to_string(scenario_path).with_context(|| format!("cannot read {}", scenario_path.display()))?;
    let scenario = cascade_sim::parse_scenario(&text)?;
    let trace = cascade_sim::simulate(&model, &scenario)?;
    for (i, step) in trace.steps.iter().enumerate() {
        println!("{i:>4} {:?}", step.kind);
    }
    Ok(ExitCode::SUCCESS)
}
