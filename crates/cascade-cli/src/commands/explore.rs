//! `cascade explore <file> <scenario>`: every ordering the scenario allows,
//! as distinct outcomes with a witness each (see
//! `docs/features/exploration.md`).
//!
//! Exit codes: 0 one outcome and no anomalies; 1 several outcomes or any
//! anomaly; 2 errors; 3 incomplete (the state bound was hit).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cascade_core::Model;
use cascade_sim::{
    Exploration, ExploreOptions, OrderingModel, Outcome, PlaySession, ScenarioFileError, SimError, differing_instances,
    instance_text, replay_witness, scenario_to_yaml, timer_text,
};
use serde_json::{Value, json};

use crate::OutputFormat;
use crate::commands::load_or_report;
use crate::commands::simulate::report;

pub struct ExploreArgs {
    pub file: PathBuf,
    pub scenario: PathBuf,
    pub options: ExploreOptions,
    pub format: OutputFormat,
    pub save_witness: Option<PathBuf>,
}

pub fn run(args: &ExploreArgs) -> anyhow::Result<ExitCode> {
    let model = match load_or_report(&args.file)? {
        Ok(model) => model,
        Err(code) => return Ok(code),
    };
    let scenario = match cascade_sim::load_scenario_file(&args.scenario) {
        Ok(scenario) => scenario,
        Err(ScenarioFileError::Invalid { path, source }) => return Ok(report(&path, &source)),
        Err(err @ ScenarioFileError::Io { .. }) => return Err(err.into()),
    };
    let exploration = match cascade_sim::explore(&model, &scenario, &args.options) {
        Ok(exploration) => exploration,
        Err(SimError::Scenario(err)) => return Ok(report(&args.scenario, &err)),
        Err(err) => return Err(err.into()),
    };
    let witnesses = witness_scenarios(&model, &exploration, &scenario.name)?;
    match args.format {
        OutputFormat::Text => print!("{}", text(&model, &scenario.name, &args.options, &exploration, &witnesses)),
        OutputFormat::Json => {
            println!("{}", json_report(&model, &scenario.name, &args.options, &exploration, &witnesses))
        }
    }
    if let Some(dir) = &args.save_witness {
        save(dir, &args.scenario, &witnesses)?;
    }
    Ok(exit_code(&exploration))
}

