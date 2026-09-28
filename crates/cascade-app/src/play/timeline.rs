//! The timeline strip: one chip per recorded action, the current position,
//! and the saved branches.

use cascade_sim::{PlayAction, Timeline};

/// Short text for an action chip.
pub fn action_label(action: &PlayAction) -> String {
    match action {
        PlayAction::AddInstance { name, machine, .. } => match name {
            Some(name) => format!("+ {machine} {name}"),
            None => format!("+ {machine}"),
        },
        PlayAction::RemoveInstance { name } => format!("− {name}"),
        PlayAction::Fire { source, trigger, target, .. } => format!("{source}: {} → {target}", trigger.trigger),
        PlayAction::Step { choice: None } => "step".to_owned(),
        PlayAction::Step { choice: Some(n) } => format!("deliver #{}", n + 1),
        PlayAction::RunUntilQuiet => "run until quiet".to_owned(),
    }
}

/// One chip. Clicking it seeks to `position`: the timeline with this action
/// applied (0 is the start, before any action).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    pub label: String,
    pub position: usize,
    /// At or before the current position.
    pub applied: bool,
    /// This is the current position.
    pub current: bool,
}

/// The start chip, then one chip per action.
pub fn chips(timeline: &Timeline) -> Vec<Chip> {
    let position = timeline.position.min(timeline.actions.len());
    let start = Chip { label: "start".to_owned(), position: 0, applied: true, current: position == 0 };
    std::iter::once(start)
        .chain(timeline.actions.iter().enumerate().map(|(i, action)| Chip {
            label: action_label(action),
            position: i + 1,
            applied: i < position,
            current: i + 1 == position,
        }))
        .collect()
}

/// Whether acting now forks a branch (the position is before the end).
pub fn acting_forks(timeline: &Timeline) -> bool {
    timeline.position < timeline.actions.len()
}

/// "Branch 1: from step 2, 3 actions (step, run until quiet, …)".
pub fn branch_label(index: usize, branch: &cascade_sim::Branch) -> String {
    let preview: Vec<String> = branch.actions.iter().take(2).map(action_label).collect();
    let more = if branch.actions.len() > 2 { ", …" } else { "" };
    format!(
        "Branch {}: from position {}, {} action{} ({}{more})",
        index + 1,
        branch.fork,
        branch.actions.len(),
        if branch.actions.len() == 1 { "" } else { "s" },
        preview.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use cascade_core::definition::TriggerRef;
    use cascade_sim::Branch;

    use super::*;

    fn fire() -> PlayAction {
        PlayAction::Fire {
            source: "User".into(),
            trigger: TriggerRef { machine: "Order".into(), trigger: "pay".into() },
            target: "o1".into(),
            payload: BTreeMap::new(),
        }
    }

    fn add() -> PlayAction {
        PlayAction::AddInstance {
            name: Some("o1".into()),
            machine: "Order".into(),
            fields: BTreeMap::new(),
            state: None,
        }
    }

    #[test]
    fn labels() {
        assert_eq!(action_label(&add()), "+ Order o1");
        assert_eq!(
            action_label(&PlayAction::AddInstance {
                name: None,
                machine: "Order".into(),
                fields: BTreeMap::new(),
                state: None
            }),
            "+ Order"
        );
        assert_eq!(action_label(&PlayAction::RemoveInstance { name: "o1".into() }), "− o1");
        assert_eq!(action_label(&fire()), "User: pay → o1");
        assert_eq!(action_label(&PlayAction::Step { choice: None }), "step");
        assert_eq!(action_label(&PlayAction::Step { choice: Some(2) }), "deliver #3");
        assert_eq!(action_label(&PlayAction::RunUntilQuiet), "run until quiet");
    }

    #[test]
    fn chips_mark_applied_and_current() {
        let timeline =
            Timeline { actions: vec![add(), fire(), PlayAction::RunUntilQuiet], position: 2, branches: vec![] };
        let chips = chips(&timeline);
        assert_eq!(chips.len(), 4);
        assert_eq!(chips.iter().map(|c| c.position).collect::<Vec<_>>(), [0, 1, 2, 3]);
        assert_eq!(chips.iter().map(|c| c.applied).collect::<Vec<_>>(), [true, true, true, false]);
        assert_eq!(chips.iter().map(|c| c.current).collect::<Vec<_>>(), [false, false, true, false]);
        assert!(acting_forks(&timeline));
    }

    #[test]
    fn an_empty_timeline_is_at_the_start() {
        let timeline = Timeline::default();
        let chips = chips(&timeline);
        assert_eq!(chips, [Chip { label: "start".into(), position: 0, applied: true, current: true }]);
        assert!(!acting_forks(&timeline));
    }

    #[test]
    fn a_position_past_the_end_is_clamped() {
        let timeline = Timeline { actions: vec![add()], position: 7, branches: vec![] };
        assert!(chips(&timeline)[1].current);
        assert!(!acting_forks(&timeline));
    }

    #[test]
    fn branch_labels() {
        let branch = Branch { fork: 1, actions: vec![fire(), PlayAction::RunUntilQuiet, fire()] };
        assert_eq!(
            branch_label(0, &branch),
            "Branch 1: from position 1, 3 actions (User: pay → o1, run until quiet, …)"
        );
        let one = Branch { fork: 0, actions: vec![add()] };
        assert_eq!(branch_label(1, &one), "Branch 2: from position 0, 1 action (+ Order o1)");
    }
}
