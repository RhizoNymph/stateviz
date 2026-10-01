//! Interactive play: a simulator you drive one action at a time.
//!
//! A [`PlaySession`] holds instances, the FIFO queue and a [`Trace`] like a
//! batch run, but the host decides what happens next: add an instance, fire
//! an external trigger, deliver the queue's head or *any* pending item
//! (to explore orderings by hand), or run until the queue drains. Semantics
//! are exactly the batch simulator's (see the crate docs); the batch
//! `simulate` is a session driven by a scenario.
//!
//! Every action is recorded on a [`Timeline`]. Actions name things (machine,
//! instance, source and trigger names, not ids), so a session can be
//! [`PlaySession::replay`]ed against a new model after the definition is
//! edited, and saved as a scenario. Seeking back and then acting forks the
//! timeline: the abandoned future is kept as a [`Branch`] to return to.
//!
//! # Rewinding and branching
//!
//! The session keeps only the state at the current position. Seeking
//! forward applies the next actions; seeking back replays the timeline from
//! the start, which is deterministic and cheap for designer-sized systems.
//! Acting at a position before the end forks: the whole old line is saved
//! as a [`Branch`] (unless the action is the same as the next one, which
//! just moves forward), and [`PlaySession::switch_branch`] swaps the current
//! line with a saved one.
//!
//! # Removing instances
//!
//! A removed instance leaves play: selectors and targets no longer see it,
//! fires already queued for it are discarded (events it emitted stay
//! queued), and its name is never given out again. Its lifeline and last
//! state stay in the trace.

mod save;
mod view;

use std::collections::BTreeMap;

use cascade_core::Model;
use cascade_core::definition::TriggerRef;
use cascade_core::ids::{EventId, MachineId, RuleId, StateId};

use crate::engine::core::Core;
use crate::engine::drive::{self, Player};
use crate::engine::exec::{Schedule, exec};
use crate::error::SimError;
use crate::scenario::{Payload, Scenario, validate};
use crate::trace::{LifelineIx, StepIx, Trace};

pub use crate::scenario::scenario_to_yaml;

/// One thing the player does. Model-independent: everything is by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlayAction {
    /// Create an instance. `name: None` picks `<machine-lowercase><n>`
    /// with the smallest `n` never used (the timeline records the name it
    /// got); `state: None` starts in the machine's initial state (a path or
    /// unique local name otherwise). Names are never reused, even after the
    /// instance that had one was removed.
    AddInstance { name: Option<String>, machine: String, fields: BTreeMap<String, String>, state: Option<String> },
    /// Take an instance out of play. Fires queued for it are discarded.
    RemoveInstance { name: String },
    /// An external source fires a trigger at an instance. The trigger is
    /// delivered at once; the transition's emitted events join the queue.
    Fire { source: String, trigger: TriggerRef, target: String, payload: BTreeMap<String, String> },
    /// Deliver one queue item: the head when `choice` is `None`, otherwise
    /// the pending item at that position in [`PlaySession::pending`].
    Step { choice: Option<u32> },
    /// Deliver queue heads until the queue is empty. The session as a whole
    /// delivers at most [`crate::STEP_LIMIT`] items, like a batch run.
    RunUntilQuiet,
}

/// Identifies a queue item for as long as it is queued. Ids are given out
/// in queueing order and never reused within a line of play; replaying the
/// same actions gives the same ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PendingId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingKind {
    /// An emitted event waiting to be delivered to its handlers.
    Event { event: EventId },
    /// A controller's fire waiting to be delivered to its target instance.
    Fire { rule: RuleId, target: LifelineIx },
}

/// A queue item, in queue order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingItem {
    pub id: PendingId,
    pub kind: PendingKind,
    /// The trace step that queued it.
    pub cause: StepIx,
    /// Short human label, e.g. `OrderPaid from o1` or `Fulfillment → s1: start`.
    pub label: String,
}

/// An instance's current situation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceState {
    pub lifeline: LifelineIx,
    pub name: String,
    pub machine: MachineId,
    /// The current leaf state.
    pub state: StateId,
    pub fields: BTreeMap<String, String>,
}

/// An external trigger the player could fire right now, for a palette of
/// buttons: every (source, trigger) pair × every instance of the trigger's
/// machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailableFire {
    pub source: String,
    pub trigger: TriggerRef,
    pub target: String,
    /// Whether the target's current state accepts the trigger (firing a
    /// disabled one is allowed and records a drop).
    pub accepted: bool,
}

/// What one action produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionOutcome {
    /// Trace steps appended by the action.
    pub steps: std::ops::Range<usize>,
}

