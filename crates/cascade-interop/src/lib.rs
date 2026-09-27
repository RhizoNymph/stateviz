//! Cascade interop: import other state machine formats, export to formats
//! other tools read, write definitions back to YAML, and read definitions at
//! git refs for diff mode.
//!
//! Owner: `feat/interop-and-diff` implements every function here (and
//! `cascade_core::diff`). The signatures are the contract the CLI and app
//! code against.
//!
//! ```text
//! XState JSON ─┐                          ┌─▶ SCXML
//! SCXML ───────┼─▶ Definition ─▶ Model ───┼─▶ Mermaid
//!              │        │                 └─▶ P skeleton
//!              │        └─▶ YAML text
//! git show ────┘ (text at a ref, for diff mode)
//! ```

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use cascade_core::{Definition, Model};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImportFormat {
    /// An XState v5 machine config (`createMachine({...})` argument) as JSON.
    XState,
    Scxml,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExportFormat {
    Scxml,
    Mermaid,
    /// A P language skeleton for model checking.
    P,
    /// The definition, re-emitted as YAML.
    Yaml,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown format `{0}`")]
pub struct UnknownFormat(pub String);

impl FromStr for ImportFormat {
    type Err = UnknownFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "xstate" => Ok(ImportFormat::XState),
            "scxml" => Ok(ImportFormat::Scxml),
            _ => Err(UnknownFormat(s.to_owned())),
        }
    }
}

impl FromStr for ExportFormat {
    type Err = UnknownFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "scxml" => Ok(ExportFormat::Scxml),
            "mermaid" => Ok(ExportFormat::Mermaid),
            "p" => Ok(ExportFormat::P),
            "yaml" => Ok(ExportFormat::Yaml),
            _ => Err(UnknownFormat(s.to_owned())),
        }
    }
}

impl fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ExportFormat::Scxml => "scxml",
            ExportFormat::Mermaid => "mermaid",
            ExportFormat::P => "p",
            ExportFormat::Yaml => "yaml",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InteropError {
    #[error("{0} is not implemented yet")]
    NotImplemented(&'static str),
    #[error("invalid {format} input: {message}")]
    Invalid { format: &'static str, message: String },
    #[error("{format} cannot represent {what}")]
    Unsupported { format: &'static str, what: String },
}

/// Convert another format into a definition.
///
/// Stub until `feat/interop-and-diff` lands.
pub fn import(format: ImportFormat, text: &str) -> Result<Definition, InteropError> {
    let _ = (format, text);
    Err(InteropError::NotImplemented("import"))
}

/// Render a model in another format.
///
/// Stub until `feat/interop-and-diff` lands.
pub fn export(format: ExportFormat, model: &Model) -> Result<String, InteropError> {
    let _ = (format, model);
    Err(InteropError::NotImplemented("export"))
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git is not available: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("{} is not inside a git repository", path.display())]
    NotARepository { path: PathBuf },
    #[error("git could not read `{rev}:{}`: {stderr}", path.display())]
    Show { rev: String, path: PathBuf, stderr: String },
    #[error("git output for `{rev}` is not UTF-8")]
    NotUtf8 { rev: String },
    #[error("git support is not implemented yet")]
    NotImplemented,
}

/// The contents of `file` (a path in the working tree) at git revision
/// `rev`, as `git show <rev>:<path-relative-to-repo-root>` prints it.
///
/// Stub until `feat/interop-and-diff` lands.
pub fn read_at_rev(file: &Path, rev: &str) -> Result<String, GitError> {
    let _ = (file, rev);
    Err(GitError::NotImplemented)
}
