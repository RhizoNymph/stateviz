//! Performing one [`PlayAction`] on a [`Core`]: names are looked up in the
//! model and the core, every problem is reported before anything changes,
//! then the core operation runs. Both the play session and the batch
//! simulator go through here, so they share one set of semantics.

use cascade_core::Model;
use cascade_core::definition::TriggerRef;
use cascade_core::ids::{ExternalId, MachineId, StateId, TriggerId};
use cascade_core::parse::grammar::is_valid_name;

use super::Swap;
use super::core::{Core, Queued};
use super::instance::{Instance, InstanceIx};
use super::schedule::SwapState;
use crate::error::{ScenarioErrorKind, SimError};
use crate::scenario::{Payload, lookup_state};
use crate::session::PlayAction;

/// Which queue item `RunUntilQuiet` delivers next.
#[derive(Debug)]
pub(crate) enum Schedule {
    /// The head, always.
    Fifo,
    /// FIFO with one race swap (batch race replays only).
    Swap { swap: Swap, state: SwapState<Queued> },
}

impl Schedule {
    pub fn new(swap: Option<Swap>) -> Self {
        match swap {
            None => Schedule::Fifo,
            Some(swap) => Schedule::Swap { swap, state: SwapState::Armed },
        }
    }

    /// Whether the swap happened: the overtaker went first.
    pub fn swapped(&self) -> bool {
        matches!(self, Schedule::Swap { state: SwapState::Swapped, .. })
    }

    /// Whether an item is held back outside the queue.
    pub fn is_holding(&self) -> bool {
        matches!(self, Schedule::Swap { state: SwapState::Holding(_), .. })
    }
}

fn problem(kind: ScenarioErrorKind) -> SimError {
    SimError::Action(kind)
}

/// Perform `action`. Returns the action as it should be recorded: an
/// `AddInstance` without a name gets the name it was given. On error nothing
/// has changed, except that a `RunUntilQuiet` hitting the step limit has
/// delivered what it delivered before failing.
pub(crate) fn exec(
    core: &mut Core,
    model: &Model,
    action: &PlayAction,
    schedule: &mut Schedule,
) -> Result<PlayAction, SimError> {
    match action {
        PlayAction::AddInstance { name, machine, fields, state } => {
            let (machine_id, start) = resolve_instance(model, machine, fields, state.as_deref())?;
            let name = match name {
                Some(name) if !is_valid_name(name) => {
                    return Err(problem(ScenarioErrorKind::InvalidName {
                        context: "instance name".to_owned(),
                        name: name.clone(),
                    }));
                }
                Some(name) if core.is_taken(name) => {
                    return Err(problem(ScenarioErrorKind::NameTaken { name: name.clone() }));
                }
                Some(name) => name.clone(),
                None => core.free_name(model, machine_id),
            };
            core.add_instance(Instance::new(model, name.clone(), machine_id, fields.clone(), start));
            Ok(PlayAction::AddInstance {
                name: Some(name),
                machine: machine.clone(),
                fields: fields.clone(),
                state: state.clone(),
            })
        }
        PlayAction::RemoveInstance { name } => {
            let ix = alive(core, name)?;
            core.remove_instance(ix);
            Ok(action.clone())
        }
        PlayAction::Fire { source, trigger, target, payload } => {
            let (source_id, trigger_id, target_ix) = resolve_fire(core, model, source, trigger, target, payload)?;
            core.external_fire(model, source_id, target_ix, trigger_id, payload);
            Ok(action.clone())
        }
        PlayAction::Step { choice } => {
            let pending = core.queue().len();
            if pending == 0 {
                return Err(problem(ScenarioErrorKind::QueueEmpty));
            }
            let position = choice.unwrap_or(0);
            match usize::try_from(position) {
                Ok(at) if at < pending => core.deliver_at(model, at)?,
                _ => return Err(problem(ScenarioErrorKind::NoPendingItem { position, pending })),
            }
            Ok(action.clone())
        }
        PlayAction::RunUntilQuiet => {
            run_until_quiet(core, model, schedule)?;
            Ok(action.clone())
        }
    }
}

