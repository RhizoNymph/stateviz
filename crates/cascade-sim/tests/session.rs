//! Playing a system one action at a time: instances, external fires, the
//! queue, manual delivery order, removal and the step limit.

mod common;
mod support;

use cascade_core::Model;
use cascade_sim::{
    AvailableFire, Lifeline, PendingKind, PlayAction, PlaySession, STEP_LIMIT, ScenarioErrorKind, SimError, StepIx,
    TraceStepKind, race_orderings, simulate,
};
use common::{finals, lines, model, scenario, strs};
use support::{
    ORDERS, PING, RACE, RACE_SCENARIO, SPAWNING, add, add_auto, add_in, apply, apply_all, apply_err, choose, fire,
    fire_with, remove, run_all, session, step, trigger, unnamed,
};

/// The race fixture's two instances.
fn race_start() -> Vec<PlayAction> {
    vec![add("o1", "Order", &[("orderId", "1")]), add("s1", "Shipment", &[("orderId", "1")])]
}

fn race_session(model: &Model, more: &[PlayAction]) -> PlaySession {
    let mut actions = race_start();
    actions.extend_from_slice(more);
    session(model, &actions)
}

fn capture() -> PlayAction {
    fire("PaymentGateway", "Order.capture_ok", "o1")
}

fn labels(session: &PlaySession) -> Vec<String> {
    session.pending().into_iter().map(|p| p.label).collect()
}

#[test]
fn a_new_session_is_empty() {
    let model = model(RACE);
    let session = PlaySession::new(&model);
    assert!(session.trace().steps.is_empty());
    assert!(session.trace().lifelines.is_empty());
    assert!(session.instances().is_empty());
    assert!(session.pending().is_empty());
    assert!(session.available_fires(&model).is_empty());
    assert_eq!(session.timeline().position, 0);
}

#[test]
fn instances_are_listed_in_lifeline_order() {
    let model = model(RACE);
    // Shipment first, but Order comes first in the model.
    let session = session(&model, &[add("s1", "Shipment", &[("orderId", "1")]), add("o1", "Order", &[])]);
    let instances = session.instances();
    let names: Vec<&str> = instances.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["o1", "s1"]);
    for instance in &instances {
        match &session.trace().lifelines[instance.lifeline.index()] {
            Lifeline::Instance { name, machine } => {
                assert_eq!(name, &instance.name);
                assert_eq!(*machine, instance.machine);
            }
            other => panic!("expected an instance lifeline, got {other:?}"),
        }
        assert_eq!(session.trace().final_states.get(&instance.lifeline), Some(&instance.state));
    }
    assert_eq!(model.state(instances[0].state).path, "pending");
    assert_eq!(instances[1].fields.get("orderId").map(String::as_str), Some("1"));
    // Adding instances records no steps.
    assert!(session.trace().steps.is_empty());
}

#[test]
fn instances_can_start_in_any_state() {
    let model = model(RACE);
    let session = session(&model, &[add_in("s1", "Shipment", "on_hold")]);
    assert_eq!(model.state(session.instances()[0].state).path, "on_hold");
}

#[test]
fn unnamed_instances_get_the_next_free_name() {
    let model = model(RACE);
    let session = session(&model, &[add_auto("Order"), add("order2", "Order", &[]), add_auto("Order")]);
    let names: Vec<String> = session.instances().into_iter().map(|i| i.name).collect();
    assert_eq!(names, strs(&["order1", "order2", "order3"]));
    // The timeline records the name the instance got.
    assert_eq!(
        session.timeline().actions[0],
        PlayAction::AddInstance {
            name: Some("order1".to_owned()),
            machine: "Order".to_owned(),
            fields: Default::default(),
            state: None
        }
    );
}

#[test]
fn a_fire_is_delivered_at_once_and_its_events_are_queued() {
    let model = model(RACE);
    let mut session = race_session(&model, &[]);
    let outcome = apply(&mut session, &model, capture());
    assert_eq!(outcome.steps, 0..3);
    assert_eq!(
        lines(&model, session.trace()),
        strs(&["ext PaymentGateway -> o1 capture_ok", "o1 pending -> paid", "o1 emit OrderPaid"])
    );
    let pending = session.pending();
    assert_eq!(pending.len(), 1);
    let Some(paid) = model.event_by_name("OrderPaid") else { panic!("no OrderPaid") };
    assert_eq!(pending[0].kind, PendingKind::Event { event: paid });
    assert_eq!(pending[0].cause, StepIx(2));
    assert_eq!(pending[0].label, "OrderPaid from o1");
}

