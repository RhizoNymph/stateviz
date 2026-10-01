//! Why a surgical patch could not be made.

use cascade_core::edit::EditError;

/// A surgical patch failed. [`SurgeryError::Edit`] means the op does not
/// apply to this definition at all (reported to the caller when
/// `edit::apply` cannot say so itself); [`SurgeryError::Unsupported`] means
/// the op is fine but the text has a shape the patcher does not edit in place,
/// so the caller falls back to a full rewrite.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SurgeryError {
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error("cannot patch in place: {0}")]
    Unsupported(#[from] Unsupported),
}

/// Text shapes the patcher leaves to the rewrite fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Unsupported {
    #[error("the text is not a single YAML document")]
    NotOneDocument,
    #[error("the YAML uses tags, anchors or aliases")]
    TagsOrAnchors,
    #[error("a mapping key is not a scalar")]
    ComplexKey,
    #[error("a flow collection could not be scanned")]
    FlowScan,
    #[error("a block entry does not start its own line")]
    SharedLine,
    #[error("a scalar is written as a block or multi-line scalar")]
    ComplexScalar,
    #[error("the definition has an unexpected shape here")]
    Shape,
    #[error("a flow collection with comments would have to be restructured")]
    CommentInFlow,
    #[error("a single rule mapping would have to become a list")]
    SingleRuleMapping,
    #[error("the root mapping would become empty")]
    EmptyBlockMapping,
    #[error("the patched text does not parse")]
    Unparseable,
}

pub(crate) type Result<T> = std::result::Result<T, SurgeryError>;

pub(crate) fn not_found(what: &'static str, name: impl Into<String>) -> SurgeryError {
    SurgeryError::Edit(EditError::NotFound { what, name: name.into() })
}

pub(crate) fn name_taken(what: &'static str, name: impl Into<String>) -> SurgeryError {
    SurgeryError::Edit(EditError::NameTaken { what, name: name.into() })
}

pub(crate) fn out_of_range(what: &'static str, index: usize, len: usize) -> SurgeryError {
    SurgeryError::Edit(EditError::IndexOutOfRange { what, index, len })
}
