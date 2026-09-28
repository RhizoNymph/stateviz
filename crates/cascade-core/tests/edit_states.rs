//! State edits: adding, removing and renaming states, with reference
//! propagation, qualification of bare names that would become ambiguous,
//! cascading removal of transitions, and initial fix-ups. Every successful
//! edit is checked against the inverse law by `ok`.

mod edit_support;

use cascade_core::definition::StateKindDef;
use cascade_core::edit::{EditError, EditOp};
use cascade_core::{DiagnosticKind, ElementKey, resolve};
use edit_support::*;

fn invalid(err: &EditError, pred: impl Fn(&DiagnosticKind) -> bool) -> bool {
    match err {
        EditError::Invalid(load) => load.diagnostics.iter().any(|d| pred(&d.kind)),
        _ => false,
    }
}

fn state_key(machine: &str, path: &str) -> ElementKey {
    ElementKey::State { machine: machine.into(), path: path.into() }
}

fn transition_key(machine: &str, from: &str, to: &str, trigger: &str) -> ElementKey {
    ElementKey::Transition {
        machine: machine.into(),
        from: from.into(),
        to: to.into(),
        trigger: trigger.into(),
        ordinal: 0,
    }
}

fn add_state(machine: &str, parent: Option<&str>, name: &str, index: Option<usize>) -> EditOp {
    EditOp::AddState { machine: machine.into(), parent: parent.map(Into::into), state: leaf(name), index }
}

fn remove_state(machine: &str, path: &str) -> EditOp {
    EditOp::RemoveState { machine: machine.into(), path: path.into() }
}

fn rename_state(machine: &str, path: &str, to: &str) -> EditOp {
    EditOp::RenameState { machine: machine.into(), path: path.into(), to: to.into() }
}

fn child_names(def: &cascade_core::Definition, machine_name: &str, path: &str) -> Vec<String> {
    let parent = state(def, machine_name, path).unwrap_or_else(|| panic!("no state {path}"));
    parent.states.iter().map(|s| s.name.value.clone()).collect()
}

fn state_initial(def: &cascade_core::Definition, machine_name: &str, path: &str) -> Option<String> {
    state(def, machine_name, path).and_then(|s| s.initial.as_ref().map(|i| i.value.clone()))
}

// --- Adding --------------------------------------------------------------------

#[test]
fn add_top_level_state() {
    let def = parse(ORDER_FULFILLMENT);
    let applied = ok(&def, add_state("Order", None, "refunded", None));
    let names: Vec<_> = machine(&applied.definition, "Order").states.iter().map(|s| s.name.value.as_str()).collect();
    assert_eq!(names, ["draft", "pending", "paid", "cancelled", "refunded"]);
    assert_eq!(applied.touched, [state_key("Order", "refunded")]);
    assert_eq!(applied.inverse, remove_state("Order", "refunded"));
}

#[test]
fn add_nested_state_at_index() {
    let def = parse(NESTED);
    let applied = ok(&def, add_state("Job", Some("paused"), "sleeping", Some(0)));
    assert_eq!(child_names(&applied.definition, "Job", "paused"), ["sleeping", "waiting", "resuming"]);
    assert_eq!(applied.touched, [state_key("Job", "paused.sleeping")]);
}

#[test]
fn add_child_to_atomic_state_makes_it_compound() {
    let def = parse(NESTED);
    let applied = ok(&def, add_state("Job", Some("failed"), "logged", None));
    let model = resolve(applied.definition.clone()).expect("resolves");
    let job = model.machine_by_name("Job").expect("Job");
    let failed = model.state_by_path(job, "failed").expect("failed");
    assert!(model.state(failed).is_compound());
}

#[test]
fn add_subtree_reports_every_new_state() {
    let def = parse(ORDER_FULFILLMENT);
    let subtree = state_def("review", StateKindDef::Normal, &[leaf("queued"), leaf("checking")]);
    let applied = ok(&def, EditOp::AddState { machine: "Order".into(), parent: None, state: subtree, index: Some(1) });
    assert_eq!(
        applied.touched,
        [state_key("Order", "review"), state_key("Order", "review.queued"), state_key("Order", "review.checking")]
    );
}

