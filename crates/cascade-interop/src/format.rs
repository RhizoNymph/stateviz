//! The formats Cascade imports from and exports to, with their CLI names.

use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImportFormat {
    /// An XState v5 machine config (`createMachine({...})` argument) as JSON,
    /// an array of configs, or `{ "machines": { "Name": config } }`.
    XState,
    Scxml,
}

impl ImportFormat {
    pub const ALL: [ImportFormat; 2] = [ImportFormat::XState, ImportFormat::Scxml];

    /// The name used on the command line (`--from xstate`).
    pub const fn name(self) -> &'static str {
        match self {
            ImportFormat::XState => "xstate",
            ImportFormat::Scxml => "scxml",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExportFormat {
    /// One SCXML document: a top-level `<parallel>` with a region per machine
    /// and per controller, Cascade-only data in the `cascade:` namespace.
    Scxml,
    /// A Mermaid `stateDiagram-v2` of every machine's structure.
    Mermaid,
    /// A Mermaid `flowchart LR` of the causal graph.
    MermaidCausal,
    /// A P language skeleton for model checking.
    P,
    /// The definition, re-emitted as YAML.
    Yaml,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 5] =
        [ExportFormat::Scxml, ExportFormat::Mermaid, ExportFormat::MermaidCausal, ExportFormat::P, ExportFormat::Yaml];

    /// The name used on the command line (`--to mermaid-causal`).
    pub const fn name(self) -> &'static str {
        match self {
            ExportFormat::Scxml => "scxml",
            ExportFormat::Mermaid => "mermaid",
            ExportFormat::MermaidCausal => "mermaid-causal",
            ExportFormat::P => "p",
            ExportFormat::Yaml => "yaml",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown format `{0}`")]
pub struct UnknownFormat(pub String);

impl FromStr for ImportFormat {
    type Err = UnknownFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let wanted = s.to_ascii_lowercase();
        Self::ALL.into_iter().find(|f| f.name() == wanted).ok_or_else(|| UnknownFormat(s.to_owned()))
    }
}

impl FromStr for ExportFormat {
    type Err = UnknownFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let wanted = s.to_ascii_lowercase();
        Self::ALL.into_iter().find(|f| f.name() == wanted).ok_or_else(|| UnknownFormat(s.to_owned()))
    }
}

impl fmt::Display for ImportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for format in ExportFormat::ALL {
            assert_eq!(format.to_string().parse::<ExportFormat>(), Ok(format));
        }
        for format in ImportFormat::ALL {
            assert_eq!(format.to_string().parse::<ImportFormat>(), Ok(format));
        }
    }

    #[test]
    fn parsing_ignores_case_and_rejects_unknown_names() {
        assert_eq!("SCXML".parse::<ExportFormat>(), Ok(ExportFormat::Scxml));
        assert_eq!("Mermaid-Causal".parse::<ExportFormat>(), Ok(ExportFormat::MermaidCausal));
        assert_eq!("XState".parse::<ImportFormat>(), Ok(ImportFormat::XState));
        assert_eq!("dot".parse::<ExportFormat>(), Err(UnknownFormat("dot".to_owned())));
        assert_eq!("mermaid".parse::<ImportFormat>(), Err(UnknownFormat("mermaid".to_owned())));
    }
}
