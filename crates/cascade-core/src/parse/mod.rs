//! YAML text → [`Definition`].
//!
//! Parsing is total over well-formed YAML: every shape problem becomes a
//! [`Diagnostic`](crate::error::Diagnostic) and parsing continues with the
//! next sibling, so one run reports every problem in the file.

pub mod grammar;
mod node;
mod yaml;

use crate::definition::Definition;
use crate::error::LoadError;

/// Parse definition text into a [`Definition`] without resolving names.
pub fn parse_definition(text: &str) -> Result<Definition, LoadError> {
    yaml::parse(text)
}
