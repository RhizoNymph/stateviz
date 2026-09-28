//! The definition file on disk: atomic writes, recognising the app's own
//! writes, and the starter text for a new file.
//!
//! The file watcher reports every change to the definition, including the
//! ones the app makes when it saves an edit. [`DiskSync`] remembers a hash
//! of the text the app last read or wrote; a reload whose text hashes the
//! same is the app's own write (or a touch without changes) and is ignored,
//! so the undo history survives. Any other text is an external edit.

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The text written by `cascade-app --new`: one machine with one state.
pub const NEW_FILE_TEXT: &str = "\
# A Cascade definition. Build it in the app (Build mode) or edit it by hand.
machines:
  Machine:
    states: [idle]
";

/// FNV-1a over the bytes: deterministic across runs and platforms, unlike
/// the std hasher, so logs can quote it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContentHash(pub u64);

impl ContentHash {
    pub fn of(text: &str) -> ContentHash {
        const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        ContentHash(text.bytes().fold(OFFSET, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(PRIME)))
    }
}

/// What a reload read, compared with what the app knows is on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskChange {
    /// The same text the app last read or wrote: its own write echoing back.
    Own,
    /// Someone else changed the file.
    External,
}

/// The app's idea of the definition's content on disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiskSync {
    known: Option<ContentHash>,
}

impl DiskSync {
    /// The app read `text` from the file, or wrote it there.
    pub fn note(&mut self, text: &str) {
        self.known = Some(ContentHash::of(text));
    }

    /// Forget what was known (a different file was opened).
    pub fn reset(&mut self) {
        self.known = None;
    }

    pub fn classify(&self, text: &str) -> DiskChange {
        if self.known == Some(ContentHash::of(text)) { DiskChange::Own } else { DiskChange::External }
    }
}

/// The temporary file an atomic write goes through: hidden, next to the
/// target (same filesystem, so the rename is atomic), and never classified
/// as the definition by the watcher.
pub fn temp_path(path: &Path) -> PathBuf {
    let name = path.file_name().map_or_else(|| "definition".into(), |n| n.to_string_lossy().into_owned());
    path.with_file_name(format!(".{name}.cascade-app.tmp"))
}

/// Write `text` to `path` atomically: write a temporary file, flush it to
/// disk, rename it over the target.
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = temp_path(path);
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        // Best effort: the temporary file is useless now.
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Why a new file could not be created.
#[derive(Debug, thiserror::Error)]
pub enum NewFileError {
    #[error("{} already exists; open it without --new", .0.display())]
    Exists(PathBuf),
    #[error("cannot create {}: {source}", path.display())]
    Io { path: PathBuf, source: std::io::Error },
}

/// Create `path` with [`NEW_FILE_TEXT`] (and its parent directories).
/// Refuses to overwrite an existing file.
pub fn create_new(path: &Path) -> Result<(), NewFileError> {
    if path.exists() {
        return Err(NewFileError::Exists(path.to_owned()));
    }
    let io = |source| NewFileError::Io { path: path.to_owned(), source };
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    write_atomic(path, NEW_FILE_TEXT).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cascade-disk-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn hash_is_stable_and_content_sensitive() {
        assert_eq!(ContentHash::of(""), ContentHash(0xcbf2_9ce4_8422_2325));
        assert_eq!(ContentHash::of("a"), ContentHash(0xaf63_dc4c_8601_ec8c));
        assert_ne!(ContentHash::of("machines: {}"), ContentHash::of("machines: {} "));
    }

    #[test]
    fn own_writes_are_recognised() {
        let mut sync = DiskSync::default();
        assert_eq!(sync.classify("x"), DiskChange::External, "nothing known yet");
        sync.note("loaded text");
        assert_eq!(sync.classify("loaded text"), DiskChange::Own);
        sync.note("written text");
        assert_eq!(sync.classify("written text"), DiskChange::Own);
        assert_eq!(sync.classify("loaded text"), DiskChange::External, "only the latest counts");
        assert_eq!(sync.classify("hand edit"), DiskChange::External);
        sync.reset();
        assert_eq!(sync, DiskSync::default());
        assert_eq!(sync.classify("written text"), DiskChange::External);
    }

    #[test]
    fn temp_file_is_hidden_next_to_the_target() {
        assert_eq!(temp_path(Path::new("/w/cascade.yaml")), PathBuf::from("/w/.cascade.yaml.cascade-app.tmp"));
    }

    #[test]
    fn atomic_write_replaces_content_and_leaves_no_temp_file() {
        let dir = scratch("atomic");
        let path = dir.join("cascade.yaml");
        std::fs::write(&path, "old").expect("write");
        write_atomic(&path, "new").expect("atomic write");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "new");
        assert!(!temp_path(&path).exists());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn atomic_write_into_a_missing_directory_fails_cleanly() {
        let path = std::env::temp_dir().join("cascade-no-such-dir-xyz").join("a.yaml");
        assert!(write_atomic(&path, "x").is_err());
        assert!(!temp_path(&path).exists());
    }

    #[test]
    fn new_file_text_loads_as_one_machine_with_one_state() {
        let analyzed = crate::document::analyze_text(NEW_FILE_TEXT).expect("starter text loads");
        assert_eq!(analyzed.model.machine_count(), 1);
        assert_eq!(analyzed.model.state_count(), 1);
    }

    #[test]
    fn create_new_writes_the_starter_and_refuses_to_overwrite() {
        let dir = scratch("new");
        let path = dir.join("sub").join("system.yaml");
        create_new(&path).expect("created");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), NEW_FILE_TEXT);
        assert!(matches!(create_new(&path), Err(NewFileError::Exists(_))));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(dir.join("sub"));
        let _ = std::fs::remove_dir(&dir);
    }
}
