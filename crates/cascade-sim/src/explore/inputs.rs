//! A scenario as exploration inputs: its external fires and waits, applied
//! in file order at their timing, with the directives that need no timing
//! (`choose`, `create`, `remove`) chained to the input before them.
//!
//! - An `after-quiescence` input (the default) can only be applied when the
//!   queue is empty; an `immediate` one at any point after the input before
//!   it, so exploration interleaves it with the running cascade.
//! - With `choices`, `choose` directives are ignored too (every option is
//!   explored), with a note.
//! - Manual queue directives (`step`, `{ step: n }`, `run`) choose an
//!   ordering, which exploration enumerates instead: they are ignored, with
//!   a note. So is `end: pause` (outcomes are quiet states).

use cascade_core::Model;
use cascade_core::expr::Duration;

use crate::engine::core::Core;
use crate::engine::drive::{step_target, trigger_ref};
use crate::error::SimError;
use crate::scenario::{ResolvedEntry, ResolvedInstance, ResolvedScenario, ResolvedStep, ScenarioEnd, StepTiming};
use crate::session::PlayAction;

/// One input the exploration applies.
#[derive(Clone, Debug)]
pub(crate) struct Input {
    pub kind: InputKind,
    pub timing: StepTiming,
    /// Directives applied right after it, in file order.
    pub chained: Vec<PlayAction>,
}

#[derive(Clone, Debug)]
pub(crate) enum InputKind {
    Fire(ResolvedStep),
    Wait(Option<Duration>),
}

impl Input {
    /// The input as a play action in the current state: a targetless fire
    /// names the fired machine's only live instance now.
    pub fn action(&self, model: &Model, core: &Core) -> Result<PlayAction, SimError> {
        match &self.kind {
            InputKind::Wait(by) => Ok(PlayAction::Advance { by: *by }),
            InputKind::Fire(step) => {
                let target = step_target(model, core, step)?;
                Ok(PlayAction::Fire {
                    source: model.external(step.source).name.clone(),
                    trigger: trigger_ref(model, step.trigger),
                    target: core.instance(target).name.clone(),
                    payload: step.payload.clone(),
                })
            }
        }
    }

    /// `Customer fires Order.place at o1` / `wait 30m` / `wait`, for
    /// anomalies.
    pub fn text(&self, model: &Model) -> String {
        match &self.kind {
            InputKind::Wait(Some(by)) => format!("wait {by}"),
            InputKind::Wait(None) => "wait".to_owned(),
            InputKind::Fire(step) => {
                let target = match &step.target {
                    crate::scenario::StepTarget::Named(name) => name.value.clone(),
                    crate::scenario::StepTarget::TheOne(machine) => format!("the {}", model.machine(*machine).name),
                };
                format!("{} fires {} at {target}", model.external(step.source).name, trigger_ref(model, step.trigger))
            }
        }
    }
}

/// A scenario split into what happens at the start and the inputs.
#[derive(Clone, Debug, Default)]
pub(crate) struct Inputs {
    /// Declared instances, then the directives before the first input.
    pub start: Vec<PlayAction>,
    pub inputs: Vec<Input>,
    pub notes: Vec<String>,
}

pub(crate) fn from_scenario(model: &Model, scenario: &ResolvedScenario, choices: bool) -> Inputs {
    let mut out =
        Inputs { start: scenario.instances.iter().map(|i| add_action(model, i)).collect(), ..Inputs::default() };
    let mut ignored = 0usize;
    let mut ignored_choices = 0usize;
    for entry in &scenario.entries {
        let chained = match entry {
            ResolvedEntry::Fire(step) => {
                out.inputs.push(Input { kind: InputKind::Fire(step.clone()), timing: step.timing, chained: vec![] });
                continue;
            }
            ResolvedEntry::Wait { by, timing, .. } => {
                out.inputs.push(Input { kind: InputKind::Wait(*by), timing: *timing, chained: vec![] });
                continue;
            }
            ResolvedEntry::Deliver { .. } | ResolvedEntry::Run { .. } => {
                ignored += 1;
                continue;
            }
            ResolvedEntry::Choose { .. } if choices => {
                ignored_choices += 1;
                continue;
            }
            ResolvedEntry::Choose { option, .. } => PlayAction::Choose { option: option.clone() },
            ResolvedEntry::Create { instance, .. } => add_action(model, instance),
            ResolvedEntry::Remove { name, .. } => PlayAction::RemoveInstance { name: name.clone() },
        };
        match out.inputs.last_mut() {
            Some(input) => input.chained.push(chained),
            None => out.start.push(chained),
        }
    }
    if ignored > 0 {
        let plural = if ignored == 1 { "directive was" } else { "directives were" };
        out.notes.push(format!(
            "{ignored} manual queue {plural} ignored (step/run choose an ordering; exploration enumerates them)"
        ));
    }
    if ignored_choices > 0 {
        let plural = if ignored_choices == 1 { "decision was" } else { "decisions were" };
        out.notes.push(format!("{ignored_choices} choose {plural} ignored (every option is explored)"));
    }
    if scenario.end == ScenarioEnd::Pause {
        out.notes.push("end: pause was ignored (outcomes are states with an empty queue)".to_owned());
    }
    out
}

fn add_action(model: &Model, instance: &ResolvedInstance) -> PlayAction {
    PlayAction::AddInstance {
        name: Some(instance.name.clone()),
        machine: model.machine(instance.machine).name.clone(),
        fields: instance.fields.clone(),
        state: instance.state.clone(),
    }
}