pub fn exit_code(exploration: &Exploration) -> ExitCode {
    if !exploration.complete {
        ExitCode::from(3)
    } else if exploration.outcomes.len() == 1 && exploration.outcomes.iter().all(|o| o.anomalies.is_empty()) {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// Each witness as a scenario file's text (replayed from an empty session
/// and saved, so it simulates to the outcome).
fn witness_scenarios(model: &Model, exploration: &Exploration, name: &str) -> anyhow::Result<Vec<String>> {
    let start = PlaySession::new(model);
    exploration
        .outcomes
        .iter()
        .enumerate()
        .map(|(i, outcome)| {
            let session = replay_witness(&start, model, &outcome.witness)?;
            let scenario = session.to_scenario(&format!("{name} (outcome {})", i + 1))?;
            Ok(scenario_to_yaml(&scenario))
        })
        .collect()
}

/// The `steps:` entries of a scenario text.
fn step_lines(yaml: &str) -> Vec<&str> {
    yaml.lines().skip_while(|l| !l.starts_with("steps:")).skip(1).filter(|l| l.starts_with("  ")).collect()
}

fn options_text(options: &ExploreOptions) -> String {
    format!(
        "ordering: {}, choices: {}, timers: {}{}",
        options.ordering.name(),
        if options.choices { "every option" } else { "defaults" },
        if options.timers_anytime { "any time" } else { "when the scenario waits" },
        if options.reduction { "" } else { ", no reduction" }
    )
}

fn plural(n: impl Into<u64>, one: &str, many: &str) -> String {
    let n = n.into();
    format!("{n} {}", if n == 1 { one } else { many })
}

fn text(
    model: &Model,
    name: &str,
    options: &ExploreOptions,
    exploration: &Exploration,
    witnesses: &[String],
) -> String {
    let mut out = format!("Exploration: {name}\n");
    out.push_str(&format!("Options: {}\n", options_text(options)));
    let status = if exploration.complete {
        "complete".to_owned()
    } else {
        format!("INCOMPLETE: bound of {} hit", options.max_states)
    };
    out.push_str(&format!(
        "{}, {} ({status})\n",
        plural(exploration.outcomes.len() as u64, "outcome", "outcomes"),
        plural(exploration.states as u64, "state", "states")
    ));
    for note in &exploration.notes {
        out.push_str(&format!("Note: {note}\n"));
    }
    let first = exploration.outcomes.first();
    for (i, outcome) in exploration.outcomes.iter().enumerate() {
        out.push('\n');
        out.push_str(&format!("Outcome {} ({}):\n", i + 1, plural(outcome.paths, "path", "paths")));
        match first {
            Some(first) if i > 0 => {
                let differing = differing_instances(first, outcome);
                if differing.is_empty() {
                    out.push_str("  Instances: as in outcome 1\n");
                } else {
                    out.push_str("  Instances (differing from outcome 1):\n");
                }
                for name in differing {
                    match outcome.instances.get(name) {
                        Some(value) => out.push_str(&format!("    {}\n", instance_text(model, name, value))),
                        None => out.push_str(&format!("    {name}: absent\n")),
                    }
                }
            }
            _ => {
                out.push_str("  Instances:\n");
                for (name, value) in &outcome.instances {
                    out.push_str(&format!("    {}\n", instance_text(model, name, value)));
                }
            }
        }
        if !outcome.timers.is_empty() {
            out.push_str("  Timers armed:\n");
            for timer in &outcome.timers {
                out.push_str(&format!("    {}\n", timer_text(model, timer)));
            }
        }
        if outcome.anomalies.is_empty() {
            out.push_str("  Anomalies: none\n");
        } else {
            out.push_str("  Anomalies:\n");
            for anomaly in &outcome.anomalies {
                out.push_str(&format!("    {anomaly}\n"));
            }
        }
        out.push_str("  Witness:\n");
        for line in witnesses.get(i).map(|w| step_lines(w)).unwrap_or_default() {
            out.push_str(&format!("  {line}\n"));
        }
    }
    out
}

fn outcome_json(model: &Model, index: usize, outcome: &Outcome, witness: Option<&String>) -> Value {
    let instances: Vec<Value> = outcome
        .instances
        .iter()
        .map(|(name, (state, fields))| {
            let s = model.state(*state);
            json!({ "name": name, "machine": model.machine(s.machine).name, "state": s.path, "fields": fields })
        })
        .collect();
    let timers: Vec<Value> = outcome
        .timers
        .iter()
        .map(|t| json!({ "instance": t.instance, "trigger": model.trigger(t.trigger).name, "remaining": t.remaining.to_string() }))
        .collect();
    let anomalies: Vec<String> = outcome.anomalies.iter().map(ToString::to_string).collect();
    let steps: Vec<String> = witness
        .map(|w| step_lines(w).into_iter().map(|l| l.trim_start().trim_start_matches("- ").to_owned()).collect())
        .unwrap_or_default();
    json!({
        "index": index,
        "paths": outcome.paths,
        "instances": instances,
        "timers": timers,
        "anomalies": anomalies,
        "witness": steps,
    })
}

fn json_report(
    model: &Model,
    name: &str,
    options: &ExploreOptions,
    exploration: &Exploration,
    witnesses: &[String],
) -> Value {
    let outcomes: Vec<Value> =
        exploration.outcomes.iter().enumerate().map(|(i, o)| outcome_json(model, i, o, witnesses.get(i))).collect();
    json!({
        "scenario": name,
        "options": {
            "ordering": options.ordering.name(),
            "choices": options.choices,
            "timers_anytime": options.timers_anytime,
            "max_states": options.max_states,
            "reduction": options.reduction,
        },
        "states": exploration.states,
        "complete": exploration.complete,
        "notes": exploration.notes,
        "outcomes": outcomes,
    })
}

/// Write each witness as `<dir>/<scenario stem>-outcome-<n>.yaml`.
fn save(dir: &Path, scenario: &Path, witnesses: &[String]) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| anyhow::anyhow!("cannot create {}: {e}", dir.display()))?;
    let stem = scenario.file_stem().and_then(|s| s.to_str()).unwrap_or("scenario");
    let stem = stem.trim_end_matches(".scenario");
    for (i, text) in witnesses.iter().enumerate() {
        let path = dir.join(format!("{stem}-outcome-{}.yaml", i + 1));
        std::fs::write(&path, text).map_err(|e| anyhow::anyhow!("cannot write {}: {e}", path.display()))?;
    }
    eprintln!("saved {} to {}", plural(witnesses.len() as u64, "witness", "witnesses"), dir.display());
    Ok(())
}

pub fn ordering(unordered: bool) -> OrderingModel {
    if unordered { OrderingModel::Unordered } else { OrderingModel::PerConsumer }
}
