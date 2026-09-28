//! Helpers for the YAML patch tests: fixtures, a line diff for golden
//! expectations, comment bookkeeping and a span-free structural check.
//!
//! `cascade_core::edit::apply` is implemented on a sibling branch. Until it
//! lands, `patch_text` cannot compare its output with core's semantics, so
//! the tests carry their own expectation: each case states the definition
//! it expects as a mutation of the parsed original ([`expect_definition`]),
//! compared through `to_yaml`, which ignores spans. Once `apply` works,
//! `patch_text` checks the same thing internally and these expectations
//! double as a cross-check of the two implementations.

#![allow(dead_code)]

use std::path::Path;

use cascade_core::definition::{
    ControllerDef, Definition, EventDef, ExternalDef, HandlerDef, MachineDef, RuleDef, StateDef, StateKindDef,
    TargetSpec, TransitionDef, TriggerRef,
};
use cascade_core::edit::EditOp;
use cascade_core::parse::grammar::{parse_target, parse_trigger_ref};
use cascade_core::span::SourceSpan;
use cascade_core::{Spanned, load_str, parse_definition};
use cascade_interop::{PatchError, Patched, patch_text, to_yaml};

pub fn workspace_file(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(relative);
    match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => panic!("cannot read {}: {err}", path.display()),
    }
}

pub fn fixture(name: &str) -> String {
    workspace_file(&format!("crates/cascade-interop/tests/fixtures/patch/{name}"))
}

pub fn shop() -> String {
    workspace_file("examples/shop/cascade.yaml")
}

pub fn order_fulfillment() -> String {
    workspace_file("examples/order-fulfillment/cascade.yaml")
}

/// Every text the tests patch, by name.
pub fn corpus() -> Vec<(&'static str, String)> {
    vec![
        ("shop", shop()),
        ("order-fulfillment", order_fulfillment()),
        ("nested", fixture("nested.yaml")),
        ("flow", fixture("flow.yaml")),
        ("block", fixture("block.yaml")),
    ]
}

// --- Building ops ---------------------------------------------------------------

pub fn s(value: &str) -> Spanned<String> {
    Spanned::synthetic(value.to_owned())
}

pub fn state(name: &str) -> StateDef {
    StateDef {
        name: s(name),
        kind: Spanned::synthetic(StateKindDef::Normal),
        initial: None,
        states: Vec::new(),
        span: SourceSpan::unknown(),
    }
}

pub fn final_state(name: &str) -> StateDef {
    StateDef { kind: Spanned::synthetic(StateKindDef::Final), ..state(name) }
}

pub fn compound(name: &str, initial: Option<&str>, children: Vec<StateDef>) -> StateDef {
    StateDef { initial: initial.map(s), states: children, ..state(name) }
}

pub fn transition(from: &[&str], to: &str, on: &str) -> TransitionDef {
    TransitionDef {
        from: from.iter().map(|f| s(f)).collect(),
        to: s(to),
        on: s(on),
        guard: None,
        emits: Vec::new(),
        bounded: false,
        span: SourceSpan::unknown(),
    }
}

pub fn emitting(mut t: TransitionDef, emits: &[&str]) -> TransitionDef {
    t.emits = emits.iter().map(|e| s(e)).collect();
    t
}

pub fn guarded(mut t: TransitionDef, guard: &str) -> TransitionDef {
    t.guard = Some(s(guard));
    t
}

pub fn trigger(text: &str) -> TriggerRef {
    match parse_trigger_ref(text) {
        Some(r) => r,
        None => panic!("bad trigger ref {text}"),
    }
}

pub fn target(text: &str) -> Spanned<TargetSpec> {
    match parse_target(text) {
        Ok(spec) => Spanned::synthetic(spec),
        Err(err) => panic!("bad selector {text}: {err:?}"),
    }
}

pub fn rule(fire: &str, target_text: Option<&str>, when: Option<&str>) -> RuleDef {
    RuleDef {
        fire: Spanned::synthetic(trigger(fire)),
        target: target_text.map(target),
        when: when.map(s),
        bounded: false,
        span: SourceSpan::unknown(),
    }
}

pub fn handler(event: &str, rules: Vec<RuleDef>) -> HandlerDef {
    HandlerDef { event: s(event), rules, span: SourceSpan::unknown() }
}

pub fn controller(name: &str, handlers: Vec<HandlerDef>) -> ControllerDef {
    ControllerDef { name: s(name), on: handlers, span: SourceSpan::unknown() }
}

pub fn event(name: &str, payload: &[&str]) -> EventDef {
    EventDef { name: s(name), payload: payload.iter().map(|p| s(p)).collect(), span: SourceSpan::unknown() }
}

pub fn external(name: &str, triggers: &[&str]) -> ExternalDef {
    ExternalDef {
        name: s(name),
        triggers: triggers.iter().map(|t| Spanned::synthetic(trigger(t))).collect(),
        span: SourceSpan::unknown(),
    }
}

pub fn machine(name: &str, states: Vec<StateDef>, transitions: Vec<TransitionDef>) -> MachineDef {
    MachineDef {
        name: s(name),
        color: None,
        domain: None,
        initial: None,
        fields: Vec::new(),
        states,
        transitions,
        span: SourceSpan::unknown(),
    }
}

// --- Patching ---------------------------------------------------------------------

