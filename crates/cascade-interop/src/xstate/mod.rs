//! XState v5 import.

use crate::error::InteropError;
use crate::import::{ImportOptions, Imported};

pub(crate) fn import(text: &str, options: &ImportOptions) -> Result<Imported, InteropError> {
    let _ = (text, options);
    Err(InteropError::Unsupported { format: "XState", location: String::new(), what: "anything yet".to_owned() })
}
