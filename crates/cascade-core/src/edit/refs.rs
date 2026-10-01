//! State trees and state references inside one machine.
//!
//! A state reference (transition `from`/`to`, a machine's `initial`) is
//! either a full dotted path or a bare local name, resolved like the
//! resolver does: a full path wins; otherwise the local name must be unique
//! in the machine. [`retarget`] keeps every reference pointing at the same
//! state after the tree changes, preserving how it was written:
//!
//! - A reference written as the full path is rewritten to the state's new
//!   full path.
//! - A bare local name stays bare (with the state's new local name) when that
//!   name still resolves to the same state, and is written as the full path
//!   when it would now be ambiguous or captured by a top-level state of that
//!   name.

use crate::definition::{MachineDef, StateDef};

/// Every state of a machine, pre-order: `(full path, local name)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct StateIndex {
    states: Vec<(String, String)>,
}

impl StateIndex {
    pub fn of(states: &[StateDef]) -> Self {
        let mut index = Self::default();
        index.collect(states, None);
        index
    }

    fn collect(&mut self, states: &[StateDef], parent: Option<&str>) {
        for s in states {
            let path = join(parent, &s.name.value);
            self.states.push((path.clone(), s.name.value.clone()));
            self.collect(&s.states, Some(&path));
        }
    }

    /// The full path a reference resolves to, if it resolves.
    pub fn resolve(&self, reference: &str) -> Option<&str> {
        if let Some((path, _)) = self.states.iter().find(|(path, _)| path == reference) {
            return Some(path);
        }
        let mut matches = self.states.iter().filter(|(_, name)| name == reference);
        match (matches.next(), matches.next()) {
            (Some((path, _)), None) => Some(path),
            _ => None,
        }
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.states.iter().map(|(path, _)| path.as_str())
    }
}

pub(super) fn join(parent: Option<&str>, name: &str) -> String {
    match parent {
        Some(parent) => format!("{parent}.{name}"),
        None => name.to_owned(),
    }
}

/// `(parent path, local name)` of a path.
pub(super) fn split(path: &str) -> (Option<&str>, &str) {
    match path.rsplit_once('.') {
        Some((parent, name)) => (Some(parent), name),
        None => (None, path),
    }
}

/// Whether `path` is `root` or one of its descendants.
pub(super) fn is_within(path: &str, root: &str) -> bool {
    path.strip_prefix(root).is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

pub(super) fn find<'a>(states: &'a [StateDef], path: &str) -> Option<&'a StateDef> {
    let mut current: Option<&StateDef> = None;
    let mut level = states;
    for segment in path.split('.') {
        let found = level.iter().find(|s| s.name.value == segment)?;
        level = &found.states;
        current = Some(found);
    }
    current
}

pub(super) fn find_mut<'a>(states: &'a mut [StateDef], path: &str) -> Option<&'a mut StateDef> {
    let (first, rest) = match path.split_once('.') {
        Some((first, rest)) => (first, Some(rest)),
        None => (path, None),
    };
    let found = states.iter_mut().find(|s| s.name.value == first)?;
    match rest {
        Some(rest) => find_mut(&mut found.states, rest),
        None => Some(found),
    }
}

/// The child list of `parent`, or the machine's top-level states.
pub(super) fn children_mut<'a>(machine: &'a mut MachineDef, parent: Option<&str>) -> Option<&'a mut Vec<StateDef>> {
    match parent {
        None => Some(&mut machine.states),
        Some(parent) => find_mut(&mut machine.states, parent).map(|s| &mut s.states),
    }
}

/// Full paths of `state` (at `path`) and its descendants, pre-order.
pub(super) fn subtree_paths(state: &StateDef, path: &str) -> Vec<String> {
    let mut out = vec![path.to_owned()];
    for child in &state.states {
        out.extend(subtree_paths(child, &join(Some(path), &child.name.value)));
    }
    out
}

/// What [`retarget`] rewrote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Retargeted {
    /// Indices of transition entries with a rewritten reference.
    pub transitions: Vec<usize>,
    pub initial: bool,
}

/// Keep every state reference of `machine` pointing at the same state after
/// its tree changed. `before` indexes the tree the references were written
/// against; `moved` maps an old full path to its new one (identity for
/// states that did not move). References that did not resolve before are
/// left alone.
pub(super) fn retarget(machine: &mut MachineDef, before: &StateIndex, moved: &dyn Fn(&str) -> String) -> Retargeted {
    let after = StateIndex::of(&machine.states);
    let rewrite = |reference: &str| -> Option<String> {
        let old = before.resolve(reference)?;
        let new = moved(old);
        let written = render(reference, old, &new, &after);
        (written != reference).then_some(written)
    };

    let mut out = Retargeted::default();
    for (i, t) in machine.transitions.iter_mut().enumerate() {
        let mut changed = false;
        for reference in t.from.iter_mut().chain(std::iter::once(&mut t.to)) {
            if let Some(written) = rewrite(&reference.value) {
                reference.value = written;
                changed = true;
            }
        }
        if changed {
            out.transitions.push(i);
        }
    }
    if let Some(initial) = machine.initial.as_mut()
        && let Some(written) = rewrite(&initial.value)
    {
        initial.value = written;
        out.initial = true;
    }
    out
}

