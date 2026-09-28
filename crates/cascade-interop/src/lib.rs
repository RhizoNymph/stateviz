//! Cascade interop: import other state machine formats, export to formats
//! other tools read, write definitions back to YAML, and read definitions at
//! git refs for diff mode.
//!
//! ```text
//! XState JSON ─┐                                ┌─▶ SCXML
//! SCXML ───────┴─▶ chart IR ─▶ Definition ─▶ Model ─┼─▶ Mermaid (structure, causal)
//!                                   │               └─▶ P skeleton
//!                                   └─▶ YAML text
//! git show ─▶ text at a revision (diff mode)
//! ```
//!
//! Every export is total: a resolved [`Model`] always has a representation,
//! with the losses documented per format in `docs/features/interop-and-diff.md`.

mod error;
mod format;
mod git;
mod import;
mod mermaid;
mod p_lang;
pub mod patch;
mod scxml;
mod xstate;
mod yaml;

use cascade_core::Model;

pub use error::InteropError;
pub use format::{ExportFormat, ImportFormat, UnknownFormat};
pub use git::{GitError, read_at_rev};
pub use import::{ImportOptions, ImportWarning, Imported, NameKind, WarningKind};
pub use patch::{PatchError, Patched, patch_text};
pub use yaml::to_yaml;

/// Convert another format into a definition, with the default
/// [`ImportOptions`].
pub fn import(format: ImportFormat, text: &str) -> Result<Imported, InteropError> {
    import_with(format, text, &ImportOptions::default())
}

/// Convert another format into a definition.
///
/// The definition is built to resolve: names are sanitized, and triggers
/// that nothing in the input fires are exposed by synthesized external
/// sources. Resolve it with [`cascade_core::resolve`] to get a model.
pub fn import_with(format: ImportFormat, text: &str, options: &ImportOptions) -> Result<Imported, InteropError> {
    match format {
        ImportFormat::XState => xstate::import(text, options),
        ImportFormat::Scxml => scxml::import(text, options),
    }
}

/// Render a model in another format.
pub fn export(format: ExportFormat, model: &Model) -> Result<String, InteropError> {
    Ok(match format {
        ExportFormat::Scxml => scxml::export(model),
        ExportFormat::Mermaid => mermaid::structure(model),
        ExportFormat::MermaidCausal => mermaid::causal(model),
        ExportFormat::P => p_lang::export(model),
        ExportFormat::Yaml => yaml::to_yaml(model.definition()),
    })
}