#[test]
fn add_state_rejections() {
    let def = parse(NESTED);
    assert_eq!(
        rejected(&def, add_state("Job", Some("running"), "computing", None)),
        EditError::NameTaken { what: "state", name: "Job.running.computing".into() }
    );
    assert_eq!(
        rejected(&def, add_state("Job", None, "queued", None)),
        EditError::NameTaken { what: "state", name: "Job.queued".into() }
    );
    assert_eq!(
        rejected(&def, add_state("Job", Some("nowhere"), "x", None)),
        EditError::NotFound { what: "state", name: "Job.nowhere".into() }
    );
    assert_eq!(
        rejected(&def, add_state("Nope", None, "x", None)),
        EditError::NotFound { what: "machine", name: "Nope".into() }
    );
    let err = rejected(&def, add_state("Job", Some("done"), "x", None));
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::ChildrenNotAllowed { .. })), "{err:?}");
    assert_eq!(rejected(&def, add_state("Job", None, "not ok", None)), EditError::InvalidName("not ok".into()));
    assert_eq!(
        rejected(&def, add_state("Job", Some("paused"), "x", Some(3))),
        EditError::IndexOutOfRange { what: "state", index: 3, len: 2 }
    );
    // Two children with the same name inside the added subtree.
    let twins = state_def("twins", StateKindDef::Normal, &[leaf("a"), leaf("a")]);
    let err = rejected(&def, EditOp::AddState { machine: "Job".into(), parent: None, state: twins, index: None });
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::Duplicate { .. })), "{err:?}");
}

#[test]
fn add_state_qualifies_bare_names_it_would_make_ambiguous() {
    let def = parse(NESTED);
    let applied = ok(&def, add_state("Job", Some("paused"), "fetching", None));
    let ts = transitions(&applied.definition, "Job");
    assert_eq!(ts[1], "running.fetching -> computing @ fetched");
    assert_eq!(ts[5], "running.fetching -> running.fetching @ retry");
    assert_eq!(ts[6], "running.fetching -> running.fetching @ retry");
    // Unaffected references keep their spelling.
    assert_eq!(ts[3], "running.computing|waiting -> done @ finish");
    assert_eq!(ts[4], "failed -> hist @ resume");
    // The compound initial names a child, never a path.
    assert_eq!(state_initial(&applied.definition, "Job", "running").as_deref(), Some("fetching"));
    assert!(applied.touched.contains(&state_key("Job", "paused.fetching")));
    assert!(applied.touched.contains(&transition_key("Job", "running.fetching", "running.computing", "fetched")));
}

#[test]
fn add_top_level_state_qualifies_names_it_would_capture() {
    // A top-level `hist` would win over the bare local name `hist`, so the
    // existing reference to `running.hist` is written out in full.
    let def = parse(NESTED);
    let applied = ok(&def, add_state("Job", None, "hist", None));
    assert_eq!(transitions(&applied.definition, "Job")[4], "failed -> running.hist @ resume");
}

#[test]
fn add_state_qualifies_a_bare_machine_initial() {
    let def = parse(NESTED);
    let def =
        ok(&def, EditOp::SetMachineInitial { machine: "Job".into(), initial: Some("computing".into()) }).definition;
    let applied = ok(&def, add_state("Job", Some("paused"), "computing", None));
    assert_eq!(initial(&applied.definition, "Job").as_deref(), Some("running.computing"));
    assert!(applied.touched.contains(&ElementKey::Machine { machine: "Job".into() }));
}

// --- Removing ------------------------------------------------------------------

#[test]
fn remove_state_removes_its_transitions() {
    let def = parse(ORDER_FULFILLMENT);
    let applied = ok(&def, remove_state("Order", "pending"));
    assert!(transitions(&applied.definition, "Order").is_empty());
    assert!(applied.touched.contains(&state_key("Order", "pending")));
    assert!(applied.touched.contains(&transition_key("Order", "draft", "pending", "submit")));
    assert!(applied.touched.contains(&transition_key("Order", "pending", "paid", "capture_ok")));
    // Transitions of other machines stay.
    assert_eq!(transitions(&applied.definition, "Shipment").len(), 2);
}

