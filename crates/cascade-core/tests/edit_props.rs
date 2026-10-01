//! The inverse law across every op on every fixture, and across random op
//! sequences: `apply(apply(d, op).definition, inverse).definition == d`
//! (ignoring spans), redo reproduces the edit, a whole undo stack restores
//! the start, and a batch of the same ops behaves like the sequence.

mod edit_support;

use cascade_core::definition::{Definition, StateDef, StateKindDef};
use cascade_core::edit::{EditError, EditOp, apply};
use cascade_core::{PaletteColor, resolve};
use edit_support::*;
use proptest::prelude::*;
use proptest::sample::Index;

/// Every state path of a machine, pre-order.
fn state_paths(states: &[StateDef], prefix: Option<&str>, out: &mut Vec<(String, String)>) {
    for s in states {
        let path = match prefix {
            Some(p) => format!("{p}.{}", s.name.value),
            None => s.name.value.clone(),
        };
        out.push((path.clone(), s.name.value.clone()));
        state_paths(&s.states, Some(&path), out);
    }
}

fn used_events(def: &Definition) -> Vec<String> {
    let mut out: Vec<String> = def.events.iter().map(|e| e.name.value.clone()).collect();
    let emitted = def.machines.iter().flat_map(|m| m.transitions.iter().flat_map(|t| t.emits.iter()));
    let handled = def.controllers.iter().flat_map(|c| c.on.iter().map(|h| &h.event));
    for name in emitted.chain(handled) {
        if !out.contains(&name.value) {
            out.push(name.value.clone());
        }
    }
    out
}

