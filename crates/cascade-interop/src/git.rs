//! Reading a definition file as it was at a git revision (diff mode).
//!
//! Shells out to the `git` binary so every repository layout git supports
//! (worktrees, submodules, sparse checkouts) works without a library.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git is not available: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("{} does not name a file", path.display())]
    InvalidPath { path: PathBuf },
    #[error("`{rev}` is not a valid revision name")]
    InvalidRev { rev: String },
    #[error("{} is not inside a git repository", path.display())]
    NotARepository { path: PathBuf },
    #[error("{} is outside the repository at {}", path.display(), root.display())]
    OutsideRepository { path: PathBuf, root: PathBuf },
    #[error("git could not read `{rev}:{}`: {stderr}", path.display())]
    Show { rev: String, path: PathBuf, stderr: String },
    #[error("git output for `{rev}` is not UTF-8")]
    NotUtf8 { rev: String },
}

/// The contents of `file` (a path in the working tree) at git revision
/// `rev`, as `git show <rev>:<path-relative-to-repo-root>` prints it.
///
/// `file` need not exist in the working tree (it may have been deleted
/// since `rev`), but its directory must.
pub fn read_at_rev(file: &Path, rev: &str) -> Result<String, GitError> {
    // A revision starting with `-` would be read as an option by git.
    if rev.is_empty() || rev.starts_with('-') || rev.contains(['\0', '\n']) {
        return Err(GitError::InvalidRev { rev: rev.to_owned() });
    }
    let file_name = file.file_name().ok_or_else(|| GitError::InvalidPath { path: file.to_owned() })?;
    let dir = match file.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_owned(),
        _ => PathBuf::from("."),
    };

    let toplevel = run_git(&dir, &[OsString::from("rev-parse"), OsString::from("--show-toplevel")])?;
    if !toplevel.status.success() {
        return Err(GitError::NotARepository { path: file.to_owned() });
    }
    let root = PathBuf::from(String::from_utf8_lossy(&toplevel.stdout).trim_end_matches(['\n', '\r']));

    // git prints the resolved top level, so resolve the file's directory the
    // same way before taking the relative path.
    let real_dir = dir.canonicalize().map_err(|_| GitError::NotARepository { path: file.to_owned() })?;
    let real_root = root.canonicalize().unwrap_or_else(|_| root.clone());
    let relative_dir = real_dir
        .strip_prefix(&real_root)
        .map_err(|_| GitError::OutsideRepository { path: file.to_owned(), root: real_root.clone() })?;

    let relative = relative_dir.join(file_name);
    let mut spec = OsString::from(rev);
    spec.push(":");
    spec.push(git_path(&relative));

    let shown = run_git(&dir, &[OsString::from("show"), spec])?;
    if !shown.status.success() {
        return Err(GitError::Show {
            rev: rev.to_owned(),
            path: relative,
            stderr: String::from_utf8_lossy(&shown.stderr).trim().to_owned(),
        });
    }
    String::from_utf8(shown.stdout).map_err(|_| GitError::NotUtf8 { rev: rev.to_owned() })
}

fn run_git(dir: &Path, args: &[OsString]) -> Result<Output, GitError> {
    Command::new("git").arg("-C").arg(dir).args(args).output().map_err(GitError::Spawn)
}

/// A repository-relative path with `/` separators, as git expects in
/// `<rev>:<path>` on every platform.
fn git_path(relative: &Path) -> OsString {
    let mut out = OsString::new();
    for (i, component) in relative.components().filter(|c| matches!(c, Component::Normal(_))).enumerate() {
        if i > 0 {
            out.push("/");
        }
        out.push(component.as_os_str());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_paths_use_forward_slashes() {
        let path: PathBuf = ["examples", "shop", "cascade.yaml"].iter().collect();
        assert_eq!(git_path(&path), OsString::from("examples/shop/cascade.yaml"));
        assert_eq!(git_path(Path::new("cascade.yaml")), OsString::from("cascade.yaml"));
    }

    #[test]
    fn rejects_option_like_and_empty_revisions() {
        for rev in ["", "--output=/tmp/x", "-p", "HEAD\nmain"] {
            assert!(matches!(read_at_rev(Path::new("cascade.yaml"), rev), Err(GitError::InvalidRev { .. })), "{rev:?}");
        }
    }

    #[test]
    fn rejects_paths_without_a_file_name() {
        assert!(matches!(read_at_rev(Path::new("/"), "HEAD"), Err(GitError::InvalidPath { .. })));
    }
}
