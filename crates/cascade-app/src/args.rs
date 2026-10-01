//! Command-line arguments.

use std::path::PathBuf;

use cascade_scene::{ViewLinkError, ViewState};
use clap::{Parser, ValueEnum};

use crate::mode::AppMode;

/// `--mode` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ModeArg {
    View,
    Build,
    Play,
}

/// Cascade: visualize a system of interacting state machines.
#[derive(Debug, Parser)]
#[command(name = "cascade-app", version, about)]
pub struct Args {
    /// The definition file (YAML).
    pub file: PathBuf,
    /// Create the file with a starter definition (one machine, one state)
    /// and open it in Build mode. Refuses to overwrite an existing file.
    #[arg(long)]
    pub new: bool,
    /// Start in this mode (default: view, or build with --new).
    #[arg(long, value_enum)]
    pub mode: Option<ModeArg>,
    /// Open this view: a `cascade://` link as copied from the app.
    #[arg(long, value_name = "LINK")]
    pub view: Option<String>,
}

impl Args {
    /// The mode to start in.
    pub fn initial_mode(&self) -> AppMode {
        match self.mode {
            Some(ModeArg::View) => AppMode::View,
            Some(ModeArg::Build) => AppMode::Build,
            Some(ModeArg::Play) => AppMode::Play,
            None if self.new => AppMode::Build,
            None => AppMode::View,
        }
    }

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

    #[test]
    fn new_takes_the_file_to_create() {
        let args = Args::try_parse_from(["cascade-app", "--new", "fresh.yaml"]).expect("parses");
        assert!(args.new);
        assert_eq!(args.file, PathBuf::from("fresh.yaml"));
        assert!(!Args::try_parse_from(["cascade-app", "x.yaml"]).expect("parses").new);
        assert!(Args::try_parse_from(["cascade-app", "--new"]).is_err(), "needs a path");
    }

    #[test]
    fn initial_mode() {
        let parse = |args: &[&str]| Args::try_parse_from(args).expect("parses").initial_mode();
        assert_eq!(parse(&["cascade-app", "x.yaml"]), AppMode::View);
        assert_eq!(parse(&["cascade-app", "--new", "x.yaml"]), AppMode::Build);
        assert_eq!(parse(&["cascade-app", "x.yaml", "--mode", "play"]), AppMode::Play);
        assert_eq!(parse(&["cascade-app", "--new", "x.yaml", "--mode", "view"]), AppMode::View);
        assert!(Args::try_parse_from(["cascade-app", "x.yaml", "--mode", "edit"]).is_err());
    }
}
