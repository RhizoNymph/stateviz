//! Evaluating target filters and guards against instance fields and the
//! payload of the event being handled, and spawn assignments.

use cascade_core::expr::{Assignment, Expr, MapScope, Types, eval, value};

use crate::scenario::Payload;

/// Whether an instance with `fields` passes `filter` for an event carrying
/// `payload`. No filter matches every instance. A test with a missing side
/// is false, so a missing field or payload key never matches.
pub(crate) fn matches(fields: &Payload, filter: Option<&Expr>, payload: &Payload, types: &Types) -> bool {
    filter.is_none_or(|filter| eval(filter, &MapScope { event: payload, self_fields: fields }, types))
}

/// The fields of a spawned instance: each `field = value` assignment whose
/// value is present. Assignments from payload keys the event lacks leave the
/// field unset; there is no instance yet, so `self.x` is never present.
pub(crate) fn assign(assignments: &[Assignment], payload: &Payload) -> Payload {
    let none = Payload::new();
    let scope = MapScope { event: payload, self_fields: &none };
    assignments.iter().filter_map(|a| value(&a.value, &scope).map(|v| (a.field.clone(), v.to_owned()))).collect()
}

#[cfg(test)]
mod tests {
    use cascade_core::expr::{ExprContext, Literal, Operand, parse_expr};

    use super::*;

    fn map(pairs: &[(&str, &str)]) -> Payload {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    fn filter(text: &str) -> Expr {
        match parse_expr(text, ExprContext::Filter) {
            Ok(e) => e,
            Err(err) => panic!("{err}"),
        }
    }

    #[test]
    fn event_fields_and_literals() {
        let types = Types::untyped();
        let fields = map(&[("orderId", "1"), ("carrier", "ups")]);
        let payload = map(&[("orderId", "1")]);
        let both = filter("orderId == event.orderId and carrier == ups");
        assert!(matches(&fields, Some(&both), &payload, &types));
        assert!(!matches(&fields, Some(&filter("carrier == dhl")), &payload, &types));
        assert!(!matches(&map(&[("orderId", "2")]), Some(&filter("orderId == event.orderId")), &payload, &types));
        assert!(matches(&fields, None, &payload, &types));
        // #32: the filter can test the event alone.
        assert!(matches(&fields, Some(&filter("event.orderId == 1")), &payload, &types));
    }

    #[test]
    fn missing_sides_never_match() {
        let types = Types::untyped();
        let predicate = filter("orderId == event.orderId");
        assert!(!matches(&map(&[]), Some(&predicate), &map(&[("orderId", "1")]), &types));
        assert!(!matches(&map(&[("orderId", "1")]), Some(&predicate), &map(&[]), &types));
        assert!(!matches(&map(&[]), Some(&predicate), &map(&[]), &types));
    }

    #[test]
    fn spawn_assignments_skip_missing_payload_keys() {
        let assignments = [
            Assignment::new("orderId", Operand::Event("orderId".into())),
            Assignment::new("carrier", Operand::Lit(Literal::new("ups"))),
            Assignment::new("note", Operand::Event("missing".into())),
        ];
        assert_eq!(assign(&assignments, &map(&[("orderId", "7")])), map(&[("orderId", "7"), ("carrier", "ups")]));
    }
}
