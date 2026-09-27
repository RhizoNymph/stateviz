//! Cascade simulator: runs scenario files against a model with queued FIFO
//! event semantics and records a [`Trace`] for the trace view.
//!
//! Semantics: an emitted event joins the back of one global FIFO queue. The
//! simulator takes the queue's head, delivers it to each subscribed
//! controller in definition order, and each matching rule's fire is queued
//! in turn. Scenario steps wait for the queue to drain unless marked
//! immediate, so external triggers can interleave with a running cascade.
//! A race candidate is replayed twice with the two contested fires swapped.

mod error;
mod scenario;
mod trace;

use cascade_core::{FindingDetail, Model};

pub use error::{ScenarioError, SimError};
pub use scenario::{InstanceDecl, Scenario, Step, StepTiming, parse_scenario};
pub use trace::{Lifeline, LifelineIx, StepIx, Trace, TraceStep, TraceStepKind};

/// Run `scenario` against `model` in plain FIFO order.
///
/// Stub until `feat/simulator` lands.
pub fn simulate(model: &Model, scenario: &Scenario) -> Result<Trace, SimError> {
    let _ = (model, scenario);
    Err(SimError::NotImplemented)
}

/// Both orderings of a race candidate, each as a full trace of `scenario`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RaceTraces {
    /// The contested fires in their FIFO order.
    pub as_queued: Trace,
    /// The contested fires swapped.
    pub swapped: Trace,
}

/// Replay `scenario` with the two fires of a race candidate in both orders.
///
/// Stub until `feat/simulator` lands.
pub fn race_orderings(model: &Model, scenario: &Scenario, race: &FindingDetail) -> Result<RaceTraces, SimError> {
    let _ = (model, scenario);
    match race {
        FindingDetail::RaceCandidate { .. } => Err(SimError::NotImplemented),
        _ => Err(SimError::NotARace),
    }
}
