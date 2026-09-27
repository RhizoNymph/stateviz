//! Cascade simulator: runs scenario files against a model with queued FIFO
//! event semantics and records a [`Trace`] for the trace view.
//!
//! # Semantics
//!
//! - **One global FIFO queue.** A transition's emitted events join the back
//!   of the queue. The simulator takes the head: an event is delivered to
//!   each subscribed controller (handler) in definition order, and each of
//!   the handler's rules picks its target(s) and *queues* a fire. Fires are
//!   queue items too, so the fires of two controllers interleave with each
//!   other and with the events their transitions emit.
//! - **Scenario steps** are external triggers. A step waits until the queue
//!   has drained ([`StepTiming::AfterQuiescence`], the default), unless it is
//!   marked [`StepTiming::Immediate`]: then it runs right after the previous
//!   step, before the cascade that step queued, so external triggers can
//!   interleave with a running cascade. A step's trigger is delivered to its
//!   target at once; the events that transition emits join the queue.
//! - **Delivering a trigger** takes the first enabled transition in
//!   definition order ([`Model::enabled_transitions`]). Guards and `when:`
//!   conditions are free text and are not evaluated. No enabled transition
//!   means the trigger is dropped. Compound targets are entered by default
//!   entry; history targets restore the last active leaf under their parent
//!   (deep) or its active child by default entry (shallow), and enter the
//!   parent by default when it was never active.
//! - **Payloads.** An emitted event carries the emitting instance's fields
//!   overlaid with the payload that came with the trigger: the step's
//!   `payload:` for a scenario step, the handled event's payload for a
//!   controller fire.
//! - **Selectors.** `field == event.x` compares the instance's field with the
//!   payload's `x`, a literal compares with the literal; a missing side never
//!   matches. A one-instance selector must match exactly one instance
//!   ([`TraceStepKind::NoTarget`] / [`TraceStepKind::Ambiguous`] otherwise),
//!   `all` fans out in instance order, `new` spawns an instance named
//!   `<machine-lowercase><n>` and fires at it.
//! - **Races.** [`race_orderings`] replays a race candidate a second time
//!   with its two contested fires delivered in the opposite order.
//!
//! See `docs/features/simulator.md` for the scenario format and details.

mod describe;
mod engine;
mod error;
mod race;
mod scenario;
mod trace;

use std::collections::BTreeMap;

use cascade_core::{FindingDetail, Model};

pub use describe::{causal_depths, lifeline_label, payload_text, selector_text, step_text};
pub use engine::STEP_LIMIT;
pub use error::{ScenarioDiagnostic, ScenarioError, ScenarioErrorKind, SimError};
pub use scenario::{
    DiscoverError, InstanceDecl, Payload, ResolvedScenario, Scenario, ScenarioFileError, Step, StepTiming, ValueEntry,
    ValueMap, discover_scenarios, load_scenario_file, parse_scenario, validate,
};
pub use trace::{Lifeline, LifelineIx, StepIx, Trace, TraceStep, TraceStepKind};

/// A trace plus the payloads that travelled with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimRun {
    pub trace: Trace,
    /// The payload of every [`TraceStepKind::Emit`] step, and of every
    /// [`TraceStepKind::ExternalFire`] step whose scenario step has one.
    pub payloads: BTreeMap<StepIx, Payload>,
}

/// Run `scenario` against `model` in plain FIFO order.
pub fn simulate(model: &Model, scenario: &Scenario) -> Result<Trace, SimError> {
    simulate_run(model, scenario).map(|run| run.trace)
}

/// Like [`simulate`], keeping the payloads.
pub fn simulate_run(model: &Model, scenario: &Scenario) -> Result<SimRun, SimError> {
    let resolved = validate(model, scenario)?;
    Ok(engine::run(model, &resolved, None)?.run)
}

/// Both orderings of a race candidate, each as a full trace of `scenario`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RaceTraces {
    /// The contested fires in their FIFO order.
    pub as_queued: Trace,
    /// The contested fires swapped.
    pub swapped: Trace,
}

/// [`RaceTraces`] with payloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RaceRuns {
    pub as_queued: SimRun,
    pub swapped: SimRun,
}

/// Replay `scenario` with the two fires of a race candidate in both orders.
///
/// The contested pair is the earliest-delivered pair of fires, one from each
/// of the finding's rules, at the same instance, both caused by the same
/// emission of the finding's origin event. `as_queued` is the plain FIFO run;
/// in `swapped` the fire FIFO delivered second goes first. Each trace's
/// [`Trace::ordering`] names the controller whose fire goes first, e.g.
/// "Fulfillment first".
///
/// Errors: [`SimError::NotARace`] for other findings,
/// [`SimError::RaceNotReached`] when the scenario never produces such a
/// pair, [`SimError::RaceNotSwappable`] when the second fire only happens
/// because the first was delivered.
pub fn race_orderings(model: &Model, scenario: &Scenario, race: &FindingDetail) -> Result<RaceTraces, SimError> {
    race_runs(model, scenario, race)
        .map(|runs| RaceTraces { as_queued: runs.as_queued.trace, swapped: runs.swapped.trace })
}

/// Like [`race_orderings`], keeping the payloads.
pub fn race_runs(model: &Model, scenario: &Scenario, race: &FindingDetail) -> Result<RaceRuns, SimError> {
    race::race_runs(model, scenario, race)
}
