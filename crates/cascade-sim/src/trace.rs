//! A simulated run: what happened, in what order, and why.

use std::collections::BTreeMap;

use cascade_core::expr::Duration;
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
    /// An instance takes a transition. `from` is the instance's leaf state
    /// before and `to` the leaf it ends in (after default entry or a history
    /// restore), so either may be nested inside the transition's declared
    /// states.
    Transition { instance: LifelineIx, transition: TransitionId, from: StateId, to: StateId },
    /// No transition accepts the trigger in the instance's current state.
    Dropped { instance: LifelineIx, trigger: TriggerId, state: StateId },
    /// A transition emits an event; it joins the back of the queue.
    Emit { instance: LifelineIx, event: EventId },
    /// A queued event reaches a controller subscribed to it.
    Deliver { controller: LifelineIx, event: EventId, handler: HandlerId },
    /// A controller rule fires a trigger at an instance. The fire joins the
    /// queue; the step that delivers it (a `Transition` or `Dropped` on the
    /// target) names this step as its cause.
    Fire { controller: LifelineIx, target: LifelineIx, rule: RuleId },
    /// A spawn rule creates a new instance.
    Spawn { controller: LifelineIx, instance: LifelineIx, rule: RuleId },
    /// A rule's selector matched no instance.
    NoTarget { controller: LifelineIx, rule: RuleId },
    /// A one-instance selector matched several instances.
    Ambiguous { controller: LifelineIx, rule: RuleId, candidates: Vec<LifelineIx> },
    /// A delivered event is offered to a subscribing instance, which takes
    /// it (an instance that ignores the event records nothing). Cause: the
    /// `Emit`; the `Transition` that follows names this step as its cause.
    Notify { instance: LifelineIx, event: EventId },
    /// The clock advanced by `by` to `now` (a scenario `wait` or
    /// `PlayAction::Advance`; no cause).
    Wait { by: Duration, now: Duration },
    /// An armed timer came due and joined the queue. Cause: the `Wait`; the
    /// delivery (`Transition` or `Dropped`) names this step as its cause.
    TimerExpired { instance: LifelineIx, trigger: TriggerId },
    /// A choice point: `chosen` is one of `options`, taken from a pending
    /// decision or, when `defaulted`, the first option. `at` is the
    /// instance or controller deciding.
    Choice { at: LifelineIx, options: Vec<String>, chosen: String, defaulted: bool },
    /// A transition's `sets` assigned fields (after its `Transition` step).
    SetFields { instance: LifelineIx, fields: BTreeMap<String, String> },
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
