//! Offering triggers to instances and delivering events.
//!
//! ```text
//! offer(trigger) ──pick──▶ nothing ──▶ Dropped (command, timer) | ignored (subscription)
//!                        ├▶ one ──────────────────────────────┐
//!                        └▶ choice point ──▶ Choice ──────────┤
//!                                                             ▼
//!          Transition ──▶ SetFields (when `sets` assigned) ──▶ Emit… ──▶ queue
//! event ──▶ subscribing instance: offer(subscription) (Notify first when taken)
//!       └─▶ controller: Deliver ──▶ each rule: [Choice] ──▶ Fire | Spawn+Fire | NoTarget | Ambiguous
//! ```
//!
//! Picking a transition: the enabled candidates
//! ([`Model::enabled_transitions`]) whose subscription `where` holds are
//! split by guard. An expression guard must hold; no guard always holds.
//! When `?choice` candidates remain, the remaining candidates (choices,
//! holding expressions, unguarded ones and `else`) form a choice point if
//! there are at least two. Otherwise the first holding candidate is taken,
//! and `else` only when none holds.

use cascade_core::Model;
use cascade_core::expr::{Condition, Expr, Guard, MapScope, Types, eval, value};
use cascade_core::ids::{ControllerId, EventId, ExternalId, MachineId, RuleId, StateId, TransitionId, TriggerId};
use cascade_core::model::Target;

use super::core::{ConsumerRef, Core, QueueItem};
use super::instance::{Instance, InstanceIx};
use super::record::{Participant, RawKind, StepRef};
use super::select;
use crate::scenario::Payload;

/// The label of an unguarded candidate at a choice point.
pub(crate) const UNGUARDED: &str = "unguarded";

/// What a trigger does to an instance.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Pick {
    /// No candidate: dropped, or ignored for a subscription.
    Nothing,
    Take(TransitionId),
    /// A choice point: `options[i]` labels `candidates[i]`.
    Choose {
        options: Vec<String>,
        candidates: Vec<TransitionId>,
    },
}

/// Pick the transition `trigger` takes from `leaf`, reading `self` and
/// `event` from `scope`.
fn pick(model: &Model, leaf: StateId, trigger: TriggerId, scope: &MapScope<'_>, types: &Types) -> Pick {
    let enabled: Vec<TransitionId> = model
        .enabled_transitions(leaf, trigger)
        .into_iter()
        .filter(|&t| model.transition(t).filter.as_ref().is_none_or(|f| eval(f, scope, types)))
        .collect();

    let mut remaining: Vec<(TransitionId, String)> = Vec::new();
    let mut has_choice = false;
    let mut first_holding = None;
    let mut first_else = None;
    for &t in &enabled {
        match &model.transition(t).guard {
            Some(Guard::Expr(guard)) => {
                if eval(guard, scope, types) {
                    first_holding.get_or_insert(t);
                    remaining.push((t, guard.to_string()));
                }
            }
            None => {
                first_holding.get_or_insert(t);
                remaining.push((t, UNGUARDED.to_owned()));
            }
            Some(Guard::Choice(label)) => {
                has_choice = true;
                remaining.push((t, label.clone()));
            }
            Some(Guard::Else) => {
                first_else.get_or_insert(t);
                remaining.push((t, "else".to_owned()));
            }
        }
    }

    if has_choice {
        return match remaining.as_slice() {
            [] => Pick::Nothing,
            [(only, _)] => Pick::Take(*only),
            _ => {
                let (candidates, options) = remaining.into_iter().unzip();
                Pick::Choose { options, candidates }
            }
        };
    }
    match first_holding.or(first_else) {
        Some(t) => Pick::Take(t),
        None => Pick::Nothing,
    }
}

