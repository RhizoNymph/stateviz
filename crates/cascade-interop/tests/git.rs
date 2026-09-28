//! Reading a file at a git revision, against a throwaway repository under
//! the system temp directory.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use cascade_interop::{GitError, read_at_rev};

/// A temporary git repository, removed on drop.
struct Repo {
    root: PathBuf,
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

impl Repo {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "cascade-git-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp repo dir");
        let repo = Self { root };
        repo.git(&["init", "-q"]);
        repo
    }

    fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
            .args(["-c", "init.defaultBranch=main", "-c", "core.hooksPath=/dev/null"])
            .args(args)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?} failed");
    }

    fn write(&self, relative: &str, text: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create dirs");
        }
        std::fs::write(&path, text).expect("write file");
        path
    }

    fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", message]);
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn reads_a_file_at_earlier_revisions() {
    let repo = Repo::new();
    let file = repo.write("defs/cascade.yaml", b"version: 1\n");
    repo.commit("one");
    repo.write("defs/cascade.yaml", b"version: 2\n");
    repo.commit("two");
    repo.write("defs/cascade.yaml", b"version: 3 (uncommitted)\n");

    assert_eq!(read_at_rev(&file, "HEAD").expect("HEAD"), "version: 2\n");
    assert_eq!(read_at_rev(&file, "HEAD~1").expect("HEAD~1"), "version: 1\n");
    assert_eq!(read_at_rev(&file, "main").expect("branch"), "version: 2\n");
}

#[test]
fn reads_files_deleted_from_the_working_tree() {
    let repo = Repo::new();
    let file = repo.write("cascade.yaml", b"machines: {}\n");
    repo.commit("add");
    std::fs::remove_file(&file).expect("delete");
    assert_eq!(read_at_rev(&file, "HEAD").expect("still in HEAD"), "machines: {}\n");
}

#[test]
fn works_with_paths_through_dot_dot_segments() {
    let repo = Repo::new();
    repo.write("a/cascade.yaml", b"x\n");
    std::fs::create_dir_all(repo.root.join("b")).expect("mkdir");
    repo.commit("add");
    let odd = repo.root.join("b").join("..").join("a").join("cascade.yaml");
    assert_eq!(read_at_rev(&odd, "HEAD").expect("normalized"), "x\n");
}

#[test]
fn unknown_revisions_and_missing_files_are_show_errors() {
    let repo = Repo::new();
    let file = repo.write("cascade.yaml", b"x\n");
    repo.commit("add");
    match read_at_rev(&file, "no-such-branch") {
        Err(GitError::Show { rev, path, stderr }) => {
            assert_eq!(rev, "no-such-branch");
            assert_eq!(path, Path::new("cascade.yaml"));
            assert!(!stderr.is_empty());
        }
        other => panic!("{other:?}"),
    }
    let later = repo.write("new.yaml", b"y\n");
    assert!(matches!(read_at_rev(&later, "HEAD"), Err(GitError::Show { .. })));
}

#[test]
fn non_utf8_content_is_reported() {
    let repo = Repo::new();
    let file = repo.write("cascade.yaml", &[0xff, 0xfe, 0x00, 0x41]);
    repo.commit("binary");
    assert!(matches!(read_at_rev(&file, "HEAD"), Err(GitError::NotUtf8 { .. })));
}

#[test]
fn directories_outside_any_repository_are_reported() {
    let dir = std::env::temp_dir().join(format!("cascade-no-repo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    // Only meaningful when the temp directory is not itself inside a repo.
    let inside = Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(true);
    if !inside {
        assert!(matches!(read_at_rev(&dir.join("cascade.yaml"), "HEAD"), Err(GitError::NotARepository { .. })));
    }
    let _ = std::fs::remove_dir(&dir);
}

#[test]
fn option_like_revisions_are_rejected_before_running_git() {
    let repo = Repo::new();
    let file = repo.write("cascade.yaml", b"x\n");
    repo.commit("add");
    assert!(matches!(read_at_rev(&file, "--output=/tmp/pwned"), Err(GitError::InvalidRev { .. })));
}