#[test]
fn remove_compound_state_removes_transitions_of_descendants() {
    let def = parse(SHOP);
    let applied = ok(&def, remove_state("Order", "placed"));
    assert_eq!(
        transitions(&applied.definition, "Order"),
        [
            "fulfilling -> delivered @ shipment_delivered",
            "delivered -> closed @ return_window_expired",
            "delivered -> returned @ return_received"
        ]
    );
    assert!(applied.touched.contains(&state_key("Order", "placed")));
    assert!(applied.touched.contains(&state_key("Order", "placed.paid")));
    assert!(applied.touched.contains(&transition_key("Order", "placed.paid", "fulfilling", "stock_reserved")));
}

#[test]
fn remove_state_trims_multi_source_transitions() {
    let def = parse(NESTED);
    let applied = ok(&def, remove_state("Job", "paused"));
    assert_eq!(
        transitions(&applied.definition, "Job"),
        [
            "queued -> running @ start",
            "fetching -> computing @ fetched",
            "running -> failed @ crash",
            "running.computing -> done @ finish",
            "failed -> hist @ resume",
            "fetching -> fetching @ retry",
            "fetching -> fetching @ retry",
        ]
    );
    // Only the expansion that left the removed state counts as removed.
    assert!(applied.touched.contains(&transition_key("Job", "paused.waiting", "done", "finish")));
    assert!(!applied.touched.contains(&transition_key("Job", "running.computing", "done", "finish")));
    // Undo puts `waiting` back into the list.
    let EditOp::Batch(_) = applied.inverse else { panic!("expected a batch inverse, got {:?}", applied.inverse) };
}

#[test]
fn remove_state_resets_initials_that_pointed_at_it() {
    let order = parse(ORDER_FULFILLMENT);
    let applied = ok(&order, remove_state("Order", "draft"));
    assert_eq!(initial(&applied.definition, "Order"), None);
    assert!(applied.touched.contains(&ElementKey::Machine { machine: "Order".into() }));

    let nested = parse(NESTED);
    let applied = ok(&nested, remove_state("Job", "running.fetching"));
    assert_eq!(state_initial(&applied.definition, "Job", "running"), None);
    assert!(applied.touched.contains(&state_key("Job", "running")));
    assert_eq!(initial(&applied.definition, "Job").as_deref(), Some("queued"));
    assert_eq!(transitions(&applied.definition, "Job").len(), 7);

    // A machine initial inside a removed compound state is reset too.
    let def = ok(&nested, EditOp::SetMachineInitial { machine: "Job".into(), initial: Some("waiting".into()) });
    let applied = ok(&def.definition, remove_state("Job", "paused"));
    assert_eq!(initial(&applied.definition, "Job"), None);
}

#[test]
fn remove_state_rejections() {
    let def = parse(NESTED);
    assert_eq!(
        rejected(&def, remove_state("Job", "running.nope")),
        EditError::NotFound { what: "state", name: "Job.running.nope".into() }
    );
    assert_eq!(
        rejected(&def, remove_state("Job", "fetching")),
        EditError::NotFound { what: "state", name: "Job.fetching".into() }
    );
    let single = parse("machines:\n  Lone:\n    states: [only]\n");
    let err = rejected(&single, remove_state("Lone", "only"));
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::EmptyMachine { .. })), "{err:?}");
    let err = rejected(&def, EditOp::Batch(vec![remove_state("Monitor", "idle"), remove_state("Monitor", "alerting")]));
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::EmptyMachine { .. })), "{err:?}");
    // The new first state would be a history state.
    let history_first = parse(
        "machines:\n  H:\n    states:\n      - a\n      - h: { kind: history }\n      - b\n    transitions:\n      - { from: b, to: h, on: back }\n",
    );
    let err = rejected(&history_first, remove_state("H", "a"));
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::InitialIsHistory { .. })), "{err:?}");
}

// --- Renaming --------------------------------------------------------------------

