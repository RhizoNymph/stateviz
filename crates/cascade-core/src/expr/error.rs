//! Parse errors of the embedded languages (expressions, field types,
//! durations), with the column they occur at.

use std::fmt;

/// A parse error at a 1-based character column of the parsed text. A column
/// one past the end means "at end of text".
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub struct ExprError {
    pub message: String,
    pub column: usize,
}

impl ExprError {
    pub fn new(message: impl Into<String>, column: usize) -> Self {
        Self { message: message.into(), column }
    }
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (column {})", self.message, self.column)
    }
}