/// Patch and require an in-place edit whose result resolves.
pub fn patched(text: &str, op: &EditOp) -> String {
    match patch_text(text, op) {
        Ok(Patched { text: out, rewritten: false }) => {
            if let Err(err) = load_str(&out) {
                panic!("patched text does not resolve:\n{err}\n---\n{out}");
            }
            out
        }
        Ok(Patched { rewritten: true, .. }) => panic!("{op:?} fell back to a rewrite"),
        Err(err) => panic!("{op:?} failed: {err}\n---\n{text}"),
    }
}

pub fn try_patch(text: &str, op: &EditOp) -> Result<Patched, PatchError> {
    patch_text(text, op)
}

/// Patch, then compare the line diff with `expected` and the definition
/// with `mutate` applied to the original. Returns the patched text.
pub fn check(text: &str, op: &EditOp, expected_diff: &str, mutate: impl FnOnce(&mut Definition)) -> String {
    let out = patched(text, op);
    let diff = line_diff(text, &out);
    assert_eq!(diff.trim_end(), expected_diff.trim_end(), "diff for {op:?}\n--- patched ---\n{out}");
    expect_definition(text, &out, mutate);
    out
}

/// The patched text parses to the original definition with `mutate`
/// applied, ignoring spans (compared through `to_yaml`).
pub fn expect_definition(original: &str, out: &str, mutate: impl FnOnce(&mut Definition)) {
    let mut expected = parse(original);
    mutate(&mut expected);
    let actual = parse(out);
    assert_eq!(to_yaml(&actual), to_yaml(&expected), "patched definition differs\n--- patched ---\n{out}");
}

pub fn parse(text: &str) -> Definition {
    match parse_definition(text) {
        Ok(def) => def,
        Err(err) => panic!("does not parse:\n{err}\n---\n{text}"),
    }
}

// --- Line diff ---------------------------------------------------------------------------

/// A compact line diff: `@@ N` (1-based line in the old text) before each
/// hunk, then `-old` and `+new` lines.
pub fn line_diff(old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = String::new();
    let mut in_hunk = false;
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            i += 1;
            j += 1;
            in_hunk = false;
            continue;
        }
        if !in_hunk {
            out.push_str(&format!("@@ {}\n", i + 1));
            in_hunk = true;
        }
        if i < a.len() && (j == b.len() || lcs[i + 1][j] >= lcs[i][j + 1]) {
            out.push_str(&format!("-{}\n", a[i]));
            i += 1;
        } else {
            out.push_str(&format!("+{}\n", b[j]));
            j += 1;
        }
    }
    if old.ends_with('\n') != new.ends_with('\n') {
        out.push_str("\\ trailing newline changed\n");
    }
    out
}

// --- Comments ------------------------------------------------------------------------------

/// Every comment in a YAML text (from `#` to the end of the line), ignoring
/// `#` inside quotes.
pub fn comments(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in text.lines() {
        let mut quote: Option<char> = None;
        let mut prev = ' ';
        for (i, c) in line.char_indices() {
            match quote {
                Some(q) if c == q => quote = None,
                Some(_) => {}
                None if c == '"' || c == '\'' => {
                    if prev == ' ' || prev == '[' || prev == '{' || prev == ',' || prev == ':' {
                        quote = Some(c);
                    }
                }
                None if c == '#' && (prev == ' ' || i == 0) => {
                    found.push(line[i..].to_owned());
                    break;
                }
                None => {}
            }
            prev = c;
        }
    }
    found
}

/// Every comment of `old` is in `new`, except those in `removed`.
pub fn assert_comments_kept(old: &str, new: &str, removed: &[&str]) {
    let after = comments(new);
    for comment in comments(old) {
        if removed.iter().any(|r| comment.contains(r)) {
            assert!(!after.contains(&comment), "comment should have been removed: {comment}\n---\n{new}");
        } else {
            assert!(after.contains(&comment), "comment lost: {comment}\n---\n{new}");
        }
    }
}

// --- Expected-definition helpers ------------------------------------------------------

pub fn machine_mut<'d>(def: &'d mut Definition, name: &str) -> &'d mut MachineDef {
    match def.machines.iter_mut().find(|m| m.name.value == name) {
        Some(m) => m,
        None => panic!("no machine {name}"),
    }
}

pub fn states_mut<'d>(states: &'d mut [StateDef], path: &str) -> &'d mut StateDef {
    let (first, rest) = match path.split_once('.') {
        Some((first, rest)) => (first, Some(rest)),
        None => (path, None),
    };
    let Some(found) = states.iter_mut().find(|s| s.name.value == first) else {
        panic!("no state {path}");
    };
    match rest {
        Some(rest) => states_mut(&mut found.states, rest),
        None => found,
    }
}

pub fn state_mut<'d>(def: &'d mut Definition, machine: &str, path: &str) -> &'d mut StateDef {
    states_mut(&mut machine_mut(def, machine).states, path)
}

pub fn controller_mut<'d>(def: &'d mut Definition, name: &str) -> &'d mut ControllerDef {
    match def.controllers.iter_mut().find(|c| c.name.value == name) {
        Some(c) => c,
        None => panic!("no controller {name}"),
    }
}

pub fn handler_mut<'d>(def: &'d mut Definition, controller: &str, event: &str) -> &'d mut HandlerDef {
    match controller_mut(def, controller).on.iter_mut().find(|h| h.event.value == event) {
        Some(h) => h,
        None => panic!("no handler {controller}/{event}"),
    }
}

pub fn external_mut<'d>(def: &'d mut Definition, name: &str) -> &'d mut ExternalDef {
    match def.external.iter_mut().find(|x| x.name.value == name) {
        Some(x) => x,
        None => panic!("no external source {name}"),
    }
}
