//! Importing other statechart formats.
//!
//! Both importers parse their input into the shared statechart IR
//! ([`chart`]), which [`lower`] turns into a Cascade [`Definition`]: names
//! are sanitized, targets become state paths, entry/exit actions are
//! attributed to transitions, and (unless the input carries Cascade's own
//! wiring) a routing controller and external sources are synthesized so the
//! result resolves and renders without hand edits.

pub(crate) mod chart;
pub(crate) mod lower;
pub(crate) mod names;
pub(crate) mod wiring;

use std::fmt;

use cascade_core::Definition;

/// Conventions the importers follow. The defaults are what `cascade import`
/// uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportOptions {
    /// An XState action named `<prefix><Event>` (as a string, or as the
    /// `type` of an action object) emits `Event`. Default: `emit:`, so
    /// `"emit:OrderPaid"` emits `OrderPaid`.
    pub emit_prefix: String,
    /// The external source that fires every trigger no emitted event covers
    /// (user input, other code). Default: `Environment`.
    pub environment_source: String,
    /// The external source that fires delayed (`after`) triggers. Default:
    /// `Clock`.
    pub clock_source: String,
    /// The controller that routes emitted events to the machines that handle
    /// them. Default: `EventRouter`.
    pub router_controller: String,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            emit_prefix: "emit:".to_owned(),
            environment_source: "Environment".to_owned(),
            clock_source: "Clock".to_owned(),
            router_controller: "EventRouter".to_owned(),
        }
    }
}

/// A successful import: the definition plus everything that was renamed,
/// approximated or dropped on the way.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    pub definition: Definition,
    pub warnings: Vec<ImportWarning>,
}

/// Something an import changed or dropped. The definition is still complete
/// enough to resolve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportWarning {
    /// Where in the input (JSON path or SCXML element path).
    pub location: String,
    pub kind: WarningKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WarningKind {
    /// A name was not a valid Cascade name, or collided with a sibling, and
    /// was renamed.
    Renamed { what: NameKind, original: String, name: String },
    /// Input with no Cascade equivalent, skipped.
    Ignored { what: String },
    /// Input mapped onto Cascade with an approximation.
    Approximated { what: String, how: String },
}

/// What kind of element a renamed name belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NameKind {
    Machine,
    State,
    Trigger,
    Event,
    Controller,
    Source,
}

impl NameKind {
    pub const fn noun(self) -> &'static str {
        match self {
            NameKind::Machine => "machine",
            NameKind::State => "state",
            NameKind::Trigger => "trigger",
            NameKind::Event => "event",
            NameKind::Controller => "controller",
            NameKind::Source => "external source",
        }
    }
}

impl fmt::Display for ImportWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.location)?;
        match &self.kind {
            WarningKind::Renamed { what, original, name } => {
                write!(f, "renamed {} `{original}` to `{name}`", what.noun())
            }
            WarningKind::Ignored { what } => write!(f, "ignored {what}"),
            WarningKind::Approximated { what, how } => write!(f, "approximated {what}: {how}"),
        }
    }
}

/// Collects warnings during one import.
#[derive(Debug, Default)]
pub(crate) struct Warnings {
    pub list: Vec<ImportWarning>,
}

impl Warnings {
    pub fn push(&mut self, location: impl Into<String>, kind: WarningKind) {
        self.list.push(ImportWarning { location: location.into(), kind });
    }

    pub fn ignored(&mut self, location: impl Into<String>, what: impl Into<String>) {
        self.push(location, WarningKind::Ignored { what: what.into() });
    }

    pub fn approximated(&mut self, location: impl Into<String>, what: impl Into<String>, how: impl Into<String>) {
        self.push(location, WarningKind::Approximated { what: what.into(), how: how.into() });
    }
}
