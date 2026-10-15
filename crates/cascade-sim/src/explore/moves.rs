//! Moves: what can happen next in a state under an ordering model, what
//! each move may touch (for independence), and performing one.
//!
//! A move is a *base* (deliver a queue item to one consumer, apply the
//! next scenario input, or expire the earliest armed timers) plus the
//! decisions pushed just before it. Without `choices` the decision list is
//! empty and choice points take pending decisions or defaults; with
//! `choices` a base is performed once per combination of options at the
//! choice points it hits (each pushed as a `Choose` right before it).

use std::collections::{BTreeSet, HashSet};

use cascade_core::Model;
use cascade_core::ids::TriggerId;
use cascade_core::model::Target;

use super::inputs::{Input, Inputs};
use super::options::{ExploreOptions, OrderingModel};
use super::outcome::Anomaly;
use crate::engine::core::{ConsumerRef, Core, QueueItem, Queued};
use crate::engine::exec::{Schedule, exec};
use crate::engine::instance::InstanceIx;
use crate::engine::record::RawKind;
use crate::error::SimError;
use crate::scenario::StepTiming;
use crate::session::{PendingId, PlayAction};

/// Everything the search needs to know about a point of play.
#[derive(Clone, Debug)]
pub(crate) struct SearchState {
    pub core: Core,
    /// The next scenario input.
    pub input: usize,
    /// Anomalies met on the way here (part of the search key: two paths to
    /// the same simulator state with different anomalies are different
    /// search states).
    pub anomalies: BTreeSet<Anomaly>,
}

/// A consumer as a channel: events to one consumer keep emission order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Channel {
    Consumer(ConsumerRef),
    /// Fires from one controller to one instance keep their order.
    Fires(cascade_core::ids::ControllerId, InstanceIx),
}

/// The channels a queue item is ordered in under [`OrderingModel::PerConsumer`]:
/// an event in each remaining consumer's, a fire in its (controller,
/// target) pair's; a timer in none.
pub(crate) fn channels(model: &Model, queued: &Queued) -> Vec<Channel> {
    match &queued.item {
        QueueItem::Event { .. } => queued.consumers_left.iter().map(|c| Channel::Consumer(c.target)).collect(),
        QueueItem::Fire { rule, target, .. } => vec![Channel::Fires(model.rule(*rule).controller, *target)],
        QueueItem::Timer { .. } => vec![],
    }
}

/// What a move does, independent of decisions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Base {
    /// Deliver the item at `position` to `consumer` (its only remaining
    /// consumer, or a fire's or timer's target, when `None`).
    Deliver { position: usize, id: PendingId, consumer: Option<String> },
    /// Apply scenario input `index`.
    Input(usize),
    /// Expire the earliest armed timers (`Advance { by: None }`).
    Advance,
}

/// A move's identity along one line of play (pending ids are stable while
/// queued), for sleep sets.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum LineId {
    Deliver { id: PendingId, consumer: Option<String>, decisions: Vec<String> },
    Input { index: usize, decisions: Vec<String> },
    Advance,
}

/// A move's identity by content (what is delivered to whom), the same for
/// equivalent states reached along different lines: stored sleep sets and
/// witness edges use it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum MoveDesc {
    Deliver {
        item: String,
        consumer: Option<String>,
        decisions: Vec<String>,
    },
    Input {
        index: usize,
        decisions: Vec<String>,
    },
    Advance,
    /// The next delivery would exceed the step limit: stop here.
    Halt,
}

/// What a move may read and change, conservatively.
#[derive(Clone, Debug, Default)]
pub(crate) struct Footprint {
    /// Dependent with every other move (inputs, clock, spawns, pending
    /// decisions).
    pub global: bool,
    pub consumer: Option<ConsumerRef>,
    pub writes: Vec<InstanceIx>,
    pub reads: Vec<InstanceIx>,
    /// Consumers of the events it may emit.
    pub emits_to: Vec<ConsumerRef>,
}

/// Whether two moves enabled in the same state commute: performing them in
/// either order reaches the same search state, and neither disables the
/// other. When unsure, they are dependent.
pub(crate) fn independent(ordering: OrderingModel, a: &Footprint, b: &Footprint) -> bool {
    if a.global || b.global {
        return false;
    }
    if a.consumer.is_some() && a.consumer == b.consumer {
        return false;
    }
    let meets = |x: &[InstanceIx], y: &[InstanceIx]| x.iter().any(|i| y.contains(i));
    if meets(&a.writes, &b.writes) || meets(&a.writes, &b.reads) || meets(&b.writes, &a.reads) {
        return false;
    }
    // Two emissions to one consumer land in its channel in the order the
    // moves ran; unordered queues don't care.
    ordering == OrderingModel::Unordered || !a.emits_to.iter().any(|c| b.emits_to.contains(c))
}

/// A base move enabled in a state, with what it touches.
#[derive(Clone, Debug)]
pub(crate) struct Enabled {
    pub base: Base,
    pub footprint: Footprint,
    /// The delivered item's content text (see `key::item_text`), for
    /// [`MoveDesc`].
    pub item: Option<String>,
}