impl Core {
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
        self.offer(model, target, trigger, payload, fire, None);
    }

    /// Offer `trigger` to an instance with `context`, the payload that came
    /// with it. `subscription` is the delivered event when the trigger is a
    /// subscription: then nothing to take means the event is ignored (no
    /// step), and a taken one records `Notify` first.
    pub(super) fn offer(
        &mut self,
        model: &Model,
        target: InstanceIx,
        trigger: TriggerId,
        context: &Payload,
        cause: StepRef,
        subscription: Option<EventId>,
    ) {
        let instance = &self.instances[target.0];
        let from = instance.leaf();
        let types = model.types_for(instance.machine, subscription);
        let scope = MapScope { event: context, self_fields: &instance.fields };
        let picked = pick(model, from, trigger, &scope, &types);

        let mut cause = cause;
        if picked != Pick::Nothing
            && let Some(event) = subscription
        {
            cause = self.recorder.push(Some(cause), RawKind::Notify { instance: target, event });
        }
        let transition = match picked {
            Pick::Nothing => {
                if subscription.is_none() {
                    self.recorder.push(Some(cause), RawKind::Dropped { instance: target, trigger, state: from });
                }
                return;
            }
            Pick::Take(t) => t,
            Pick::Choose { options, candidates } => {
                let (ix, chosen, defaulted) = self.decide(&options);
                let at = Participant::Instance(target);
                cause = self.recorder.push(Some(cause), RawKind::Choice { at, options, chosen, defaulted });
                candidates[ix]
            }
        };
        self.take(model, target, transition, context, cause);
    }

    /// Take `transition`: leave and enter states (cancelling and arming
    /// timers), apply `sets`, and queue each emitted event with the
    /// instance's fields overlaid with `context`, then its `with` entries.
    fn take(&mut self, model: &Model, target: InstanceIx, transition: TransitionId, context: &Payload, cause: StepRef) {
        let t = model.transition(transition);
        let instance = &mut self.instances[target.0];
        let from = instance.leaf();
        let before = instance.fields.clone();
        let change = instance.take(model, transition);
        let to = change.to;

        let assigned: Payload = {
            let scope = MapScope { event: context, self_fields: &before };
            t.sets.iter().filter_map(|a| value(&a.value, &scope).map(|v| (a.field.clone(), v.to_owned()))).collect()
        };
        instance.fields.extend(assigned.iter().map(|(k, v)| (k.clone(), v.clone())));
        let fields = instance.fields.clone();
        let label_from = instance.name.clone();

        self.cancel(target, &change.exited);
        self.arm(model, target, &change.entered);

        let step = self.recorder.push(Some(cause), RawKind::Transition { instance: target, transition, from, to });
        if !assigned.is_empty() {
            self.recorder.push(Some(step), RawKind::SetFields { instance: target, fields: assigned });
        }

        let mut base = fields.clone();
        base.extend(context.iter().map(|(k, v)| (k.clone(), v.clone())));
        for emit in &t.emits {
            let mut payload = base.clone();
            let scope = MapScope { event: context, self_fields: &fields };
            for assignment in &emit.with {
                if let Some(v) = value(&assignment.value, &scope) {
                    payload.insert(assignment.field.clone(), v.to_owned());
                }
            }
            let event = emit.event;
            let emitted = self.recorder.push(Some(step), RawKind::Emit { instance: target, event });
            self.recorder.set_payload(emitted, payload.clone());
            let label = format!("{} from {label_from}", model.event(event).name);
            let consumers = self.event_consumers(model, event);
            self.enqueue(label, consumers, QueueItem::Event { event, payload, emitted });
        }
    }

    /// Deliver an event to one consumer: a subscribing instance is offered
    /// its subscription trigger; a controller runs its handlers' rules.
    pub(super) fn deliver_event_to(
        &mut self,
        model: &Model,
        event: EventId,
        payload: &Payload,
        emitted: StepRef,
        consumer: ConsumerRef,
    ) {
        match consumer {
            ConsumerRef::Instance(ix) => {
                let instance = &self.instances[ix.0];
                if instance.removed {
                    return;
                }
                if let Some(trigger) = model.subscription_trigger(instance.machine, event) {
                    self.offer(model, ix, trigger, payload, emitted, Some(event));
                }
            }
            ConsumerRef::Controller(controller) => {
                for &handler in &model.event(event).handlers {
                    let h = model.handler(handler);
                    if h.controller != controller {
                        continue;
                    }
                    let deliver = self.recorder.push(Some(emitted), RawKind::Deliver { controller, event, handler });
                    for &rule in &h.rules {
                        self.apply_rule(model, rule, payload, deliver);
                    }
                }
            }
        }
    }

    fn apply_rule(&mut self, model: &Model, rule: RuleId, payload: &Payload, deliver: StepRef) {
        let r = model.rule(rule);
        let controller = r.controller;
        let machine = model.trigger(r.trigger).machine;
        let mut cause = deliver;
        match &r.condition {
            None => {}
            // A condition that evaluates false means the rule does not apply.
            Some(Condition::Expr(condition)) => {
                let none = Payload::new();
                let scope = MapScope { event: payload, self_fields: &none };
                if !eval(condition, &scope, &model.event_types(r.event)) {
                    return;
                }
            }
            // A choice: the rule applies when the label is chosen.
            Some(Condition::Choice(label)) => {
                let options = vec![label.clone(), format!("not {label}")];
                let (ix, chosen, defaulted) = self.decide(&options);
                let at = Participant::Controller(controller);
                cause = self.recorder.push(Some(deliver), RawKind::Choice { at, options, chosen, defaulted });
                if ix != 0 {
                    return;
                }
            }
        }
        let filter = r.target.filter();
        match &r.target {
            Target::One { .. } => {
                let candidates = self.matching(model, machine, r.event, filter, payload);
                match candidates.as_slice() {
                    [] => {
                        self.recorder.push(Some(cause), RawKind::NoTarget { controller, rule });
                    }
                    [one] => self.fire(model, rule, controller, *one, payload, cause),
                    many => {
                        let candidates = many.to_vec();
                        self.recorder.push(Some(cause), RawKind::Ambiguous { controller, rule, candidates });
                    }
                }
            }
            Target::All { .. } => {
                let candidates = self.matching(model, machine, r.event, filter, payload);
                if candidates.is_empty() {
                    self.recorder.push(Some(cause), RawKind::NoTarget { controller, rule });
                }
                for target in candidates {
                    self.fire(model, rule, controller, target, payload, cause);
                }
            }
            Target::Spawn { assignments } => {
                let fields = select::assign(assignments, payload);
                let name = self.spawn_name(model, machine);
                let initial = model.machine(machine).initial;
                let instance = self.add_instance(model, Instance::new(model, name, machine, fields, initial));
                let spawn = self.recorder.push(Some(cause), RawKind::Spawn { controller, instance, rule });
                self.fire(model, rule, controller, instance, payload, spawn);
            }
        }
    }

    fn matching(
        &self,
        model: &Model,
        machine: MachineId,
        event: EventId,
        filter: Option<&Expr>,
        payload: &Payload,
    ) -> Vec<InstanceIx> {
        let types = model.types_for(machine, Some(event));
        self.instances_of(machine)
            .into_iter()
            .filter(|ix| select::matches(&self.instances[ix.0].fields, filter, payload, &types))
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
        let consumers = self.target_consumer(target);
        self.enqueue(label, consumers, QueueItem::Fire { rule, target, payload: payload.clone(), fired, occurrence });
    }
}