/// A broad set of ops against `def`: every element removed, renamed to a
/// fresh name and to colliding names, setters with each value, additions at
/// several positions. Many are rejected; the rest must satisfy the laws.
fn candidate_ops(def: &Definition) -> Vec<EditOp> {
    let mut ops = Vec::new();
    let machine_names: Vec<String> = def.machines.iter().map(|m| m.name.value.clone()).collect();
    let events = used_events(def);

    ops.push(EditOp::SetSystemName { name: Some("Renamed".into()) });
    ops.push(EditOp::SetSystemName { name: None });
    ops.push(EditOp::AddMachine {
        machine: machine_def("Extra", &["a", "b"], vec![transition(&["a"], "b", "go")]),
        index: Some(0),
    });

    for m in &def.machines {
        let name = m.name.value.clone();
        let mut paths = Vec::new();
        state_paths(&m.states, None, &mut paths);
        let locals: Vec<String> = paths.iter().map(|(_, local)| local.clone()).collect();

        ops.push(EditOp::RemoveMachine { machine: name.clone() });
        ops.push(EditOp::RenameMachine { from: name.clone(), to: "Renamed".into() });
        for other in &machine_names {
            ops.push(EditOp::RenameMachine { from: name.clone(), to: other.clone() });
        }
        ops.push(EditOp::SetMachineColor { machine: name.clone(), color: Some(PaletteColor::Purple) });
        ops.push(EditOp::SetMachineColor { machine: name.clone(), color: None });
        ops.push(EditOp::SetMachineDomain { machine: name.clone(), domain: Some("ops".into()) });
        ops.push(EditOp::SetMachineDomain { machine: name.clone(), domain: None });
        ops.push(EditOp::SetMachineInitial { machine: name.clone(), initial: None });
        ops.push(EditOp::SetMachineFields { machine: name.clone(), fields: Vec::new() });
        ops.push(EditOp::SetMachineFields {
            machine: name.clone(),
            fields: vec!["orderId".into(), "jobId".into(), "customerId".into()],
        });
        for (path, local) in &paths {
            ops.push(EditOp::SetMachineInitial { machine: name.clone(), initial: Some(path.clone()) });
            ops.push(EditOp::SetMachineInitial { machine: name.clone(), initial: Some(local.clone()) });
        }
        for new_name in std::iter::once("fresh").chain(locals.iter().map(String::as_str)) {
            ops.push(EditOp::AddState { machine: name.clone(), parent: None, state: leaf(new_name), index: None });
            ops.push(EditOp::AddState { machine: name.clone(), parent: None, state: leaf(new_name), index: Some(0) });
        }

        for (path, _) in &paths {
            ops.push(EditOp::RemoveState { machine: name.clone(), path: path.clone() });
            for to in std::iter::once("fresh").chain(locals.iter().map(String::as_str)) {
                ops.push(EditOp::RenameState { machine: name.clone(), path: path.clone(), to: to.into() });
                ops.push(EditOp::AddState {
                    machine: name.clone(),
                    parent: Some(path.clone()),
                    state: leaf(to),
                    index: Some(0),
                });
            }
            for kind in [StateKindDef::Normal, StateKindDef::Final, StateKindDef::History, StateKindDef::DeepHistory] {
                ops.push(EditOp::SetStateKind { machine: name.clone(), path: path.clone(), kind });
            }
            ops.push(EditOp::SetStateInitial { machine: name.clone(), path: path.clone(), initial: None });
            for local in &locals {
                ops.push(EditOp::SetStateInitial {
                    machine: name.clone(),
                    path: path.clone(),
                    initial: Some(local.clone()),
                });
            }
            ops.push(EditOp::AddTransition {
                machine: name.clone(),
                transition: transition(&[path], &paths[0].0, "go"),
                index: Some(0),
            });
        }

        for (i, t) in m.transitions.iter().enumerate() {
            ops.push(EditOp::RemoveTransition { machine: name.clone(), index: i });
            let mut changed = t.clone();
            changed.guard = if t.guard.is_some() { None } else { Some(s("maybe")) };
            changed.bounded = !t.bounded;
            ops.push(EditOp::UpdateTransition { machine: name.clone(), index: i, transition: changed });
            let mut retargeted = t.clone();
            retargeted.to = s(&paths[0].0);
            ops.push(EditOp::UpdateTransition { machine: name.clone(), index: i, transition: retargeted });
        }
    }

    ops.push(EditOp::DeclareEvent { event: event_def("Fresh", &["orderId"]), index: Some(0) });
    ops.push(EditOp::DeclareEvent { event: event_def("Fresh", &[]), index: None });
    for event in &events {
        ops.push(EditOp::DeclareEvent { event: event_def(event, &["id"]), index: None });
        ops.push(EditOp::RemoveEventDeclaration { event: event.clone() });
        ops.push(EditOp::RenameEvent { from: event.clone(), to: "Fresh".into() });
        for other in &events {
            ops.push(EditOp::RenameEvent { from: event.clone(), to: other.clone() });
        }
    }

    for c in &def.controllers {
        let name = c.name.value.clone();
        ops.push(EditOp::RemoveController { controller: name.clone() });
        ops.push(EditOp::RenameController { from: name.clone(), to: "Fresh".into() });
        if let Some(first) = def.controllers.first() {
            ops.push(EditOp::RenameController { from: name.clone(), to: first.name.value.clone() });
        }
        for event in &events {
            ops.push(EditOp::AddHandler {
                controller: name.clone(),
                handler: handler_def(event, vec![]),
                index: Some(0),
            });
        }
        for h in &c.on {
            let event = h.event.value.clone();
            ops.push(EditOp::RemoveHandler { controller: name.clone(), event: event.clone() });
            for (i, r) in h.rules.iter().enumerate() {
                ops.push(EditOp::RemoveRule { controller: name.clone(), event: event.clone(), index: i });
                let mut changed = r.clone();
                changed.bounded = !r.bounded;
                changed.when = None;
                ops.push(EditOp::UpdateRule {
                    controller: name.clone(),
                    event: event.clone(),
                    index: i,
                    rule: changed,
                });
                ops.push(EditOp::AddRule {
                    controller: name.clone(),
                    event: event.clone(),
                    rule: r.clone(),
                    index: None,
                });
            }
        }
    }
    ops.push(EditOp::AddController { controller: controller_def("Fresh", vec![]), index: None });

    for x in &def.external {
        let name = x.name.value.clone();
        ops.push(EditOp::RemoveExternal { external: name.clone() });
        ops.push(EditOp::RenameExternal { from: name.clone(), to: "Fresh".into() });
        ops.push(EditOp::SetExternalTriggers { external: name.clone(), triggers: Vec::new() });
        let reversed = x.triggers.iter().rev().map(|t| t.value.clone()).collect();
        ops.push(EditOp::SetExternalTriggers { external: name.clone(), triggers: reversed });
    }
    ops.push(EditOp::AddExternal { external: external_def("Fresh", &[]), index: Some(0) });

    ops
}