/// The base moves enabled in `state`, in a fixed order: queue items by
/// position (an event's consumers in delivery order), then the next input,
/// then advancing the clock.
pub(crate) fn enabled(model: &Model, state: &SearchState, inputs: &Inputs, options: &ExploreOptions) -> Vec<Enabled> {
    let core = &state.core;
    let queue = core.queue();
    let deciding = !core.decisions().is_empty();
    let mut out = Vec::new();
    let mut seen_items: HashSet<(String, Option<String>)> = HashSet::new();
    // Channels some earlier item is still queued in.
    let mut busy: HashSet<Channel> = HashSet::new();
    let per_consumer = options.ordering == OrderingModel::PerConsumer;
    for (position, queued) in queue.iter().enumerate() {
        let item_channels = channels(model, queued);
        let blocked = |c: &Channel| per_consumer && busy.contains(c);
        let item = super::key::item_text(model, core, queued);
        let mut push = |consumer: Option<String>, footprint: Footprint, out: &mut Vec<Enabled>| {
            // Identical items lead to identical states; offer one.
            let identity = (item.clone(), consumer.clone());
            if !seen_items.insert(identity) {
                return;
            }
            let mut footprint = footprint;
            footprint.global |= deciding;
            out.push(Enabled {
                base: Base::Deliver { position, id: queued.id, consumer },
                footprint,
                item: Some(item.clone()),
            });
        };
        match &queued.item {
            QueueItem::Event { event, .. } => {
                if queued.consumers_left.is_empty() {
                    push(None, Footprint::default(), &mut out);
                    continue;
                }
                let only = queued.consumers_left.len() == 1;
                for consumer in &queued.consumers_left {
                    if blocked(&Channel::Consumer(consumer.target)) {
                        continue;
                    }
                    let footprint = match consumer.target {
                        ConsumerRef::Instance(ix) => {
                            match model.subscription_trigger(core.instance(ix).machine, *event) {
                                Some(trigger) => instance_footprint(model, core, ix, trigger),
                                None => Footprint::default(),
                            }
                        }
                        ConsumerRef::Controller(controller) => controller_footprint(model, core, controller, *event),
                    };
                    let footprint = Footprint { consumer: Some(consumer.target), ..footprint };
                    push((!only).then(|| consumer.name.clone()), footprint, &mut out);
                }
            }
            QueueItem::Fire { rule, target, .. } => {
                if !blocked(&Channel::Fires(model.rule(*rule).controller, *target)) {
                    let footprint = Footprint {
                        consumer: Some(ConsumerRef::Instance(*target)),
                        ..instance_footprint(model, core, *target, model.rule(*rule).trigger)
                    };
                    push(None, footprint, &mut out);
                }
            }
            QueueItem::Timer { trigger, target, .. } => {
                let footprint = Footprint {
                    consumer: Some(ConsumerRef::Instance(*target)),
                    ..instance_footprint(model, core, *target, *trigger)
                };
                push(None, footprint, &mut out);
            }
        }
        busy.extend(item_channels);
    }
    if let Some(input) = inputs.inputs.get(state.input)
        && (input.timing == StepTiming::Immediate || queue.is_empty())
    {
        let global = Footprint { global: true, ..Footprint::default() };
        out.push(Enabled { base: Base::Input(state.input), footprint: global, item: None });
    }
    if options.timers_anytime && !core.armed().is_empty() {
        let global = Footprint { global: true, ..Footprint::default() };
        out.push(Enabled { base: Base::Advance, footprint: global, item: None });
    }
    out
}

/// Offering `trigger` to `ix`: reads and changes the instance, and may emit
/// the events of any transition enabled from its leaf.
fn instance_footprint(model: &Model, core: &Core, ix: InstanceIx, trigger: TriggerId) -> Footprint {
    let instance = core.instance(ix);
    let mut emits_to = Vec::new();
    for t in model.enabled_transitions(instance.leaf(), trigger) {
        for emit in &model.transition(t).emits {
            for consumer in core.event_consumers(model, emit.event) {
                if !emits_to.contains(&consumer.target) {
                    emits_to.push(consumer.target);
                }
            }
        }
    }
    Footprint { global: false, consumer: None, writes: vec![ix], reads: vec![ix], emits_to }
}

/// A controller handling `event`: its rules read every live instance of the
/// machines they target; a spawn changes the instance set (global).
fn controller_footprint(
    model: &Model,
    core: &Core,
    controller: cascade_core::ids::ControllerId,
    event: cascade_core::ids::EventId,
) -> Footprint {
    let mut footprint = Footprint::default();
    for &handler in &model.event(event).handlers {
        let h = model.handler(handler);
        if h.controller != controller {
            continue;
        }
        for &rule in &h.rules {
            let r = model.rule(rule);
            if matches!(r.target, Target::Spawn { .. }) {
                footprint.global = true;
            }
            for ix in core.instances_of(model.trigger(r.trigger).machine) {
                if !footprint.reads.contains(&ix) {
                    footprint.reads.push(ix);
                }
            }
        }
    }
    footprint
}

