//! The steppable simulator: instances, the queue, history (inside each
//! instance), payloads and the recorder, with one operation per thing that
//! can happen. It holds no model; every operation takes the one the state
//! was built with.
//!
//! Callers check names and positions first (see `exec`); the operations
//! here assume the ids and positions they are given are valid.

use std::collections::{HashMap, VecDeque};

use cascade_core::Model;
use cascade_core::definition::FieldClause;
use cascade_core::ids::{ControllerId, EventId, ExternalId, MachineId, RuleId, TriggerId};
use cascade_core::model::Target;

use super::instance::{Instance, InstanceIx};
use super::record::{Finished, RawKind, Recorder, StepRef};
use super::schedule::{SwapState, next_swapped};
use super::{STEP_LIMIT, Swap, select};
use crate::error::SimError;
use crate::scenario::Payload;
use crate::session::PendingId;

/// A queue item with its stable id and display label.
#[derive(Clone, Debug)]
pub(crate) struct Queued {
    pub id: PendingId,
    /// `OrderPaid from o1` or `Fulfillment → s1: start`, fixed when queued.
    pub label: String,
    pub item: QueueItem,
}

#[derive(Clone, Debug)]
pub(crate) enum QueueItem {
    Event { event: EventId, payload: Payload, emitted: StepRef },
    Fire { rule: RuleId, target: InstanceIx, payload: Payload, fired: StepRef, occurrence: u32 },
}

