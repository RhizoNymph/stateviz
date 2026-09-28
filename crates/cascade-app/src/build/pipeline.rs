//! Committing one edit: apply it to the definition, patch the file text,
//! and check the patched text loads, all before anything touches the disk.
//!
//! ```text
//! EditOp ─edit::apply(definition)─▶ Applied { inverse, touched }
//!        ─patch_text(file text)───▶ Patched { text, rewritten }
//!        ─analyze_text(text)──────▶ Analyzed (the model to show next)
//! ```
//!
//! Pure: the host writes [`Commit::text`] to disk, records
//! [`Commit::applied`]'s inverse for undo and shows [`Commit::analyzed`].
//! The two library calls are parameters ([`commit_with`]) so the pipeline is
//! tested with fakes while they are stubs.

use cascade_core::Definition;
use cascade_core::edit::{Applied, EditError, EditOp};
use cascade_interop::{PatchError, Patched};

use crate::document::{Analyzed, analyze_text};

/// A committed edit, ready to be written.
#[derive(Debug)]
pub struct Commit {
    pub applied: Applied,
    /// The new file text.
    pub text: String,
    /// The whole file was re-emitted: comments and formatting are gone.
    pub rewritten: bool,
    /// The new text, loaded.
    pub analyzed: Analyzed,
}

/// Why an edit changed nothing.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CommitError {
    /// The op is invalid for this definition (or editing is not
    /// implemented yet).
    #[error("{0}")]
    Edit(#[from] EditError),
    /// The file text could not be patched.
    #[error("cannot save the edit: {0}")]
    Patch(#[from] PatchError),
    /// The patched text does not load; nothing was written.
    #[error("the edited file would not load: {}", .0.join("; "))]
    Invalid(Vec<String>),
}

/// [`commit_with`] using the real `edit::apply` and `patch_text`.
pub fn commit(text: &str, definition: &Definition, op: &EditOp) -> Result<Commit, CommitError> {
    commit_with(text, definition, op, cascade_core::edit::apply, cascade_interop::patch_text)
}

/// Apply, patch and check `op`. Nothing is returned unless every stage
/// succeeds.
pub fn commit_with(
    text: &str,
    definition: &Definition,
    op: &EditOp,
    apply: impl FnOnce(&Definition, &EditOp) -> Result<Applied, EditError>,
    patch: impl FnOnce(&str, &EditOp) -> Result<Patched, PatchError>,
) -> Result<Commit, CommitError> {
    let applied = apply(definition, op)?;
    let patched = patch(text, op)?;
    let analyzed = analyze_text(&patched.text).map_err(|failure| CommitError::Invalid(failure.lines()))?;
    Ok(Commit { applied, text: patched.text, rewritten: patched.rewritten, analyzed })
}

#[cfg(test)]
mod tests {
    use cascade_core::ElementKey;

    use super::*;

    const TEXT: &str = "machines:\n  Order:\n    states: [draft, paid]\n";
    const EDITED: &str = "machines:\n  Order:\n    states: [draft, paid, done]\n";

    fn definition() -> Definition {
        cascade_core::parse_definition(TEXT).expect("parses")
    }

    fn op() -> EditOp {
        EditOp::RemoveMachine { machine: "Order".into() }
    }

    fn fake_apply(definition: &Definition, _: &EditOp) -> Result<Applied, EditError> {
        Ok(Applied {
            definition: definition.clone(),
            inverse: EditOp::SetSystemName { name: None },
            touched: vec![ElementKey::State { machine: "Order".into(), path: "done".into() }],
        })
    }

    #[test]
    fn a_successful_commit_carries_everything_the_host_needs() {
        let commit = commit_with(TEXT, &definition(), &op(), fake_apply, |_, _| {
            Ok(Patched { text: EDITED.into(), rewritten: true })
        })
        .expect("commits");
        assert_eq!(commit.text, EDITED);
        assert!(commit.rewritten);
        assert_eq!(commit.applied.inverse, EditOp::SetSystemName { name: None });
        assert_eq!(commit.analyzed.model.state_count(), 3);
    }

    #[test]
    fn an_edit_error_stops_before_patching() {
        let result = commit_with(
            TEXT,
            &definition(),
            &op(),
            |_, _| Err(EditError::NotFound { what: "machine", name: "X".into() }),
            |_, _| panic!("must not patch after a failed apply"),
        );
        let error = result.expect_err("fails");
        assert_eq!(error, CommitError::Edit(EditError::NotFound { what: "machine", name: "X".into() }));
        assert_eq!(error.to_string(), "no machine `X`");
    }

    #[test]
    fn a_patch_error_is_reported() {
        let error = commit_with(TEXT, &definition(), &op(), fake_apply, |_, _| Err(PatchError::NotImplemented))
            .expect_err("fails");
        assert_eq!(error, CommitError::Patch(PatchError::NotImplemented));
        assert_eq!(error.to_string(), "cannot save the edit: patching is not implemented yet");
    }

    #[test]
    fn patched_text_that_does_not_load_is_rejected() {
        let error = commit_with(TEXT, &definition(), &op(), fake_apply, |_, _| {
            Ok(Patched {
                text: "machines:\n  Order:\n    states: [a]\n    initial: nowhere\n".into(),
                rewritten: false,
            })
        })
        .expect_err("fails");
        match error {
            CommitError::Invalid(lines) => assert!(!lines.is_empty()),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn the_real_pipeline_rejects_an_invalid_op_as_an_edit_error() {
        let bad = EditOp::RemoveMachine { machine: "NoSuchMachine".into() };
        assert!(matches!(commit(TEXT, &definition(), &bad), Err(CommitError::Edit(_))));
    }

    #[test]
    fn the_real_pipeline_commits_a_valid_op_or_says_it_is_not_implemented() {
        let add = EditOp::SetSystemName { name: Some("Shop".into()) };
        match commit(TEXT, &definition(), &add) {
            Ok(commit) => {
                assert!(commit.text.contains("Shop"));
                assert_eq!(commit.analyzed.model.definition().system.as_ref().map(|s| s.value.as_str()), Some("Shop"));
            }
            Err(CommitError::Edit(EditError::NotImplemented) | CommitError::Patch(PatchError::NotImplemented)) => {}
            Err(other) => panic!("unexpected {other:?}"),
        }
    }
}
