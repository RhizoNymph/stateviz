//! SCXML import and export.

use cascade_core::Model;

use crate::error::InteropError;
use crate::import::{ImportOptions, Imported};

pub(crate) fn import(text: &str, options: &ImportOptions) -> Result<Imported, InteropError> {
    let _ = (text, options);
    Err(InteropError::Unsupported { format: "SCXML", location: String::new(), what: "anything yet".to_owned() })
}

pub(crate) fn export(model: &Model) -> String {
    let _ = model;
    String::new()
}