#[test]
fn rename_state_updates_transitions_and_initial() {
    let def = parse(ORDER_FULFILLMENT);
    let applied = ok(&def, rename_state("Order", "pending", "awaiting"));
    assert_eq!(
        transitions(&applied.definition, "Order"),
        ["draft -> awaiting @ submit", "awaiting -> paid @ capture_ok", "awaiting -> cancelled @ timeout"]
    );
    assert_eq!(applied.inverse, rename_state("Order", "awaiting", "pending"));
    assert!(applied.touched.contains(&state_key("Order", "awaiting")));
    assert!(applied.touched.contains(&transition_key("Order", "draft", "awaiting", "submit")));

    let applied = ok(&def, rename_state("Order", "draft", "start"));
    assert_eq!(initial(&applied.definition, "Order").as_deref(), Some("start"));
}

#[test]
fn rename_compound_state_rewrites_descendant_paths() {
    let def = parse(SHOP);
    let applied = ok(&def, rename_state("Order", "placed", "ordered"));
    let ts = transitions(&applied.definition, "Order");
    assert_eq!(ts[0], "cart -> ordered @ checkout");
    assert_eq!(ts[1], "ordered.awaiting_payment -> ordered.paid @ payment_captured");
    assert_eq!(ts[2], "ordered.paid -> fulfilling @ stock_reserved");
    assert_eq!(ts[6], "ordered -> cancelled @ cancel");
    assert!(applied.touched.contains(&state_key("Order", "ordered")));
    assert!(applied.touched.contains(&state_key("Order", "ordered.paid")));

    // A machine initial written as a dotted path follows too.
    let def = ok(&def, EditOp::SetMachineInitial { machine: "Order".into(), initial: Some("placed.paid".into()) });
    let applied = ok(&def.definition, rename_state("Order", "placed", "ordered"));
    assert_eq!(initial(&applied.definition, "Order").as_deref(), Some("ordered.paid"));
}

#[test]
fn rename_initial_child_updates_the_parent_initial() {
    let def = parse(SHOP);
    let applied = ok(&def, rename_state("Order", "placed.awaiting_payment", "unpaid"));
    assert_eq!(state_initial(&applied.definition, "Order", "placed").as_deref(), Some("unpaid"));
    assert_eq!(transitions(&applied.definition, "Order")[1], "placed.unpaid -> placed.paid @ payment_captured");
    assert!(applied.touched.contains(&state_key("Order", "placed")));
}

#[test]
fn rename_keeps_bare_names_bare_when_they_stay_unique() {
    let def = parse(NESTED);
    let applied = ok(&def, rename_state("Job", "running.fetching", "loading"));
    let ts = transitions(&applied.definition, "Job");
    assert_eq!(ts[1], "loading -> computing @ fetched");
    assert_eq!(ts[5], "loading -> loading @ retry");
    assert_eq!(state_initial(&applied.definition, "Job", "running").as_deref(), Some("loading"));
    // Dotted references to siblings are untouched.
    assert_eq!(ts[8], "resuming -> running.hist @ resume");
}

#[test]
fn rename_into_a_collision_writes_full_paths() {
    // `paused.waiting` → `paused.fetching`: both the references to the
    // renamed state and the references to `running.fetching` would be
    // ambiguous as bare names, so all of them are written out in full.
    let def = parse(NESTED);
    let applied = ok(&def, rename_state("Job", "paused.waiting", "fetching"));
    let ts = transitions(&applied.definition, "Job");
    assert_eq!(ts[1], "running.fetching -> computing @ fetched");
    assert_eq!(ts[3], "running.computing|paused.fetching -> done @ finish");
    assert_eq!(ts[5], "running.fetching -> running.fetching @ retry");
    assert_eq!(ts[9], "paused.fetching -> resuming @ wake");
    // Undo restores the original bare spellings (checked by `ok`), so the
    // inverse carries the exact entries.
    let EditOp::Batch(ops) = &applied.inverse else { panic!("expected a batch inverse, got {:?}", applied.inverse) };
    assert_eq!(ops[0], rename_state("Job", "paused.fetching", "waiting"));
}

