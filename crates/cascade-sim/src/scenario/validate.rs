//! Scenario × model → [`ResolvedScenario`]: every name looked up, every
//! problem reported with the span it was written at.
//!
//! Entries are checked in order, tracking which instances exist at each
//! point (declared, created and not yet removed), so a step may only target
//! an instance created before it. Spawned instances are only known when the
//! run gets there; names a spawn rule could give are accepted and checked
//! then.

use std::collections::HashSet;

use cascade_core::Model;
use cascade_core::ids::{ExternalId, MachineId, StateId, TriggerId};
use cascade_core::model::Target;
use cascade_core::span::{SourceSpan, Spanned};

use super::{Directive, InstanceDecl, Payload, Scenario, ScenarioEnd, ScenarioEntry, Step, StepTiming};
use crate::error::{ScenarioDiagnostic, ScenarioError, ScenarioErrorKind};

/// A scenario checked against one model, with names resolved to typed ids.
/// Only [`validate`] builds one, so a `ResolvedScenario` is always valid for
/// the model it was validated against (and meaningless for any other).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedScenario {
    pub(crate) name: String,
    pub(crate) instances: Vec<ResolvedInstance>,
    /// `steps:` in file order.
    pub(crate) entries: Vec<ResolvedEntry>,
    pub(crate) end: ScenarioEnd,
}

impl ResolvedScenario {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// The number of external triggers (directives not counted).
    pub fn step_count(&self) -> usize {
        self.entries.iter().filter(|e| matches!(e, ResolvedEntry::Fire(_))).count()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedInstance {
    pub name: String,
    pub machine: MachineId,
    pub fields: Payload,
    /// The state as written (full path), `None` for the machine's initial
    /// state.
    pub state: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedStep {
    pub source: ExternalId,
    pub trigger: TriggerId,
    pub target: StepTarget,
    pub payload: Payload,
    pub timing: StepTiming,
    pub span: SourceSpan,
}

/// One `steps:` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResolvedEntry {
    Fire(ResolvedStep),
    Deliver { choice: Option<u32>, span: SourceSpan },
    Run { span: SourceSpan },
    Create { instance: ResolvedInstance, span: SourceSpan },
    Remove { name: String, span: SourceSpan },
}

/// Which instance a step fires at. Both forms are looked up when the step
/// runs, because controllers may spawn instances before then.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StepTarget {
    /// A declared instance, or a name a spawn rule will give an instance.
    Named(Spanned<String>),
    /// The only instance of the fired machine.
    TheOne(MachineId),
}

/// The instances known to exist at one point of the scenario: name →
/// machine (`None` when the machine is unknown, so steps naming the
/// instance are not reported a second time), in creation order.
#[derive(Default)]
struct Known<'s> {
    alive: Vec<(&'s str, Option<MachineId>)>,
    /// Every name declared or created so far, removed or not.
    ever: HashSet<&'s str>,
}

impl<'s> Known<'s> {
    fn get(&self, name: &str) -> Option<Option<MachineId>> {
        self.alive.iter().find(|(n, _)| *n == name).map(|(_, m)| *m)
    }

