//! `cascade simulate <file> <scenario>`: print a scenario's trace, or with
//! `--race <n>` both orderings of the n-th race candidate, or with
//! `--interactive` play the system at a prompt (see `play`).
//!
//! Text output is one line per step, indented by causal depth, with `← n`
//! naming the cause when it is not the line above. `--format json` prints
//! the same trace as JSON.
//!
//! Owner: `feat/simulator`; `--interactive` from `feat/sim-session`.

mod play;

use std::path::Path;
use std::process::ExitCode;

use anyhow::bail;
use cascade_core::{CausalGraph, Check, ElementRef, Model, analyze};
use cascade_sim::{
    Lifeline, ScenarioError, ScenarioFileError, SimError, SimRun, TraceStepKind, causal_depths, lifeline_label,
    step_text,
};
use serde_json::{Value, json};

use crate::OutputFormat;
use crate::commands::load_or_report;

pub fn run(
    file: &Path,
    scenario_path: Option<&Path>,
    format: OutputFormat,
    race: Option<usize>,
    interactive: bool,
) -> anyhow::Result<ExitCode> {
    let model = match load_or_report(file)? {
        Ok(model) => model,
        Err(code) => return Ok(code),
    };
    let scenario = match scenario_path.map(cascade_sim::load_scenario_file) {
        None => None,
        Some(Ok(scenario)) => Some(scenario),
        Some(Err(ScenarioFileError::Invalid { path, source })) => return Ok(report(&path, &source)),
        Some(Err(err @ ScenarioFileError::Io { .. })) => return Err(err.into()),
    };
    if interactive {
        return play::run(file, &model, scenario.as_ref().zip(scenario_path));
    }
    let (Some(scenario), Some(scenario_path)) = (scenario, scenario_path) else {
        bail!("a scenario file is required unless --interactive is given");
    };

    match race {
        None => {
            let run = match cascade_sim::simulate_run(&model, &scenario) {
                Ok(run) => run,
                Err(SimError::Scenario(err)) => return Ok(report(scenario_path, &err)),
                Err(err) => return Err(err.into()),
            };
            match format {
                OutputFormat::Text => print!("{}", trace_text(&model, &run)),
                OutputFormat::Json => println!("{}", trace_json(&model, &run)),
            }
        }
        Some(index) => {
            let graph = CausalGraph::build(&model);
            let findings = analyze(&model, &graph);
            let races: Vec<_> = findings.iter().filter(|f| f.check() == Check::RaceCandidate).collect();
            let Some(finding) = races.get(index) else {
                bail!(
                    "there is no race candidate #{index}; the definition has {} (numbered from 0 in `cascade check` order)",
                    races.len()
                );
            };
            let runs = match cascade_sim::race_runs(&model, &scenario, &finding.detail) {
                Ok(runs) => runs,
                Err(SimError::Scenario(err)) => return Ok(report(scenario_path, &err)),
                Err(err) => return Err(err.into()),
            };
            match format {
                OutputFormat::Text => {
                    println!("Race candidate #{index}: {}", finding.message);
                    for (run, which) in [(&runs.as_queued, "as queued"), (&runs.swapped, "swapped")] {
                        let label = run.trace.ordering.as_deref().unwrap_or("ordering");
                        println!();
                        println!("== {label} ({which}) ==");
                        print!("{}", trace_text(&model, run));
                    }
                }
                OutputFormat::Json => {
                    let subjects: Vec<String> =
                        finding.detail.subjects().into_iter().map(|e| model.key_of(e).to_string()).collect();
                    let out = json!({
                        "race": { "index": index, "message": finding.message, "subjects": subjects },
                        "as_queued": trace_json(&model, &runs.as_queued),
                        "swapped": trace_json(&model, &runs.swapped),
                    });
                    println!("{out}");
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Print scenario diagnostics as `path:line:col: error: …`; exit code 2.
pub(crate) fn report(path: &Path, err: &ScenarioError) -> ExitCode {
    for d in &err.diagnostics {
        eprintln!("{}:{}: error: {}", path.display(), d.span, d.kind);
    }
    ExitCode::from(2)
}

pub(crate) fn trace_text(model: &Model, run: &SimRun) -> String {
    let trace = &run.trace;
    let mut out = format!("Scenario: {}\n", trace.scenario);
    let lifelines: Vec<String> = trace.lifelines.iter().map(|l| lifeline_label(model, l)).collect();
    out.push_str(&format!("Lifelines: {}\n", lifelines.join(", ")));

    out.push_str(&step_lines(model, run, 0..trace.steps.len()));

    out.push_str("Final states:\n");
    for (ix, state) in &trace.final_states {
        let who = trace.lifelines.get(ix.index()).map_or_else(|| "?".to_owned(), |l| lifeline_label(model, l));
        out.push_str(&format!("  {who}: {}\n", model.state(*state).path));
    }
    out
}

/// The lines for `steps`: index, indent by causal depth, text, and `← n`
/// when the cause is not the line above.
pub(crate) fn step_lines(model: &Model, run: &SimRun, steps: std::ops::Range<usize>) -> String {
    let trace = &run.trace;
    let depths = causal_depths(trace);
    let mut out = String::new();
    for (i, step) in trace.steps.iter().enumerate().skip(steps.start).take(steps.len()) {
        let depth = depths.get(i).copied().unwrap_or(0);
        let text = step_text(model, trace, step, run.payloads.get(&cascade_sim::StepIx(to_u32(i))));
        let cause = match step.cause {
            Some(c) if c.index() + 1 != i => format!("  ← {}", c.index()),
            _ => String::new(),
        };
        out.push_str(&format!("{i:>4}  {}{text}{cause}\n", "  ".repeat(depth)));
    }
    out
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn trace_json(model: &Model, run: &SimRun) -> Value {
    let trace = &run.trace;
    let key = |e: ElementRef| model.key_of(e).to_string();
    let lifelines: Vec<Value> = trace
        .lifelines
        .iter()
        .map(|l| match l {
            Lifeline::External { source } => json!({ "kind": "external", "name": model.external(*source).name }),
            Lifeline::Instance { machine, name } => {
                json!({ "kind": "instance", "machine": model.machine(*machine).name, "name": name })
            }
            Lifeline::Controller { controller } => {
                json!({ "kind": "controller", "name": model.controller(*controller).name })
            }
        })
        .collect();

    let depths = causal_depths(trace);
    let steps: Vec<Value> = trace
        .steps
        .iter()
        .enumerate()
        .map(|(i, step)| {
            let payload = run.payloads.get(&cascade_sim::StepIx(to_u32(i)));
            let mut value = match &step.kind {
                TraceStepKind::ExternalFire { source, target, trigger } => json!({
                    "kind": "external-fire",
                    "source": source.0,
                    "target": target.0,
                    "trigger": key(ElementRef::Trigger(*trigger)),
                }),
                TraceStepKind::Transition { instance, transition, from, to } => json!({
                    "kind": "transition",
                    "instance": instance.0,
                    "transition": key(ElementRef::Transition(*transition)),
                    "from": model.state(*from).path,
                    "to": model.state(*to).path,
                }),
                TraceStepKind::Dropped { instance, trigger, state } => json!({
                    "kind": "dropped",
                    "instance": instance.0,
                    "trigger": key(ElementRef::Trigger(*trigger)),
                    "state": model.state(*state).path,
                }),
                TraceStepKind::Emit { instance, event } => json!({
                    "kind": "emit",
                    "instance": instance.0,
                    "event": model.event(*event).name,
                }),
                TraceStepKind::Deliver { controller, event, handler } => json!({
                    "kind": "deliver",
                    "controller": controller.0,
                    "event": model.event(*event).name,
                    "handler": key(ElementRef::Handler(*handler)),
                }),
                TraceStepKind::Fire { controller, target, rule } => json!({
                    "kind": "fire",
                    "controller": controller.0,
                    "target": target.0,
                    "rule": key(ElementRef::Rule(*rule)),
                }),
                TraceStepKind::Spawn { controller, instance, rule } => json!({
                    "kind": "spawn",
                    "controller": controller.0,
                    "instance": instance.0,
                    "rule": key(ElementRef::Rule(*rule)),
                }),
                TraceStepKind::NoTarget { controller, rule } => json!({
                    "kind": "no-target",
                    "controller": controller.0,
                    "rule": key(ElementRef::Rule(*rule)),
                }),
                TraceStepKind::Ambiguous { controller, rule, candidates } => json!({
                    "kind": "ambiguous",
                    "controller": controller.0,
                    "rule": key(ElementRef::Rule(*rule)),
                    "candidates": candidates.iter().map(|c| c.0).collect::<Vec<_>>(),
                }),
            };
            if let Value::Object(map) = &mut value {
                map.insert("index".to_owned(), json!(i));
                map.insert("cause".to_owned(), json!(step.cause.map(|c| c.0)));
                map.insert("depth".to_owned(), json!(depths.get(i).copied().unwrap_or(0)));
                map.insert("text".to_owned(), json!(step_text(model, trace, step, payload)));
                if let Some(payload) = payload {
                    map.insert("payload".to_owned(), json!(payload));
                }
            }
            value
        })
        .collect();

    let final_states: Vec<Value> = trace
        .final_states
        .iter()
        .map(|(ix, state)| {
            let name = match trace.lifelines.get(ix.index()) {
                Some(Lifeline::Instance { name, .. }) => name.clone(),
                _ => String::new(),
            };
            json!({ "lifeline": ix.0, "instance": name, "state": model.state(*state).path })
        })
        .collect();

    json!({
        "scenario": trace.scenario,
        "ordering": trace.ordering,
        "lifelines": lifelines,
        "steps": steps,
        "final_states": final_states,
    })
}
