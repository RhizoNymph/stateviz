//! The steppable simulator: instances, the queue, history (inside each
//! instance), payloads, the clock and armed timers, pending decisions and
//! the recorder, with one operation per thing that can happen. It holds no
//! model; every operation takes the one the state was built with.
//!
//! Callers check names and positions first (see `exec`); the operations
//! here assume the ids and positions they are given are valid.
//!
//! - `deliver`: offering triggers to instances (guards, choices, `sets`,
//!   emits), delivering events to subscribers and controllers.
//! - `clock`: arming, cancelling and expiring timers.

use std::collections::{HashMap, VecDeque};

use cascade_core::Model;
use cascade_core::expr::Duration;
use cascade_core::ids::{ControllerId, EventId, MachineId, RuleId, TriggerId};

use super::instance::{Instance, InstanceIx};
use super::record::{Finished, Participant, Recorder, StepRef};
use super::schedule::{SwapState, next_swapped};
use super::{STEP_LIMIT, Swap};
use crate::error::SimError;
use crate::scenario::Payload;
use crate::session::PendingId;

/// A queue item with its stable id and display label.
#[derive(Clone, Debug)]
pub(crate) struct Queued {
    pub id: PendingId,
    /// `OrderPaid from o1`, `Fulfillment → s1: start` or `timer → o1:
    /// after-30m`, fixed when queued.
    pub label: String,
    /// Who still has to receive it, in delivery order (see
    /// `PendingItem::consumers_left`). A fire or timer has its target only.
    pub consumers_left: Vec<Consumer>,
    pub item: QueueItem,
}

/// One receiver of a queue item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Consumer {
    pub name: String,
    pub target: ConsumerRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum ConsumerRef {
    /// A subscribing instance (for an event) or the target (fire, timer).
    Instance(InstanceIx),
    /// A controller with handlers for the event.
    Controller(ControllerId),
}

#[derive(Clone, Debug)]
pub(crate) enum QueueItem {
    Event { event: EventId, payload: Payload, emitted: StepRef },
    Fire { rule: RuleId, target: InstanceIx, payload: Payload, fired: StepRef, occurrence: u32 },
    Timer { trigger: TriggerId, target: InstanceIx, expired: StepRef },
}

impl QueueItem {
    /// The step that queued the item.
    pub fn cause(&self) -> StepRef {
        match self {
            QueueItem::Event { emitted, .. } => *emitted,
            QueueItem::Fire { fired, .. } => *fired,
            QueueItem::Timer { expired, .. } => *expired,
        }
    }
}

/// A timer armed by a state an instance is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Armed {
    pub instance: InstanceIx,
    pub trigger: TriggerId,
    /// The state whose entry armed it; leaving the state cancels it.
    pub state: cascade_core::ids::StateId,
    pub deadline: Duration,
    /// Arming order, for ties between equal deadlines.
    pub seq: u64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Core {
    /// In creation order; removed instances stay (flagged).
    pub(super) instances: Vec<Instance>,
    /// Every name ever given, removed instances included: names are never
    /// reused.
    pub(super) by_name: HashMap<String, InstanceIx>,
    pub(super) queue: VecDeque<Queued>,
    pub(super) recorder: Recorder,
    /// Queue deliveries so far (a whole item or one consumer of an event),
    /// for the step limit.
    pub(super) delivered: usize,
    /// Fires queued so far per (rule, target), for `FireKey::occurrence`.
    pub(super) fire_counts: HashMap<(RuleId, InstanceIx), u32>,
    /// Next suffix tried for each machine's spawned instance names.
    pub(super) spawn_counters: HashMap<MachineId, u32>,
    pub(super) next_id: u32,
    /// The virtual clock.
    pub(super) now: Duration,
    /// Armed timers, sorted by (deadline, seq).
    pub(super) armed: Vec<Armed>,
    pub(super) next_arm: u64,
    /// Pending decisions, oldest first.
    decisions: Vec<String>,
    /// Each pending decision's push order within the core (0 for the first
    /// `Choose`), parallel to `decisions`, so a batch run can report unused
    /// decisions at their directive.
    decision_seqs: Vec<usize>,
    next_decision: usize,
}