#[test]
fn stepping_delivers_the_head() {
    let model = model(RACE);
    let mut session = race_session(&model, &[capture()]);
    let outcome = apply(&mut session, &model, step());
    assert_eq!(outcome.steps, 3..7);
    assert_eq!(labels(&session), strs(&["Fulfillment → s1: start", "Billing → s1: hold"]));

    let pending = session.pending();
    let s1 = session.instances().into_iter().find(|i| i.name == "s1").map(|i| i.lifeline);
    for (item, controller) in pending.iter().zip(["Fulfillment", "Billing"]) {
        let PendingKind::Fire { rule, target } = &item.kind else { panic!("expected a fire, got {item:?}") };
        assert_eq!(model.controller(model.rule(*rule).controller).name, controller);
        assert_eq!(Some(*target), s1);
        assert!(matches!(session.trace().steps[item.cause.index()].kind, TraceStepKind::Fire { .. }));
    }
}

#[test]
fn choosing_a_pending_item_plays_the_other_side_of_the_race() {
    let model = model(RACE);
    let session = race_session(&model, &[capture(), step(), choose(1), step()]);
    let Some(race) = cascade_core::analyze(&model, &cascade_core::CausalGraph::build(&model))
        .into_iter()
        .find(|f| matches!(f.detail, cascade_core::FindingDetail::RaceCandidate { .. }))
    else {
        panic!("the fixture has a race candidate");
    };
    let both = match race_orderings(&model, &scenario(RACE_SCENARIO), &race.detail) {
        Ok(both) => both,
        Err(err) => panic!("{err}"),
    };
    assert_eq!(unnamed(session.trace()), unnamed(&both.swapped));
    assert_eq!(finals(&model, session.trace()), ["o1: paid", "s1: on_hold"]);

    // And stepping heads only is the FIFO side.
    let fifo = race_session(&model, &[capture(), step(), step(), step()]);
    assert_eq!(unnamed(fifo.trace()), unnamed(&both.as_queued));
}

#[test]
fn pending_ids_are_stable_while_queued() {
    let model = model(RACE);
    let mut session = race_session(&model, &[capture(), step()]);
    let before = session.pending();
    apply(&mut session, &model, choose(1));
    let after = session.pending();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].id, before[0].id);
    assert_eq!(after[0].label, before[0].label);

    // New items get new ids.
    apply(&mut session, &model, step());
    apply(&mut session, &model, capture());
    assert!(session.pending().is_empty());
    let mut again = race_session(&model, &[capture()]);
    let first = again.pending()[0].id;
    apply(&mut again, &model, step());
    assert!(again.pending().iter().all(|p| p.id != first));
}

#[test]
fn run_until_quiet_matches_the_batch_run() {
    let model = model(RACE);
    let session = race_session(&model, &[capture(), run_all()]);
    let batch = match simulate(&model, &scenario(RACE_SCENARIO)) {
        Ok(trace) => trace,
        Err(err) => panic!("{err}"),
    };
    assert_eq!(unnamed(session.trace()), unnamed(&batch));
    assert!(session.pending().is_empty());
}

#[test]
fn run_until_quiet_on_a_quiet_queue_changes_nothing_but_is_recorded() {
    let model = model(RACE);
    let mut session = race_session(&model, &[]);
    let outcome = apply(&mut session, &model, run_all());
    assert_eq!(outcome.steps, 0..0);
    assert_eq!(session.timeline().actions.len(), 3);
}

#[test]
fn payloads_travel_with_the_session() {
    let model = model(RACE);
    let session = race_session(&model, &[fire_with("PaymentGateway", "Order.capture_ok", "o1", &[("amount", "42")])]);
    assert_eq!(session.payloads().get(&StepIx(0)), Some(&support::map(&[("amount", "42")])));
    assert_eq!(session.payloads().get(&StepIx(2)), Some(&support::map(&[("amount", "42"), ("orderId", "1")])));
}

