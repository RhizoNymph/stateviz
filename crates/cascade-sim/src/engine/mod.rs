//! The simulator proper: instances, one global FIFO queue, trace recording.
//!
//! ```text
//! scenario step ──▶ ExternalFire ──▶ Transition | Dropped ──▶ Emit ──▶ queue
//! queue head: Event ──▶ Deliver (per handler) ──▶ Fire | Spawn+Fire | NoTarget | Ambiguous
//!                                                   └──▶ queue
//! queue head: Fire  ──▶ Transition | Dropped ──▶ Emit ──▶ queue
//! ```

mod instance;
mod record;
mod schedule;
mod select;

use std::collections::{HashMap, VecDeque};

use cascade_core::Model;
use cascade_core::definition::FieldClause;
use cascade_core::ids::{ControllerId, EventId, MachineId, RuleId, TriggerId};
use cascade_core::model::Target;

use instance::{Instance, InstanceIx};
use record::{RawKind, Recorder, StepRef};
use schedule::{SwapState, next_swapped};

use crate::SimRun;
use crate::error::{ScenarioError, ScenarioErrorKind, SimError};
use crate::scenario::{Payload, ResolvedScenario, ResolvedStep, StepTarget, StepTiming};

/// At most this many queue items (events and controller fires) are
/// delivered in one run; past it the run fails with
/// [`SimError::StepLimit`].
pub const STEP_LIMIT: usize = 10_000;

/// Identifies one fire across deterministic reruns of a scenario: the
/// `occurrence`-th (0-based) fire of `rule` at the instance named
/// `instance`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FireKey {
    pub rule: RuleId,
    pub instance: String,
    pub occurrence: u32,
}

/// Reorder two contested fires: `yielder` lets the next fire of `overtaker`
/// at the same instance go first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Swap {
    pub yielder: FireKey,
    pub overtaker: RuleId,
}

pub(crate) struct Outcome {
    pub run: SimRun,
    /// With a [`Swap`]: whether the overtaker really went first.
    pub swapped: bool,
}

/// Run a validated scenario, in FIFO order or with one swap.
pub(crate) fn run(model: &Model, scenario: &ResolvedScenario, swap: Option<Swap>) -> Result<Outcome, SimError> {
    let mut engine = Engine::new(model, scenario, swap);
    engine.run_steps(&scenario.steps)?;
    let swapped = matches!(engine.swap, Some((_, SwapState::Swapped)));
    tracing::debug!(
        scenario = %scenario.name,
        steps = engine.recorder.len(),
        delivered = engine.delivered,
        instances = engine.instances.len(),
        swapped,
        "simulation finished"
    );
    let Engine { recorder, instances, .. } = engine;
    Ok(Outcome { run: recorder.finish(&scenario.name, &instances), swapped })
}

#[derive(Debug)]
enum QueueItem {
    Event { event: EventId, payload: Payload, emitted: StepRef },
    Fire { rule: RuleId, target: InstanceIx, payload: Payload, fired: StepRef, occurrence: u32 },
}

struct Engine<'m> {
    model: &'m Model,
    instances: Vec<Instance>,
    by_name: HashMap<String, InstanceIx>,
    queue: VecDeque<QueueItem>,
    recorder: Recorder,
    /// Queue items delivered so far, for the step limit.
    delivered: usize,
    /// Fires queued so far per (rule, target), for [`FireKey::occurrence`].
    fire_counts: HashMap<(RuleId, InstanceIx), u32>,
    /// Next suffix tried for each machine's spawned instance names.
    spawn_counters: HashMap<MachineId, u32>,
    swap: Option<(Swap, SwapState<QueueItem>)>,
}

impl<'m> Engine<'m> {
    fn new(model: &'m Model, scenario: &ResolvedScenario, swap: Option<Swap>) -> Self {
        let mut engine = Engine {
            model,
            instances: Vec::with_capacity(scenario.instances.len()),
            by_name: HashMap::new(),
            queue: VecDeque::new(),
            recorder: Recorder::default(),
            delivered: 0,
            fire_counts: HashMap::new(),
            spawn_counters: HashMap::new(),
            swap: swap.map(|s| (s, SwapState::Armed)),
        };
        for decl in &scenario.instances {
            engine.add_instance(Instance::new(model, decl.name.clone(), decl.machine, decl.fields.clone(), decl.start));
        }
        engine
    }

