//! State references as the resolver reads them: a full dotted path first,
//! else a local name that is unique in the machine.

use cascade_core::definition::{StateDef, TransitionDef};

/// The state paths of one machine, in pre-order.
#[derive(Clone, Debug, Default)]
pub(crate) struct StateTree {
    /// `(path, local name)`.
    states: Vec<(String, String)>,
}

impl StateTree {
    pub fn new(states: &[StateDef]) -> Self {
        let mut tree = Self::default();
        tree.add(states, None);
        tree
    }

    fn add(&mut self, states: &[StateDef], parent: Option<&str>) {
        for state in states {
            let path = match parent {
                Some(p) => format!("{p}.{}", state.name.value),
                None => state.name.value.clone(),
            };
            self.states.push((path.clone(), state.name.value.clone()));
            self.add(&state.states, Some(&path));
        }
    }

    pub fn contains(&self, path: &str) -> bool {
        self.states.iter().any(|(p, _)| p == path)
    }

    /// The path a reference resolves to.
    pub fn resolve(&self, reference: &str) -> Option<&str> {
        if let Some((path, _)) = self.states.iter().find(|(p, _)| p == reference) {
            return Some(path);
        }
        let mut matches = self.states.iter().filter(|(_, name)| name == reference);
        match (matches.next(), matches.next()) {
            (Some((path, _)), None) => Some(path),
            _ => None,
        }
    }

    /// This tree with the state at `path` (and its descendants) moved to
    /// `new_path`, which differs only in the last segment.
    pub fn renamed(&self, path: &str, new_path: &str, new_name: &str) -> Self {
        let states = self
            .states
            .iter()
            .map(|(p, name)| match moved(p, path, new_path) {
                Some(moved) if p == path => (moved, new_name.to_owned()),
                Some(moved) => (moved, name.clone()),
                None => (p.clone(), name.clone()),
            })
            .collect();
        Self { states }
    }
}

/// `p` rebased from `path` to `new_path` when it is `path` or below it.
pub(crate) fn moved(p: &str, path: &str, new_path: &str) -> Option<String> {
    if p == path {
        Some(new_path.to_owned())
    } else {
        p.strip_prefix(path).and_then(|rest| rest.strip_prefix('.')).map(|rest| format!("{new_path}.{rest}"))
    }
}

/// Whether `p` is `path` or a descendant of it.
pub(crate) fn within(p: &str, path: &str) -> bool {
    p == path || p.strip_prefix(path).is_some_and(|rest| rest.starts_with('.'))
}

/// The text a reference should have after renaming the state at `path` to
/// `new_name`, or `None` when it can stay. The reference keeps its form
/// (full path or local name) when that still resolves to the same state,
/// else it becomes the full path.
pub(crate) fn renamed_reference(
    old: &StateTree,
    new: &StateTree,
    reference: &str,
    path: &str,
    new_path: &str,
    new_name: &str,
) -> Option<String> {
    let target = old.resolve(reference)?;
    let new_target = moved(target, path, new_path).unwrap_or_else(|| target.to_owned());
    let candidate = if reference == target {
        new_target.clone()
    } else if target == path {
        new_name.to_owned()
    } else {
        reference.to_owned()
    };
    let text = if new.resolve(&candidate) == Some(new_target.as_str()) { candidate } else { new_target };
    (text != reference).then_some(text)
}

/// Which states a transition's sources and target resolve to.
pub(crate) fn endpoints<'t>(tree: &'t StateTree, t: &TransitionDef) -> (Vec<Option<&'t str>>, Option<&'t str>) {
    let from = t.from.iter().map(|f| tree.resolve(&f.value)).collect();
    (from, tree.resolve(&t.to.value))
}

#[cfg(test)]
mod tests {
    use cascade_core::Spanned;
    use cascade_core::definition::StateKindDef;
    use cascade_core::span::SourceSpan;

    use super::*;

    fn st(name: &str, children: Vec<StateDef>) -> StateDef {
        StateDef {
            name: Spanned::synthetic(name.to_owned()),
            kind: Spanned::synthetic(StateKindDef::Normal),
            initial: None,
            states: children,
            span: SourceSpan::unknown(),
        }
    }

    fn tree() -> StateTree {
        StateTree::new(&[
            st("cart", vec![]),
            st("placed", vec![st("waiting", vec![]), st("paid", vec![])]),
            st("done", vec![]),
        ])
    }

    #[test]
    fn resolves_paths_then_unique_local_names() {
        let t = tree();
        assert_eq!(t.resolve("placed.paid"), Some("placed.paid"));
        assert_eq!(t.resolve("paid"), Some("placed.paid"));
        assert_eq!(t.resolve("nope"), None);
    }

    #[test]
    fn references_keep_their_form() {
        let old = tree();
        let new = old.renamed("placed", "ordered", "ordered");
        let r = |text: &str| renamed_reference(&old, &new, text, "placed", "ordered", "ordered");
        assert_eq!(r("placed").as_deref(), Some("ordered"));
        assert_eq!(r("placed.paid").as_deref(), Some("ordered.paid"));
        assert_eq!(r("paid"), None);
        assert_eq!(r("cart"), None);
    }

    #[test]
    fn a_local_name_that_becomes_ambiguous_turns_into_a_path() {
        let old = tree();
        let new = old.renamed("done", "paid", "paid");
        let r = |text: &str| renamed_reference(&old, &new, text, "done", "paid", "paid");
        // `paid` was the unique local name of `placed.paid`; now `paid` is a
        // top-level path, so the reference must spell out the old target.
        assert_eq!(r("paid").as_deref(), Some("placed.paid"));
        assert_eq!(r("done").as_deref(), Some("paid"));
    }
}
