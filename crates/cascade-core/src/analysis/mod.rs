//! Static analysis: design bugs in a well-formed model.
//!
//! Seven checks from the spec plus one informational note. Each finding
//! carries a typed [`FindingDetail`] naming the elements involved, so views
//! can badge and focus them without parsing messages.
//!
//! Owner: the `feat/static-analysis` workstream implements [`analyze`]. The
//! types in this module are the contract the CLI, scene builders and app
//! code against.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::causal::CausalGraph;
use crate::ids::{EventId, HandlerId, MachineId, RuleId, StateId, TransitionId, TriggerId};
use crate::key::ElementRef;
use crate::model::Model;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Check {
    /// A controller fires a trigger that no transition in the target machine
    /// accepts.
    InvalidFire,
    /// One state has two transitions on the same trigger without mutually
    /// exclusive guards.
    Nondeterminism,
    /// The causal graph has a cycle, so a transition can eventually
    /// re-trigger itself. Silenced when any transition or rule on the cycle
    /// is marked `bounded: true`.
    CascadeCycle,
    /// An event is emitted but no controller subscribes to it.
    UnhandledEvent,
    /// A controller subscribes to an event nothing emits.
    OrphanController,
    /// No path from the initial state reaches the state, counting
    /// cross-machine fires and external sources.
    UnreachableState,
    /// One originating event leads, through different controllers, to two
    /// fires on the same machine instance.
    RaceCandidate,
    /// A fired trigger is accepted only from some states of the target; the
    /// fire is dropped in the others.
    StateDependentFire,
}

impl Check {
    pub const ALL: [Check; 8] = [
        Check::InvalidFire,
        Check::Nondeterminism,
        Check::CascadeCycle,
        Check::UnhandledEvent,
        Check::OrphanController,
        Check::UnreachableState,
        Check::RaceCandidate,
        Check::StateDependentFire,
    ];

    /// Stable kebab-case code, used in CLI output and JSON.
    pub const fn code(self) -> &'static str {
        match self {
            Check::InvalidFire => "invalid-fire",
            Check::Nondeterminism => "nondeterminism",
            Check::CascadeCycle => "cascade-cycle",
            Check::UnhandledEvent => "unhandled-event",
            Check::OrphanController => "orphan-controller",
            Check::UnreachableState => "unreachable-state",
            Check::RaceCandidate => "race-candidate",
            Check::StateDependentFire => "state-dependent-fire",
        }
    }

    pub const fn default_severity(self) -> Severity {
        match self {
            Check::InvalidFire | Check::Nondeterminism => Severity::Error,
            Check::CascadeCycle | Check::UnhandledEvent | Check::OrphanController | Check::UnreachableState => {
                Severity::Warning
            }
            Check::RaceCandidate | Check::StateDependentFire => Severity::Info,
        }
    }
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// One link of a cascade cycle: `transition` is taken, and (through an
/// event and a handler) `rule` fires the next step's transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CycleStep {
    pub transition: TransitionId,
    pub rule: RuleId,
}

/// The elements a finding is about. Which variant appears determines the
/// finding's [`Check`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FindingDetail {
    InvalidFire {
        rule: RuleId,
        trigger: TriggerId,
    },
    Nondeterminism {
        state: StateId,
        trigger: TriggerId,
        /// At least two transitions leaving `state` on `trigger`.
        transitions: Vec<TransitionId>,
    },
    CascadeCycle {
        /// The cycle in causal order, starting anywhere; the last step's
        /// rule fires the first step's transition.
        first: CycleStep,
        rest: Vec<CycleStep>,
    },
    UnhandledEvent {
        event: EventId,
    },
    OrphanController {
        handler: HandlerId,
    },
    UnreachableState {
        state: StateId,
    },
    RaceCandidate {
        /// The event whose cascade reaches both fires.
        origin: EventId,
        /// The machine both rules fire into.
        machine: MachineId,
        /// Two rules in different controllers that may hit the same instance.
        first: RuleId,
        second: RuleId,
    },
    StateDependentFire {
        rule: RuleId,
        trigger: TriggerId,
        /// States of the target machine where the fire would be dropped.
        dropped_in: Vec<StateId>,
    },
}