/// How to write a reference to `new` that used to be written `reference`
/// for `old`.
fn render(reference: &str, old: &str, new: &str, after: &StateIndex) -> String {
    if reference == old {
        return new.to_owned();
    }
    let (_, local) = split(new);
    if after.resolve(local) == Some(new) { local.to_owned() } else { new.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition::{StateKindDef, TransitionDef};
    use crate::span::{SourceSpan, Spanned};

    fn state(name: &str, children: Vec<StateDef>) -> StateDef {
        StateDef {
            name: Spanned::synthetic(name.to_owned()),
            kind: Spanned::synthetic(StateKindDef::Normal),
            initial: None,
            states: children,
            span: SourceSpan::unknown(),
        }
    }

    fn tree() -> Vec<StateDef> {
        vec![
            state("a", vec![state("x", vec![]), state("y", vec![])]),
            state("b", vec![state("x", vec![]), state("z", vec![])]),
            state("y2", vec![]),
        ]
    }

    #[test]
    fn resolution_prefers_full_paths_then_unique_local_names() {
        let index = StateIndex::of(&tree());
        assert_eq!(index.resolve("a.x"), Some("a.x"));
        assert_eq!(index.resolve("x"), None, "ambiguous");
        assert_eq!(index.resolve("z"), Some("b.z"));
        assert_eq!(index.resolve("a"), Some("a"));
        assert_eq!(index.resolve("q"), None);
        assert_eq!(index.resolve("x.a"), None);
        let paths: Vec<_> = index.paths().collect();
        assert_eq!(paths, ["a", "a.x", "a.y", "b", "b.x", "b.z", "y2"]);
    }

    #[test]
    fn path_helpers() {
        assert_eq!(split("a.b.c"), (Some("a.b"), "c"));
        assert_eq!(split("a"), (None, "a"));
        assert!(is_within("a.b", "a"));
        assert!(is_within("a", "a"));
        assert!(!is_within("ab", "a"));
        assert!(!is_within("b.a", "a"));
        let states = tree();
        assert_eq!(find(&states, "b.z").map(|s| s.name.value.as_str()), Some("z"));
        assert!(find(&states, "b.y").is_none());
        assert_eq!(subtree_paths(&states[0], "a"), ["a", "a.x", "a.y"]);
    }

    fn machine_with(references: &[(&str, &str)]) -> MachineDef {
        MachineDef {
            name: Spanned::synthetic("M".to_owned()),
            color: None,
            domain: None,
            initial: None,
            fields: Vec::new(),
            states: tree(),
            transitions: references
                .iter()
                .map(|(from, to)| TransitionDef {
                    from: vec![Spanned::synthetic((*from).to_owned())],
                    to: Spanned::synthetic((*to).to_owned()),
                    on: Spanned::synthetic("go".to_owned()),
                    guard: None,
                    emits: Vec::new(),
                    bounded: false,
                    span: SourceSpan::unknown(),
                })
                .collect(),
            span: SourceSpan::unknown(),
        }
    }

    fn written(machine: &MachineDef) -> Vec<(String, String)> {
        machine.transitions.iter().map(|t| (t.from[0].value.clone(), t.to.value.clone())).collect()
    }

    #[test]
    fn retarget_keeps_style_and_qualifies_ambiguous_names() {
        let mut machine = machine_with(&[("z", "a.y"), ("y", "y2")]);
        let before = StateIndex::of(&machine.states);
        // Add `a.z`: the bare `z` meaning `b.z` becomes ambiguous.
        machine.states[0].states.push(state("z", vec![]));
        let out = retarget(&mut machine, &before, &|p| p.to_owned());
        assert_eq!(written(&machine), [("b.z".into(), "a.y".into()), ("y".into(), "y2".into())]);
        assert_eq!(out.transitions, [0]);
    }

    #[test]
    fn retarget_follows_moves() {
        let mut machine = machine_with(&[("z", "b.x"), ("a.y", "y")]);
        let before = StateIndex::of(&machine.states);
        machine.states[1].name.value = "c".into();
        let out = retarget(&mut machine, &before, &|p| {
            if is_within(p, "b") { format!("c{}", &p[1..]) } else { p.to_owned() }
        });
        assert_eq!(written(&machine), [("z".into(), "c.x".into()), ("a.y".into(), "y".into())]);
        assert_eq!(out.transitions, [0]);
        assert!(!out.initial);
    }

    #[test]
    fn unresolvable_references_are_left_alone() {
        let mut machine = machine_with(&[("x", "nowhere")]);
        let before = StateIndex::of(&machine.states);
        let out = retarget(&mut machine, &before, &|p| format!("{p}!"));
        assert_eq!(written(&machine), [("x".into(), "nowhere".into())]);
        assert!(out.transitions.is_empty());
    }
}