impl QueueItem {
    /// The step that queued the item.
    pub fn cause(&self) -> StepRef {
        match self {
            QueueItem::Event { emitted, .. } => *emitted,
            QueueItem::Fire { fired, .. } => *fired,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Core {
    /// In creation order; removed instances stay (flagged).
    instances: Vec<Instance>,
    /// Every name ever given, removed instances included: names are never
    /// reused.
    by_name: HashMap<String, InstanceIx>,
    queue: VecDeque<Queued>,
    recorder: Recorder,
    /// Queue items delivered so far, for the step limit.
    delivered: usize,
    /// Fires queued so far per (rule, target), for `FireKey::occurrence`.
    fire_counts: HashMap<(RuleId, InstanceIx), u32>,
    /// Next suffix tried for each machine's spawned instance names.
    spawn_counters: HashMap<MachineId, u32>,
    next_id: u32,
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

    pub fn finish(&self, scenario: &str) -> Finished {
        self.recorder.finish(scenario, &self.instances)
    }

    // --- Instances ----------------------------------------------------------------

    /// Add an instance whose name is not taken.
    pub fn add_instance(&mut self, instance: Instance) -> InstanceIx {
        let ix = InstanceIx(self.instances.len());
        self.by_name.insert(instance.name.clone(), ix);
        self.instances.push(instance);
        ix
    }

    /// Take an instance out of play. Fires queued for it are discarded (it
    /// can no longer receive them); events it emitted stay queued, since
    /// they were already sent.
    pub fn remove_instance(&mut self, ix: InstanceIx) {
        self.instances[ix.0].removed = true;
        self.queue.retain(|q| !matches!(q.item, QueueItem::Fire { target, .. } if target == ix));
    }

    // --- External fires and delivery ---------------------------------------------

    /// An external source fires `trigger` at `target`: recorded, then
    /// delivered at once.
    pub fn external_fire(
        &mut self,
        model: &Model,
        source: ExternalId,
        target: InstanceIx,
        trigger: TriggerId,
        payload: &Payload,
    ) {
        let fire = self.recorder.push(None, RawKind::ExternalFire { source, target, trigger });
        if !payload.is_empty() {
            self.recorder.set_payload(fire, payload.clone());
        }
        self.deliver_trigger(model, target, trigger, payload, fire);
    }

    /// Deliver the queue item at `position` (which must exist). Fails,
    /// changing nothing, when the step limit is reached.
    pub fn deliver_at(&mut self, model: &Model, position: usize) -> Result<(), SimError> {
        if self.delivered >= STEP_LIMIT {
            return Err(step_limit());
        }
        match self.queue.remove(position) {
            Some(queued) => self.deliver(model, queued),
            None => Ok(()),
        }
    }

    /// Deliver an item already taken off the queue.
    pub fn deliver(&mut self, model: &Model, queued: Queued) -> Result<(), SimError> {
        self.delivered += 1;
        if self.delivered > STEP_LIMIT {
            return Err(step_limit());
        }
        match queued.item {
            QueueItem::Event { event, payload, emitted } => self.deliver_event(model, event, &payload, emitted),
            QueueItem::Fire { rule, target, payload, fired, .. } => {
                self.deliver_trigger(model, target, model.rule(rule).trigger, &payload, fired);
            }
        }
        Ok(())
    }

    /// Take the next item under a race swap (see `schedule`).
    pub fn pop_swapped(&mut self, swap: &Swap, state: &mut SwapState<Queued>) -> Option<Queued> {
        let instances = &self.instances;
        let is_fire_at = |q: &Queued, rule: RuleId, name: &str| match &q.item {
            QueueItem::Fire { rule: r, target, .. } => *r == rule && instances[target.0].name == name,
            QueueItem::Event { .. } => false,
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

    /// Deliver `trigger` to an instance: take the first enabled transition in
    /// definition order (guards are not evaluated) or drop the trigger. Each
    /// emitted event carries the instance's fields overlaid with `context`,
    /// the payload that came with the trigger.
    fn deliver_trigger(
        &mut self,
        model: &Model,
        target: InstanceIx,
        trigger: TriggerId,
        context: &Payload,
        cause: StepRef,
    ) {
        let instance = &mut self.instances[target.0];
        let from = instance.leaf();
        let Some(&transition) = model.enabled_transitions(from, trigger).first() else {
            self.recorder.push(Some(cause), RawKind::Dropped { instance: target, trigger, state: from });
            return;
        };
        let to = instance.take(model, transition);
        let mut payload = instance.fields.clone();
        payload.extend(context.iter().map(|(k, v)| (k.clone(), v.clone())));
        let label_from = instance.name.clone();

        let step = self.recorder.push(Some(cause), RawKind::Transition { instance: target, transition, from, to });
        for &event in &model.transition(transition).emits {
            let emitted = self.recorder.push(Some(step), RawKind::Emit { instance: target, event });
            self.recorder.set_payload(emitted, payload.clone());
            let label = format!("{} from {label_from}", model.event(event).name);
            self.enqueue(label, QueueItem::Event { event, payload: payload.clone(), emitted });
        }
    }

    /// Deliver an event to every subscribed handler in definition order.
    fn deliver_event(&mut self, model: &Model, event: EventId, payload: &Payload, emitted: StepRef) {
        for &handler in &model.event(event).handlers {
            let h = model.handler(handler);
            let deliver =
                self.recorder.push(Some(emitted), RawKind::Deliver { controller: h.controller, event, handler });
            for &rule in &h.rules {
                self.apply_rule(model, rule, payload, deliver);
            }
        }
    }

    fn apply_rule(&mut self, model: &Model, rule: RuleId, payload: &Payload, deliver: StepRef) {
        let r = model.rule(rule);
        let controller = r.controller;
        let machine = model.trigger(r.trigger).machine;
        match &r.target {
            Target::One { predicates } => {
                let candidates = self.matching(machine, predicates, payload);
                match candidates.as_slice() {
                    [] => {
                        self.recorder.push(Some(deliver), RawKind::NoTarget { controller, rule });
                    }
                    [one] => self.fire(model, rule, controller, *one, payload, deliver),
                    many => {
                        let candidates = many.to_vec();
                        self.recorder.push(Some(deliver), RawKind::Ambiguous { controller, rule, candidates });
                    }
                }
            }
            Target::All { predicates } => {
                let candidates = self.matching(machine, predicates, payload);
                if candidates.is_empty() {
                    self.recorder.push(Some(deliver), RawKind::NoTarget { controller, rule });
                }
                for target in candidates {
                    self.fire(model, rule, controller, target, payload, deliver);
                }
            }
            Target::Spawn { assignments } => {
                let fields = select::assign(assignments, payload);
                let name = self.spawn_name(model, machine);
                let initial = model.machine(machine).initial;
                let instance = self.add_instance(Instance::new(model, name, machine, fields, initial));
                let spawn = self.recorder.push(Some(deliver), RawKind::Spawn { controller, instance, rule });
                self.fire(model, rule, controller, instance, payload, spawn);
            }
        }
    }

    fn matching(&self, machine: MachineId, predicates: &[FieldClause], payload: &Payload) -> Vec<InstanceIx> {
        self.instances_of(machine)
            .into_iter()
            .filter(|ix| select::matches(&self.instances[ix.0].fields, predicates, payload))
            .collect()
    }

    /// Record a fire and queue its delivery.
    fn fire(
        &mut self,
        model: &Model,
        rule: RuleId,
        controller: ControllerId,
        target: InstanceIx,
        payload: &Payload,
        cause: StepRef,
    ) {
        let fired = self.recorder.push(Some(cause), RawKind::Fire { controller, target, rule });
        let count = self.fire_counts.entry((rule, target)).or_insert(0);
        let occurrence = *count;
        *count += 1;
        let label = format!(
            "{} → {}: {}",
            model.controller(controller).name,
            self.instances[target.0].name,
            model.trigger(model.rule(rule).trigger).name
        );
        self.enqueue(label, QueueItem::Fire { rule, target, payload: payload.clone(), fired, occurrence });
    }

    fn enqueue(&mut self, label: String, item: QueueItem) {
        let id = PendingId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.queue.push_back(Queued { id, label, item });
    }

    /// `<machine-lowercase><n>` with the smallest unused `n`, counting up
    /// from the last spawn of this machine.
    fn spawn_name(&mut self, model: &Model, machine: MachineId) -> String {
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