/// A performed move.
#[derive(Clone, Debug)]
pub(crate) struct Performed {
    pub state: SearchState,
    /// The play actions it took, decisions first.
    pub actions: Vec<PlayAction>,
    /// The options of the first choice point that took its default (with
    /// `choices`, the move branches there).
    pub defaulted: Option<Vec<String>>,
}

/// The result of performing a move.
#[derive(Clone, Debug)]
pub(crate) enum Step {
    Done(Box<Performed>),
    /// The step limit: the move cannot be performed.
    Halt,
}

/// Perform `base` with `decisions` pushed first.
pub(crate) fn perform(
    model: &Model,
    state: &SearchState,
    inputs: &Inputs,
    base: &Base,
    decisions: &[String],
) -> Result<Step, SimError> {
    let mut next = state.clone();
    let from = next.core.step_count();
    let mut actions = Vec::new();
    for option in decisions {
        let action = PlayAction::Choose { option: option.clone() };
        actions.push(exec(&mut next.core, model, &action, &mut Schedule::Fifo)?);
    }
    let outcome = match base {
        Base::Deliver { position, consumer, .. } => {
            let choice = if *position == 0 && consumer.is_none() { None } else { Some(to_u32(*position)) };
            let action = PlayAction::Step { choice, consumer: consumer.clone() };
            exec(&mut next.core, model, &action, &mut Schedule::Fifo).map(|recorded| actions.push(recorded))
        }
        Base::Advance => {
            let action = PlayAction::Advance { by: None };
            exec(&mut next.core, model, &action, &mut Schedule::Fifo).map(|recorded| actions.push(recorded))
        }
        Base::Input(index) => match inputs.inputs.get(*index) {
            Some(input) => {
                apply_input(model, &mut next, input, &mut actions);
                next.input = index + 1;
                Ok(())
            }
            None => Ok(()),
        },
    };
    match outcome {
        Ok(()) => {}
        Err(SimError::StepLimit(_)) => return Ok(Step::Halt),
        Err(err) => return Err(err),
    }
    let mut defaulted = None;
    for kind in next.core.kinds_since(from) {
        if let Some(anomaly) = anomaly_of(model, &next.core, kind) {
            next.anomalies.insert(anomaly);
        }
        if let RawKind::Choice { options, defaulted: true, .. } = kind
            && defaulted.is_none()
        {
            defaulted = Some(options.clone());
        }
    }
    next.core.forget_trace();
    Ok(Step::Done(Box::new(Performed { state: next, actions, defaulted })))
}

/// Apply an input and its chained directives. An action that fails in this
/// ordering is skipped and recorded as [`Anomaly::InputFailed`].
fn apply_input(model: &Model, state: &mut SearchState, input: &Input, actions: &mut Vec<PlayAction>) {
    let first = input.action(model, &state.core);
    let mut apply = |state: &mut SearchState, action: Result<PlayAction, SimError>, text: String| {
        let result = action.and_then(|action| exec(&mut state.core, model, &action, &mut Schedule::Fifo));
        match result {
            Ok(recorded) => actions.push(recorded),
            Err(err) => {
                tracing::debug!(input = %text, error = %err, "exploration skipped an input");
                state.anomalies.insert(Anomaly::InputFailed { input: text, error: error_text(&err) });
            }
        }
    };
    apply(state, first, input.text(model));
    for action in &input.chained {
        apply(state, Ok(action.clone()), action_text(action));
    }
}

/// An error without its source position (inputs fail per ordering, not
/// per line).
fn error_text(err: &SimError) -> String {
    match err {
        SimError::Action(kind) => kind.to_string(),
        SimError::Scenario(err) => err.diagnostics.iter().map(|d| d.kind.to_string()).collect::<Vec<_>>().join("; "),
        other => other.to_string(),
    }
}

fn action_text(action: &PlayAction) -> String {
    match action {
        PlayAction::AddInstance { name, machine, .. } => {
            format!("create {} ({machine})", name.as_deref().unwrap_or("?"))
        }
        PlayAction::RemoveInstance { name } => format!("remove {name}"),
        PlayAction::Choose { option } => format!("choose {option}"),
        other => format!("{other:?}"),
    }
}

/// The anomaly a recorded step reports, if any.
pub(crate) fn anomaly_of(model: &Model, core: &Core, kind: &RawKind) -> Option<Anomaly> {
    let name = |ix: InstanceIx| core.instance(ix).name.clone();
    let rule_label = |rule| model.label_of(cascade_core::ElementRef::Rule(rule));
    match kind {
        RawKind::NoTarget { rule, .. } => Some(Anomaly::NoTarget { rule: rule_label(*rule) }),
        RawKind::Ambiguous { rule, candidates, .. } => Some(Anomaly::Ambiguous {
            rule: rule_label(*rule),
            candidates: candidates.iter().map(|&c| name(c)).collect(),
        }),
        RawKind::Dropped { instance, trigger, state } => Some(Anomaly::Dropped {
            instance: name(*instance),
            trigger: model.trigger(*trigger).name.clone(),
            state: model.state(*state).path.clone(),
        }),
        _ => None,
    }
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}