fn check_laws(def: &Definition, op: &EditOp) -> Result<Definition, EditError> {
    let before = def.clone();
    let result = apply(def, op);
    assert_eq!(def, &before, "apply must not modify its input");
    let applied = result?;
    assert_resolves(&applied.definition);
    assert_inverse(def, &applied);
    Ok(applied.definition)
}

#[test]
fn every_candidate_op_on_every_fixture_obeys_the_inverse_law() {
    let mut applied = 0usize;
    let mut rejected_count = 0usize;
    for (name, def) in fixtures() {
        for op in candidate_ops(&def) {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check_laws(&def, &op)));
            match result {
                Ok(Ok(_)) => applied += 1,
                Ok(Err(_)) => rejected_count += 1,
                Err(panic) => {
                    eprintln!("fixture {name}, op {op:?}");
                    std::panic::resume_unwind(panic);
                }
            }
        }
    }
    assert!(applied > 1000, "only {applied} ops applied");
    assert!(rejected_count > 100, "only {rejected_count} ops rejected");
}

#[test]
fn pairs_of_ops_as_batches_obey_the_inverse_law() {
    for (_, def) in fixtures() {
        let ops = candidate_ops(&def);
        // A deterministic spread of pairs.
        for (i, first) in ops.iter().enumerate().step_by(7) {
            let second = &ops[(i * 31 + 5) % ops.len()];
            let batch = EditOp::Batch(vec![first.clone(), second.clone()]);
            let _ = check_laws(&def, &batch);
        }
    }
}

fn fixture_strategy() -> impl Strategy<Value = usize> {
    0..fixtures().len()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn random_op_sequences_undo_and_batch_cleanly(
        fixture in fixture_strategy(),
        picks in proptest::collection::vec(any::<Index>(), 1..8),
    ) {
        let (_, start) = fixtures().swap_remove(fixture);
        let mut current = start.clone();
        let mut done = Vec::new();
        let mut undo = Vec::new();
        for pick in picks {
            let ops = candidate_ops(&current);
            let op = ops[pick.index(ops.len())].clone();
            if let Ok(applied) = apply(&current, &op) {
                assert_resolves(&applied.definition);
                assert_inverse(&current, &applied);
                undo.push(applied.inverse);
                done.push(op);
                current = applied.definition;
            }
        }

        // Undoing everything restores the start.
        let mut rewound = current.clone();
        for inverse in undo.iter().rev() {
            rewound = match apply(&rewound, inverse) {
                Ok(applied) => applied.definition,
                Err(err) => panic!("undo {inverse:?} failed: {err}"),
            };
        }
        assert_same(&rewound, &start, "the undo stack should restore the start");

        // The same ops as one batch give the same result, and its inverse
        // restores the start.
        let batch = match apply(&start, &EditOp::Batch(done)) {
            Ok(applied) => applied,
            Err(err) => panic!("the batch of successful ops failed: {err}"),
        };
        assert_same(&batch.definition, &current, "the batch should match the sequence");
        assert_inverse(&start, &batch);
        prop_assert!(resolve(batch.definition).is_ok());
    }
}
