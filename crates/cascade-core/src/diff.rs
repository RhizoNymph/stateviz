//! Comparing two versions of a definition (diff mode, design review).
//!
//! Elements are matched by [`ElementKey`], so a rename shows as a removal
//! plus an addition.
//!
//! [`diff_models`] classifies every key of either model: only in the new
//! model → [`DiffStatus::Added`]; only in the old → [`DiffStatus::Removed`];
//! in both with different attributes → [`DiffStatus::Changed`]. Attributes
//! are compared by names and paths, never by ids or source spans, so moving
//! text around is not a change. Derived facts (which transitions accept a
//! trigger, which rules fire it, which sources expose it) are not
//! attributes: a trigger is only ever added or removed.
//!
//! [`merge_for_display`] builds one model holding every element of the new
//! version plus the removed elements of the old one (the ghosts), so a view
//! can draw both versions at once. It works on definitions:
//!
//! 1. Start from the new definition, so new elements keep their spans.
//! 2. Rewrite its state references to full paths (ghost states could make a
//!    unique local name ambiguous) and make every initial state explicit
//!    (a ghost inserted first must not become the default).
//! 3. Rebuild each removed element from the old *model* (full paths, no
//!    spans) and insert it next to its old neighbours: whole machines,
//!    state subtrees, transitions (appended, so duplicate-transition
//!    ordinals line up), controllers, handlers, rules and sources.
//! 4. Keep the union resolvable: ghosts that cannot exist in the new
//!    structure (children or outgoing transitions of a state that became
//!    final or history) are dropped; in strict-events mode removed events
//!    are declared; payload and field lists that ghost selectors rely on are
//!    widened.
//! 5. Resolve, then give every element the span it has in the new file (a
//!    ghost can be an event's or trigger's first mention in the union).
//!
//! Owner: the `feat/interop-and-diff` workstream. The types here are the
//! contract the scene builders and app code against.

mod compare;
mod merge;
mod rebuild;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::LoadError;
use crate::key::ElementKey;
use crate::model::Model;
use crate::resolve::resolve;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiffStatus {
    Unchanged,
    /// Only in the new version: drawn with a green outline.
    Added,
    /// Only in the old version: drawn as a red ghost.
    Removed,
    /// In both, with different attributes (guard, emits, target, color, …).
    Changed,
}

/// Per-element status between an old and a new model. Elements not listed
/// are unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDiff {
    statuses: BTreeMap<ElementKey, DiffStatus>,
}

impl ModelDiff {
    pub fn from_statuses(statuses: impl IntoIterator<Item = (ElementKey, DiffStatus)>) -> Self {
        Self { statuses: statuses.into_iter().filter(|(_, status)| *status != DiffStatus::Unchanged).collect() }
    }

    pub fn status(&self, key: &ElementKey) -> DiffStatus {
        self.statuses.get(key).copied().unwrap_or(DiffStatus::Unchanged)
    }

    /// Every element that is not unchanged, in key order.
    pub fn entries(&self) -> impl Iterator<Item = (&ElementKey, DiffStatus)> {
        self.statuses.iter().map(|(k, s)| (k, *s))
    }

    /// The elements with `status`, in key order. Empty for
    /// [`DiffStatus::Unchanged`], which is never stored.
    pub fn with_status(&self, status: DiffStatus) -> impl Iterator<Item = &ElementKey> {
        self.statuses.iter().filter(move |(_, s)| **s == status).map(|(k, _)| k)
    }

    pub fn is_empty(&self) -> bool {
        self.statuses.is_empty()
    }

    pub fn count(&self, status: DiffStatus) -> usize {
        self.statuses.values().filter(|&&s| s == status).count()
    }
}

/// Classify every element of `old` and `new`.
///
/// Compared attributes: machine color, domain, initial state and fields;
/// state kind and a compound state's initial child; transition guard,
/// emitted events (in order) and `bounded`; event payload and whether it is
/// declared; rule fired trigger, target selector, condition and `bounded`;
/// an external source's triggers (as a set). Triggers, controllers and
/// handlers have no attributes of their own.
pub fn diff_models(old: &Model, new: &Model) -> ModelDiff {
    let old_attrs = compare::attributes_by_key(old);
    let new_attrs = compare::attributes_by_key(new);
    let removed_or_changed = old_attrs.iter().map(|(key, attrs)| {
        let status = match new_attrs.get(key) {
            None => DiffStatus::Removed,
            Some(new) if new != attrs => DiffStatus::Changed,
            Some(_) => DiffStatus::Unchanged,
        };
        (key.clone(), status)
    });
    let added =
        new_attrs.keys().filter(|key| !old_attrs.contains_key(*key)).map(|key| (key.clone(), DiffStatus::Added));
    ModelDiff::from_statuses(removed_or_changed.chain(added))
}

/// A model containing every element of `new` plus the removed elements of
/// `old` (the ghosts), with the diff that classifies them, so one view can
/// draw both versions at once.
///
/// Elements of `new` keep their source spans; ghosts have unknown spans.
/// Changed elements appear with their new attributes. A removed element that
/// cannot exist in the new structure (a child or outgoing transition of a
/// state that is now final or history, or a declared-only event when the new
/// version does not declare events) has no ghost, though the diff still
/// reports it. Returns the resolver's error if the union does not resolve.
pub fn merge_for_display(old: &Model, new: &Model) -> Result<(Model, ModelDiff), LoadError> {
    let union = merge::union_definition(old, new);
    let mut merged = resolve(union)?;
    merge::restore_spans(&mut merged, new);
    Ok((merged, diff_models(old, new)))
}
