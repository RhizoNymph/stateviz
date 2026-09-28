//! Evaluating target selector clauses against instance fields and the
//! payload of the event being handled.

use cascade_core::definition::{FieldClause, ValueExpr};

use crate::scenario::Payload;

/// The value a clause's right-hand side stands for: `event.x` is the
/// payload's `x` (absent when the payload has no `x`), a literal is itself.
fn value<'a>(expr: &'a ValueExpr, payload: &'a Payload) -> Option<&'a str> {
    match expr {
        ValueExpr::EventField(field) => payload.get(field).map(String::as_str),
        ValueExpr::Literal(literal) => Some(literal),
    }
}

/// Whether an instance with `fields` satisfies every `field == value`
/// predicate. A predicate holds only when both sides are present and equal,
/// so a missing field or payload key never matches. No predicates match
/// every instance.
pub(crate) fn matches(fields: &Payload, predicates: &[FieldClause], payload: &Payload) -> bool {
    predicates.iter().all(|clause| match (fields.get(&clause.field), value(&clause.value, payload)) {
        (Some(have), Some(want)) => have == want,
        _ => false,
    })
}

/// The fields of a spawned instance: each `field = value` assignment whose
/// value is present. Assignments from payload keys the event lacks leave the
/// field unset.
pub(crate) fn assign(assignments: &[FieldClause], payload: &Payload) -> Payload {
    assignments
        .iter()
        .filter_map(|clause| value(&clause.value, payload).map(|v| (clause.field.clone(), v.to_owned())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> Payload {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    fn clause(field: &str, value: ValueExpr) -> FieldClause {
        FieldClause { field: field.to_owned(), value }
    }

    #[test]
    fn event_fields_and_literals() {
        let fields = map(&[("orderId", "1"), ("carrier", "ups")]);
        let payload = map(&[("orderId", "1")]);
        let by_event = clause("orderId", ValueExpr::EventField("orderId".into()));
        let by_literal = clause("carrier", ValueExpr::Literal("ups".into()));
        assert!(matches(&fields, &[by_event.clone(), by_literal.clone()], &payload));
        assert!(!matches(&fields, &[clause("carrier", ValueExpr::Literal("dhl".into()))], &payload));
        assert!(!matches(&map(&[("orderId", "2")]), std::slice::from_ref(&by_event), &payload));
        assert!(matches(&fields, &[], &payload));
    }

    #[test]
    fn missing_sides_never_match() {
        let predicate = clause("orderId", ValueExpr::EventField("orderId".into()));
        assert!(!matches(&map(&[]), std::slice::from_ref(&predicate), &map(&[("orderId", "1")])));
        assert!(!matches(&map(&[("orderId", "1")]), std::slice::from_ref(&predicate), &map(&[])));
        assert!(!matches(&map(&[]), &[predicate], &map(&[])));
    }

    #[test]
    fn spawn_assignments_skip_missing_payload_keys() {
        let assignments = [
            clause("orderId", ValueExpr::EventField("orderId".into())),
            clause("carrier", ValueExpr::Literal("ups".into())),
            clause("note", ValueExpr::EventField("missing".into())),
        ];
        assert_eq!(assign(&assignments, &map(&[("orderId", "7")])), map(&[("orderId", "7"), ("carrier", "ups")]));
    }
}
