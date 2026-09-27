//! Recording a run and turning it into a [`Trace`].
//!
//! Steps are recorded against engine participants (external sources,
//! instances, controllers) because lifeline positions depend on which
//! participants appear, which is only known once the run ends. [`Recorder::
//! finish`] then orders the lifelines and rewrites every step.

use std::collections::BTreeMap;

use cascade_core::ids::{
    ControllerId, EventId, ExternalId, HandlerId, MachineId, RuleId, StateId, TransitionId, TriggerId,
};

use super::instance::{Instance, InstanceIx};
use crate::SimRun;
use crate::scenario::Payload;
use crate::trace::{Lifeline, LifelineIx, StepIx, Trace, TraceStep, TraceStepKind};

/// Index into the recorder's steps; becomes a [`StepIx`].
pub(crate) type StepRef = usize;

/// [`TraceStepKind`] with engine participants instead of lifelines.
#[derive(Clone, Debug)]
pub(crate) enum RawKind {
    ExternalFire { source: ExternalId, target: InstanceIx, trigger: TriggerId },
    Transition { instance: InstanceIx, transition: TransitionId, from: StateId, to: StateId },
    Dropped { instance: InstanceIx, trigger: TriggerId, state: StateId },
    Emit { instance: InstanceIx, event: EventId },
    Deliver { controller: ControllerId, event: EventId, handler: HandlerId },
    Fire { controller: ControllerId, target: InstanceIx, rule: RuleId },
    Spawn { controller: ControllerId, instance: InstanceIx, rule: RuleId },
    NoTarget { controller: ControllerId, rule: RuleId },
    Ambiguous { controller: ControllerId, rule: RuleId, candidates: Vec<InstanceIx> },
}

#[derive(Clone, Debug)]
struct RawStep {
    cause: Option<StepRef>,
    kind: RawKind,
}

/// A lifeline's sort key. The derived order is the lifeline order: external
/// sources in model order, then instances grouped by machine in model order
/// (each group in creation order: declared, then spawned), then controllers
/// in model order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum LifelineKey {
    External(ExternalId),
    Instance(MachineId, InstanceIx),
    Controller(ControllerId),
}

#[derive(Debug, Default)]
pub(crate) struct Recorder {
    steps: Vec<RawStep>,
    payloads: BTreeMap<StepRef, Payload>,
}

impl Recorder {
    pub fn push(&mut self, cause: Option<StepRef>, kind: RawKind) -> StepRef {
        self.steps.push(RawStep { cause, kind });
        self.steps.len() - 1
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn set_payload(&mut self, step: StepRef, payload: Payload) {
        self.payloads.insert(step, payload);
    }

    pub fn finish(self, scenario: &str, instances: &[Instance]) -> SimRun {
        let instance_key = |ix: InstanceIx| {
            // Every InstanceIx the engine records indexes `instances`.
            let machine = instances.get(ix.0).map(|i| i.machine);
            machine.map(|m| LifelineKey::Instance(m, ix))
        };

        let mut keys: Vec<LifelineKey> =
            instances.iter().enumerate().map(|(i, inst)| LifelineKey::Instance(inst.machine, InstanceIx(i))).collect();
        for step in &self.steps {
            match &step.kind {
                RawKind::ExternalFire { source, .. } => keys.push(LifelineKey::External(*source)),
                RawKind::Deliver { controller, .. }
                | RawKind::Fire { controller, .. }
                | RawKind::Spawn { controller, .. }
                | RawKind::NoTarget { controller, .. }
                | RawKind::Ambiguous { controller, .. } => keys.push(LifelineKey::Controller(*controller)),
                RawKind::Transition { .. } | RawKind::Dropped { .. } | RawKind::Emit { .. } => {}
            }
        }
        keys.sort_unstable();
        keys.dedup();

        // `keys` holds every participant the steps mention, so the insertion
        // point is the participant's position.
        let ix_of = |key: LifelineKey| LifelineIx(to_u32(keys.partition_point(|k| *k < key)));
        let inst = |ix: InstanceIx| instance_key(ix).map_or(LifelineIx(u32::MAX), ix_of);
        let ctl = |c: ControllerId| ix_of(LifelineKey::Controller(c));

        let lifelines = keys
            .iter()
            .map(|key| match *key {
                LifelineKey::External(source) => Lifeline::External { source },
                LifelineKey::Instance(machine, ix) => {
                    let name = instances.get(ix.0).map(|i| i.name.clone()).unwrap_or_default();
                    Lifeline::Instance { machine, name }
                }
                LifelineKey::Controller(controller) => Lifeline::Controller { controller },
            })
            .collect();

        let steps = self
            .steps
            .into_iter()
            .map(|step| TraceStep {
                cause: step.cause.map(|c| StepIx(to_u32(c))),
                kind: match step.kind {
                    RawKind::ExternalFire { source, target, trigger } => TraceStepKind::ExternalFire {
                        source: ix_of(LifelineKey::External(source)),
                        target: inst(target),
                        trigger,
                    },
                    RawKind::Transition { instance, transition, from, to } => {
                        TraceStepKind::Transition { instance: inst(instance), transition, from, to }
                    }
                    RawKind::Dropped { instance, trigger, state } => {
                        TraceStepKind::Dropped { instance: inst(instance), trigger, state }
                    }
                    RawKind::Emit { instance, event } => TraceStepKind::Emit { instance: inst(instance), event },
                    RawKind::Deliver { controller, event, handler } => {
                        TraceStepKind::Deliver { controller: ctl(controller), event, handler }
                    }
                    RawKind::Fire { controller, target, rule } => {
                        TraceStepKind::Fire { controller: ctl(controller), target: inst(target), rule }
                    }
                    RawKind::Spawn { controller, instance, rule } => {
                        TraceStepKind::Spawn { controller: ctl(controller), instance: inst(instance), rule }
                    }
                    RawKind::NoTarget { controller, rule } => {
                        TraceStepKind::NoTarget { controller: ctl(controller), rule }
                    }
                    RawKind::Ambiguous { controller, rule, candidates } => TraceStepKind::Ambiguous {
                        controller: ctl(controller),
                        rule,
                        candidates: candidates.into_iter().map(inst).collect(),
                    },
                },
            })
            .collect();

        let final_states =
            instances.iter().enumerate().map(|(i, instance)| (inst(InstanceIx(i)), instance.leaf())).collect();
        let payloads = self.payloads.into_iter().map(|(step, payload)| (StepIx(to_u32(step)), payload)).collect();

        SimRun {
            trace: Trace { scenario: scenario.to_owned(), ordering: None, lifelines, steps, final_states },
            payloads,
        }
    }
}

/// Runs are bounded far below `u32::MAX` steps by the step limit.
fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}
