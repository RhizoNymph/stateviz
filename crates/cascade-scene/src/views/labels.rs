//! The words for what a transition says beyond its endpoints: what
//! triggers it (by kind), its subscription filter, its guard and its
//! `sets`. Every view that labels transitions takes its text from here, so
//! pills, arrows, causal edges and traces read the same.
//!
//! | Part | Text | Style |
//! | --- | --- | --- |
//! | Command trigger | `capture` | upright |
//! | Subscription | `on PaymentCaptured`, filter `where orderId == event.orderId` | upright |
//! | Timer | `◷ 30s` | upright |
//! | Expression guard | `[event.amount > 0]` (canonical text) | upright |
//! | Choice guard, `else` | `[?card declined]`, `[else]` | italic |
//! | `sets` | `sets fromAgent, status` (field names only) | upright |
//!
//! A label that carries a choice anywhere (a guard, a rule's `when`) is
//! italic as a whole: labels are single runs of text.

use cascade_core::expr::{Assignment, Condition, Duration, Guard};
use cascade_core::model::{Transition, TriggerKind};
use cascade_core::{Model, TriggerId};

use crate::scene::{CLOCK_GLYPH, FontStyle};

/// `◷ 30s`.
pub(crate) fn clock(delay: Duration) -> String {
    format!("{CLOCK_GLYPH} {delay}")
}

/// What fires a trigger, by kind: the command's name, `on Event`, or the
/// timer's clock and delay.
pub(crate) fn trigger_line(model: &Model, trigger: TriggerId) -> String {
    let t = model.trigger(trigger);
    match t.kind {
        TriggerKind::Command => t.name.clone(),
        TriggerKind::Event(event) => format!("on {}", model.event(event).name),
        TriggerKind::Timer(delay) => clock(delay),
    }
}

/// A subscription's `where` filter in filter-context canonical text.
pub(crate) fn filter_line(transition: &Transition) -> Option<String> {
    transition.filter.as_ref().map(|f| format!("where {}", f.display_filter()))
}

/// Whether a guard is a decision the data does not carry.
pub(crate) fn guard_is_choice(guard: &Guard) -> bool {
    matches!(guard, Guard::Choice(_) | Guard::Else)
}

pub(crate) fn condition_is_choice(condition: &Condition) -> bool {
    matches!(condition, Condition::Choice(_))
}

/// Whether canonical guard or condition text is a choice: `?label` or
/// `else` (an expression's canonical text never starts with `?` and `else`
/// is reserved).
pub(crate) fn text_is_choice(text: &str) -> bool {
    text.starts_with('?') || text == "else"
}

/// The italic style for labels carrying a choice.
pub(crate) fn style_for(choice: bool) -> FontStyle {
    if choice { FontStyle::Italic } else { FontStyle::Normal }
}

/// `[guard]` in canonical text, with its style.
pub(crate) fn guard_label(guard: Option<&Guard>) -> Option<(String, FontStyle)> {
    guard.map(|g| (format!("[{g}]"), style_for(guard_is_choice(g))))
}

/// `sets a, b`: the assigned fields, in order, each once.
pub(crate) fn sets_label(sets: &[Assignment]) -> Option<String> {
    let mut names: Vec<&str> = Vec::with_capacity(sets.len());
    for a in sets {
        if !names.contains(&a.field.as_str()) {
            names.push(&a.field);
        }
    }
    (!names.is_empty()).then(|| format!("sets {}", names.join(", ")))
}

/// A timer trigger's delay, `None` for other kinds.
pub(crate) fn timer_delay(model: &Model, trigger: TriggerId) -> Option<Duration> {
    match model.trigger(trigger).kind {
        TriggerKind::Timer(delay) => Some(delay),
        TriggerKind::Command | TriggerKind::Event(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cascade_core::load_str;

    const MODEL: &str = r#"
machines:
  Order:
    fields: { orderId: string, paidBy: string, tier: string }
    states: [a, b, c]
    transitions:
      - { from: a, to: b, on: go, guard: "?card declined" }
      - { from: a, to: c, on: go, guard: else }
      - from: b
        to: c
        on: { event: Paid, where: "orderId == event.orderId and tier == gold" }
        sets: { paidBy: event.method, tier: gold }
      - { from: c, to: a, after: 90s, guard: "self.tier == gold" }
events:
  Paid:
    payload: [orderId, method, other]
"#;

    fn transition(model: &Model, i: usize) -> &Transition {
        model.transition(model.transition_ids().nth(i).expect("transition"))
    }

    #[test]
    fn labels_by_kind() {
        let model = match load_str(MODEL) {
            Ok(m) => m,
            Err(err) => panic!("{err}"),
        };
        let (go, other, paid, timer) =
            (transition(&model, 0), transition(&model, 1), transition(&model, 2), transition(&model, 3));
        assert_eq!(trigger_line(&model, go.trigger), "go");
        assert_eq!(trigger_line(&model, paid.trigger), "on Paid");
        assert_eq!(trigger_line(&model, timer.trigger), "◷ 90s");
        assert_eq!(filter_line(paid).as_deref(), Some("where orderId == event.orderId and tier == gold"));
        assert_eq!(filter_line(go), None);
        assert_eq!(guard_label(go.guard.as_ref()), Some(("[?card declined]".into(), FontStyle::Italic)));
        assert_eq!(guard_label(other.guard.as_ref()), Some(("[else]".into(), FontStyle::Italic)));
        assert_eq!(guard_label(timer.guard.as_ref()), Some(("[self.tier == gold]".into(), FontStyle::Normal)));
        assert_eq!(sets_label(&paid.sets).as_deref(), Some("sets paidBy, tier"));
        assert_eq!(sets_label(&go.sets), None);
        assert_eq!(timer_delay(&model, timer.trigger), Some(Duration::from_secs(90)));
        assert_eq!(timer_delay(&model, go.trigger), None);
    }

    #[test]
    fn choice_text() {
        assert!(text_is_choice("?risky"));
        assert!(text_is_choice("else"));
        assert!(!text_is_choice("event.x > 0"));
        assert!(!text_is_choice("elsewhere == a"));
    }
}
