//! Load diagnostics: everything that stops a definition file from becoming a
//! [`Model`](crate::Model).
//!
//! These are distinct from analysis findings. A diagnostic means the file is
//! malformed or refers to something that does not exist; a finding means the
//! model is well-formed but the design has a bug.

use std::fmt;

use crate::span::SourceSpan;

/// Every problem found while loading, in source order. Loading collects as
/// many diagnostics as it can rather than stopping at the first.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub struct LoadError {
    pub diagnostics: Vec<Diagnostic>,
}

impl LoadError {
    pub fn single(kind: DiagnosticKind, span: SourceSpan) -> Self {
        Self { diagnostics: vec![Diagnostic { kind, span }] }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, diagnostic) in self.diagnostics.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{diagnostic}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub span: SourceSpan,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.span, self.kind)
    }
}

/// What a YAML node was expected to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expected {
    Mapping,
    Sequence,
    String,
    Bool,
    StringOrSequence,
    SequenceOrMapping,
}

impl fmt::Display for Expected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Expected::Mapping => "a mapping",
            Expected::Sequence => "a sequence",
            Expected::String => "a string",
            Expected::Bool => "true or false",
            Expected::StringOrSequence => "a string or a sequence of strings",
            Expected::SequenceOrMapping => "a sequence or a mapping",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DiagnosticKind {
    // --- YAML shape -------------------------------------------------------
    #[error("YAML syntax error: {message}")]
    YamlSyntax { message: String },
    #[error("the file contains no YAML document")]
    EmptyDocument,
    #[error("the file contains more than one YAML document")]
    MultipleDocuments,
    #[error("YAML anchors and aliases are not supported")]
    AliasNotSupported,
    #[error("{context}: expected {expected}")]
    WrongType { context: String, expected: Expected },
    #[error("{context}: unknown key `{key}`")]
    UnknownKey { context: String, key: String },
    #[error("{context}: missing required key `{key}`")]
    MissingKey { context: String, key: String },
    #[error(
        "{context}: `{name}` is not a valid name (use letters, digits, `_` and `-`, starting with a letter or `_`)"
    )]
    InvalidName { context: String, name: String },
    #[error("{context}: duplicate {what} `{name}`")]
    Duplicate { context: String, what: &'static str, name: String },

    // --- Embedded grammars -------------------------------------------------
    #[error("{0}")]
    UnknownColor(#[from] crate::color::UnknownColor),
    #[error("unknown state kind `{0}`; expected normal, final, history or deep-history")]
    UnknownStateKind(String),
    #[error("`{text}` is not a trigger reference; expected `Machine.trigger`")]
    InvalidTriggerRef { text: String },
    #[error("invalid target selector `{text}`: {reason}")]
    InvalidSelector { text: String, reason: String },

    // --- References ---------------------------------------------------------
    #[error("machine `{machine}` has no states")]
    EmptyMachine { machine: String },
    #[error("unknown machine `{name}`")]
    UnknownMachine { name: String },
    #[error("machine `{machine}` has no state `{name}`")]
    UnknownState { machine: String, name: String },
    #[error("state name `{name}` in machine `{machine}` is ambiguous; use one of: {}", candidates.join(", "))]
    AmbiguousState { machine: String, name: String, candidates: Vec<String> },
    #[error("initial state `{initial}` of `{parent}` is not one of its direct children")]
    InitialNotChild { parent: String, initial: String },
    #[error("{kind} state `{state}` cannot have child states")]
    ChildrenNotAllowed { state: String, kind: &'static str },
    #[error("final state `{state}` cannot have outgoing transitions")]
    TransitionFromFinal { state: String },
    #[error("history state `{state}` cannot have outgoing transitions")]
    TransitionFromHistory { state: String },
    #[error("a history state cannot be a machine's or a compound state's initial state (`{state}`)")]
    InitialIsHistory { state: String },
    #[error("rule fires `{fire}` but its target selects machine `{target}`")]
    TargetMachineMismatch { fire: String, target: String },
    #[error("event `{event}` is not declared in `events:`")]
    UndeclaredEvent { event: String },
    #[error("machine `{machine}` declares no field `{field}` (declared: {})", declared.join(", "))]
    UnknownField { machine: String, field: String, declared: Vec<String> },
    #[error("event `{event}` declares no payload field `{field}` (declared: {})", declared.join(", "))]
    UnknownPayloadField { event: String, field: String, declared: Vec<String> },
}
