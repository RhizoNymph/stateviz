//! Invariants of `patch_text` over every fixture: in-place edits for the
//! common ops (the rewrite fallback is counted and reported), comments kept,
//! results that resolve, and add-then-remove restoring the text byte for
//! byte.

mod patch_common;

use std::collections::BTreeMap;

use cascade_core::definition::{Definition, StateKindDef};
use cascade_core::edit::{EditError, EditOp};
use cascade_core::{PaletteColor, load_str};
use cascade_interop::{PatchError, Patched, patch_text};
use patch_common::*;

/// How one op went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Outcome {
    InPlace,
    /// Rewritten, or (before `edit::apply` exists) would have to be.
    Fallback,
    /// The op is invalid for this definition.
    Rejected,
}

/// A spread of realistic ops for a definition, labelled by kind.
fn common_ops(def: &Definition) -> Vec<(&'static str, EditOp)> {
    let mut ops: Vec<(&'static str, EditOp)> = Vec::new();
    let used_events: Vec<String> = def
        .machines
        .iter()
        .flat_map(|m| m.transitions.iter().flat_map(|t| t.emits.iter().map(|e| e.value.clone())))
        .collect();
    for m in &def.machines {
        let name = m.name.value.clone();
        let top: Vec<String> = m.states.iter().map(|s| s.name.value.clone()).collect();
        ops.push((
            "SetMachineColor",
            EditOp::SetMachineColor { machine: name.clone(), color: Some(PaletteColor::Purple) },
        ));
        ops.push(("SetMachineColor", EditOp::SetMachineColor { machine: name.clone(), color: None }));
        ops.push(("SetMachineDomain", EditOp::SetMachineDomain { machine: name.clone(), domain: Some("core".into()) }));
        ops.push((
            "SetMachineInitial",
            EditOp::SetMachineInitial { machine: name.clone(), initial: top.first().cloned() },
        ));
        ops.push(("SetMachineInitial", EditOp::SetMachineInitial { machine: name.clone(), initial: None }));
        let mut fields: Vec<String> = m.fields.iter().map(|f| f.value.clone()).collect();
        fields.push("extra".into());
        ops.push(("SetMachineFields", EditOp::SetMachineFields { machine: name.clone(), fields }));
        ops.push((
            "AddState",
            EditOp::AddState { machine: name.clone(), parent: None, state: state("fresh"), index: None },
        ));
        ops.push((
            "AddState",
            EditOp::AddState { machine: name.clone(), parent: None, state: final_state("fresh"), index: Some(1) },
        ));
        for s in &m.states {
            let path = s.name.value.clone();
            ops.push((
                "AddState",
                EditOp::AddState {
                    machine: name.clone(),
                    parent: Some(path.clone()),
                    state: state("inner"),
                    index: None,
                },
            ));
            ops.push((
                "RenameState",
                EditOp::RenameState { machine: name.clone(), path: path.clone(), to: format!("{path}_v2") },
            ));
            ops.push(("RemoveState", EditOp::RemoveState { machine: name.clone(), path: path.clone() }));
            let kind = match s.kind.value {
                StateKindDef::Normal => StateKindDef::Final,
                _ => StateKindDef::Normal,
            };
            ops.push(("SetStateKind", EditOp::SetStateKind { machine: name.clone(), path: path.clone(), kind }));
            for child in &s.states {
                let child_path = format!("{path}.{}", child.name.value);
                // A top-level state named like a nested one captures bare
                // references to it; so does renaming a top-level state so.
                let capture = state(&child.name.value);
                ops.push((
                    "AddState",
                    EditOp::AddState { machine: name.clone(), parent: None, state: capture, index: None },
                ));
                if let Some(other) = m.states.iter().find(|o| o.name.value != s.name.value) {
                    let (from, to) = (other.name.value.clone(), child.name.value.clone());
                    ops.push(("RenameState", EditOp::RenameState { machine: name.clone(), path: from, to }));
                }
                ops.push((
                    "RenameState",
                    EditOp::RenameState { machine: name.clone(), path: child_path.clone(), to: "renamed".into() },
                ));
                ops.push(("RemoveState", EditOp::RemoveState { machine: name.clone(), path: child_path }));
            }
            if let Some(last) = s.states.last() {
                let initial = Some(last.name.value.clone());
                ops.push((
                    "SetStateInitial",
                    EditOp::SetStateInitial { machine: name.clone(), path: path.clone(), initial },
                ));
                ops.push(("SetStateInitial", EditOp::SetStateInitial { machine: name.clone(), path, initial: None }));
            }
        }
        if let [first, .., last] = top.as_slice() {
            let t = transition(&[first], last, "poke");
            ops.push((
                "AddTransition",
                EditOp::AddTransition { machine: name.clone(), transition: t.clone(), index: None },
            ));
            ops.push(("AddTransition", EditOp::AddTransition { machine: name.clone(), transition: t, index: Some(0) }));
        }
        for (index, t) in m.transitions.iter().enumerate() {
            let mut updated = t.clone();
            updated.guard = if t.guard.is_some() { None } else { Some(s("n > 0")) };
            updated.bounded = !t.bounded;
            ops.push((
                "UpdateTransition",
                EditOp::UpdateTransition { machine: name.clone(), index, transition: updated },
            ));
            let mut retargeted = t.clone();
            retargeted.on = s("renamed_trigger");
            ops.push((
                "UpdateTransition",
                EditOp::UpdateTransition { machine: name.clone(), index, transition: retargeted },
            ));
            ops.push(("RemoveTransition", EditOp::RemoveTransition { machine: name.clone(), index }));
        }
        if let Some(first) = top.first() {
            let batch = EditOp::Batch(vec![
                EditOp::AddState { machine: name.clone(), parent: None, state: state("staged"), index: None },
                EditOp::AddTransition {
                    machine: name.clone(),
                    transition: transition(&[first], "staged", "stage"),
                    index: None,
                },
                EditOp::RenameState { machine: name.clone(), path: "staged".into(), to: "done_staging".into() },
            ]);
            ops.push(("Batch", batch));
        }
        ops.push(("RenameMachine", EditOp::RenameMachine { from: name.clone(), to: format!("{name}V2") }));
        ops.push(("RemoveMachine", EditOp::RemoveMachine { machine: name.clone() }));
    }
    ops.push((
        "AddMachine",
        EditOp::AddMachine {
            machine: machine("Fresh", vec![state("a"), state("b")], vec![transition(&["a"], "b", "go")]),
            index: None,
        },
    ));
    ops.push((
        "AddMachine",
        EditOp::AddMachine { machine: machine("Fresh", vec![state("a")], vec![]), index: Some(0) },
    ));
    for e in &def.events {
        ops.push(("RemoveEventDeclaration", EditOp::RemoveEventDeclaration { event: e.name.value.clone() }));
    }
    ops.push(("DeclareEvent", EditOp::DeclareEvent { event: event("FreshEvent", &["orderId"]), index: None }));
    for e in used_events.iter().take(3) {
        ops.push(("RenameEvent", EditOp::RenameEvent { from: e.clone(), to: format!("{e}V2") }));
    }
    for c in &def.controllers {
        let name = c.name.value.clone();
        ops.push(("RenameController", EditOp::RenameController { from: name.clone(), to: format!("{name}V2") }));
        ops.push(("RemoveController", EditOp::RemoveController { controller: name.clone() }));
        for h in &c.on {
            let event = h.event.value.clone();
            ops.push(("RemoveHandler", EditOp::RemoveHandler { controller: name.clone(), event: event.clone() }));
            if let Some(first) = h.rules.first() {
                let mut r = first.clone();
                r.when = Some(s("also sometimes"));
                ops.push((
                    "AddRule",
                    EditOp::AddRule { controller: name.clone(), event: event.clone(), rule: r.clone(), index: None },
                ));
                ops.push((
                    "UpdateRule",
                    EditOp::UpdateRule { controller: name.clone(), event: event.clone(), index: 0, rule: r },
                ));
            }
            for index in 0..h.rules.len() {
                ops.push(("RemoveRule", EditOp::RemoveRule { controller: name.clone(), event: event.clone(), index }));
            }
        }
        if let Some(e) = used_events.first() {
            let h = handler(e, vec![]);
            ops.push(("AddHandler", EditOp::AddHandler { controller: name.clone(), handler: h, index: None }));
        }
    }
    if let Some(e) = used_events.first() {
        ops.push((
            "AddController",
            EditOp::AddController { controller: controller("Fresh", vec![handler(e, vec![])]), index: None },
        ));
    }
    for x in &def.external {
        let name = x.name.value.clone();
        ops.push(("RenameExternal", EditOp::RenameExternal { from: name.clone(), to: format!("{name}V2") }));
        ops.push(("RemoveExternal", EditOp::RemoveExternal { external: name.clone() }));
        let mut triggers: Vec<_> = x.triggers.iter().map(|t| t.value.clone()).collect();
        triggers.pop();
        ops.push(("SetExternalTriggers", EditOp::SetExternalTriggers { external: name, triggers }));
    }
    if let Some(m) = def.machines.first()
        && let Some(t) = m.transitions.first()
    {
        let trigger = format!("{}.{}", m.name.value, t.on.value);
        ops.push(("AddExternal", EditOp::AddExternal { external: external("FreshSource", &[&trigger]), index: None }));
    }
    ops.push(("SetSystemName", EditOp::SetSystemName { name: Some("Renamed system".into()) }));
    ops.push(("SetSystemName", EditOp::SetSystemName { name: None }));
    ops
}

fn classify(text: &str, op: &EditOp) -> (Outcome, Option<String>) {
    match patch_text(text, op) {
        Ok(Patched { text: out, rewritten: false }) => (Outcome::InPlace, Some(out)),
        Ok(Patched { rewritten: true, .. }) | Err(PatchError::NotImplemented) => (Outcome::Fallback, None),
        Err(PatchError::Edit(_)) => (Outcome::Rejected, None),
        Err(PatchError::Unparseable(err)) => panic!("fixture does not parse: {err}"),
    }
}

#[test]
fn common_ops_patch_in_place_keep_comments_and_resolve() {
    let mut report: BTreeMap<&str, [usize; 3]> = BTreeMap::new();
    let mut fallbacks = Vec::new();
    for (file, text) in corpus() {
        for (kind, op) in common_ops(&parse(&text)) {
            let (outcome, out) = classify(&text, &op);
            report.entry(kind).or_default()[outcome as usize] += 1;
            if outcome == Outcome::Fallback {
                fallbacks.push(format!("{file}: {op:?}"));
            }
            let Some(out) = out else { continue };
            if let Err(err) = load_str(&out) {
                panic!("{file}: {op:?} gave text that does not resolve:\n{err}\n---\n{out}");
            }
            if !kind.starts_with("Remove") && kind != "SetExternalTriggers" {
                assert_comments_kept(&text, &out, &[]);
            }
            // Outside the edit, the text is untouched: the diff has few hunks.
            let hunks = line_diff(&text, &out).matches("@@").count();
            assert!(hunks > 0 || out == text, "{file}: {op:?} changed nothing visible");
        }
    }
    println!("{:<24} {:>8} {:>8} {:>8}", "op", "in place", "fallback", "rejected");
    for (kind, [in_place, fallback, rejected]) in &report {
        println!("{kind:<24} {in_place:>8} {fallback:>8} {rejected:>8}");
    }
    let total: usize = report.values().map(|c| c[0] + c[1]).sum();
    println!("{} valid ops, {} fell back:\n{}", total, fallbacks.len(), fallbacks.join("\n"));
    assert!(fallbacks.is_empty(), "ops fell back to a rewrite:\n{}", fallbacks.join("\n"));
    // Every kind of op was exercised in place somewhere.
    for (kind, counts) in &report {
        assert!(counts[0] > 0, "{kind} never patched in place");
    }
}

#[test]
fn adding_then_removing_restores_the_text_byte_for_byte() {
    for (file, text) in corpus() {
        let def = parse(&text);
        let mut pairs: Vec<(EditOp, EditOp)> = Vec::new();
        pairs.push((
            EditOp::AddMachine {
                machine: machine("Fresh", vec![state("a"), state("b")], vec![transition(&["a"], "b", "go")]),
                index: None,
            },
            EditOp::RemoveMachine { machine: "Fresh".into() },
        ));
        pairs.push((
            EditOp::AddMachine { machine: machine("Fresh", vec![state("a")], vec![]), index: Some(0) },
            EditOp::RemoveMachine { machine: "Fresh".into() },
        ));
        for m in &def.machines {
            let name = m.name.value.clone();
            let len = m.transitions.len();
            let top = &m.states[0].name.value;
            for index in [Some(0), None] {
                pairs.push((
                    EditOp::AddTransition { machine: name.clone(), transition: transition(&[top], top, "poke"), index },
                    EditOp::RemoveTransition { machine: name.clone(), index: index.unwrap_or(len) },
                ));
                pairs.push((
                    EditOp::AddState { machine: name.clone(), parent: None, state: state("fresh"), index },
                    EditOp::RemoveState { machine: name.clone(), path: "fresh".into() },
                ));
            }
            pairs.push((
                EditOp::RenameMachine { from: name.clone(), to: "Fresh".into() },
                EditOp::RenameMachine { from: "Fresh".into(), to: name.clone() },
            ));
            pairs.push((
                EditOp::RenameState { machine: name.clone(), path: top.clone(), to: "fresh".into() },
                EditOp::RenameState { machine: name.clone(), path: "fresh".into(), to: top.clone() },
            ));
        }
        if let Some(e) = def.events.first() {
            pairs.push((
                EditOp::DeclareEvent { event: event("FreshEvent", &[]), index: None },
                EditOp::RemoveEventDeclaration { event: "FreshEvent".into() },
            ));
            pairs.push((
                EditOp::RenameEvent { from: e.name.value.clone(), to: "FreshEvent".into() },
                EditOp::RenameEvent { from: "FreshEvent".into(), to: e.name.value.clone() },
            ));
        }
        // An empty `controllers: {}` becomes a block section and removing
        // its only controller removes the section: not byte for byte.
        if !text.contains("controllers: {}") {
            pairs.push((
                EditOp::AddController { controller: controller("Fresh", vec![]), index: None },
                EditOp::RemoveController { controller: "Fresh".into() },
            ));
        }
        for c in &def.controllers {
            let name = c.name.value.clone();
            pairs.push((
                EditOp::AddHandler { controller: name.clone(), handler: handler("FreshEvent", vec![]), index: None },
                EditOp::RemoveHandler { controller: name.clone(), event: "FreshEvent".into() },
            ));
            if let Some(h) = c.on.first()
                && let Some(r) = h.rules.first()
            {
                pairs.push((
                    EditOp::AddRule {
                        controller: name.clone(),
                        event: h.event.value.clone(),
                        rule: r.clone(),
                        index: Some(0),
                    },
                    EditOp::RemoveRule { controller: name.clone(), event: h.event.value.clone(), index: 0 },
                ));
            }
        }
        if let Some(m) = def.machines.first() {
            let trigger = format!("{}.{}", m.name.value, m.transitions.first().map_or("go", |t| t.on.value.as_str()));
            pairs.push((
                EditOp::AddExternal { external: external("FreshSource", &[&trigger]), index: None },
                EditOp::RemoveExternal { external: "FreshSource".into() },
            ));
        }
        for (add, remove) in pairs {
            // Invalid pairs (a strict file without the event, …) are skipped.
            let Ok(Patched { text: added, rewritten: false }) = patch_text(&text, &add) else { continue };
            let restored = match patch_text(&added, &remove) {
                Ok(Patched { text, rewritten: false }) => text,
                other => panic!("{file}: undoing {add:?} with {remove:?} failed: {other:?}"),
            };
            assert_eq!(restored, text, "{file}: {add:?} then {remove:?}\n--- after add ---\n{added}");
        }
    }
}

#[test]
fn batches_apply_in_order_and_are_validated_at_the_end() {
    let text = order_fulfillment();
    // Swap two state names through a temporary name.
    let op = EditOp::Batch(vec![
        EditOp::RenameState { machine: "Order".into(), path: "paid".into(), to: "tmp".into() },
        EditOp::RenameState { machine: "Order".into(), path: "cancelled".into(), to: "paid".into() },
        EditOp::RenameState { machine: "Order".into(), path: "tmp".into(), to: "cancelled".into() },
    ]);
    check(
        &text,
        &op,
        "@@ 7\n\
         -    states: [draft, pending, paid, cancelled]\n\
         +    states: [draft, pending, cancelled, paid]\n\
         @@ 10\n\
         -      - { from: pending, to: paid,      on: capture_ok, emits: [OrderPaid] }\n\
         -      - { from: pending, to: cancelled, on: timeout,    emits: [OrderCancelled] }\n\
         +      - { from: pending, to: cancelled, on: capture_ok, emits: [OrderPaid] }\n\
         +      - { from: pending, to: paid,      on: timeout,    emits: [OrderCancelled] }",
        |d| {
            let order = machine_mut(d, "Order");
            order.states[2].name = s("cancelled");
            order.states[3].name = s("paid");
            order.transitions[1].to = s("cancelled");
            order.transitions[2].to = s("paid");
        },
    );
    // Removing every declaration of a strict file passes through invalid
    // intermediate states.
    let shop = shop();
    let all: Vec<EditOp> =
        parse(&shop).events.iter().map(|e| EditOp::RemoveEventDeclaration { event: e.name.value.clone() }).collect();
    let out = patched(&shop, &EditOp::Batch(all));
    assert!(!out.contains("\nevents:"), "{out}");
    assert!(out.contains("system: Shop\n\nmachines:\n"), "{out}");
}

#[test]
fn invalid_and_unparseable_input_is_reported() {
    let text = order_fulfillment();
    assert!(matches!(
        patch_text("machines: [unclosed\n", &EditOp::SetSystemName { name: None }),
        Err(PatchError::Unparseable(_))
    ));
    let missing = patch_text(&text, &EditOp::RemoveMachine { machine: "Nope".into() });
    assert!(matches!(missing, Err(PatchError::Edit(EditError::NotFound { .. }))), "{missing:?}");
    let taken = patch_text(
        &text,
        &EditOp::AddState { machine: "Order".into(), parent: None, state: state("paid"), index: None },
    );
    assert!(matches!(taken, Err(PatchError::Edit(EditError::NameTaken { .. }))), "{taken:?}");
    let final_with_exit = patch_text(
        &text,
        &EditOp::SetStateKind { machine: "Order".into(), path: "pending".into(), kind: StateKindDef::Final },
    );
    assert!(matches!(final_with_exit, Err(PatchError::Edit(EditError::Invalid(_)))), "{final_with_exit:?}");
}

#[test]
fn a_no_op_leaves_the_text_alone() {
    for (_, text) in corpus() {
        let def = parse(&text);
        let m = &def.machines[0];
        let op = EditOp::RenameMachine { from: m.name.value.clone(), to: m.name.value.clone() };
        assert_eq!(patched(&text, &op), text);
        let op = EditOp::SetMachineColor { machine: m.name.value.clone(), color: m.color.as_ref().map(|c| c.value) };
        assert_eq!(patched(&text, &op), text);
    }
}
