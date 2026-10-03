//! `cascade new`: write a blank definition and open it in the app's Build
//! mode, ready to add the first machine.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::commands::open::launch;

/// The blank definition: no machines yet. It loads, and the app's first
/// "add machine" edit patches it in place (keeping the comment).
pub const BLANK_DEFINITION: &str = "\
# A Cascade definition. Build it in the app (Build mode, ctrl/cmd-2) or edit
# it by hand; see docs/features/definition-format.md for the format.
machines: {}
";

#[derive(Debug, thiserror::Error)]
pub enum NewError {
    #[error("{} already exists; open it with `cascade open {}`", .0.display(), .0.display())]
    Exists(PathBuf),
    #[error("cannot create {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub fn run(file: &Path, open: bool) -> anyhow::Result<ExitCode> {
    create_blank(file)?;
    eprintln!("created {}", file.display());
    if open {
        launch(&[file.into(), OsString::from("--mode"), OsString::from("build")])?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Create `file` with [`BLANK_DEFINITION`], making parent directories as
/// needed. Never overwrites: an existing file is an error and stays as is.
pub fn create_blank(file: &Path) -> Result<(), NewError> {
    let io = |source| NewError::Io { path: file.to_owned(), source };
    if let Some(parent) = file.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    let mut handle = match std::fs::OpenOptions::new().write(true).create_new(true).open(file) {
        Ok(handle) => handle,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => return Err(NewError::Exists(file.to_owned())),
        Err(err) => return Err(io(err)),
    };
    handle.write_all(BLANK_DEFINITION.as_bytes()).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_definition_loads_with_no_machines() {
        let model = cascade_core::load_str(BLANK_DEFINITION).expect("loads");
        assert_eq!(model.machine_count(), 0);
    }
}
