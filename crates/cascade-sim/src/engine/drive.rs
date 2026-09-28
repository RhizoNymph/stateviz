//! Driving a player with a validated scenario: each entry becomes the
//! [`PlayAction`]s a player would perform, looked up as the run reaches it.
//!
//! - Declared instances are added first, in declaration order.
//! - An `after-quiescence` fire runs the queue until quiet first (only when
//!   something is queued, so no empty runs are recorded); an `immediate` one
//!   does not. A fire with no `target:` fires at the fired machine's only
//!   live instance at that moment.
//! - Directives map one to one: `step` / `{ step: n }` → `Step`, `run` →
//!   `RunUntilQuiet`, `create` → `AddInstance`, `remove` → `RemoveInstance`.
//! - With `end: drain` (the default) the queue runs until quiet at the end.
//!
//! Problems found while running are reported at the entry's span.

use cascade_core::Model;
use cascade_core::definition::TriggerRef;
use cascade_core::ids::TriggerId;
use cascade_core::span::SourceSpan;

use super::core::Core;
use super::instance::InstanceIx;
use crate::error::{ScenarioError, ScenarioErrorKind, SimError};
use crate::scenario::{
    ResolvedEntry, ResolvedInstance, ResolvedScenario, ResolvedStep, ScenarioEnd, StepTarget, StepTiming,
};
use crate::session::PlayAction;

/// Something that performs actions on a core: the batch simulator or a play
/// session.
pub(crate) trait Player {
    fn core(&self) -> &Core;
    /// Nothing is queued (or held back by a race swap).
    fn is_quiet(&self) -> bool;
    fn act(&mut self, model: &Model, action: PlayAction) -> Result<(), SimError>;
}

pub(crate) fn drive(model: &Model, scenario: &ResolvedScenario, player: &mut impl Player) -> Result<(), SimError> {
    for instance in &scenario.instances {
        player.act(model, add_action(model, instance))?;
    }
    for entry in &scenario.entries {
        match entry {
            ResolvedEntry::Fire(step) => {
                if step.timing == StepTiming::AfterQuiescence && !player.is_quiet() {
                    player.act(model, PlayAction::RunUntilQuiet)?;
                }
                let target = step_target(model, player.core(), step)?;
                let action = PlayAction::Fire {
                    source: model.external(step.source).name.clone(),
                    trigger: trigger_ref(model, step.trigger),
                    target: player.core().instance(target).name.clone(),
                    payload: step.payload.clone(),
                };
                at(step.span, player.act(model, action))?;
            }
            ResolvedEntry::Deliver { choice, span } => {
                at(*span, player.act(model, PlayAction::Step { choice: *choice }))?;
            }
            ResolvedEntry::Run { span } => at(*span, player.act(model, PlayAction::RunUntilQuiet))?,
            ResolvedEntry::Create { instance, span } => at(*span, player.act(model, add_action(model, instance)))?,
            ResolvedEntry::Remove { name, span } => {
                at(*span, player.act(model, PlayAction::RemoveInstance { name: name.clone() }))?;
            }
        }
    }
    if scenario.end == ScenarioEnd::Drain && !player.is_quiet() {
        player.act(model, PlayAction::RunUntilQuiet)?;
    }
    Ok(())
}

/// Give an action's problem the span of the entry it came from.
fn at(span: SourceSpan, result: Result<(), SimError>) -> Result<(), SimError> {
    result.map_err(|err| match err {
        SimError::Action(kind) => SimError::Scenario(ScenarioError::single(kind, span)),
        other => other,
    })
}

fn add_action(model: &Model, instance: &ResolvedInstance) -> PlayAction {
    PlayAction::AddInstance {
        name: Some(instance.name.clone()),
        machine: model.machine(instance.machine).name.clone(),
        fields: instance.fields.clone(),
        state: instance.state.clone(),
    }
}

pub(crate) fn trigger_ref(model: &Model, trigger: TriggerId) -> TriggerRef {
    let t = model.trigger(trigger);
    TriggerRef { machine: model.machine(t.machine).name.clone(), trigger: t.name.clone() }
}

/// The instance a step fires at, as it stands when the step runs.
fn step_target(model: &Model, core: &Core, step: &ResolvedStep) -> Result<InstanceIx, SimError> {
    let fired_machine = model.trigger(step.trigger).machine;
    match &step.target {
        StepTarget::Named(name) => {
            let Some(ix) = core.alive_named(name.as_str()) else {
                let kind = ScenarioErrorKind::UnknownInstance { name: name.value.clone() };
                return Err(ScenarioError::single(kind, name.span).into());
            };
            let machine = core.instance(ix).machine;
            if machine != fired_machine {
                let kind = ScenarioErrorKind::TargetMachineMismatch {
                    instance: name.value.clone(),
                    machine: model.machine(machine).name.clone(),
                    fire: trigger_ref(model, step.trigger).to_string(),
                };
                return Err(ScenarioError::single(kind, name.span).into());
            }
            Ok(ix)
        }
        StepTarget::TheOne(machine) => {
            let candidates = core.instances_of(*machine);
            match candidates.as_slice() {
                [one] => Ok(*one),
                [] => {
                    let kind = ScenarioErrorKind::NoInstance { machine: model.machine(*machine).name.clone() };
                    Err(ScenarioError::single(kind, step.span).into())
                }
                many => {
                    let kind = ScenarioErrorKind::AmbiguousInstance {
                        machine: model.machine(*machine).name.clone(),
                        candidates: many.iter().map(|ix| core.instance(*ix).name.clone()).collect(),
                    };
                    Err(ScenarioError::single(kind, step.span).into())
                }
            }
        }
    }
}
