//! `cascade open`: launch the native app (`cascade-app`) next to this binary,
//! or on `PATH`.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitCode};

use anyhow::Context;

pub fn run(file: &Path, view: Option<&str>) -> anyhow::Result<ExitCode> {
    let mut args: Vec<OsString> = vec![file.into()];
    if let Some(link) = view {
        args.extend(["--view".into(), link.into()]);
    }
    launch(&args)?;
    Ok(ExitCode::SUCCESS)
}

/// Start `cascade-app` with `args` and return without waiting for it.
pub fn launch(args: &[OsString]) -> anyhow::Result<()> {
    Command::new(app_program()).args(args).spawn().context("cannot start cascade-app")?;
    Ok(())
}

/// `cascade-app` beside this binary when it exists there, otherwise the
/// name alone so `PATH` decides.
fn app_program() -> OsString {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("cascade-app")))
        .filter(|p| p.exists())
        .map_or_else(|| "cascade-app".into(), |p| p.into_os_string())
}
