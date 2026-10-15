//! The choice prompt: before a queue item is delivered, ask for every
//! decision the delivery needs that no pending decision already covers,
//! then push one `Choose` per answer and the step.
//!
//! ```text
//! begin(request) ──▶ Go([Step])                       no undecided choice point
//!                └─▶ Ask(prompt for point 1)
//! pick(prompt, option) ──▶ Ask(prompt for the next undecided point)
//!                      └─▶ Go([Choose a, Choose b, …, Step])
//! ```
//!
//! Pure. The choice points come from a [`ChoiceOracle`]: the session's
//! `choice_points` with the answers so far pushed as pending decisions (on
//! a copy of the session, so nothing is recorded until the user is done).
//! A point is decided when a pending decision is one of its options; each
//! decision is used once, oldest first, as the simulator takes them.

use cascade_core::Model;
use cascade_sim::{ChoicePoint, PlayAction, PlaySession};

/// A queue delivery: `Step { choice, consumer }`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StepRequest {
    /// The queue position; `None` for the head.
    pub choice: Option<u32>,
    /// One consumer only; `None` for all that are left.
    pub consumer: Option<String>,
}

impl StepRequest {
    pub fn action(&self) -> PlayAction {
        PlayAction::Step { choice: self.choice, consumer: self.consumer.clone() }
    }
}

/// Choice points and pending decisions as they would be after pushing
/// `answers`.
pub trait ChoiceOracle {
    /// `(choice points the step hits, pending decisions)` with `answers`
    /// pushed as decisions after the current ones.
    fn after(&self, answers: &[String], request: &StepRequest) -> Result<(Vec<ChoicePoint>, Vec<String>), String>;
}

/// The live oracle: the session itself, or a copy with the answers pushed.
pub struct SessionOracle<'a> {
    pub session: &'a PlaySession,
    pub model: &'a Model,
}

impl ChoiceOracle for SessionOracle<'_> {
    fn after(&self, answers: &[String], request: &StepRequest) -> Result<(Vec<ChoicePoint>, Vec<String>), String> {
        let points = |session: &PlaySession| {
            session
                .choice_points(self.model, request.choice, request.consumer.as_deref())
                .map(|points| (points, session.pending_decisions().to_vec()))
                .map_err(|e| e.to_string())
        };
        if answers.is_empty() {
            return points(self.session);
        }
        let mut copy = self.session.clone();
        for option in answers {
            copy.apply(self.model, PlayAction::Choose { option: option.clone() }).map_err(|e| e.to_string())?;
        }
        points(&copy)
    }
}

/// The prompt shown while a step waits for a decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoicePrompt {
    pub request: StepRequest,
    /// Answers given so far, in order.
    pub answers: Vec<String>,
    /// The point to decide now.
    pub point: ChoicePoint,
    /// Its position among the step's choice points (0-based), and how many
    /// there are, for "choice 2 of 3".
    pub index: usize,
    pub total: usize,
}

impl ChoicePrompt {
    /// "o1 chooses (2 of 3)".
    pub fn title(&self) -> String {
        if self.total > 1 {
            format!("{} chooses ({} of {})", self.point.at, self.index + 1, self.total)
        } else {
            format!("{} chooses", self.point.at)
        }
    }
}

/// What to do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptStep {
    /// Ask the user.
    Ask(ChoicePrompt),
    /// Perform these actions in order: one `Choose` per answer, then the
    /// step.
    Go(Vec<PlayAction>),
}

/// The first point no pending decision covers, with its index. Decisions
/// are consumed oldest first, one per point, as the simulator does.
pub fn first_undecided(points: &[ChoicePoint], pending: &[String]) -> Option<(usize, ChoicePoint)> {
    let mut left: Vec<&String> = pending.iter().collect();
    for (index, point) in points.iter().enumerate() {
        match left.iter().position(|d| point.options.contains(d)) {
            Some(used) => {
                left.remove(used);
            }
            None => return Some((index, point.clone())),
        }
    }
    None
}