#[test]
fn available_fires_cover_every_source_trigger_and_instance() {
    let model = model(ORDERS);
    let mut session = session(&model, &[add("o1", "Order", &[]), add("o2", "Order", &[]), add("s1", "Shipment", &[])]);
    let show = |fires: Vec<AvailableFire>| -> Vec<String> {
        fires.into_iter().map(|f| format!("{} {} {} {}", f.source, f.trigger, f.target, f.accepted)).collect()
    };
    assert_eq!(
        show(session.available_fires(&model)),
        strs(&[
            "Customer Order.submit o1 true",
            "Customer Order.submit o2 true",
            "PaymentGateway Order.capture_ok o1 false",
            "PaymentGateway Order.capture_ok o2 false",
            "Clock Order.timeout o1 false",
            "Clock Order.timeout o2 false",
        ])
    );
    apply(&mut session, &model, fire("Customer", "Order.submit", "o1"));
    let fires = session.available_fires(&model);
    let accepted: Vec<String> = show(fires).into_iter().filter(|f| f.ends_with("true")).collect();
    assert_eq!(
        accepted,
        strs(&[
            "Customer Order.submit o2 true",
            "PaymentGateway Order.capture_ok o1 true",
            "Clock Order.timeout o1 true"
        ])
    );
}

#[test]
fn firing_a_trigger_the_state_does_not_accept_records_a_drop() {
    let model = model(ORDERS);
    let session = session(&model, &[add("o1", "Order", &[]), fire("Clock", "Order.timeout", "o1")]);
    assert_eq!(lines(&model, session.trace()), strs(&["ext Clock -> o1 timeout", "o1 drop timeout @ draft"]));
}

type KindCheck = fn(&ScenarioErrorKind) -> bool;

#[test]
fn rejected_actions_change_nothing() {
    let model = model(RACE);
    let mut session = race_session(&model, &[capture()]);
    let trace = session.trace().clone();
    let timeline = session.timeline().clone();
    let pending = session.pending();

    let cases: Vec<(PlayAction, KindCheck)> = vec![
        (add("z1", "Zeppelin", &[]), |k| matches!(k, ScenarioErrorKind::UnknownMachine { .. })),
        (add("o1", "Order", &[]), |k| matches!(k, ScenarioErrorKind::NameTaken { .. })),
        (add("o 3", "Order", &[]), |k| matches!(k, ScenarioErrorKind::InvalidName { .. })),
        (add("o3", "Order", &[("colour", "red")]), |k| matches!(k, ScenarioErrorKind::UnknownField { .. })),
        (add_in("o3", "Order", "gone"), |k| matches!(k, ScenarioErrorKind::UnknownState { .. })),
        (fire("Courier", "Order.capture_ok", "o1"), |k| matches!(k, ScenarioErrorKind::UnknownSource { .. })),
        (fire("PaymentGateway", "Order.refund", "o1"), |k| matches!(k, ScenarioErrorKind::UnknownTrigger { .. })),
        (fire("PaymentGateway", "Shipment.start", "s1"), |k| matches!(k, ScenarioErrorKind::SourceCannotFire { .. })),
        (fire("PaymentGateway", "Order.capture_ok", "nobody"), |k| {
            matches!(k, ScenarioErrorKind::UnknownInstance { .. })
        }),
        (fire("PaymentGateway", "Order.capture_ok", "s1"), |k| {
            matches!(k, ScenarioErrorKind::TargetMachineMismatch { .. })
        }),
        (fire_with("PaymentGateway", "Order.capture_ok", "o1", &[("-x", "1")]), |k| {
            matches!(k, ScenarioErrorKind::InvalidName { .. })
        }),
        (remove("nobody"), |k| matches!(k, ScenarioErrorKind::UnknownInstance { .. })),
        (choose(1), |k| matches!(k, ScenarioErrorKind::NoPendingItem { position: 1, pending: 1 })),
    ];
    for (action, expected) in cases {
        match apply_err(&mut session, &model, action.clone()) {
            SimError::Action(kind) => assert!(expected(&kind), "{action:?}: {kind}"),
            other => panic!("{action:?}: expected an action error, got {other:?}"),
        }
        assert_eq!(session.trace(), &trace, "{action:?}");
        assert_eq!(session.timeline(), &timeline, "{action:?}");
        assert_eq!(session.pending(), pending, "{action:?}");
    }
}

#[test]
fn stepping_an_empty_queue_is_an_error() {
    let model = model(RACE);
    let mut session = race_session(&model, &[]);
    assert_eq!(apply_err(&mut session, &model, step()), SimError::Action(ScenarioErrorKind::QueueEmpty));
    assert_eq!(apply_err(&mut session, &model, choose(0)), SimError::Action(ScenarioErrorKind::QueueEmpty));
    assert_eq!(session.timeline().actions.len(), 2);
}