    fn of_machine(&self, machine: MachineId) -> Vec<String> {
        self.alive.iter().filter(|(_, m)| *m == Some(machine)).map(|(n, _)| (*n).to_owned()).collect()
    }
}

/// Check `scenario` against `model`: machines, fields, starting states,
/// sources, triggers, whether each source may fire its trigger, step
/// targets, and created and removed instances. Every problem is reported,
/// sorted by position.
///
/// A step target that is not declared is accepted when a spawn rule could
/// create it (`<machine-lowercase><n>`, e.g. `shipment1`, for a machine some
/// rule spawns); whether it exists is checked when the step runs. The same
/// goes for removing such a name.
pub fn validate(model: &Model, scenario: &Scenario) -> Result<ResolvedScenario, ScenarioError> {
    let mut diags = Vec::new();
    let mut push = |kind, span| diags.push(ScenarioDiagnostic { kind, span });

    let mut known = Known::default();
    let mut instances = Vec::with_capacity(scenario.instances.len());
    for decl in &scenario.instances {
        match declare(model, decl, &mut known) {
            Ok(instance) => instances.push(instance),
            Err(problems) => problems.into_iter().for_each(|(kind, span)| push(kind, span)),
        }
    }

    let spawnable: HashSet<MachineId> = model
        .rules()
        .filter(|(_, rule)| matches!(rule.target, Target::Spawn { .. }))
        .map(|(_, rule)| model.trigger(rule.trigger).machine)
        .collect();

    let mut entries = Vec::new();
    for entry in scenario.entries() {
        let resolved = match entry {
            ScenarioEntry::Fire(step) => resolve_step(model, step, &known, &spawnable).map(ResolvedEntry::Fire),
            ScenarioEntry::Directive(Directive::Deliver { choice, span }) => {
                Ok(ResolvedEntry::Deliver { choice: *choice, span: *span })
            }
            ScenarioEntry::Directive(Directive::Run { span }) => Ok(ResolvedEntry::Run { span: *span }),
            ScenarioEntry::Directive(Directive::Create(decl)) => {
                declare(model, decl, &mut known).map(|instance| ResolvedEntry::Create { instance, span: decl.span })
            }
            ScenarioEntry::Directive(Directive::Remove { name, span }) => {
                resolve_remove(model, name, &mut known, &spawnable)
                    .map(|()| ResolvedEntry::Remove { name: name.value.clone(), span: *span })
            }
        };
        match resolved {
            Ok(entry) => entries.push(entry),
            Err(problems) => problems.into_iter().for_each(|(kind, span)| push(kind, span)),
        }
    }

    ScenarioError::from_list(diags)?;
    Ok(ResolvedScenario { name: scenario.name.clone(), instances, entries, end: scenario.end })
}

type Problems = Vec<(ScenarioErrorKind, SourceSpan)>;

/// A declared or created instance: its name must be new, its machine known.
fn declare<'s>(model: &Model, decl: &'s InstanceDecl, known: &mut Known<'s>) -> Result<ResolvedInstance, Problems> {
    if known.ever.contains(decl.name.as_str()) {
        return Err(vec![(ScenarioErrorKind::DuplicateInstance { name: decl.name.value.clone() }, decl.name.span)]);
    }
    let machine = model.machine_by_name(&decl.machine.value);
    known.ever.insert(decl.name.as_str());
    known.alive.push((decl.name.as_str(), machine));
    let Some(machine) = machine else {
        return Err(vec![(ScenarioErrorKind::UnknownMachine { name: decl.machine.value.clone() }, decl.machine.span)]);
    };
    resolve_instance(model, machine, decl)
}

fn resolve_remove(
    model: &Model,
    name: &Spanned<String>,
    known: &mut Known<'_>,
    spawnable: &HashSet<MachineId>,
) -> Result<(), Problems> {
    if let Some(at) = known.alive.iter().position(|(n, _)| *n == name.as_str()) {
        known.alive.remove(at);
        return Ok(());
    }
    if spawnable.iter().any(|&m| is_spawn_name(model, m, name.as_str())) {
        return Ok(());
    }
    Err(vec![(ScenarioErrorKind::UnknownInstance { name: name.value.clone() }, name.span)])
}

fn resolve_instance(model: &Model, machine: MachineId, decl: &InstanceDecl) -> Result<ResolvedInstance, Problems> {
    let mut problems = Problems::new();
    let m = model.machine(machine);
    if !m.fields.is_empty() {
        for (field, entry) in decl.fields.iter() {
            if !m.fields.iter().any(|f| f == field) {
                let kind = ScenarioErrorKind::UnknownField {
                    machine: m.name.clone(),
                    field: field.to_owned(),
                    declared: m.fields.clone(),
                };
                problems.push((kind, entry.key_span));
            }
        }
    }

    let start = match &decl.state {
        None => Some(m.initial),
        Some(state) => match lookup_state(model, machine, &state.value) {
            Ok(id) if model.state(id).is_history() => {
                let kind =
                    ScenarioErrorKind::HistoryStart { machine: m.name.clone(), state: model.state(id).path.clone() };
                problems.push((kind, state.span));
                None
            }
            Ok(id) => Some(id),
            Err(kind) => {
                problems.push((kind, state.span));
                None
            }
        },
    };

    match start {
        Some(start) if problems.is_empty() => Ok(ResolvedInstance {
            name: decl.name.value.clone(),
            machine,
            fields: decl.fields.to_payload(),
            state: decl.state.as_ref().map(|_| model.state(start).path.clone()),
        }),
        _ => Err(problems),
    }
}

