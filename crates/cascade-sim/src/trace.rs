//! A simulated run: what happened, in what order, and why.

use std::collections::BTreeMap;

use cascade_core::ids::{
    ControllerId, EventId, ExternalId, HandlerId, MachineId, RuleId, StateId, TransitionId, TriggerId,
};

/// Index into [`Trace::lifelines`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LifelineIx(pub u32);

impl LifelineIx {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Index into [`Trace::steps`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StepIx(pub u32);

impl StepIx {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A vertical line in the sequence diagram. Lifelines follow definition
/// order (sources, then machine instances by machine, then controllers), so
/// a scenario always lays out the same way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lifeline {
    External { source: ExternalId },
    Instance { machine: MachineId, name: String },
    Controller { controller: ControllerId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceStepKind {
    /// An external source fires a trigger at an instance.
    ExternalFire { source: LifelineIx, target: LifelineIx, trigger: TriggerId },
    /// An instance takes a transition.
    Transition { instance: LifelineIx, transition: TransitionId, from: StateId, to: StateId },
    /// No transition accepts the trigger in the instance's current state.
    Dropped { instance: LifelineIx, trigger: TriggerId, state: StateId },
    /// A transition emits an event; it joins the back of the queue.
    Emit { instance: LifelineIx, event: EventId },
    /// A queued event reaches a controller subscribed to it.
    Deliver { controller: LifelineIx, event: EventId, handler: HandlerId },
    /// A controller rule fires a trigger at an instance.
    Fire { controller: LifelineIx, target: LifelineIx, rule: RuleId },
    /// A spawn rule creates a new instance.
    Spawn { controller: LifelineIx, instance: LifelineIx, rule: RuleId },
    /// A rule's selector matched no instance.
    NoTarget { controller: LifelineIx, rule: RuleId },
    /// A one-instance selector matched several instances.
    Ambiguous { controller: LifelineIx, rule: RuleId, candidates: Vec<LifelineIx> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceStep {
    /// The step that directly caused this one; `None` for scenario steps.
    pub cause: Option<StepIx>,
    pub kind: TraceStepKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trace {
    pub scenario: String,
    /// A label for this ordering when the trace is one side of a race, e.g.
    /// "Fulfillment first".
    pub ordering: Option<String>,
    pub lifelines: Vec<Lifeline>,
    pub steps: Vec<TraceStep>,
    /// Each instance lifeline's state after the run.
    pub final_states: BTreeMap<LifelineIx, StateId>,
}
