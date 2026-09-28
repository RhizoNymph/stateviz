//! `cascade`: check, render, export, import, diff and simulate Cascade
//! definitions from the command line and CI.
//!
//! Exit codes: 0 success; 1 `check` found errors (or warnings with
//! `--deny-warnings`); 2 the definition or arguments are invalid, or I/O
//! failed.

mod commands;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "cascade", version, about = "A visualizer and checker for interacting state machines")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Validate a definition and run the static checks.
    Check {
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
        /// Treat warnings as errors for the exit code.
        #[arg(long)]
        deny_warnings: bool,
    },
    /// Render a view to SVG or PNG (chosen by the output extension).
    Render {
        file: PathBuf,
        #[arg(long, default_value = "causal")]
        view: String,
        /// A `cascade://` view link to apply (selection, cone, filters…).
        #[arg(long)]
        state: Option<String>,
        /// Scenario file for the trace view.
        #[arg(long)]
        scenario: Option<PathBuf>,
        #[arg(long, short)]
        out: PathBuf,
        /// Use the dark theme.
        #[arg(long)]
        dark: bool,
        /// Pixels per scene unit for PNG output.
        #[arg(long, default_value_t = 2.0)]
        scale: f32,
    },
    /// Export a definition to another format: scxml, mermaid, p or yaml.
    Export {
        file: PathBuf,
        #[arg(long)]
        to: String,
        /// Write here instead of standard output.
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Convert an XState (JSON) or SCXML file into a Cascade YAML definition.
    Import {
        file: PathBuf,
        #[arg(long)]
        from: String,
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Summarize what changed in a definition between two git revisions.
    Diff {
        file: PathBuf,
        #[arg(long)]
        base: String,
        /// Defaults to the working tree.
        #[arg(long)]
        head: Option<String>,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Run a scenario through the simulator and print the trace.
    Simulate {
        file: PathBuf,
        /// The scenario to run; with `--interactive`, where to start (optional).
        #[arg(required_unless_present = "interactive")]
        scenario: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
        /// Print both orderings of this race candidate (numbered from 0 in
        /// `cascade check` order).
        #[arg(long)]
        race: Option<usize>,
        /// Play step by step at a prompt (type `help` for the commands).
        #[arg(long, conflicts_with_all = ["race", "format"])]
        interactive: bool,
    },
    /// Open the definition in the native app.
    Open {
        file: PathBuf,
        /// A `cascade://` view link to open at.
        #[arg(long)]
        view: Option<String>,
    },
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let result = match cli.command {
        Command::Check { file, format, deny_warnings } => commands::check::run(&file, format, deny_warnings),
        Command::Render { file, view, state, scenario, out, dark, scale } => {
            commands::render::run(&commands::render::RenderArgs { file, view, state, scenario, out, dark, scale })
        }
        Command::Export { file, to, out } => commands::export::run(&file, &to, out.as_deref()),
        Command::Import { file, from, out } => commands::import::run(&file, &from, out.as_deref()),
        Command::Diff { file, base, head, format } => commands::diff::run(&file, &base, head.as_deref(), format),
        Command::Simulate { file, scenario, format, race, interactive } => {
            commands::simulate::run(&file, scenario.as_deref(), format, race, interactive)
        }
        Command::Open { file, view } => commands::open::run(&file, view.as_deref()),
    };
    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}
