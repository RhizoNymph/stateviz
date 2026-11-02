//! The play panel's clock: the current time, the armed timers and what the
//! Advance control does with the typed duration.
//!
//! Pure.

use cascade_core::Model;
use cascade_core::expr::{Duration, parse_duration};
use cascade_sim::{ArmedTimer, PlayAction};

/// One armed timer as the panel lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimerRow {
    pub instance: String,
    /// The trigger's name (`after-30m`).
    pub trigger: String,
    pub deadline: Duration,
    /// Time left until the deadline (zero when due).
    pub due_in: Duration,
}

impl TimerRow {
    /// "in 30s", or "due now".
    pub fn due_text(&self) -> String {
        if self.due_in.is_zero() { "due now".to_owned() } else { format!("in {}", self.due_in) }
    }
}

/// The armed timers in deadline order, relative to `now`.
pub fn timer_rows(model: &Model, now: Duration, timers: &[ArmedTimer]) -> Vec<TimerRow> {
    let mut rows: Vec<TimerRow> = timers
        .iter()
        .map(|t| TimerRow {
            instance: t.instance.clone(),
            trigger: model.trigger(t.trigger).name.clone(),
            deadline: t.deadline,
            due_in: t.deadline.saturating_sub(now),
        })
        .collect();
    // Stable: ties keep the session's (arming) order.
    rows.sort_by_key(|r| r.deadline);
    rows
}

/// The Advance control: an empty box advances to the next deadline, a
/// duration advances by it. Advancing by zero is refused.
pub fn advance_action(text: &str) -> Result<PlayAction, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(PlayAction::Advance { by: None });
    }
    let by = parse_duration(text).map_err(|e| format!("Advance by: {}", e.message))?;
    if by.is_zero() {
        return Err("Advance by: give a duration above zero (e.g. 30s, 5m)".to_owned());
    }
    Ok(PlayAction::Advance { by: Some(by) })
}

/// The button label for the typed text.
pub fn advance_label(text: &str) -> String {
    match advance_action(text) {
        Ok(PlayAction::Advance { by: Some(by) }) => format!("Advance {by}"),
        _ => "Advance".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "\
machines:
  Order:
    states: [placed, expired, done]
    transitions:
      - { from: placed, to: expired, after: 30m }
      - { from: placed, to: done, after: 2h }
";

    fn model() -> Model {
        cascade_core::load_str(TEXT).expect("loads")
    }

    fn trigger(model: &Model, name: &str) -> cascade_core::TriggerId {
        model.triggers().find(|(_, t)| t.name == name).map(|(id, _)| id).expect("the trigger exists")
    }

    #[test]
    fn rows_are_relative_to_now_in_deadline_order() {
        let m = model();
        let timers = [
            ArmedTimer {
                instance: "o2".into(),
                trigger: trigger(&m, "after-2h"),
                deadline: Duration::from_secs(7_200 + 60),
            },
            ArmedTimer {
                instance: "o1".into(),
                trigger: trigger(&m, "after-30m"),
                deadline: Duration::from_secs(1_800),
            },
        ];
        let rows = timer_rows(&m, Duration::from_secs(60), &timers);
        assert_eq!(rows.iter().map(|r| r.instance.as_str()).collect::<Vec<_>>(), ["o1", "o2"]);
        assert_eq!(rows[0].trigger, "after-30m");
        assert_eq!(rows[0].due_in, Duration::from_secs(1_740));
        assert_eq!(rows[0].due_text(), "in 29m");
        assert_eq!(rows[1].due_text(), "in 2h");
    }

    #[test]
    fn overdue_timers_are_due_now() {
        let m = model();
        let timers = [ArmedTimer {
            instance: "o1".into(),
            trigger: trigger(&m, "after-30m"),
            deadline: Duration::from_secs(10),
        }];
        let rows = timer_rows(&m, Duration::from_secs(20), &timers);
        assert!(rows[0].due_in.is_zero());
        assert_eq!(rows[0].due_text(), "due now");
        assert!(timer_rows(&m, Duration::ZERO, &[]).is_empty());
    }

    #[test]
    fn advance_reads_the_box() {
        assert_eq!(advance_action(""), Ok(PlayAction::Advance { by: None }));
        assert_eq!(advance_action("  "), Ok(PlayAction::Advance { by: None }));
        assert_eq!(advance_action("90s"), Ok(PlayAction::Advance { by: Some(Duration::from_secs(90)) }));
        assert!(advance_action("0s").unwrap_err().contains("above zero"));
        assert!(advance_action("soon").unwrap_err().starts_with("Advance by:"));
        assert_eq!(advance_label("90s"), "Advance 90s");
        assert_eq!(advance_label("120s"), "Advance 2m");
        assert_eq!(advance_label(""), "Advance");
        assert_eq!(advance_label("bad"), "Advance");
    }
}
