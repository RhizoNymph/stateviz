//! Import errors. Exports cannot fail: every model has a representation in
//! every export format (with the losses documented per format).

/// Why an import failed. `format` is the display name of the input format
/// (`XState`, `SCXML`); `location` points into the input (a JSON path such
/// as `machines.Fetch.states.loading.on.FETCH`, or an SCXML element path such
/// as `scxml/state#loading/transition[2]`).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InteropError {
    /// The input is not well-formed JSON or XML.
    #[error("{format} syntax error at {line}:{col}: {message}")]
    Syntax { format: &'static str, line: u32, col: u32, message: String },
    /// Well-formed input that does not follow the format's schema, or refers
    /// to something that does not exist.
    #[error("invalid {format} input at {location}: {message}")]
    Invalid { format: &'static str, location: String, message: String },
    /// A construct the Cascade model cannot represent (parallel states,
    /// eventless transitions, …).
    #[error("{format} import does not support {what} (at {location})")]
    Unsupported { format: &'static str, location: String, what: String },
}
