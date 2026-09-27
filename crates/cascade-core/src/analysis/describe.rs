//! Wording shared by the checks' one-line messages.

use crate::ids::{RuleId, StateId, TriggerId};
use crate::key::ElementRef;
use crate::model::Model;

/// `Shipment.start`.
pub(super) fn trigger(model: &Model, trigger: TriggerId) -> String {
    model.label_of(ElementRef::Trigger(trigger))
}

/// `Order.placed.paid`.
pub(super) fn state(model: &Model, state: StateId) -> String {
    model.label_of(ElementRef::State(state))
}

/// `Fulfillment fires Shipment.start on OrderPaid`.
pub(super) fn rule(model: &Model, rule: RuleId) -> String {
    let r = model.rule(rule);
    format!(
        "{} fires {} on {}",
        model.controller(r.controller).name,
        trigger(model, r.trigger),
        model.event(r.event).name
    )
}

/// `a`, `a and b`, `a, b and c`.
pub(super) fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// `1 transition`, `2 transitions`.
pub(super) fn count(n: usize, noun: &str) -> String {
    if n == 1 { format!("1 {noun}") } else { format!("{n} {noun}s") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_read_naturally() {
        let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        assert_eq!(list(&s(&[])), "");
        assert_eq!(list(&s(&["a"])), "a");
        assert_eq!(list(&s(&["a", "b"])), "a and b");
        assert_eq!(list(&s(&["a", "b", "c"])), "a, b and c");
    }

    #[test]
    fn counts_pluralize() {
        assert_eq!(count(1, "state"), "1 state");
        assert_eq!(count(0, "state"), "0 states");
        assert_eq!(count(3, "state"), "3 states");
    }
}