#[test]
fn removing_an_instance_discards_the_fires_queued_for_it() {
    let model = model(RACE);
    let mut session = race_session(&model, &[capture(), step()]);
    assert_eq!(session.pending().len(), 2);
    let outcome = apply(&mut session, &model, remove("s1"));
    assert_eq!(outcome.steps, 7..7);
    assert!(session.pending().is_empty());
    let names: Vec<String> = session.instances().into_iter().map(|i| i.name).collect();
    assert_eq!(names, strs(&["o1"]));
    // The lifeline and its last state stay in the trace.
    assert_eq!(finals(&model, session.trace()), ["o1: paid", "s1: idle"]);
    // The instance is gone for good, and its name stays taken.
    assert!(session.available_fires(&model).iter().all(|f| f.target != "s1"));
    assert!(matches!(
        apply_err(&mut session, &model, fire("PaymentGateway", "Order.capture_ok", "s1")),
        SimError::Action(ScenarioErrorKind::UnknownInstance { .. })
    ));
    assert!(matches!(
        apply_err(&mut session, &model, add("s1", "Shipment", &[])),
        SimError::Action(ScenarioErrorKind::NameTaken { .. })
    ));
    assert!(matches!(
        apply_err(&mut session, &model, remove("s1")),
        SimError::Action(ScenarioErrorKind::UnknownInstance { .. })
    ));
}

#[test]
fn removing_the_emitter_keeps_its_queued_events() {
    let model = model(RACE);
    let mut session = race_session(&model, &[capture(), remove("o1")]);
    assert_eq!(labels(&session), strs(&["OrderPaid from o1"]));
    apply(&mut session, &model, step());
    assert_eq!(labels(&session), strs(&["Fulfillment → s1: start", "Billing → s1: hold"]));
}

#[test]
fn removed_instances_are_not_selected() {
    let model = model(RACE);
    let session = session(
        &model,
        &[
            add("o1", "Order", &[("orderId", "1")]),
            add("s1", "Shipment", &[("orderId", "1")]),
            add("s2", "Shipment", &[("orderId", "1")]),
            remove("s1"),
            capture(),
            step(),
        ],
    );
    // Only s2 matches now, so the rules are not ambiguous.
    assert_eq!(labels(&session), strs(&["Fulfillment → s2: start", "Billing → s2: hold"]));
}

#[test]
fn auto_names_never_reuse_a_removed_name() {
    let model = model(SPAWNING);
    let mut session = session(&model, &[add_auto("Shipment"), remove("shipment1"), add_auto("Shipment")]);
    let names: Vec<String> = session.instances().into_iter().map(|i| i.name).collect();
    assert_eq!(names, strs(&["shipment2"]));
    // Spawns skip taken names too.
    apply_all(
        &mut session,
        &model,
        &[add("o1", "Order", &[("orderId", "1")]), fire("PaymentGateway", "Order.capture_ok", "o1"), run_all()],
    );
    let names: Vec<String> = session.instances().into_iter().map(|i| i.name).collect();
    assert_eq!(names, strs(&["o1", "shipment2", "shipment3"]));
}

#[test]
fn run_until_quiet_is_bounded_by_the_step_limit() {
    let model = model(PING);
    let mut session = session(&model, &[add("p", "Ping", &[]), fire("Starter", "Ping.go", "p")]);
    let trace = session.trace().clone();
    let pending = session.pending();
    assert_eq!(apply_err(&mut session, &model, run_all()), SimError::StepLimit(STEP_LIMIT));
    // Nothing happened.
    assert_eq!(session.trace(), &trace);
    assert_eq!(session.pending(), pending);
    assert_eq!(session.timeline().actions.len(), 2);
    // Stepping by hand still works.
    apply(&mut session, &model, step());
    assert_eq!(session.pending().len(), 1);
}

#[test]
fn a_session_driven_step_by_step_matches_one_that_runs() {
    let model = model(ORDERS);
    let start = [
        add("o1", "Order", &[("orderId", "1")]),
        add("s1", "Shipment", &[("orderId", "1")]),
        fire("Customer", "Order.submit", "o1"),
        fire("PaymentGateway", "Order.capture_ok", "o1"),
    ];
    let mut stepped = session(&model, &start);
    while !stepped.pending().is_empty() {
        apply(&mut stepped, &model, step());
    }
    let mut ran = session(&model, &start);
    apply(&mut ran, &model, run_all());
    assert_eq!(stepped.trace(), ran.trace());
    assert_eq!(trigger("Order.submit").to_string(), "Order.submit");
}