impl FindingDetail {
    /// The steps of a cascade cycle in order; empty for other findings.
    pub fn cycle_steps(&self) -> Vec<CycleStep> {
        match self {
            FindingDetail::CascadeCycle { first, rest } => {
                std::iter::once(*first).chain(rest.iter().copied()).collect()
            }
            _ => Vec::new(),
        }
    }

    pub const fn check(&self) -> Check {
        match self {
            FindingDetail::InvalidFire { .. } => Check::InvalidFire,
            FindingDetail::Nondeterminism { .. } => Check::Nondeterminism,
            FindingDetail::CascadeCycle { .. } => Check::CascadeCycle,
            FindingDetail::UnhandledEvent { .. } => Check::UnhandledEvent,
            FindingDetail::OrphanController { .. } => Check::OrphanController,
            FindingDetail::UnreachableState { .. } => Check::UnreachableState,
            FindingDetail::RaceCandidate { .. } => Check::RaceCandidate,
            FindingDetail::StateDependentFire { .. } => Check::StateDependentFire,
        }
    }

    /// The element to focus when the finding is clicked.
    pub fn primary(&self) -> ElementRef {
        match self {
            FindingDetail::InvalidFire { rule, .. }
            | FindingDetail::RaceCandidate { first: rule, .. }
            | FindingDetail::StateDependentFire { rule, .. } => ElementRef::Rule(*rule),
            FindingDetail::Nondeterminism { state, .. } | FindingDetail::UnreachableState { state } => {
                ElementRef::State(*state)
            }
            FindingDetail::CascadeCycle { first, .. } => ElementRef::Transition(first.transition),
            FindingDetail::UnhandledEvent { event } => ElementRef::Event(*event),
            FindingDetail::OrphanController { handler } => ElementRef::Handler(*handler),
        }
    }

    /// Every element the finding should badge.
    pub fn subjects(&self) -> Vec<ElementRef> {
        match self {
            FindingDetail::InvalidFire { rule, trigger } => {
                vec![ElementRef::Rule(*rule), ElementRef::Trigger(*trigger)]
            }
            FindingDetail::Nondeterminism { state, transitions, .. } => std::iter::once(ElementRef::State(*state))
                .chain(transitions.iter().map(|&t| ElementRef::Transition(t)))
                .collect(),
            FindingDetail::CascadeCycle { first, rest } => std::iter::once(first)
                .chain(rest)
                .flat_map(|step| [ElementRef::Transition(step.transition), ElementRef::Rule(step.rule)])
                .collect(),
            FindingDetail::UnhandledEvent { event } => vec![ElementRef::Event(*event)],
            FindingDetail::OrphanController { handler } => vec![ElementRef::Handler(*handler)],
            FindingDetail::UnreachableState { state } => vec![ElementRef::State(*state)],
            FindingDetail::RaceCandidate { first, second, .. } => {
                vec![ElementRef::Rule(*first), ElementRef::Rule(*second)]
            }
            FindingDetail::StateDependentFire { rule, dropped_in, .. } => std::iter::once(ElementRef::Rule(*rule))
                .chain(dropped_in.iter().map(|&s| ElementRef::State(s)))
                .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Finding {
    pub severity: Severity,
    pub detail: FindingDetail,
    /// One-line human-readable description naming the elements.
    pub message: String,
}

impl Finding {
    pub const fn check(&self) -> Check {
        self.detail.check()
    }
}

/// Run every check. Findings are ordered by severity (errors first), then
/// by check, then by the source position of their primary element.
///
/// Stub: returns no findings until `feat/static-analysis` lands.
pub fn analyze(model: &Model, graph: &CausalGraph) -> Vec<Finding> {
    let _ = (model, graph);
    Vec::new()
}

/// Whether any finding is an error, i.e. whether `cascade check` fails.
pub fn has_errors(findings: &[Finding]) -> bool {
    findings.iter().any(|f| f.severity == Severity::Error)
}
