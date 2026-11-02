//! The choice prompt: before a queue item is delivered (or an external
//! trigger fired), ask for every decision it needs that no pending decision
//! already covers, then push one `Choose` per answer and the action.
//!
//! ```text
//! begin(request) ──▶ Go([action])                     no undecided choice point
//!                └─▶ Ask(prompt for point 1)
//! pick(prompt, option) ──▶ Ask(prompt for the next undecided point)
//!                      └─▶ Go([Choose a, Choose b, …, action])
//! ```
//!
//! Pure. The choice points come from a [`ChoiceOracle`]: the session's
//! `choice_points_of` with the answers so far pushed as pending decisions
//! (on a copy of the session, so nothing is recorded until the user is
//! done). A point is decided when a pending decision is one of its
//! options; each decision is used once, oldest first, as the simulator
//! takes them. A request is anything that becomes one [`PlayAction`]
//! ([`Request`]): a queue delivery ([`StepRequest`]) or the action itself
//! (an external `Fire`).

use std::fmt::Debug;

use cascade_core::Model;
use cascade_sim::{ChoicePoint, PlayAction, PlaySession};

/// What a prompt asks about: something that becomes one play action.
pub trait Request: Clone + Debug + PartialEq {
    fn action(&self) -> PlayAction;
}

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

impl Request for StepRequest {
    fn action(&self) -> PlayAction {
        StepRequest::action(self)
    }
}

/// Any action, as is (the app asks about external fires this way).
impl Request for PlayAction {
    fn action(&self) -> PlayAction {
        self.clone()
    }
}

/// Choice points and pending decisions as they would be after pushing
/// `answers`.
pub trait ChoiceOracle<R = StepRequest> {
    /// `(choice points the request hits, pending decisions)` with
    /// `answers` pushed as decisions after the current ones.
    fn after(&self, answers: &[String], request: &R) -> Result<(Vec<ChoicePoint>, Vec<String>), String>;
}

/// The live oracle: the session itself, or a copy with the answers pushed.
pub struct SessionOracle<'a> {
    pub session: &'a PlaySession,
    pub model: &'a Model,
}

