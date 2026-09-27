//! Comparing two versions of a definition (diff mode, design review).
//!
//! Elements are matched by [`ElementKey`], so a rename shows as a removal
//! plus an addition.
//!
//! Owner: the `feat/interop-and-diff` workstream implements [`diff_models`]
//! and [`merge_for_display`]. The types here are the contract the scene
//! builders and app code against.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::LoadError;
use crate::key::ElementKey;
use crate::model::Model;

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

    pub fn is_empty(&self) -> bool {
        self.statuses.is_empty()
    }

    pub fn count(&self, status: DiffStatus) -> usize {
        self.statuses.values().filter(|&&s| s == status).count()
    }
}

/// Classify every element of `old` and `new`.
///
/// Stub: reports no differences until `feat/interop-and-diff` lands.
pub fn diff_models(old: &Model, new: &Model) -> ModelDiff {
    let _ = (old, new);
    ModelDiff::default()
}

/// A model containing every element of `new` plus the removed elements of
/// `old` (the ghosts), with the diff that classifies them, so one view can
/// draw both versions at once.
///
/// Stub: returns `new` unchanged until `feat/interop-and-diff` lands.
pub fn merge_for_display(old: &Model, new: &Model) -> Result<(Model, ModelDiff), LoadError> {
    Ok((new.clone(), diff_models(old, new)))
}
