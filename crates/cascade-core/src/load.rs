//! Convenience entry points: text or file → [`Model`].

use std::path::{Path, PathBuf};

use crate::error::LoadError;
use crate::model::Model;
use crate::parse::parse_definition;
use crate::resolve::resolve;

/// Parse and resolve definition text.
pub fn load_str(text: &str) -> Result<Model, LoadError> {
    resolve(parse_definition(text)?)
}

#[derive(Debug, thiserror::Error)]
pub enum LoadFileError {
    #[error("cannot read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{}:\n{source}", path.display())]
    Invalid {
        path: PathBuf,
        #[source]
        source: LoadError,
    },
}

/// Read, parse and resolve a definition file.
pub fn load_file(path: &Path) -> Result<Model, LoadFileError> {
    let text = std::fs::read_to_string(path).map_err(|source| LoadFileError::Io { path: path.to_owned(), source })?;
    load_str(&text).map_err(|source| LoadFileError::Invalid { path: path.to_owned(), source })
}