/// A saved alternative line of play.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    /// Where it forked from the timeline it was split from: the number of
    /// leading actions the two lines share.
    pub fork: usize,
    /// The whole line, from the start (not just the part after `fork`), so
    /// a branch stays meaningful however the current line changes later.
    pub actions: Vec<PlayAction>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Timeline {
    /// The current line of play.
    pub actions: Vec<PlayAction>,
    /// How many of `actions` are applied (seeking back lowers it).
    pub position: usize,
    pub branches: Vec<Branch>,
}

/// An interactive simulator session. Every method that takes a [`Model`]
/// must get the model the session was built or last replayed with; after
/// the definition changes, call [`PlaySession::replay`] with the new one.
#[derive(Clone, Debug)]
pub struct PlaySession {
    /// The trace's scenario name.
    name: String,
    timeline: Timeline,
    trace: Trace,
    payloads: BTreeMap<StepIx, Payload>,
    /// The state at `timeline.position`.
    core: Core,
    /// Each instance's lifeline in `trace`, by creation index.
    lifelines: Vec<LifelineIx>,
    /// For each applied action: whether the queue was empty just before it
    /// (a fire into an empty queue saves as after-quiescence, otherwise as
    /// immediate).
    quiet_before: Vec<bool>,
}

impl PlaySession {
    /// An empty session: no instances, empty queue.
    pub fn new(model: &Model) -> Self {
        let _ = model;
        Self::named(String::new())
    }

    fn named(name: String) -> Self {
        let mut session = Self {
            name,
            timeline: Timeline::default(),
            trace: Trace {
                scenario: String::new(),
                ordering: None,
                lifelines: Vec::new(),
                steps: Vec::new(),
                final_states: BTreeMap::new(),
            },
            payloads: BTreeMap::new(),
            core: Core::default(),
            lifelines: Vec::new(),
            quiet_before: Vec::new(),
        };
        session.refresh();
        session
    }

    /// Start from a scenario: its instances, then its steps as actions (see
    /// the crate docs for how each step maps). The trace is exactly what
    /// [`crate::simulate`] gives for the scenario, and the session is
    /// positioned at the end. Problems are reported with the scenario's
    /// source positions.
    pub fn from_scenario(model: &Model, scenario: &Scenario) -> Result<Self, SimError> {
        let resolved = validate(model, scenario)?;
        let mut session = Self::named(scenario.name.clone());
        drive::drive(model, &resolved, &mut Recording(&mut session))?;
        session.refresh();
        tracing::debug!(
            scenario = %scenario.name,
            actions = session.timeline.actions.len(),
            steps = session.trace.steps.len(),
            "play session started from a scenario"
        );
        Ok(session)
    }

    /// Perform an action and record it. A rejected action changes nothing.
    ///
    /// When the timeline was seeked back, acting forks it: the old line is
    /// saved as a branch first, unless `action` is the one that comes next,
    /// which just moves forward.
    pub fn apply(&mut self, model: &Model, action: PlayAction) -> Result<ActionOutcome, SimError> {
        let mut core = self.core.clone();
        let before = core.step_count();
        let quiet = core.queue().is_empty();
        let recorded = exec(&mut core, model, &action, &mut Schedule::Fifo)?;
        let after = core.step_count();

        let position = self.timeline.position;
        let timeline = &mut self.timeline;
        if timeline.actions.get(position) != Some(&recorded) {
            if position < timeline.actions.len() {
                tracing::debug!(fork = position, "play timeline forked; the old line is kept as a branch");
                timeline.branches.push(Branch { fork: position, actions: timeline.actions.clone() });
                timeline.actions.truncate(position);
            }
            timeline.actions.push(recorded);
        }
        timeline.position = position + 1;
        self.quiet_before.truncate(position);
        self.quiet_before.push(quiet);
        self.core = core;
        self.refresh();
        Ok(ActionOutcome { steps: before..after })
    }

    /// Move to `position` on the current timeline (0 = the start). On error
    /// the session is unchanged.
    pub fn seek(&mut self, model: &Model, position: usize) -> Result<(), SimError> {
        let len = self.timeline.actions.len();
        if position > len {
            return Err(SimError::NoSuchPosition { position, len });
        }
        let (core, quiet_before) = if position >= self.timeline.position {
            let mut core = self.core.clone();
            let mut quiet_before = self.quiet_before.clone();
            replay_onto(&mut core, &mut quiet_before, model, &self.timeline.actions[self.timeline.position..position])
                .map_err(|(_, err)| err)?;
            (core, quiet_before)
        } else {
            let mut core = Core::default();
            let mut quiet_before = Vec::new();
            replay_onto(&mut core, &mut quiet_before, model, &self.timeline.actions[..position])
                .map_err(|(_, err)| err)?;
            (core, quiet_before)
        };
        self.core = core;
        self.quiet_before = quiet_before;
        self.timeline.position = position;
        self.refresh();
        Ok(())
    }

