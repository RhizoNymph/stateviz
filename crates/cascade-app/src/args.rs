//! Command-line arguments.

use std::path::PathBuf;

use cascade_scene::{ViewLinkError, ViewState};
use clap::Parser;

/// Cascade: visualize a system of interacting state machines.
#[derive(Debug, Parser)]
#[command(name = "cascade-app", version, about)]
pub struct Args {
    /// The definition file (YAML).
    pub file: PathBuf,
    /// Open this view: a `cascade://` link as copied from the app.
    #[arg(long, value_name = "LINK")]
    pub view: Option<String>,
}

impl Args {
    /// The initial view state: the link's, or the default causal view.
    pub fn initial_view(&self) -> Result<ViewState, ViewLinkError> {
        self.view.as_deref().map_or_else(|| Ok(ViewState::default()), ViewState::from_link)
    }
}

#[cfg(test)]
mod tests {
    use cascade_scene::ViewKind;

    use super::*;

    #[test]
    fn parses_file_and_view_link() {
        let args = Args::try_parse_from(["cascade-app", "x.yaml", "--view", "cascade://matrix"]).expect("parses");
        assert_eq!(args.file, PathBuf::from("x.yaml"));
        assert_eq!(args.initial_view().map(|v| v.view), Ok(ViewKind::Matrix));
    }

    #[test]
    fn default_view_without_a_link() {
        let args = Args::try_parse_from(["cascade-app", "x.yaml"]).expect("parses");
        assert_eq!(args.initial_view(), Ok(ViewState::default()));
    }

    #[test]
    fn bad_links_and_missing_files_are_errors() {
        let args = Args::try_parse_from(["cascade-app", "x.yaml", "--view", "nope"]).expect("parses");
        assert!(args.initial_view().is_err());
        assert!(Args::try_parse_from(["cascade-app"]).is_err());
    }
}