    fn add_instance(&mut self, instance: Instance) -> InstanceIx {
        let ix = InstanceIx(self.instances.len());
        self.by_name.insert(instance.name.clone(), ix);
        self.instances.push(instance);
        ix
    }

    // --- Scenario steps -------------------------------------------------------

    fn run_steps(&mut self, steps: &[ResolvedStep]) -> Result<(), SimError> {
        for step in steps {
            if step.timing == StepTiming::AfterQuiescence {
                self.drain()?;
            }
            let target = self.step_target(step)?;
            let fire =
                self.recorder.push(None, RawKind::ExternalFire { source: step.source, target, trigger: step.trigger });
            if !step.payload.is_empty() {
                self.recorder.set_payload(fire, step.payload.clone());
            }
            self.deliver_trigger(target, step.trigger, &step.payload, fire);
        }
        self.drain()
    }

    /// The instance a step fires at, as it stands when the step runs.
    fn step_target(&self, step: &ResolvedStep) -> Result<InstanceIx, SimError> {
        let fired_machine = self.model.trigger(step.trigger).machine;
        match &step.target {
            StepTarget::Named(name) => {
                let Some(&ix) = self.by_name.get(name.as_str()) else {
                    let kind = ScenarioErrorKind::UnknownInstance { name: name.value.clone() };
                    return Err(ScenarioError::single(kind, name.span).into());
                };
                let machine = self.instances[ix.0].machine;
                if machine != fired_machine {
                    let kind = ScenarioErrorKind::TargetMachineMismatch {
                        instance: name.value.clone(),
                        machine: self.model.machine(machine).name.clone(),
                        fire: self.trigger_label(step.trigger),
                    };
                    return Err(ScenarioError::single(kind, name.span).into());
                }
                Ok(ix)
            }
            StepTarget::TheOne(machine) => {
                let candidates = self.instances_of(*machine);
                match candidates.as_slice() {
                    [one] => Ok(*one),
                    [] => {
                        let kind = ScenarioErrorKind::NoInstance { machine: self.model.machine(*machine).name.clone() };
                        Err(ScenarioError::single(kind, step.span).into())
                    }
                    many => {
                        let kind = ScenarioErrorKind::AmbiguousInstance {
                            machine: self.model.machine(*machine).name.clone(),
                            candidates: many.iter().map(|ix| self.instances[ix.0].name.clone()).collect(),
                        };
                        Err(ScenarioError::single(kind, step.span).into())
                    }
                }
            }
        }
    }

    fn trigger_label(&self, trigger: TriggerId) -> String {
        let t = self.model.trigger(trigger);
        format!("{}.{}", self.model.machine(t.machine).name, t.name)
    }

    /// Instances of `machine` in creation order.
    fn instances_of(&self, machine: MachineId) -> Vec<InstanceIx> {
        self.instances.iter().enumerate().filter(|(_, i)| i.machine == machine).map(|(ix, _)| InstanceIx(ix)).collect()
    }

    // --- The queue ------------------------------------------------------------

    fn drain(&mut self) -> Result<(), SimError> {
        while let Some(item) = self.next_item() {
            self.delivered += 1;
            if self.delivered > STEP_LIMIT {
                tracing::warn!(limit = STEP_LIMIT, "simulation hit the step limit");
                return Err(SimError::StepLimit(STEP_LIMIT));
            }
            match item {
                QueueItem::Event { event, payload, emitted } => self.deliver_event(event, &payload, emitted),
                QueueItem::Fire { rule, target, payload, fired, .. } => {
                    self.deliver_trigger(target, self.model.rule(rule).trigger, &payload, fired);
                }
            }
        }
        Ok(())
    }