    /// Make branch `index` the current timeline, positioned at its end; the
    /// current line takes its place in the branch list, so switching to the
    /// same index again goes back. On error the session is unchanged.
    pub fn switch_branch(&mut self, model: &Model, index: usize) -> Result<(), SimError> {
        let count = self.timeline.branches.len();
        let Some(branch) = self.timeline.branches.get(index) else {
            return Err(SimError::NoSuchBranch { index, count });
        };
        let mut core = Core::default();
        let mut quiet_before = Vec::new();
        replay_onto(&mut core, &mut quiet_before, model, &branch.actions).map_err(|(_, err)| err)?;

        let actions = branch.actions.clone();
        let old = std::mem::replace(&mut self.timeline.actions, actions);
        let fork = old.iter().zip(&self.timeline.actions).take_while(|(a, b)| a == b).count();
        self.timeline.branches[index] = Branch { fork, actions: old };
        self.timeline.position = self.timeline.actions.len();
        self.core = core;
        self.quiet_before = quiet_before;
        self.refresh();
        Ok(())
    }

    /// Rebuild this session against another model (after an edit), replaying
    /// the timeline up to its position. Stops at the first action that no
    /// longer applies and returns the session so far (positioned before that
    /// action, the rest of the line kept as its future) plus the action's
    /// index and error.
    pub fn replay(&self, model: &Model) -> (PlaySession, Option<(usize, SimError)>) {
        let mut session = Self::named(self.name.clone());
        session.timeline = Timeline { position: 0, ..self.timeline.clone() };
        let applied = &self.timeline.actions[..self.timeline.position];
        let failure = replay_onto(&mut session.core, &mut session.quiet_before, model, applied).err();
        session.timeline.position = failure.as_ref().map_or(self.timeline.position, |(at, _)| *at);
        if let Some((at, err)) = &failure {
            tracing::info!(action = at, error = %err, "replay stopped at an action the edited model rejects");
        }
        session.refresh();
        (session, failure)
    }

    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    pub fn trace(&self) -> &Trace {
        &self.trace
    }

    /// The payload of every `Emit` step, and of every `ExternalFire` step
    /// that had one, as in [`crate::SimRun::payloads`].
    pub fn payloads(&self) -> &BTreeMap<StepIx, Payload> {
        &self.payloads
    }

    /// The timeline up to its position as a scenario. Manual queue choices
    /// (`Step { choice: Some(_) }`), runs, and instances added or removed
    /// mid-session are kept as scenario steps, and a session stopped with
    /// items queued saves as `end: pause`, so running the scenario
    /// reproduces this session's trace exactly.
    pub fn to_scenario(&self, name: &str) -> Result<Scenario, SimError> {
        save::to_scenario(self, name)
    }

    /// Rebuild the trace and lifeline map from the core.
    fn refresh(&mut self) {
        let finished = self.core.finish(&self.name);
        self.trace = finished.run.trace;
        self.payloads = finished.run.payloads;
        self.lifelines = finished.lifelines;
    }
}

/// Apply `actions` to `core` in order, noting whether the queue was quiet
/// before each. On failure, the index (within `actions`) and error.
fn replay_onto(
    core: &mut Core,
    quiet_before: &mut Vec<bool>,
    model: &Model,
    actions: &[PlayAction],
) -> Result<(), (usize, SimError)> {
    for (i, action) in actions.iter().enumerate() {
        let quiet = core.queue().is_empty();
        exec(core, model, action, &mut Schedule::Fifo).map_err(|err| (i, err))?;
        quiet_before.push(quiet);
    }
    Ok(())
}

/// Drives a session from a scenario, recording each action at the end of
/// the timeline.
struct Recording<'s>(&'s mut PlaySession);

impl Player for Recording<'_> {
    fn core(&self) -> &Core {
        &self.0.core
    }

    fn is_quiet(&self) -> bool {
        self.0.core.queue().is_empty()
    }

    fn act(&mut self, model: &Model, action: PlayAction) -> Result<(), SimError> {
        let session = &mut *self.0;
        let quiet = session.core.queue().is_empty();
        let recorded = exec(&mut session.core, model, &action, &mut Schedule::Fifo)?;
        session.timeline.actions.push(recorded);
        session.timeline.position = session.timeline.actions.len();
        session.quiet_before.push(quiet);
        Ok(())
    }
}
