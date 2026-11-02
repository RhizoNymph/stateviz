//! What an exploration found: distinct outcomes, each with its anomalies
//! and the shortest witness found.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use cascade_core::expr::Duration;
use cascade_core::ids::{StateId, TriggerId};

use crate::session::PlayAction;

/// The result of [`explore`](super::explore) or
/// [`explore_from`](super::explore_from).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exploration {
    /// Distinct outcomes, sorted by witness length, then by
    /// [`Outcome::sort_text`].
    pub outcomes: Vec<Outcome>,
    /// Distinct states visited.
    pub states: usize,
    /// False when `max_states` was hit: some orderings were not explored.
    pub complete: bool,
    /// Things the user should know about how the scenario was read, e.g.
    /// manual queue directives that exploration ignores.
    pub notes: Vec<String>,
}

/// A terminal state: inputs used up and the queue empty (and, with
/// `timers_anytime`, no timer armed). Two outcomes differ in their
/// instances, their armed timers or their anomalies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Every instance (by name, removed included) with its final leaf and
    /// fields.
    pub instances: BTreeMap<String, (StateId, BTreeMap<String, String>)>,
    /// Timers still armed at the end, with the time left (always empty with
    /// `timers_anytime`). Sorted.
    pub timers: Vec<OutcomeTimer>,
    /// What went wrong on the way.
    pub anomalies: BTreeSet<Anomaly>,
    /// Actions that reproduce it from the exploration's start: from
    /// [`PlaySession::new`](crate::PlaySession::new) for `explore` (the
    /// scenario's instances are its first actions), appended to the
    /// session's timeline for `explore_from`. The shortest found.
    pub witness: Vec<PlayAction>,
    /// How many explored paths end here (saturating): acyclic paths from
    /// the start in the explored state graph. With reduction on, orderings
    /// that only differ by independent moves are not all counted.
    pub paths: u64,
}

/// A timer still armed in an outcome.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutcomeTimer {
    pub instance: String,
    pub trigger: TriggerId,
    /// The time left until its deadline.
    pub remaining: Duration,
}

/// Something that went wrong on the way to an outcome.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Anomaly {
    /// A one-instance or `all` selector matched nothing. `rule` is the
    /// rule's label, e.g. `Fulfillment on OrderPaid: fire Shipment.start`.
    NoTarget { rule: String },
    /// A one-instance selector matched several instances (by name).
    Ambiguous { rule: String, candidates: Vec<String> },
    /// A trigger arrived in a state that does not accept it.
    Dropped { instance: String, trigger: String, state: String },
    /// A scenario input could not be applied in this ordering (e.g. it
    /// targets an instance a spawn has not created yet); it was skipped.
    InputFailed { input: String, error: String },
    /// A decision (`choose`) no choice point took.
    UnusedChoice { option: String },
    /// The path hit [`STEP_LIMIT`](crate::STEP_LIMIT) deliveries; the
    /// outcome is the state before the delivery that would exceed it.
    StepLimit,
}

impl fmt::Display for Anomaly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Anomaly::NoTarget { rule } => write!(f, "no target: {rule}"),
            Anomaly::Ambiguous { rule, candidates } => {
                write!(f, "ambiguous ({}): {rule}", candidates.join(", "))
            }
            Anomaly::Dropped { instance, trigger, state } => write!(f, "{instance}: {trigger} dropped in {state}"),
            Anomaly::InputFailed { input, error } => write!(f, "input skipped: {input}: {error}"),
            Anomaly::UnusedChoice { option } => write!(f, "decision never taken: {option}"),
            Anomaly::StepLimit => write!(f, "step limit reached"),
        }
    }
}

impl Outcome {
    /// The canonical text outcomes are sorted by after witness length:
    /// instances, timers and anomalies (state ids, not names, so it needs
    /// no model).
    pub fn sort_text(&self) -> String {
        format!("{:?} {:?} {:?}", self.instances, self.timers, self.anomalies)
    }
}
