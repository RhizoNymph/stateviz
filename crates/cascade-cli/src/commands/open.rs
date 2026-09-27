//! `cascade open`: launch the native app (`cascade-app`) next to this binary,
//! or on `PATH`.

use std::path::Path;
use std::process::{Command, ExitCode};

use anyhow::Context;

pub fn run(file: &Path, view: Option<&str>) -> anyhow::Result<ExitCode> {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("cascade-app")))
        .filter(|p| p.exists());
    let program = sibling.map_or_else(|| "cascade-app".into(), |p| p.into_os_string());
    let mut cmd = Command::new(program);
    cmd.arg(file);
    if let Some(link) = view {
        cmd.arg("--view").arg(link);
    }
    cmd.spawn().context("cannot start cascade-app")?;
    Ok(ExitCode::SUCCESS)
}