    fn next_item(&mut self) -> Option<QueueItem> {
        let Some((swap, state)) = &mut self.swap else {
            return self.queue.pop_front();
        };
        let instances = &self.instances;
        let is_fire_at = |item: &QueueItem, rule: RuleId, name: &str| match item {
            QueueItem::Fire { rule: r, target, .. } => *r == rule && instances[target.0].name == name,
            QueueItem::Event { .. } => false,
        };
        let yielder = &swap.yielder;
        next_swapped(
            &mut self.queue,
            state,
            |item| {
                is_fire_at(item, yielder.rule, &yielder.instance)
                    && matches!(item, QueueItem::Fire { occurrence, .. } if *occurrence == yielder.occurrence)
            },
            |item| is_fire_at(item, swap.overtaker, &yielder.instance),
        )
    }

    // --- Delivery -------------------------------------------------------------

    /// Deliver `trigger` to an instance: take the first enabled transition in
    /// definition order (guards are not evaluated) or drop the trigger. Each
    /// emitted event carries the instance's fields overlaid with `context`,
    /// the payload that came with the trigger.
    fn deliver_trigger(&mut self, target: InstanceIx, trigger: TriggerId, context: &Payload, cause: StepRef) {
        let model = self.model;
        let instance = &mut self.instances[target.0];
        let from = instance.leaf();
        let Some(&transition) = model.enabled_transitions(from, trigger).first() else {
            self.recorder.push(Some(cause), RawKind::Dropped { instance: target, trigger, state: from });
            return;
        };
        let to = instance.take(model, transition);
        let mut payload = instance.fields.clone();
        payload.extend(context.iter().map(|(k, v)| (k.clone(), v.clone())));

        let step = self.recorder.push(Some(cause), RawKind::Transition { instance: target, transition, from, to });
        for &event in &model.transition(transition).emits {
            let emitted = self.recorder.push(Some(step), RawKind::Emit { instance: target, event });
            self.recorder.set_payload(emitted, payload.clone());
            self.queue.push_back(QueueItem::Event { event, payload: payload.clone(), emitted });
        }
    }

    /// Deliver an event to every subscribed handler in definition order.
    fn deliver_event(&mut self, event: EventId, payload: &Payload, emitted: StepRef) {
        let model = self.model;
        for &handler in &model.event(event).handlers {
            let h = model.handler(handler);
            let deliver =
                self.recorder.push(Some(emitted), RawKind::Deliver { controller: h.controller, event, handler });
            for &rule in &h.rules {
                self.apply_rule(rule, payload, deliver);
            }
        }
    }

    fn apply_rule(&mut self, rule: RuleId, payload: &Payload, deliver: StepRef) {
        let model = self.model;
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
                    [one] => self.fire(rule, controller, *one, payload, deliver),
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
                    self.fire(rule, controller, target, payload, deliver);
                }
            }
            Target::Spawn { assignments } => {
                let fields = select::assign(assignments, payload);
                let name = self.spawn_name(machine);
                let initial = model.machine(machine).initial;
                let instance = self.add_instance(Instance::new(model, name, machine, fields, initial));
                let spawn = self.recorder.push(Some(deliver), RawKind::Spawn { controller, instance, rule });
                self.fire(rule, controller, instance, payload, spawn);
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
    fn fire(&mut self, rule: RuleId, controller: ControllerId, target: InstanceIx, payload: &Payload, cause: StepRef) {
        let fired = self.recorder.push(Some(cause), RawKind::Fire { controller, target, rule });
        let count = self.fire_counts.entry((rule, target)).or_insert(0);
        let occurrence = *count;
        *count += 1;
        self.queue.push_back(QueueItem::Fire { rule, target, payload: payload.clone(), fired, occurrence });
    }

    /// `<machine-lowercase><n>` with the smallest unused `n`, counting up
    /// from the last spawn of this machine.
    fn spawn_name(&mut self, machine: MachineId) -> String {
        let prefix = crate::scenario::spawn_prefix(self.model, machine);
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