impl Core {
    // --- Reading ----------------------------------------------------------------

    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }

    pub fn instance(&self, ix: InstanceIx) -> &Instance {
        &self.instances[ix.0]
    }

    pub fn queue(&self) -> &VecDeque<Queued> {
        &self.queue
    }

    pub fn step_count(&self) -> usize {
        self.recorder.len()
    }

    pub fn now(&self) -> Duration {
        self.now
    }

    /// Armed timers in deadline order (ties: arming order).
    pub fn armed(&self) -> &[Armed] {
        &self.armed
    }

    /// Pending decisions, oldest first.
    pub fn decisions(&self) -> &[String] {
        &self.decisions
    }

    /// Pending decisions with their push order.
    pub fn decisions_with_seq(&self) -> impl Iterator<Item = (&str, usize)> + '_ {
        self.decisions.iter().map(String::as_str).zip(self.decision_seqs.iter().copied())
    }

    /// Whether an instance has ever had this name.
    pub fn is_taken(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// The live instance with this name.
    pub fn alive_named(&self, name: &str) -> Option<InstanceIx> {
        self.by_name.get(name).copied().filter(|ix| !self.instances[ix.0].removed)
    }

    /// Live instances of `machine` in creation order.
    pub fn instances_of(&self, machine: MachineId) -> Vec<InstanceIx> {
        self.instances
            .iter()
            .enumerate()
            .filter(|(_, i)| i.machine == machine && !i.removed)
            .map(|(ix, _)| InstanceIx(ix))
            .collect()
    }

    /// `<machine-lowercase><n>` with the smallest `n ≥ 1` never used, for
    /// instances the player adds without a name. Spawn naming is separate
    /// (see [`Self::spawn_name`]) and skips these names like any other.
    pub fn free_name(&self, model: &Model, machine: MachineId) -> String {
        let prefix = crate::scenario::spawn_prefix(model, machine);
        (1..=u32::MAX)
            .map(|n| format!("{prefix}{n}"))
            .find(|name| !self.is_taken(name))
            .unwrap_or_else(|| format!("{prefix}{}", self.instances.len() + 1))
    }

    /// The kinds of the steps recorded from step `from` on.
    pub fn kinds_since(&self, from: StepRef) -> impl Iterator<Item = &super::record::RawKind> + '_ {
        self.recorder.kinds_since(from)
    }

    /// Forget the recorded steps (exploration keeps no trace; see
    /// `Recorder::forget`).
    pub fn forget_trace(&mut self) {
        self.recorder.forget();
    }

    pub fn finish(&self, scenario: &str) -> Finished {
        self.recorder.finish(scenario, &self.instances)
    }

    /// The choice points recorded from step `from` on: the deciding
    /// instance's or controller's name and the options.
    pub fn choices_since(&self, model: &Model, from: StepRef) -> Vec<(String, Vec<String>)> {
        self.recorder
            .choices_since(from)
            .into_iter()
            .map(|(at, options)| {
                let name = match at {
                    Participant::Instance(ix) => self.instances[ix.0].name.clone(),
                    Participant::Controller(c) => model.controller(c).name.clone(),
                };
                (name, options)
            })
            .collect()
    }

    // --- Instances ----------------------------------------------------------------

    /// Add an instance whose name is not taken, arming the timers of every
    /// state it starts in.
    pub fn add_instance(&mut self, model: &Model, instance: Instance) -> InstanceIx {
        let ix = InstanceIx(self.instances.len());
        let entered = instance.active_states(model);
        self.by_name.insert(instance.name.clone(), ix);
        self.instances.push(instance);
        self.arm(model, ix, &entered);
        ix
    }

    /// Take an instance out of play. Fires and timers queued for it are
    /// discarded and its armed timers cancelled (it can no longer receive
    /// them), and it is no longer a consumer of queued events; events it
    /// emitted stay queued, since they were already sent.
    pub fn remove_instance(&mut self, ix: InstanceIx) {
        self.instances[ix.0].removed = true;
        self.queue.retain(|q| match q.item {
            QueueItem::Fire { target, .. } | QueueItem::Timer { target, .. } => target != ix,
            QueueItem::Event { .. } => true,
        });
        for queued in &mut self.queue {
            queued.consumers_left.retain(|c| c.target != ConsumerRef::Instance(ix));
        }
        self.armed.retain(|a| a.instance != ix);
    }

    // --- Decisions ------------------------------------------------------------------

    /// Push a pending decision.
    pub fn push_decision(&mut self, option: String) {
        self.decision_seqs.push(self.next_decision);
        self.next_decision += 1;
        self.decisions.push(option);
    }

    /// Decide at a choice point: the first pending decision that is one of
    /// `options` is consumed and taken; without one the first option is
    /// (`defaulted`). Returns the index of the option taken, its text and
    /// whether it defaulted. `options` is never empty.
    pub(super) fn decide(&mut self, options: &[String]) -> (usize, String, bool) {
        let found = self
            .decisions
            .iter()
            .enumerate()
            .find_map(|(at, d)| options.iter().position(|o| o == d).map(|ix| (at, ix)));
        match found {
            Some((at, ix)) => {
                self.decision_seqs.remove(at);
                (ix, self.decisions.remove(at), false)
            }
            None => (0, options.first().cloned().unwrap_or_default(), true),
        }
    }

    // --- Delivery -------------------------------------------------------------------

    /// Count one delivery against the step limit. Fails, changing nothing,
    /// when the limit is already reached.
    fn count_delivery(&mut self) -> Result<(), SimError> {
        if self.delivered >= STEP_LIMIT {
            return Err(step_limit());
        }
        self.delivered += 1;
        Ok(())
    }

    /// Deliver the queue item at `position` (which must exist) to all of
    /// its remaining consumers. Fails, changing nothing, when the step limit
    /// is reached.
    pub fn deliver_at(&mut self, model: &Model, position: usize) -> Result<(), SimError> {
        if self.delivered >= STEP_LIMIT {
            return Err(step_limit());
        }
        match self.queue.remove(position) {
            Some(queued) => self.deliver(model, queued),
            None => Ok(()),
        }
    }

    /// Deliver the event at `position` to its remaining consumer `name`
    /// only (both must exist); the item stays queued while others remain.
    /// A fire or timer is delivered whole.
    pub fn deliver_to_consumer(&mut self, model: &Model, position: usize, name: &str) -> Result<(), SimError> {
        let Some(queued) = self.queue.get_mut(position) else {
            return Ok(());
        };
        if !matches!(queued.item, QueueItem::Event { .. }) {
            return self.deliver_at(model, position);
        }
        let Some(at) = queued.consumers_left.iter().position(|c| c.name == name) else {
            return Ok(());
        };
        self.count_delivery()?;
        let Some(queued) = self.queue.get_mut(position) else {
            return Ok(());
        };
        let consumer = queued.consumers_left.remove(at);
        let QueueItem::Event { event, payload, emitted } = queued.item.clone() else {
            return Ok(());
        };
        if queued.consumers_left.is_empty() {
            self.queue.remove(position);
        }
        self.deliver_event_to(model, event, &payload, emitted, consumer.target);
        Ok(())
    }

    /// Deliver an item already taken off the queue to all of its remaining
    /// consumers.
    pub fn deliver(&mut self, model: &Model, queued: Queued) -> Result<(), SimError> {
        self.delivered += 1;
        if self.delivered > STEP_LIMIT {
            return Err(step_limit());
        }
        match queued.item {
            QueueItem::Event { event, payload, emitted } => {
                for consumer in queued.consumers_left {
                    self.deliver_event_to(model, event, &payload, emitted, consumer.target);
                }
            }
            QueueItem::Fire { rule, target, payload, fired, .. } => {
                self.offer(model, target, model.rule(rule).trigger, &payload, fired, None);
            }
            QueueItem::Timer { trigger, target, expired } => {
                self.offer(model, target, trigger, &Payload::new(), expired, None);
            }
        }
        Ok(())
    }

    /// Take the next item under a race swap (see `schedule`).
    pub fn pop_swapped(&mut self, swap: &Swap, state: &mut SwapState<Queued>) -> Option<Queued> {
        let instances = &self.instances;
        let is_fire_at = |q: &Queued, rule: RuleId, name: &str| match &q.item {
            QueueItem::Fire { rule: r, target, .. } => *r == rule && instances[target.0].name == name,
            QueueItem::Event { .. } | QueueItem::Timer { .. } => false,
        };
        let yielder = &swap.yielder;
        next_swapped(
            &mut self.queue,
            state,
            |q| {
                is_fire_at(q, yielder.rule, &yielder.instance)
                    && matches!(q.item, QueueItem::Fire { occurrence, .. } if occurrence == yielder.occurrence)
            },
            |q| is_fire_at(q, swap.overtaker, &yielder.instance),
        )
    }

    pub(super) fn enqueue(&mut self, label: String, consumers_left: Vec<Consumer>, item: QueueItem) {
        let id = PendingId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.queue.push_back(Queued { id, label, consumers_left, item });
    }

    /// The single consumer of a fire or timer: its target.
    pub(super) fn target_consumer(&self, target: InstanceIx) -> Vec<Consumer> {
        vec![Consumer { name: self.instances[target.0].name.clone(), target: ConsumerRef::Instance(target) }]
    }

    /// An event's consumers in delivery order, as of now: for each
    /// subscribing machine in model order, its live instances in creation
    /// order; then the controllers whose handlers receive it, in definition
    /// order.
    pub(crate) fn event_consumers(&self, model: &Model, event: EventId) -> Vec<Consumer> {
        let mut machines: Vec<MachineId> =
            model.event(event).subscribers.iter().map(|&t| model.trigger(t).machine).collect();
        machines.sort_unstable();
        machines.dedup();
        let mut consumers: Vec<Consumer> = machines
            .into_iter()
            .flat_map(|m| self.instances_of(m))
            .map(|ix| Consumer { name: self.instances[ix.0].name.clone(), target: ConsumerRef::Instance(ix) })
            .collect();
        for &handler in &model.event(event).handlers {
            let controller = model.handler(handler).controller;
            if !consumers.iter().any(|c| c.target == ConsumerRef::Controller(controller)) {
                let name = model.controller(controller).name.clone();
                consumers.push(Consumer { name, target: ConsumerRef::Controller(controller) });
            }
        }
        consumers
    }

    /// `<machine-lowercase><n>` with the smallest unused `n`, counting up
    /// from the last spawn of this machine.
    pub(super) fn spawn_name(&mut self, model: &Model, machine: MachineId) -> String {
        let prefix = crate::scenario::spawn_prefix(model, machine);
        let counter = self.spawn_counters.entry(machine).or_insert(1);
        loop {
            let name = format!("{prefix}{counter}");
            *counter += 1;
            if !self.by_name.contains_key(&name) {
                return name;
            }
        }
    }
}

fn step_limit() -> SimError {
    tracing::warn!(limit = STEP_LIMIT, "simulation hit the step limit");
    SimError::StepLimit(STEP_LIMIT)
}
