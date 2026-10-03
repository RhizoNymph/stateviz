//! One module per subcommand. Each `run` returns the process exit code, or
//! an error for exit code 2.

pub mod check;
pub mod diff;
pub mod export;
pub mod import;
pub mod new;
pub mod open;
pub mod render;
pub mod simulate;

use std::path::Path;
use std::process::ExitCode;

use cascade_core::{LoadFileError, Model};

/// Load a definition, printing diagnostics as `path:line:col: error: …`.
/// `Ok(Err(code))` means the definition is invalid and the caller should
/// exit with `code`.
pub fn load_or_report(file: &Path) -> anyhow::Result<Result<Model, ExitCode>> {
    match cascade_core::load_file(file) {
        Ok(model) => Ok(Ok(model)),
        Err(LoadFileError::Invalid { path, source }) => {
            for d in &source.diagnostics {
                eprintln!("{}:{}: error: {}", path.display(), d.span, d.kind);
            }
            Ok(Err(ExitCode::from(2)))
        }
        Err(err @ LoadFileError::Io { .. }) => Err(err.into()),
    }
}

/// Write to `out`, or standard output when `None`.
pub fn write_output(out: Option<&Path>, text: &str) -> anyhow::Result<()> {
    match out {
        Some(path) => std::fs::write(path, text).map_err(|e| anyhow::anyhow!("cannot write {}: {e}", path.display())),
        None => {
            print!("{text}");
            Ok(())
        }
    }
}
