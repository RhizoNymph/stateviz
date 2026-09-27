//! Unreachable state (warning): a least fixpoint over all machines at once,
//! so cross-machine fires and external sources count.
//!
//! - Every machine's initial state is entered.
//! - Entering a state marks it, its ancestors and its default-entry chain
//!   reachable. Entering a history pseudo-state marks it and enters its
//!   parent (the fallback when no history has been recorded yet), or the
//!   machine's initial state for a top-level history state.
//! - A transition is enabled when its source state is reachable; since
//!   ancestors of reachable states are reachable, this covers transitions
//!   declared on compound states, which apply to their descendants.
//! - A trigger is triggerable when an external source can fire it, or a rule
//!   fires it whose event is emitted by a live transition.
//! - A transition is live when it is enabled and its trigger triggerable; a
//!   live transition enters its target and emits its events.
//!
//! Guards and rule conditions are ignored (assumed satisfiable). States never
//! marked are reported, outermost first: an unreachable compound state
//! stands for all its descendants, and history pseudo-states are skipped.

use std::collections::VecDeque;

use super::describe;
use super::{Finding, FindingDetail};
use crate::ids::{EventId, StateId, TransitionId, TriggerId};
use crate::model::{Model, StateKind};

pub(super) fn check(model: &Model) -> Vec<Finding> {
    let reach = Reachability::compute(model);
    model
        .states()
        .filter(|&(id, s)| {
            !reach.is_reachable(id) && !s.is_history() && s.parent.is_none_or(|p| reach.is_reachable(p))
        })
        .map(|(id, s)| {
            let nested = descendants(model, id);
            let (what, them) = if nested == 0 {
                (format!("{} is", describe::state(model, id)), "it")
            } else {
                (
                    format!("{} and its {} are", describe::state(model, id), describe::count(nested, "nested state")),
                    "them",
                )
            };
            Finding::new(
                FindingDetail::UnreachableState { state: id },
                format!(
                    "{what} never entered: no path from {}'s initial state reaches {them} through external triggers or controller fires",
                    model.machine(s.machine).name
                ),
            )
        })
        .collect()
}

fn descendants(model: &Model, state: StateId) -> usize {
    model.state(state).children().iter().map(|&c| 1 + descendants(model, c)).sum()
}

/// Work items: something became true, so dependent transitions may now be
/// live.
enum Work {
    StateReached(StateId),
    TriggerArmed(TriggerId),
}

/// The fixpoint's result: which states can ever be entered.
pub(super) struct Reachability {
    reachable: Vec<bool>,
}

impl Reachability {
    pub(super) fn compute(model: &Model) -> Self {
        let mut solver = Solver::new(model);
        solver.run();
        Self { reachable: solver.reachable }
    }

    pub(super) fn is_reachable(&self, state: StateId) -> bool {
        self.reachable[state.index()]
    }
}

struct Solver<'m> {
    model: &'m Model,
    reachable: Vec<bool>,
    triggerable: Vec<bool>,
    live: Vec<bool>,
    emitted: Vec<bool>,
    /// Transitions declared on each state.
    outgoing: Vec<Vec<TransitionId>>,
    queue: VecDeque<Work>,
}

impl<'m> Solver<'m> {
    fn new(model: &'m Model) -> Self {
        let mut outgoing = vec![Vec::new(); model.state_count()];
        for (id, t) in model.transitions() {
            outgoing[t.from.index()].push(id);
        }
        Self {
            model,
            reachable: vec![false; model.state_count()],
            triggerable: vec![false; model.trigger_count()],
            live: vec![false; model.transition_count()],
            emitted: vec![false; model.event_count()],
            outgoing,
            queue: VecDeque::new(),
        }
    }

    fn run(&mut self) {
        let model = self.model;
        for (id, trigger) in model.triggers() {
            if !trigger.sources.is_empty() {
                self.arm(id);
            }
        }
        for (_, machine) in model.machines() {
            self.enter(machine.initial);
        }
        while let Some(work) = self.queue.pop_front() {
            match work {
                Work::StateReached(s) => {
                    // A state is reached once, so its list is needed once.
                    for t in std::mem::take(&mut self.outgoing[s.index()]) {
                        self.try_take(t);
                    }
                }
                Work::TriggerArmed(trigger) => {
                    for &t in &model.trigger(trigger).accepted_by {
                        self.try_take(t);
                    }
                }
            }
        }
    }

    fn mark(&mut self, state: StateId) {
        if !self.reachable[state.index()] {
            self.reachable[state.index()] = true;
            self.queue.push_back(Work::StateReached(state));
        }
    }

    fn arm(&mut self, trigger: TriggerId) {
        if !self.triggerable[trigger.index()] {
            self.triggerable[trigger.index()] = true;
            self.queue.push_back(Work::TriggerArmed(trigger));
        }
    }

    /// Enter `state` as a transition target (or as a machine's initial state).
    fn enter(&mut self, state: StateId) {
        let model = self.model;
        let s = model.state(state);
        if let StateKind::History { .. } = s.kind {
            self.mark(state);
            let fallback = s.parent.unwrap_or(model.machine(s.machine).initial);
            // A top-level history state's fallback is the machine's initial
            // state, which is never a history state (resolver invariant).
            if fallback != state {
                self.enter(fallback);
            }
            return;
        }
        self.mark(state);
        for a in model.ancestors(state) {
            self.mark(a);
        }
        let mut current = state;
        while let StateKind::Compound { initial, .. } = model.state(current).kind {
            current = initial;
            self.mark(current);
        }
    }

    fn try_take(&mut self, t: TransitionId) {
        let model = self.model;
        let tr = model.transition(t);
        if self.live[t.index()] || !self.reachable[tr.from.index()] || !self.triggerable[tr.trigger.index()] {
            return;
        }
        self.live[t.index()] = true;
        self.enter(tr.to);
        for &event in &tr.emits {
            self.emit(event);
        }
    }

    fn emit(&mut self, event: EventId) {
        if self.emitted[event.index()] {
            return;
        }
        self.emitted[event.index()] = true;
        let model = self.model;
        for &h in &model.event(event).handlers {
            for &r in &model.handler(h).rules {
                self.arm(model.rule(r).trigger);
            }
        }
    }
}
