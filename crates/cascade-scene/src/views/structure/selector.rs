//! Fire labels on the build canvas: short, and said once.
//!
//! A fire's label says how its rule picks the target instance and, in
//! brackets, its `when` condition. The pill it points at already names the
//! machine and trigger, so the label drops those, and the common
//! correlation `f == event.f` shrinks to `by f`:
//!
//! | Selector | Label |
//! | --- | --- |
//! | `Payment` (the one instance) | none |
//! | `Payment where orderId == event.orderId` | `by orderId` |
//! | `Payment where orderId == event.id and region == eu` | `by orderId=event.id, region=eu` |
//! | `all Payment` | `all` |
//! | `all Payment where orderId == event.orderId` | `all by orderId` |
//! | `new Payment` | `new` |
//! | `new Payment with orderId = event.orderId` | `new with orderId` |
//!
//! The full selector stays one click away: the edge's hit target is the
//! rule, which the inspector shows in full.
//!
//! Saying it once: a drawn fire can stand for several rules (merged per
//! controller and pill); its label lists each distinct selector once and
//! each distinct condition once. Among one controller's fires into one
//! machine, a label identical to one already shown is left off: the
//! parallel wires leave the controller together, and one label covers them.

use std::collections::HashSet;

use cascade_core::definition::{FieldClause, ValueExpr};
use cascade_core::model::Target;

/// What one rule contributes to its fire's label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RuleLabel {
    pub selector: Option<String>,
    pub condition: Option<String>,
}

impl RuleLabel {
    pub fn new(target: &Target, condition: Option<&str>) -> Self {
        Self { selector: short_selector(target), condition: condition.map(str::to_owned) }
    }
}

/// `f` for `f == event.f`, else `f=value`.
fn clause(c: &FieldClause) -> String {
    match &c.value {
        ValueExpr::EventField(field) if *field == c.field => c.field.clone(),
        value => format!("{}={value}", c.field),
    }
}

fn clauses(cs: &[FieldClause]) -> String {
    cs.iter().map(clause).collect::<Vec<_>>().join(", ")
}

/// The shortened selector (see the module docs); `None` for the one
/// instance of a singleton machine, which needs no saying.
pub(super) fn short_selector(target: &Target) -> Option<String> {
    match target {
        Target::One { predicates } if predicates.is_empty() => None,
        Target::One { predicates } => Some(format!("by {}", clauses(predicates))),
        Target::All { predicates } if predicates.is_empty() => Some("all".to_owned()),
        Target::All { predicates } => Some(format!("all by {}", clauses(predicates))),
        Target::Spawn { assignments } if assignments.is_empty() => Some("new".to_owned()),
        Target::Spawn { assignments } => {
            let fields: Vec<String> = assignments.iter().map(clause).collect();
            Some(format!("new with {}", fields.join(", ")))
        }
    }
}

/// The label of a fire standing for `rules`: distinct selectors, then
/// distinct conditions in brackets. `None` when there is nothing to say.
pub(super) fn merged(rules: &[RuleLabel]) -> Option<String> {
    let mut selectors: Vec<&str> = Vec::new();
    let mut conditions: Vec<&str> = Vec::new();
    for r in rules {
        if let Some(s) = r.selector.as_deref()
            && !selectors.contains(&s)
        {
            selectors.push(s);
        }
        if let Some(c) = r.condition.as_deref()
            && !conditions.contains(&c)
        {
            conditions.push(c);
        }
    }
    let mut parts: Vec<String> = Vec::new();
    if !selectors.is_empty() {
        parts.push(selectors.join(", "));
    }
    parts.extend(conditions.iter().map(|c| format!("[{c}]")));
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Leave off labels repeated among parallel fires: `fires` are (group, label)
/// pairs in drawing order, where a group is one controller's fires into one
/// machine. Returns which labels to show.
pub(super) fn shown<G: Eq + std::hash::Hash>(fires: &[(G, Option<&str>)]) -> Vec<bool> {
    let mut seen: HashSet<(&G, &str)> = HashSet::new();
    fires.iter().map(|(g, label)| label.is_some_and(|l| seen.insert((g, l)))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eq(field: &str, event_field: &str) -> FieldClause {
        FieldClause { field: field.to_owned(), value: ValueExpr::EventField(event_field.to_owned()) }
    }

    fn lit(field: &str, value: &str) -> FieldClause {
        FieldClause { field: field.to_owned(), value: ValueExpr::Literal(value.to_owned()) }
    }

    #[test]
    fn selectors_shorten() {
        let one = |p: Vec<FieldClause>| short_selector(&Target::One { predicates: p });
        assert_eq!(one(vec![]), None, "the singleton needs no label");
        assert_eq!(one(vec![eq("orderId", "orderId")]).as_deref(), Some("by orderId"));
        assert_eq!(
            one(vec![eq("orderId", "id"), lit("region", "eu")]).as_deref(),
            Some("by orderId=event.id, region=eu")
        );
        let all = |p: Vec<FieldClause>| short_selector(&Target::All { predicates: p });
        assert_eq!(all(vec![]).as_deref(), Some("all"));
        assert_eq!(all(vec![eq("orderId", "orderId")]).as_deref(), Some("all by orderId"));
        let new = |a: Vec<FieldClause>| short_selector(&Target::Spawn { assignments: a });
        assert_eq!(new(vec![]).as_deref(), Some("new"));
        assert_eq!(new(vec![eq("orderId", "orderId")]).as_deref(), Some("new with orderId"));
        assert_eq!(new(vec![lit("tier", "gold")]).as_deref(), Some("new with tier=gold"));
    }

    #[test]
    fn merged_rules_say_each_selector_and_condition_once() {
        let by = RuleLabel { selector: Some("by orderId".into()), condition: None };
        let when =
            RuleLabel { selector: Some("by orderId".into()), condition: Some("parcel not yet delivered".into()) };
        assert_eq!(merged(&[by.clone(), when.clone()]).as_deref(), Some("by orderId [parcel not yet delivered]"));
        assert_eq!(merged(&[by.clone(), by.clone()]).as_deref(), Some("by orderId"));
        let singleton = RuleLabel { selector: None, condition: None };
        assert_eq!(merged(std::slice::from_ref(&singleton)), None, "nothing to say");
        let gated = RuleLabel { selector: None, condition: Some("risky".into()) };
        assert_eq!(merged(&[singleton, gated]).as_deref(), Some("[risky]"));
        let all = RuleLabel { selector: Some("all".into()), condition: None };
        assert_eq!(merged(&[by, all]).as_deref(), Some("by orderId, all"));
    }

    #[test]
    fn parallel_fires_show_a_repeated_label_once() {
        let fires = [
            ("Orders→Order", Some("by orderId")),
            ("Orders→Order", Some("by orderId")),
            ("Orders→Order", Some("by orderId [late]")),
            ("Billing→Order", Some("by orderId")),
            ("Orders→Order", None),
        ];
        assert_eq!(shown(&fires), [true, false, true, true, false]);
    }
}