#[test]
fn rename_to_a_top_level_name_that_captures_bare_references() {
    // A top-level `computing` would win over the bare `computing` that means
    // `running.computing`.
    let def = parse(NESTED);
    let applied = ok(&def, rename_state("Job", "failed", "computing"));
    let ts = transitions(&applied.definition, "Job");
    assert_eq!(ts[1], "fetching -> running.computing @ fetched");
    assert_eq!(ts[2], "running -> computing @ crash");
    assert_eq!(ts[4], "computing -> hist @ resume");
    let model = resolve(applied.definition.clone()).expect("resolves");
    let job = model.machine_by_name("Job").expect("Job");
    let top = model.state_by_path(job, "computing").expect("top-level computing");
    let nested = model.state_by_path(job, "running.computing").expect("nested computing");
    let fetched = model.machine(job).transitions[1];
    assert_eq!(model.transition(fetched).to, nested);
    assert_ne!(top, nested);
}

#[test]
fn rename_state_rejections() {
    let def = parse(NESTED);
    assert_eq!(
        rejected(&def, rename_state("Job", "running.fetching", "computing")),
        EditError::NameTaken { what: "state", name: "Job.running.computing".into() }
    );
    assert_eq!(rejected(&def, rename_state("Job", "running", "a.b")), EditError::InvalidName("a.b".into()));
    assert_eq!(
        rejected(&def, rename_state("Job", "sleeping", "x")),
        EditError::NotFound { what: "state", name: "Job.sleeping".into() }
    );
    let same = ok(&def, rename_state("Job", "running", "running"));
    assert_same(&same.definition, &def, "no-op rename");
}

// --- Kind and initial -------------------------------------------------------------

#[test]
fn set_state_kind() {
    let def = parse(ORDER_FULFILLMENT);
    let applied =
        ok(&def, EditOp::SetStateKind { machine: "Order".into(), path: "cancelled".into(), kind: StateKindDef::Final });
    assert_eq!(state(&applied.definition, "Order", "cancelled").map(|s| s.kind.value), Some(StateKindDef::Final));
    assert_eq!(applied.touched, [state_key("Order", "cancelled")]);

    let nested = parse(NESTED);
    let err = rejected(
        &nested,
        EditOp::SetStateKind { machine: "Job".into(), path: "failed".into(), kind: StateKindDef::Final },
    );
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::TransitionFromFinal { .. })), "{err:?}");
    let err = rejected(
        &nested,
        EditOp::SetStateKind { machine: "Job".into(), path: "running".into(), kind: StateKindDef::DeepHistory },
    );
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::ChildrenNotAllowed { .. })), "{err:?}");
    ok(
        &nested,
        EditOp::SetStateKind { machine: "Job".into(), path: "running.hist".into(), kind: StateKindDef::DeepHistory },
    );
}

#[test]
fn set_state_initial() {
    let def = parse(NESTED);
    let applied = ok(
        &def,
        EditOp::SetStateInitial { machine: "Job".into(), path: "running".into(), initial: Some("computing".into()) },
    );
    assert_eq!(state_initial(&applied.definition, "Job", "running").as_deref(), Some("computing"));
    let applied = ok(&def, EditOp::SetStateInitial { machine: "Job".into(), path: "running".into(), initial: None });
    assert_eq!(state_initial(&applied.definition, "Job", "running"), None);
    ok(
        &def,
        EditOp::SetStateInitial { machine: "Job".into(), path: "paused".into(), initial: Some("resuming".into()) },
    );

    let err = rejected(
        &def,
        EditOp::SetStateInitial { machine: "Job".into(), path: "running".into(), initial: Some("waiting".into()) },
    );
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::InitialNotChild { .. })), "{err:?}");
    let err = rejected(
        &def,
        EditOp::SetStateInitial { machine: "Job".into(), path: "running".into(), initial: Some("hist".into()) },
    );
    assert!(invalid(&err, |k| matches!(k, DiagnosticKind::InitialIsHistory { .. })), "{err:?}");
    assert_eq!(
        rejected(
            &def,
            EditOp::SetStateInitial { machine: "Job".into(), path: "running".into(), initial: Some("a b".into()) }
        ),
        EditError::InvalidName("a b".into())
    );
    assert_eq!(
        rejected(&def, EditOp::SetStateInitial { machine: "Job".into(), path: "gone".into(), initial: None }),
        EditError::NotFound { what: "state", name: "Job.gone".into() }
    );
}