fn run_until_quiet(core: &mut Core, model: &Model, schedule: &mut Schedule) -> Result<(), SimError> {
    match schedule {
        Schedule::Fifo => {
            while !core.queue().is_empty() {
                core.deliver_at(model, 0)?;
            }
        }
        Schedule::Swap { swap, state } => {
            while let Some(item) = core.pop_swapped(swap, state) {
                core.deliver(model, item)?;
            }
        }
    }
    Ok(())
}

fn alive(core: &Core, name: &str) -> Result<InstanceIx, SimError> {
    core.alive_named(name).ok_or_else(|| problem(ScenarioErrorKind::UnknownInstance { name: name.to_owned() }))
}

/// The machine and starting leaf of a new instance.
fn resolve_instance(
    model: &Model,
    machine: &str,
    fields: &Payload,
    state: Option<&str>,
) -> Result<(MachineId, StateId), SimError> {
    let Some(machine_id) = model.machine_by_name(machine) else {
        return Err(problem(ScenarioErrorKind::UnknownMachine { name: machine.to_owned() }));
    };
    let m = model.machine(machine_id);
    for field in fields.keys() {
        if !is_valid_name(field) {
            let context = "instance fields".to_owned();
            return Err(problem(ScenarioErrorKind::InvalidName { context, name: field.clone() }));
        }
        if !m.fields.is_empty() && !m.fields.contains(field) {
            return Err(problem(ScenarioErrorKind::UnknownField {
                machine: m.name.clone(),
                field: field.clone(),
                declared: m.fields.clone(),
            }));
        }
    }
    let start = match state {
        None => m.initial,
        Some(state) => {
            let id = lookup_state(model, machine_id, state).map_err(problem)?;
            if model.state(id).is_history() {
                let kind =
                    ScenarioErrorKind::HistoryStart { machine: m.name.clone(), state: model.state(id).path.clone() };
                return Err(problem(kind));
            }
            id
        }
    };
    Ok((machine_id, start))
}

fn resolve_fire(
    core: &Core,
    model: &Model,
    source: &str,
    trigger: &TriggerRef,
    target: &str,
    payload: &Payload,
) -> Result<(ExternalId, TriggerId, InstanceIx), SimError> {
    let Some(source_id) = model.external_by_name(source) else {
        return Err(problem(ScenarioErrorKind::UnknownSource { name: source.to_owned() }));
    };
    let Some(machine) = model.machine_by_name(&trigger.machine) else {
        return Err(problem(ScenarioErrorKind::UnknownMachine { name: trigger.machine.clone() }));
    };
    let Some(trigger_id) = model.trigger_by_name(machine, &trigger.trigger) else {
        return Err(problem(ScenarioErrorKind::UnknownTrigger {
            machine: trigger.machine.clone(),
            trigger: trigger.trigger.clone(),
        }));
    };
    if !model.external(source_id).triggers.contains(&trigger_id) {
        return Err(problem(ScenarioErrorKind::SourceCannotFire {
            source_name: source.to_owned(),
            trigger: trigger.to_string(),
        }));
    }
    let target_ix = alive(core, target)?;
    let target_machine = core.instance(target_ix).machine;
    if target_machine != machine {
        return Err(problem(ScenarioErrorKind::TargetMachineMismatch {
            instance: target.to_owned(),
            machine: model.machine(target_machine).name.clone(),
            fire: trigger.to_string(),
        }));
    }
    if let Some(key) = payload.keys().find(|k| !is_valid_name(k)) {
        return Err(problem(ScenarioErrorKind::InvalidName { context: "payload".to_owned(), name: key.clone() }));
    }
    Ok((source_id, trigger_id, target_ix))
}
