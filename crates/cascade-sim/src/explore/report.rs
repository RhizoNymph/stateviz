//! Plain text for hosts that list outcomes (the CLI, the app's Explore
//! panel).

use std::collections::BTreeMap;

use cascade_core::Model;
use cascade_core::ids::StateId;

use super::outcome::{Outcome, OutcomeTimer};
use crate::describe::payload_text;

/// `o1 (Order): paid {orderId: 1}` (no braces when there are no fields).
pub fn instance_text(model: &Model, name: &str, (state, fields): &(StateId, BTreeMap<String, String>)) -> String {
    let s = model.state(*state);
    let machine = &model.machine(s.machine).name;
    if fields.is_empty() {
        format!("{name} ({machine}): {}", s.path)
    } else {
        format!("{name} ({machine}): {} {}", s.path, payload_text(fields))
    }
}

/// `c1: after-30m due in 30m`.
pub fn timer_text(model: &Model, timer: &OutcomeTimer) -> String {
    format!("{}: {} due in {}", timer.instance, model.trigger(timer.trigger).name, timer.remaining)
}

/// The names of the instances whose state or fields differ between two
/// outcomes (or that only one has), in name order.
pub fn differing_instances<'a>(first: &'a Outcome, other: &'a Outcome) -> Vec<&'a str> {
    let mut names: Vec<&str> = other
        .instances
        .iter()
        .filter(|(name, value)| first.instances.get(*name) != Some(value))
        .map(|(name, _)| name.as_str())
        .collect();
    names.extend(first.instances.keys().filter(|n| !other.instances.contains_key(*n)).map(String::as_str));
    names.sort_unstable();
    names.dedup();
    names
}
