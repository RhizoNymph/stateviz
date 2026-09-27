//! Helpers shared by the interop integration tests.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cascade_core::model::{StateKind, Target};
use cascade_core::{CausalGraph, Definition, ElementRef, Model, analyze, load_str, resolve};
use cascade_interop::to_yaml;

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn read(relative: &str) -> String {
    let path = workspace_root().join(relative);
    match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => panic!("cannot read {}: {err}", path.display()),
    }
}

pub fn load(text: &str) -> Model {
    match load_str(text) {
        Ok(model) => model,
        Err(err) => panic!("expected the definition to load:\n{err}\n---\n{text}"),
    }
}

pub fn resolve_ok(definition: Definition) -> Model {
    let yaml = to_yaml(&definition);
    match resolve(definition) {
        Ok(model) => model,
        Err(err) => panic!("expected the definition to resolve:\n{err}\n---\n{yaml}"),
    }
}

/// Every example definition in the repository (`examples/*/cascade.yaml`),
/// as (path, text).
pub fn example_definitions() -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    let examples = workspace_root().join("examples");
    let Ok(entries) = std::fs::read_dir(&examples) else {
        panic!("cannot list {}", examples.display());
    };
    let mut dirs: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    for dir in dirs {
        let file = dir.join("cascade.yaml");
        if let Ok(text) = std::fs::read_to_string(&file) {
            found.push((file, text));
        }
    }
    assert!(!found.is_empty(), "no example definitions found");
    found
}

fn kind_text(model: &Model, kind: &StateKind) -> String {
    match kind {
        StateKind::Atomic => "atomic".to_owned(),
        StateKind::Compound { initial, .. } => format!("compound(initial={})", model.state(*initial).path),
        StateKind::Final => "final".to_owned(),
        StateKind::History { deep } => format!("history(deep={deep})"),
    }
}

fn target_text(target: &Target) -> String {
    format!("{target:?}")
}

fn keys(model: &Model, elements: impl IntoIterator<Item = ElementRef>) -> Vec<String> {
    elements.into_iter().map(|e| model.key_of(e).to_string()).collect()
}

fn sorted(mut items: Vec<String>) -> Vec<String> {
    items.sort();
    items
}

/// Every element of the model with every attribute and cross reference,
/// by name, in `all_elements` order. Two models with equal signatures are
/// the same design.
pub fn signature(model: &Model) -> Vec<(String, String)> {
    model
        .all_elements()
        .into_iter()
        .map(|element| {
            let attrs = match element {
                ElementRef::Machine(id) => {
                    let m = model.machine(id);
                    format!(
                        "color={:?} domain={:?} initial={} fields={:?}",
                        m.color,
                        m.domain,
                        model.state(m.initial).path,
                        m.fields
                    )
                }
                ElementRef::State(id) => kind_text(model, &model.state(id).kind),
                ElementRef::Transition(id) => {
                    let t = model.transition(id);
                    let emits: Vec<&str> = t.emits.iter().map(|&e| model.event(e).name.as_str()).collect();
                    format!("guard={:?} emits={emits:?} bounded={}", t.guard, t.bounded)
                }
                ElementRef::Trigger(id) => {
                    let t = model.trigger(id);
                    format!(
                        "accepted_by={:?} fired_by={:?} sources={:?}",
                        sorted(keys(model, t.accepted_by.iter().map(|&x| ElementRef::Transition(x)))),
                        sorted(keys(model, t.fired_by.iter().map(|&x| ElementRef::Rule(x)))),
                        sorted(keys(model, t.sources.iter().map(|&x| ElementRef::External(x)))),
                    )
                }
                ElementRef::Event(id) => {
                    let e = model.event(id);
                    format!(
                        "payload={:?} declared={} emitted_by={:?} handlers={:?}",
                        e.payload,
                        e.declared,
                        sorted(keys(model, e.emitted_by.iter().map(|&x| ElementRef::Transition(x)))),
                        sorted(keys(model, e.handlers.iter().map(|&x| ElementRef::Handler(x)))),
                    )
                }
                ElementRef::Controller(id) => {
                    format!(
                        "handlers={:?}",
                        keys(model, model.controller(id).handlers.iter().map(|&h| ElementRef::Handler(h)))
                    )
                }
                ElementRef::Handler(id) => {
                    format!("rules={:?}", keys(model, model.handler(id).rules.iter().map(|&r| ElementRef::Rule(r))))
                }
                ElementRef::Rule(id) => {
                    let r = model.rule(id);
                    format!(
                        "fires={} target={} when={:?} bounded={}",
                        model.key_of(ElementRef::Trigger(r.trigger)),
                        target_text(&r.target),
                        r.condition,
                        r.bounded
                    )
                }
                ElementRef::External(id) => {
                    let x = model.external(id);
                    format!("triggers={:?}", sorted(keys(model, x.triggers.iter().map(|&t| ElementRef::Trigger(t)))))
                }
            };
            (model.key_of(element).to_string(), attrs)
        })
        .collect()
}

/// [`signature`] keyed by element, ignoring document order.
pub fn signature_map(model: &Model) -> BTreeMap<String, String> {
    signature(model).into_iter().collect()
}

/// Assert two models describe the same design, reporting the first
/// difference readably.
pub fn assert_same_design(expected: &Model, actual: &Model, ordered: bool) {
    let (a, b) = (signature_map(expected), signature_map(actual));
    for (key, attrs) in &a {
        match b.get(key) {
            None => panic!("missing element {key} ({attrs})"),
            Some(other) => assert_eq!(attrs, other, "attributes of {key}"),
        }
    }
    for key in b.keys() {
        assert!(a.contains_key(key), "unexpected element {key}");
    }
    if ordered {
        let order = |m: &Model| signature(m).into_iter().map(|(k, _)| k).collect::<Vec<_>>();
        assert_eq!(order(expected), order(actual), "element order");
    }
}

/// YAML round trip: the emitted text parses and resolves to the same
/// design, in the same order.
pub fn assert_yaml_round_trip(model: &Model) -> Model {
    let yaml = to_yaml(model.definition());
    let reloaded = match load_str(&yaml) {
        Ok(m) => m,
        Err(err) => panic!("emitted YAML does not load:\n{err}\n---\n{yaml}"),
    };
    assert_same_design(model, &reloaded, true);
    // Emitting again is a fixed point.
    assert_eq!(to_yaml(reloaded.definition()), yaml, "second emission differs");
    reloaded
}

/// Build the causal graph and run the checks, which must not panic.
pub fn analyze_ok(model: &Model) {
    let graph = CausalGraph::build(model);
    let _ = analyze(model, &graph);
    let _ = graph.depths();
}
