//! Persisting an [`EditOp`] to definition text without losing the author's
//! comments and formatting.
//!
//! [`patch_text`] applies the op to the source text directly, using a span
//! index over the YAML to find what to replace, insert or delete, in the
//! file's own style (flow `{ … }` transitions stay flow and aligned, list-form
//! states stay lists, indentation follows the surrounding block, quoting
//! of changed scalars is kept). Only when a surgical patch is impossible
//! does it fall back to re-emitting the whole file with [`crate::to_yaml`],
//! and it says so, so the app can warn that comments were dropped.
//!
//! Semantics come from `cascade_core::edit::apply`: the patched text is
//! parsed and compared with `apply`'s definition (ignoring spans); any
//! difference falls back to the rewrite rather than returning wrong text.
//! While `apply` reports `NotImplemented` (before `feat/edit-ops` lands),
//! that self-check is skipped, the patched text is only required to
//! resolve, and ops that cannot be patched in place return
//! [`PatchError::NotImplemented`] because there is nothing to rewrite from.
//! The check switches on by itself once `apply` works.
//!
//! Invariant: parsing the patched text gives the same definition as
//! `cascade_core::edit::apply` on the parsed original, ignoring spans; the
//! text outside the edited entries is unchanged byte for byte.
//!
//! See `docs/features/build-and-play.md`, "Saving edits (YAML patch)".

mod block;
mod collection;
mod doc;
mod error;
mod ops;
mod render;
mod splice;
mod verify;

use cascade_core::edit::{self, EditError, EditOp};
use cascade_core::error::LoadError;
use cascade_core::{Definition, load_str, parse_definition};

use error::SurgeryError;

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
    /// The op cannot be patched in place and there is no applied definition
    /// to rewrite from (only while `edit::apply` is not implemented).
    #[error("patching is not implemented yet")]
    NotImplemented,
}

/// What the patched text must parse to.
enum Expected {
    /// `edit::apply`'s result.
    Core(Definition),
    /// `edit::apply` is not implemented yet: only require that the patched
    /// text resolves.
    Unavailable,
}

/// Apply `op` to definition `text`.
pub fn patch_text(text: &str, op: &EditOp) -> Result<Patched, PatchError> {
    let original = parse_definition(text).map_err(PatchError::Unparseable)?;
    let expected = match edit::apply(&original, op) {
        Ok(applied) => Expected::Core(applied.definition),
        Err(EditError::NotImplemented) => Expected::Unavailable,
        Err(err) => return Err(PatchError::Edit(err)),
    };
    let surgery = ops::apply(text, op);
    match (surgery, expected) {
        (Ok(patched), Expected::Core(definition)) => {
            let matches = parse_definition(&patched).is_ok_and(|p| verify::same_definition(&p, &definition));
            if matches {
                Ok(Patched { text: patched, rewritten: false })
            } else {
                tracing::warn!(op = ?op, "patched text differs from edit::apply; rewriting the file");
                Ok(rewrite(&definition))
            }
        }
        (Err(err), Expected::Core(definition)) => {
            tracing::info!(op = ?op, reason = %err, "cannot patch in place; rewriting the file");
            Ok(rewrite(&definition))
        }
        (Ok(patched), Expected::Unavailable) => match load_str(&patched) {
            Ok(_) => Ok(Patched { text: patched, rewritten: false }),
            Err(err) => Err(PatchError::Edit(EditError::Invalid(err))),
        },
        (Err(SurgeryError::Edit(err)), Expected::Unavailable) => Err(PatchError::Edit(err)),
        (Err(SurgeryError::Unsupported(reason)), Expected::Unavailable) => {
            tracing::info!(op = ?op, reason = %reason, "cannot patch in place and edit::apply is unavailable");
            Err(PatchError::NotImplemented)
        }
    }
}

fn rewrite(definition: &Definition) -> Patched {
    Patched { text: crate::to_yaml(definition), rewritten: true }
}
