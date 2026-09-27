//! Cascade core: the definition format, the resolved model, the derived
//! causal graph, static analysis, search and diffing.
//!
//! This crate is pure: no I/O beyond [`load_file`], no UI. The CLI, the
//! native app and every exporter build on it, so checks behave identically
//! in CI and in the UI.
//!
//! ```text
//! YAML text ──parse──▶ Definition ──resolve──▶ Model ──build──▶ CausalGraph
//!                                                 │                 │
//!                                                 └────analyze──────┴──▶ Vec<Finding>
//! ```

pub mod analysis;
pub mod causal;
pub mod color;
pub mod definition;
pub mod diff;
pub mod error;
pub mod ids;
pub mod key;
mod load;
pub mod model;
pub mod parse;
pub mod resolve;
pub mod search;
pub mod span;

pub use analysis::{Check, Finding, FindingDetail, Severity, analyze};
pub use causal::{CausalEdge, CausalEdgeKind, CausalGraph, CausalNode, Cone, Direction, EdgeIx, NodeIx};
pub use color::PaletteColor;
pub use definition::Definition;
pub use error::{Diagnostic, DiagnosticKind, LoadError};
pub use ids::{ControllerId, EventId, ExternalId, HandlerId, MachineId, RuleId, StateId, TransitionId, TriggerId};
pub use key::{ElementKey, ElementKind, ElementRef};
pub use load::{LoadFileError, load_file, load_str};
pub use model::Model;
pub use parse::parse_definition;
pub use resolve::resolve;
pub use span::{Pos, SourceSpan, Spanned};
