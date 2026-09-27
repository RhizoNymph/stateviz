//! `cascade render` in diff mode: a `diff=` view link renders the working
//! tree merged with the ghosts of a git revision.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BEFORE: &str = "\
machines:
  Order:
    states: [draft, placed, archived]
    transitions:
      - { from: draft, to: placed, on: place }
      - { from: placed, to: archived, on: archive }
external:
  Customer: [Order.place]
  Janitor: [Order.archive]
";

/// Adds `placed → draft` (and its source), removes `placed → archived`.
const AFTER: &str = "\
machines:
  Order:
    states: [draft, placed, archived]
    transitions:
      - { from: draft, to: placed, on: place }
      - { from: placed, to: draft, on: reopen }
external:
  Customer: [Order.place, Order.reopen]
";

/// A fresh git repository under the system temp directory, removed on drop.
struct Repo(PathBuf);

impl Repo {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("cascade-render-diff-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let repo = Self(dir);
        repo.git(&["init", "-q"]);
        repo
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
            .args(["-c", "init.defaultBranch=main", "-c", "core.hooksPath=/dev/null"])
            .args(args)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?} failed");
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn render(definition: &Path, link: &str, out: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cascade"))
        .args(["render"])
        .arg(definition)
        .args(["--view", "causal", "--state", link, "--out"])
        .arg(out)
        .output()
        .expect("cascade runs")
}

fn committed_then_edited(tag: &str) -> Repo {
    let repo = Repo::new(tag);
    std::fs::write(repo.path("cascade.yaml"), BEFORE).expect("write");
    repo.git(&["add", "cascade.yaml"]);
    repo.git(&["commit", "-q", "-m", "before"]);
    std::fs::write(repo.path("cascade.yaml"), AFTER).expect("write");
    repo
}

#[test]
fn diff_link_decorates_added_and_removed_elements() {
    let repo = committed_then_edited("decorates");
    let out = repo.path("diff.svg");
    let output = render(&repo.path("cascade.yaml"), "cascade://causal?diff=HEAD,", &out);
    assert!(output.status.success(), "render failed: {}", String::from_utf8_lossy(&output.stderr));
    let svg = std::fs::read_to_string(&out).expect("svg written");
    // The light theme's added (green) and removed (red) outline colors.
    assert!(svg.contains("#1a7f37"), "added elements are outlined green");
    assert!(svg.contains("#cf222e"), "removed elements are drawn as red ghosts");
    // The removed transition is still drawn, as a ghost.
    assert!(svg.contains("placed → archived"), "the removed transition appears as a ghost");
    assert!(svg.contains("placed → draft"), "the added transition appears");
}

#[test]
fn without_a_diff_link_nothing_is_decorated() {
    let repo = committed_then_edited("plain");
    let out = repo.path("plain.svg");
    let output = render(&repo.path("cascade.yaml"), "cascade://causal", &out);
    assert!(output.status.success(), "render failed: {}", String::from_utf8_lossy(&output.stderr));
    let svg = std::fs::read_to_string(&out).expect("svg written");
    assert!(!svg.contains("#1a7f37"));
    assert!(!svg.contains("placed → archived"), "no ghosts outside diff mode");
}

#[test]
fn unknown_base_revision_fails_with_exit_code_2() {
    let repo = committed_then_edited("badrev");
    let out = repo.path("x.svg");
    let output = render(&repo.path("cascade.yaml"), "cascade://causal?diff=no-such-rev,", &out);
    assert_eq!(output.status.code(), Some(2));
    assert!(!out.exists());
}
