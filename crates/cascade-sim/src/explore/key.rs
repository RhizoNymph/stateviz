//! The search key: a canonical text of everything that decides a search
//! state's future (the simulator state as in `PlaySession::state_key`, plus
//! the next input and the anomalies met so far).
//!
//! With reduction, queue orders the ordering model cannot tell apart share
//! a key: under [`OrderingModel::PerConsumer`] only the relative order of
//! items sharing a channel (an event consumer, or a controller's fires at
//! one instance) matters, so the queue is written as the canonical
//! linearization of that partial order (repeatedly the smallest item, by
//! text, with no earlier item in any of its channels); under
//! [`OrderingModel::Unordered`] the queue is a multiset. Armed timers are
//! written sorted (their arming order only orders timer items, which are
//! never constrained). Without reduction the key keeps the exact order.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt::Write as _;

use cascade_core::Model;

use super::moves::{Channel, SearchState, channels};
use super::options::OrderingModel;
use crate::engine::core::{Core, QueueItem, Queued};

/// A queue item's content: what it delivers to whom, without ids or
/// positions.
pub(crate) fn item_text(model: &Model, core: &Core, queued: &Queued) -> String {
    let consumers: Vec<&str> = queued.consumers_left.iter().map(|c| c.name.as_str()).collect();
    match &queued.item {
        QueueItem::Event { event, payload, .. } => {
            format!("event {} {payload:?} {consumers:?}", model.event(*event).name)
        }
        QueueItem::Fire { rule, target, payload, .. } => {
            format!("fire {} {} {payload:?}", rule.index(), core.instance(*target).name)
        }
        QueueItem::Timer { trigger, target, .. } => {
            format!("timer {} {}", trigger.index(), core.instance(*target).name)
        }
    }
}

pub(crate) fn search_key(model: &Model, state: &SearchState, ordering: OrderingModel, reduction: bool) -> String {
    let core = &state.core;
    let mut key = String::new();
    for instance in core.instances() {
        if instance.removed {
            let _ = write!(key, "removed {};", instance.name);
            continue;
        }
        let _ = write!(
            key,
            "instance {} {:?} {:?} {:?} {:?};",
            instance.name,
            instance.machine,
            instance.leaf(),
            instance.fields,
            instance.history_entries()
        );
    }
    for item in queue_order(model, core, ordering, reduction) {
        let _ = write!(key, "{item};");
    }
    let mut armed: Vec<String> = core
        .armed()
        .iter()
        .map(|a| {
            format!(
                "armed {} {:?} {:?} +{}",
                core.instance(a.instance).name,
                a.trigger,
                a.state,
                a.deadline.saturating_sub(core.now()).millis()
            )
        })
        .collect();
    if reduction {
        armed.sort();
    }
    for a in armed {
        let _ = write!(key, "{a};");
    }
    for decision in core.decisions() {
        let _ = write!(key, "decision {decision:?};");
    }
    let _ = write!(key, "input {};", state.input);
    for anomaly in &state.anomalies {
        let _ = write!(key, "anomaly {anomaly:?};");
    }
    key
}

/// The queue's item texts, exactly or as the canonical linearization.
fn queue_order(model: &Model, core: &Core, ordering: OrderingModel, reduction: bool) -> Vec<String> {
    let texts: Vec<String> = core.queue().iter().map(|q| item_text(model, core, q)).collect();
    if !reduction {
        return texts;
    }
    let chans: Vec<_> = match ordering {
        OrderingModel::PerConsumer => core.queue().iter().map(|q| channels(model, q)).collect(),
        OrderingModel::Unordered => vec![Vec::new(); texts.len()],
    };
    // Each channel's items in queue order; an item is ready when it heads
    // every channel it is in.
    let mut lanes: HashMap<Channel, VecDeque<usize>> = HashMap::new();
    for (i, cs) in chans.iter().enumerate() {
        for c in cs {
            lanes.entry(*c).or_default().push_back(i);
        }
    }
    let is_ready = |i: usize, lanes: &HashMap<Channel, VecDeque<usize>>| {
        chans[i].iter().all(|c| lanes.get(c).and_then(|l| l.front()) == Some(&i))
    };
    let mut ready: BTreeSet<(&str, usize)> =
        (0..texts.len()).filter(|&i| is_ready(i, &lanes)).map(|i| (texts[i].as_str(), i)).collect();
    let mut out = Vec::with_capacity(texts.len());
    while let Some((text, i)) = ready.pop_first() {
        out.push(text.to_owned());
        for c in &chans[i] {
            if let Some(lane) = lanes.get_mut(c) {
                lane.pop_front();
                if let Some(&next) = lane.front()
                    && is_ready(next, &lanes)
                {
                    ready.insert((texts[next].as_str(), next));
                }
            }
        }
    }
    out
}
