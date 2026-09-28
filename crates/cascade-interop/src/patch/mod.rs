//! Persisting an [`EditOp`] to definition text without losing the author's
//! comments and formatting.
//!
//! [`patch_text`] applies the op to the source text directly, using the
//! parser's spans to find what to replace, insert or delete, in the file's
//! own style (flow `{ … }` transitions stay flow, list-form states stay
//! lists, indentation follows the surrounding block). Only when a surgical
//! patch is impossible does it fall back to re-emitting the whole file with
//! [`crate::to_yaml`], and it says so, so the app can warn that comments were
//! dropped.
//!
//! Invariant (tested for every op kind): parsing the patched text gives the
//! same definition as `cascade_core::edit::apply` on the parsed original,
//! ignoring spans.
//!
//! Owner: the `feat/yaml-patch` workstream implements [`patch_text`].

use cascade_core::edit::{EditError, EditOp};
use cascade_core::error::LoadError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patched {
    pub text: String,
    /// `true` when the whole file was re-emitted and comments and custom
    /// formatting were lost.
    pub rewritten: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PatchError {
    /// The original text does not parse, so it cannot be patched.
    #[error("the file does not parse:\n{0}")]
    Unparseable(LoadError),
    /// The op itself is invalid for this definition.
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error("patching is not implemented yet")]
    NotImplemented,
}

/// Apply `op` to definition `text`.
///
/// Stub until `feat/yaml-patch` lands.
pub fn patch_text(text: &str, op: &EditOp) -> Result<Patched, PatchError> {
    let _ = (text, op);
    Err(PatchError::NotImplemented)
}
