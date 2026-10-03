//! `cascade new`: create a blank definition and open it in the app.
//! `--no-open` keeps these tests headless.

use std::path::PathBuf;
use std::process::{Command, Output};

/// A fresh directory under the system temp directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("cascade-cli-new-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cascade(dir: &TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cascade")).current_dir(&dir.0).args(args).output().expect("cascade runs")
}

#[test]
fn creates_a_blank_definition_that_loads() {
    let dir = TempDir::new("blank");
    let out = cascade(&dir, &["new", "system.yaml", "--no-open"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let path = dir.0.join("system.yaml");
    let text = std::fs::read_to_string(&path).expect("file written");
    let model = cascade_core::load_str(&text).expect("blank definition loads");
    assert_eq!(model.machine_count(), 0, "blank means no machines yet");
    let check = cascade(&dir, &["check", "system.yaml"]);
    assert_eq!(check.status.code(), Some(0), "a blank definition has no findings");
}

#[test]
fn defaults_to_cascade_yaml_in_the_current_directory() {
    let dir = TempDir::new("default");
    let out = cascade(&dir, &["new", "--no-open"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(dir.0.join("cascade.yaml").exists());
}

#[test]
fn creates_missing_parent_directories() {
    let dir = TempDir::new("parents");
    let out = cascade(&dir, &["new", "designs/shop/cascade.yaml", "--no-open"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(dir.0.join("designs/shop/cascade.yaml").exists());
}

#[test]
fn refuses_to_overwrite_an_existing_file() {
    let dir = TempDir::new("exists");
    let path = dir.0.join("system.yaml");
    std::fs::write(&path, "machines: { Keep: { states: [me] } }\n").expect("seed file");
    let out = cascade(&dir, &["new", "system.yaml", "--no-open"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("already exists"));
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "machines: { Keep: { states: [me] } }\n");
}
