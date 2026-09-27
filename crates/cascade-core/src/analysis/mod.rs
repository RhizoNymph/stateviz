//! Static analysis: design bugs in a well-formed model.
//!
//! Seven checks from the spec plus one informational note. Each finding
//! carries a typed [`FindingDetail`] naming the elements involved, so views
//! can badge and focus them without parsing messages.
//!
//! This module holds the contract (the types the CLI, scene builders and app
//! code against) and the orchestration in [`analyze`]. Each check lives in
//! its own submodule and returns its findings unsorted; `analyze` sorts
//! them once. See `docs/features/static-analysis.md` for every heuristic.

mod cycles;
mod describe;
mod events;
mod invalid_fire;
mod nondeterminism;
mod races;
mod reachability;
mod scc;
mod state_dependent;
#[cfg(test)]
mod tests;

use std::cmp::Reverse;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::causal::CausalGraph;
use crate::ids::{EventId, ExternalId, HandlerId, MachineId, RuleId, StateId, TransitionId, TriggerId};
use crate::key::ElementRef;
use crate::model::Model;
use crate::span::Pos;

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
    /// accepts, or an external source exposes such a trigger (a dead
    /// command).
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
    /// An external source can fire `trigger`, but no transition accepts it
    /// (a dead command). Reported under [`Check::InvalidFire`].
    DeadExternalTrigger {
        source: ExternalId,
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
            FindingDetail::InvalidFire { .. } | FindingDetail::DeadExternalTrigger { .. } => Check::InvalidFire,
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
            FindingDetail::DeadExternalTrigger { source, .. } => ElementRef::External(*source),
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
            FindingDetail::DeadExternalTrigger { source, trigger } => {
                vec![ElementRef::External(*source), ElementRef::Trigger(*trigger)]
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
    /// A finding at its check's default severity.
    pub fn new(detail: FindingDetail, message: impl Into<String>) -> Self {
        Self { severity: detail.check().default_severity(), detail, message: message.into() }
    }

    pub const fn check(&self) -> Check {
        self.detail.check()
    }
}

/// Run every check. Findings are ordered by severity (errors first), then
/// by check, then by the source position of their primary element; ties
/// (several findings on one element) fall back to element order, so the
/// result is fully deterministic.
///
/// `graph` must have been built from `model`.
pub fn analyze(model: &Model, graph: &CausalGraph) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(invalid_fire::check(model));
    findings.extend(nondeterminism::check(model));
    findings.extend(cycles::check(model, graph));
    findings.extend(events::check(model));
    findings.extend(reachability::check(model));
    findings.extend(races::check(model, graph));
    findings.extend(state_dependent::check(model));
    sort_findings(model, &mut findings);
    findings
}

/// Whether any finding is an error, i.e. whether `cascade check` fails.
pub fn has_errors(findings: &[Finding]) -> bool {
    findings.iter().any(|f| f.severity == Severity::Error)
}

type SortKey = (Reverse<Severity>, Check, Pos, ElementRef, Vec<ElementRef>);

fn sort_findings(model: &Model, findings: &mut [Finding]) {
    findings.sort_by_cached_key(|f| -> SortKey {
        let primary = f.detail.primary();
        (Reverse(f.severity), f.check(), model.span_of(primary).start, primary, f.detail.subjects())
    });
}