impl<R: Request> ChoiceOracle<R> for SessionOracle<'_> {
    fn after(&self, answers: &[String], request: &R) -> Result<(Vec<ChoicePoint>, Vec<String>), String> {
        let action = request.action();
        let points = |session: &PlaySession| {
            session
                .choice_points_of(self.model, &action)
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

/// The prompt shown while a request waits for a decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoicePrompt<R = StepRequest> {
    pub request: R,
    /// Answers given so far, in order.
    pub answers: Vec<String>,
    /// The point to decide now.
    pub point: ChoicePoint,
    /// Its position among the request's choice points (0-based), and how
    /// many there are, for "choice 2 of 3".
    pub index: usize,
    pub total: usize,
}

impl<R> ChoicePrompt<R> {
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
pub enum PromptStep<R = StepRequest> {
    /// Ask the user.
    Ask(ChoicePrompt<R>),
    /// Perform these actions in order: one `Choose` per answer, then the
    /// request's action.
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

fn next<R: Request>(oracle: &dyn ChoiceOracle<R>, request: R, answers: Vec<String>) -> Result<PromptStep<R>, String> {
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

/// Start `request`.
pub fn begin<R: Request>(oracle: &dyn ChoiceOracle<R>, request: R) -> Result<PromptStep<R>, String> {
    next(oracle, request, Vec::new())
}

/// Answer the open prompt with `option` (one of its options).
pub fn pick<R: Request>(
    oracle: &dyn ChoiceOracle<R>,
    prompt: ChoicePrompt<R>,
    option: &str,
) -> Result<PromptStep<R>, String> {
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
        // Nothing queued: the preview fails like the step would, so the
        // prompt reports the step's own error.
        let empty = "nothing is queued to deliver".to_owned();
        assert_eq!(oracle.after(&[], &head()), Err(empty.clone()));
        assert_eq!(begin(&oracle, head()), Err(empty));
    }
    // --- Against the real simulator ------------------------------------------------

    const SAGA: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/checkout-saga/cascade.yaml"));

    fn add(name: &str, machine: &str, fields: &[(&str, &str)]) -> PlayAction {
        PlayAction::AddInstance {
            name: Some(name.into()),
            machine: machine.into(),
            fields: fields.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
            state: None,
        }
    }

    fn fire(source: &str, trigger: &str, target: &str) -> PlayAction {
        PlayAction::Fire {
            source: source.into(),
            trigger: {
                let (machine, trigger) = trigger.split_once('.').expect("Machine.trigger");
                cascade_core::definition::TriggerRef { machine: machine.into(), trigger: trigger.into() }
            },
            target: target.into(),
            payload: BTreeMap::new(),
        }
    }

    fn state_of(model: &Model, session: &PlaySession, instance: &str) -> String {
        let found = session.instances().into_iter().find(|i| i.name == instance).expect("the instance");
        model.key_of(cascade_core::ElementRef::State(found.state)).to_string()
    }

    /// Run `actions` on the session, as the workspace does after `Go`.
    fn perform(model: &Model, session: &mut PlaySession, actions: Vec<PlayAction>) {
        for action in actions {
            session.apply(model, action).unwrap_or_else(|err| panic!("{err}"));
        }
    }

    /// Ask, answer and perform `request` against the live session.
    fn go<R: Request>(model: &Model, session: &mut PlaySession, request: R, answer: Option<&str>) -> Vec<PlayAction> {
        let oracle = SessionOracle { session, model };
        let actions = match (begin(&oracle, request), answer) {
            (Ok(PromptStep::Go(actions)), None) => actions,
            (Ok(PromptStep::Ask(prompt)), Some(option)) => match pick(&oracle, prompt, option) {
                Ok(PromptStep::Go(actions)) => actions,
                other => panic!("expected to go after one answer, got {other:?}"),
            },
            (other, _) => panic!("unexpected {other:?} for answer {answer:?}"),
        };
        perform(model, session, actions.clone());
        actions
    }

    fn saga() -> (Model, PlaySession) {
        let model = cascade_core::load_str(SAGA).expect("the saga loads");
        let mut session = PlaySession::new(&model);
        let order = [("orderId", "1"), ("amount", "50")];
        let actions =
            vec![add("o1", "Order", &order), add("p1", "Payment", &order), add("s1", "Stock", &[("orderId", "1")])];
        perform(&model, &mut session, actions);
        (model, session)
    }

    #[test]
    fn the_saga_asks_the_bank_before_its_answer_is_fired() {
        let (model, mut session) = saga();
        // Placing the order needs no decision.
        let place = fire("Customer", "Order.place", "o1");
        assert_eq!(go(&model, &mut session, place.clone(), None), [place]);
        perform(&model, &mut session, vec![PlayAction::RunUntilQuiet]);
        assert_eq!(state_of(&model, &session, "p1"), "state:Payment:authorizing");

        // The bank's answer is a `?choice`: the prompt asks p1 first.
        let respond = fire("Bank", "Payment.respond", "p1");
        let oracle = SessionOracle { session: &session, model: &model };
        let prompt = match begin(&oracle, respond.clone()) {
            Ok(PromptStep::Ask(prompt)) => prompt,
            other => panic!("expected a prompt, got {other:?}"),
        };
        assert_eq!(prompt.title(), "p1 chooses");
        assert_eq!(prompt.point.options, ["approved", "card declined"]);
        assert_eq!(prompt.request, respond);
        assert!(session.timeline().actions.iter().all(|a| !matches!(a, PlayAction::Choose { .. })), "nothing recorded");

        let actions = go(&model, &mut session, respond.clone(), Some("card declined"));
        assert_eq!(actions, [choose("card declined"), respond]);
        perform(&model, &mut session, vec![PlayAction::RunUntilQuiet]);
        assert_eq!(state_of(&model, &session, "p1"), "state:Payment:declined");
        assert_eq!(state_of(&model, &session, "o1"), "state:Order:cancelled");
        assert!(session.pending_decisions().is_empty(), "the decision was taken");
    }

    #[test]
    fn the_saga_happy_path_through_the_prompt() {
        let (model, mut session) = saga();
        go(&model, &mut session, fire("Customer", "Order.place", "o1"), None);
        perform(&model, &mut session, vec![PlayAction::RunUntilQuiet]);
        go(&model, &mut session, fire("Bank", "Payment.respond", "p1"), Some("approved"));
        // Deliver the rest one step at a time, each through the prompt.
        while !session.pending().is_empty() {
            go(&model, &mut session, head(), None);
        }
        assert_eq!(state_of(&model, &session, "o1"), "state:Order:confirmed");
        assert_eq!(state_of(&model, &session, "s1"), "state:Stock:reserved");
    }

    #[test]
    fn a_step_into_choice_guards_is_asked_then_chosen_then_stepped() {
        let model = cascade_core::load_str(
            r#"
machines:
  Order:
    initial: open
    states: [open, paid]
    transitions:
      - { from: open, to: paid, on: pay, emits: [Paid] }
  Payment:
    initial: authorizing
    states: [authorizing, captured, declined]
    transitions:
      - { from: authorizing, to: captured, on: respond, guard: "?approved" }
      - { from: authorizing, to: declined, on: respond, guard: "?card declined" }
controllers:
  Bank:
    on:
      Paid: { fire: Payment.respond, target: Payment }
external:
  Api: [Order.pay]
"#,
        )
        .expect("loads");
        let mut session = PlaySession::new(&model);
        perform(&model, &mut session, vec![add("o", "Order", &[]), add("p", "Payment", &[])]);
        go(&model, &mut session, fire("Api", "Order.pay", "o"), None);
        // Paid reaches Bank, which queues the fire: no decision yet.
        assert_eq!(go(&model, &mut session, head(), None), [step(&head())]);
        // Delivering the fire hits p's choice.
        let oracle = SessionOracle { session: &session, model: &model };
        let prompt = ask(begin(&oracle, head()));
        assert_eq!(
            (prompt.point.at.as_str(), prompt.point.options.as_slice()),
            ("p", &strs(&["approved", "card declined"])[..])
        );
        assert_eq!(go(&model, &mut session, head(), Some("approved")), [choose("approved"), step(&head())]);
        assert_eq!(state_of(&model, &session, "p"), "state:Payment:captured");
    }

    #[test]
    fn a_pending_decision_already_covers_the_point() {
        let (model, mut session) = saga();
        go(&model, &mut session, fire("Customer", "Order.place", "o1"), None);
        perform(&model, &mut session, vec![PlayAction::RunUntilQuiet, choose("card declined")]);
        // The decision pushed earlier answers it: no prompt.
        let respond = fire("Bank", "Payment.respond", "p1");
        assert_eq!(go(&model, &mut session, respond.clone(), None), [respond]);
        assert_eq!(state_of(&model, &session, "p1"), "state:Payment:declined");
    }

    fn strs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }
}