fn next(oracle: &dyn ChoiceOracle, request: StepRequest, answers: Vec<String>) -> Result<PromptStep, String> {
    let (points, pending) = oracle.after(&answers, &request)?;
    match first_undecided(&points, &pending) {
        Some((index, point)) => {
            if point.options.is_empty() {
                return Err(format!("{} has a choice with no options", point.at));
            }
            Ok(PromptStep::Ask(ChoicePrompt { request, answers, point, index, total: points.len() }))
        }
        None => {
            let mut actions: Vec<PlayAction> =
                answers.into_iter().map(|option| PlayAction::Choose { option }).collect();
            actions.push(request.action());
            Ok(PromptStep::Go(actions))
        }
    }
}

/// Start delivering `request`.
pub fn begin(oracle: &dyn ChoiceOracle, request: StepRequest) -> Result<PromptStep, String> {
    next(oracle, request, Vec::new())
}

/// Answer the open prompt with `option` (one of its options).
pub fn pick(oracle: &dyn ChoiceOracle, prompt: ChoicePrompt, option: &str) -> Result<PromptStep, String> {
    if !prompt.point.options.iter().any(|o| o == option) {
        return Err(format!("`{option}` is not one of {}'s options", prompt.point.at));
    }
    let mut answers = prompt.answers;
    answers.push(option.to_owned());
    next(oracle, prompt.request, answers)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::*;

    fn point(at: &str, options: &[&str]) -> ChoicePoint {
        ChoicePoint { at: at.into(), options: options.iter().map(|o| (*o).to_owned()).collect() }
    }

    type PointsFn = Box<dyn Fn(&[String]) -> Vec<ChoicePoint>>;

    /// A scripted delivery: the points depend on the answers so far (the
    /// first answer can change which later points are hit).
    struct Script {
        pending: Vec<String>,
        points: PointsFn,
        calls: RefCell<Vec<(Vec<String>, StepRequest)>>,
    }

    impl Script {
        fn new(pending: &[&str], points: impl Fn(&[String]) -> Vec<ChoicePoint> + 'static) -> Self {
            Self {
                pending: pending.iter().map(|p| (*p).to_owned()).collect(),
                points: Box::new(points),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl ChoiceOracle for Script {
        fn after(&self, answers: &[String], request: &StepRequest) -> Result<(Vec<ChoicePoint>, Vec<String>), String> {
            self.calls.borrow_mut().push((answers.to_vec(), request.clone()));
            let mut pending = self.pending.clone();
            pending.extend(answers.iter().cloned());
            Ok(((self.points)(answers), pending))
        }
    }

    struct Failing;

    impl ChoiceOracle for Failing {
        fn after(&self, _: &[String], _: &StepRequest) -> Result<(Vec<ChoicePoint>, Vec<String>), String> {
            Err("no such queue item".into())
        }
    }

    fn head() -> StepRequest {
        StepRequest::default()
    }

    fn step(request: &StepRequest) -> PlayAction {
        request.action()
    }

    fn choose(option: &str) -> PlayAction {
        PlayAction::Choose { option: option.into() }
    }

    fn ask(result: Result<PromptStep, String>) -> ChoicePrompt {
        match result {
            Ok(PromptStep::Ask(prompt)) => prompt,
            other => panic!("expected a prompt, got {other:?}"),
        }
    }

    #[test]
    fn no_choice_points_steps_at_once() {
        let script = Script::new(&[], |_| Vec::new());
        assert_eq!(begin(&script, head()), Ok(PromptStep::Go(vec![step(&head())])));
    }

    #[test]
    fn a_choice_point_is_asked_then_chosen_before_the_step() {
        let script = Script::new(&[], |_| vec![point("o1", &["card declined", "else"])]);
        let prompt = ask(begin(&script, head()));
        assert_eq!(prompt.point, point("o1", &["card declined", "else"]));
        assert_eq!(prompt.title(), "o1 chooses");
        assert_eq!(pick(&script, prompt, "else"), Ok(PromptStep::Go(vec![choose("else"), step(&head())])));
    }

    #[test]
    fn several_points_are_asked_in_order() {
        let script = Script::new(&[], |_| vec![point("o1", &["a", "b"]), point("Billing", &["x", "not x"])]);
        let first = ask(begin(&script, head()));
        assert_eq!(first.title(), "o1 chooses (1 of 2)");
        let second = ask(pick(&script, first, "b"));
        assert_eq!(second.point.at, "Billing");
        assert_eq!(second.title(), "Billing chooses (2 of 2)");
        assert_eq!(second.answers, ["b"]);
        assert_eq!(
            pick(&script, second, "not x"),
            Ok(PromptStep::Go(vec![choose("b"), choose("not x"), step(&head())]))
        );
    }

    #[test]
    fn an_answer_can_change_the_later_points() {
        let script = Script::new(&[], |answers: &[String]| match answers.first().map(String::as_str) {
            Some("ship") => vec![point("o1", &["ship", "hold"]), point("s1", &["fast", "slow"])],
            _ => vec![point("o1", &["ship", "hold"])],
        });
        let first = ask(begin(&script, head()));
        assert_eq!(pick(&script, first.clone(), "hold"), Ok(PromptStep::Go(vec![choose("hold"), step(&head())])));
        let second = ask(pick(&script, first, "ship"));
        assert_eq!(second.point.at, "s1");
    }

    #[test]
    fn pending_decisions_cover_points_without_asking() {
        let script = Script::new(&["b"], |_| vec![point("o1", &["a", "b"]), point("o2", &["a", "b"])]);
        let prompt = ask(begin(&script, head()));
        assert_eq!(prompt.point.at, "o2", "the pending `b` decides o1");
        assert_eq!(prompt.index, 1);
        let all_covered = Script::new(&["a", "b"], |_| vec![point("o1", &["a", "b"]), point("o2", &["a", "b"])]);
        assert_eq!(begin(&all_covered, head()), Ok(PromptStep::Go(vec![step(&head())])));
    }

    #[test]
    fn first_undecided_uses_each_decision_once() {
        let points = [point("o1", &["a", "b"]), point("o2", &["a", "c"])];
        assert_eq!(first_undecided(&points, &["a".into()]).map(|(i, _)| i), Some(1), "`a` went to o1");
        assert_eq!(first_undecided(&points, &["a".into(), "a".into()]), None);
        assert_eq!(first_undecided(&points, &["c".into(), "a".into()]), None, "any order, first match");
        assert_eq!(first_undecided(&points, &["z".into()]).map(|(i, _)| i), Some(0));
        assert_eq!(first_undecided(&[], &[]), None);
    }

    #[test]
    fn the_request_is_kept_through_the_prompt() {
        let request = StepRequest { choice: Some(2), consumer: Some("Fulfil".into()) };
        let script = Script::new(&[], |_| vec![point("Fulfil", &["ok", "not ok"])]);
        let prompt = ask(begin(&script, request.clone()));
        assert_eq!(prompt.request, request);
        assert_eq!(
            pick(&script, prompt, "ok"),
            Ok(PromptStep::Go(vec![
                choose("ok"),
                PlayAction::Step { choice: Some(2), consumer: Some("Fulfil".into()) }
            ]))
        );
        assert!(script.calls.borrow().iter().all(|(_, r)| *r == request));
    }

    #[test]
    fn only_offered_options_are_accepted() {
        let script = Script::new(&[], |_| vec![point("o1", &["a", "b"])]);
        let prompt = ask(begin(&script, head()));
        assert!(pick(&script, prompt, "c").unwrap_err().contains("not one of"));
    }

    #[test]
    fn errors_and_empty_options_are_reported() {
        assert_eq!(begin(&Failing, head()), Err("no such queue item".into()));
        let script = Script::new(&[], |_| vec![point("o1", &[])]);
        assert!(begin(&script, head()).unwrap_err().contains("no options"));
    }

    #[test]
    fn the_session_oracle_reads_the_live_session() {
        let model = cascade_core::load_str("machines:\n  Order:\n    states: [draft, paid]\n").expect("loads");
        let mut session = PlaySession::new(&model);
        session
            .apply(
                &model,
                PlayAction::AddInstance {
                    name: Some("o1".into()),
                    machine: "Order".into(),
                    fields: BTreeMap::new(),
                    state: None,
                },
            )
            .expect("adds");
        let oracle = SessionOracle { session: &session, model: &model };
        // Nothing to decide in a model without choices.
        assert_eq!(oracle.after(&[], &head()), Ok((Vec::new(), Vec::new())));
        assert_eq!(begin(&oracle, head()), Ok(PromptStep::Go(vec![step(&head())])));
    }
}
