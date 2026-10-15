//! What the checks know about when a transition or rule applies: the
//! expressions that must hold, and the types to read them with.
//!
//! - A transition's *data condition* is its subscription `where` filter (if
//!   any) and its `Expr` guard (if any), joined by `and`. `Choice` and
//!   `else` guards are decisions the data does not carry, so they add
//!   nothing here.
//! - An expression's *event part* keeps the top-level conjuncts that read no
//!   `self.x`: what one delivery of an event decides for every instance and
//!   every consumer alike.

use crate::expr::{Expr, Guard, Operand, Types};
use crate::ids::{EventId, TransitionId};
use crate::model::{Model, TriggerKind};

/// The event a transition's `event.x` reads, when it is known statically
/// (a subscription's event). Command and timer triggers read `any`.
pub(super) fn trigger_event(model: &Model, transition: TransitionId) -> Option<EventId> {
    match model.trigger(model.transition(transition).trigger).kind {
        TriggerKind::Event(event) => Some(event),
        TriggerKind::Command | TriggerKind::Timer(_) => None,
    }
}

/// How a transition's guard and filter read their fields.
pub(super) fn transition_types(model: &Model, transition: TransitionId) -> Types {
    model.types_for(model.transition(transition).machine, trigger_event(model, transition))
}

/// `filter and guard` for a transition; `None` when neither is an
/// expression.
pub(super) fn data_condition(model: &Model, transition: TransitionId) -> Option<Expr> {
    let t = model.transition(transition);
    let guard = t.guard.as_ref().and_then(Guard::expr);
    Expr::and(t.filter.iter().chain(guard).cloned().collect())
}

/// The top-level conjuncts of `expr` that read no `self.x`, joined by
/// `and`; `None` when there are none.
pub(super) fn event_part(expr: &Expr) -> Option<Expr> {
    let items: Vec<Expr> = match expr {
        Expr::And(items) => items.iter().filter_map(event_part).collect(),
        Expr::Cmp { .. } | Expr::In { .. } | Expr::Not(_) | Expr::Or(_) => {
            let reads_self = expr.operands().iter().any(|o| matches!(o, Operand::SelfField(_)));
            if reads_self { Vec::new() } else { vec![expr.clone()] }
        }
    };
    Expr::and(items)
}

#[cfg(test)]
mod tests {
    use super::event_part;
    use crate::expr::{ExprContext, parse_expr};

    #[test]
    fn event_part_keeps_conjuncts_without_self() {
        let e = |t: &str| parse_expr(t, ExprContext::Plain).unwrap_or_else(|err| panic!("{t}: {err}"));
        let part = |t: &str| event_part(&e(t)).map(|x| x.to_string());
        assert_eq!(part("self.id == event.id and event.policy == a").as_deref(), Some("event.policy == a"));
        assert_eq!(part("self.id == event.id"), None);
        assert_eq!(part("event.a == x or self.b == y"), None);
        assert_eq!(part("event.a == x or event.b == y").as_deref(), Some("event.a == x or event.b == y"));
        assert_eq!(
            part("(event.a == x and self.c == z) and event.b == y").as_deref(),
            Some("event.a == x and event.b == y")
        );
    }
}