/// A state by full path, or by local name when that is unique (the same rule
/// the definition format uses).
pub(crate) fn lookup_state(model: &Model, machine: MachineId, name: &str) -> Result<StateId, ScenarioErrorKind> {
    if let Some(id) = model.state_by_path(machine, name) {
        return Ok(id);
    }
    let m = model.machine(machine);
    let matches: Vec<StateId> = m.states.iter().copied().filter(|&s| model.state(s).name == name).collect();
    match matches.as_slice() {
        [one] => Ok(*one),
        [] => Err(ScenarioErrorKind::UnknownState { machine: m.name.clone(), name: name.to_owned() }),
        many => Err(ScenarioErrorKind::AmbiguousState {
            machine: m.name.clone(),
            name: name.to_owned(),
            candidates: many.iter().map(|&s| model.state(s).path.clone()).collect(),
        }),
    }
}

fn resolve_step(
    model: &Model,
    step: &Step,
    known: &Known<'_>,
    spawnable: &HashSet<MachineId>,
) -> Result<ResolvedStep, Problems> {
    let mut problems = Problems::new();

    let source = model.external_by_name(&step.source.value);
    if source.is_none() {
        problems.push((ScenarioErrorKind::UnknownSource { name: step.source.value.clone() }, step.source.span));
    }

    let fire = &step.fire;
    let machine = model.machine_by_name(&fire.value.machine);
    let trigger = match machine {
        None => {
            problems.push((ScenarioErrorKind::UnknownMachine { name: fire.value.machine.clone() }, fire.span));
            None
        }
        Some(m) => {
            let trigger = model.trigger_by_name(m, &fire.value.trigger);
            if trigger.is_none() {
                let kind = ScenarioErrorKind::UnknownTrigger {
                    machine: fire.value.machine.clone(),
                    trigger: fire.value.trigger.clone(),
                };
                problems.push((kind, fire.span));
            }
            trigger
        }
    };
    if let (Some(x), Some(t)) = (source, trigger)
        && !model.external(x).triggers.contains(&t)
    {
        let kind = ScenarioErrorKind::SourceCannotFire {
            source_name: step.source.value.clone(),
            trigger: fire.value.to_string(),
        };
        problems.push((kind, fire.span));
    }

    let target = match (&step.target, machine) {
        (Some(name), machine) => match known.get(name.as_str()) {
            Some(Some(declared_machine)) if machine.is_some_and(|m| m != declared_machine) => {
                let kind = ScenarioErrorKind::TargetMachineMismatch {
                    instance: name.value.clone(),
                    machine: model.machine(declared_machine).name.clone(),
                    fire: fire.value.to_string(),
                };
                problems.push((kind, name.span));
                None
            }
            Some(_) => Some(StepTarget::Named(name.clone())),
            None if machine.is_some_and(|m| spawnable.contains(&m) && is_spawn_name(model, m, name.as_str())) => {
                Some(StepTarget::Named(name.clone()))
            }
            None => {
                problems.push((ScenarioErrorKind::UnknownInstance { name: name.value.clone() }, name.span));
                None
            }
        },
        (None, Some(m)) => {
            let candidates = known.of_machine(m);
            match candidates.len() {
                0 if !spawnable.contains(&m) => {
                    problems
                        .push((ScenarioErrorKind::NoInstance { machine: model.machine(m).name.clone() }, step.span));
                    None
                }
                0 | 1 => Some(StepTarget::TheOne(m)),
                _ => {
                    let kind =
                        ScenarioErrorKind::AmbiguousInstance { machine: model.machine(m).name.clone(), candidates };
                    problems.push((kind, step.span));
                    None
                }
            }
        }
        (None, None) => None,
    };

    match (source, trigger, target) {
        (Some(source), Some(trigger), Some(target)) if problems.is_empty() => Ok(ResolvedStep {
            source,
            trigger,
            target,
            payload: step.payload.to_payload(),
            timing: step.timing,
            span: step.span,
        }),
        _ => Err(problems),
    }
}

/// Whether `name` is a name the simulator could give a spawned instance of
/// `machine`: the machine name in lower case followed by digits.
fn is_spawn_name(model: &Model, machine: MachineId, name: &str) -> bool {
    name.strip_prefix(&spawn_prefix(model, machine))
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

/// The prefix of spawned instance names: the machine name in lower case.
pub(crate) fn spawn_prefix(model: &Model, machine: MachineId) -> String {
    model.machine(machine).name.to_lowercase()
}
