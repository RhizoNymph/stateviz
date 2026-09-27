//! Watching the definition, its pins sidecar and its scenarios.
//!
//! `notify` delivers events on its own thread. The handler classifies each
//! path ([`WatchTargets::classify`]) and sends the resulting [`Changes`]
//! down an unbounded channel; the workspace drains it from a GPUI task,
//! debounces, and reloads. The directory is watched rather than the file,
//! because editors often save by writing a new file and renaming it over
//! the old one, which would orphan a watch on the file itself.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use notify::event::{AccessKind, AccessMode};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::document::scenarios::{SCENARIO_DIR, is_scenario_path};

/// How long to wait after the first event for a burst of events to settle.
pub const DEBOUNCE: Duration = Duration::from_millis(120);

/// Which inputs changed on disk. Merging is a set union.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub definition: bool,
    pub sidecar: bool,
    pub scenarios: bool,
}

impl Changes {
    pub const fn is_empty(self) -> bool {
        !(self.definition || self.sidecar || self.scenarios)
    }

    #[must_use]
    pub const fn merge(self, other: Changes) -> Changes {
        Changes {
            definition: self.definition || other.definition,
            sidecar: self.sidecar || other.sidecar,
            scenarios: self.scenarios || other.scenarios,
        }
    }
}

/// The files a document depends on, as absolute paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchTargets {
    pub definition: PathBuf,
    pub sidecar: PathBuf,
}

impl WatchTargets {
    /// The directory holding the definition.
    pub fn dir(&self) -> &Path {
        self.definition.parent().unwrap_or_else(|| Path::new("/"))
    }

    pub fn scenario_dir(&self) -> PathBuf {
        self.dir().join(SCENARIO_DIR)
    }

    /// What a change to `path` affects.
    pub fn classify(&self, path: &Path) -> Changes {
        Changes {
            definition: path == self.definition,
            sidecar: path == self.sidecar,
            scenarios: is_scenario_path(path, self.dir()),
        }
    }
}

/// Whether an event kind can mean new content. Opens and reads cannot;
/// closing a file written to can (inotify's "write finished").
pub fn is_relevant(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        EventKind::Any | EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Other => true,
    }
}

/// Everything an event changes.
pub fn classify_event(targets: &WatchTargets, event: &Event) -> Changes {
    if !is_relevant(&event.kind) {
        return Changes::default();
    }
    event.paths.iter().fold(Changes::default(), |acc, path| acc.merge(targets.classify(path)))
}

/// A running watcher. Dropping it stops the watch and closes the channel.
pub struct FileWatcher {
    watcher: RecommendedWatcher,
    targets: WatchTargets,
    watching_scenario_dir: bool,
}

impl FileWatcher {
    /// Watch the definition's directory (and `scenarios/` if present).
    /// Changes arrive on the returned receiver.
    pub fn start(targets: WatchTargets) -> Result<(FileWatcher, UnboundedReceiver<Changes>), notify::Error> {
        let (tx, rx) = unbounded();
        let handler_targets = targets.clone();
        let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| match result {
            Ok(event) => {
                let changes = classify_event(&handler_targets, &event);
                if !changes.is_empty() {
                    // The receiver is gone only when the workspace closed.
                    let _ = tx.unbounded_send(changes);
                }
            }
            Err(error) => tracing::warn!(%error, "file watch error"),
        })?;
        watcher.watch(targets.dir(), RecursiveMode::NonRecursive)?;
        let mut this = FileWatcher { watcher, targets, watching_scenario_dir: false };
        this.refresh_scenario_dir();
        Ok((this, rx))
    }

    /// Start watching `scenarios/` once it exists (it may be created after
    /// the app started).
    pub fn refresh_scenario_dir(&mut self) {
        let dir = self.targets.scenario_dir();
        if self.watching_scenario_dir || !dir.is_dir() {
            return;
        }
        match self.watcher.watch(&dir, RecursiveMode::NonRecursive) {
            Ok(()) => self.watching_scenario_dir = true,
            Err(error) => tracing::warn!(%error, dir = %dir.display(), "cannot watch scenario directory"),
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::channel::mpsc::TryRecvError;
    use notify::event::{CreateKind, DataChange, ModifyKind, RemoveKind};

    use super::*;

    fn targets() -> WatchTargets {
        WatchTargets { definition: "/w/cascade.yaml".into(), sidecar: "/w/cascade.layout.json".into() }
    }

    fn event(kind: EventKind, paths: &[&str]) -> Event {
        let mut e = Event::new(kind);
        for p in paths {
            e = e.add_path(PathBuf::from(p));
        }
        e
    }

    #[test]
    fn classifies_each_input() {
        let t = targets();
        assert_eq!(t.classify(Path::new("/w/cascade.yaml")), Changes { definition: true, ..Changes::default() });
        assert_eq!(t.classify(Path::new("/w/cascade.layout.json")), Changes { sidecar: true, ..Changes::default() });
        assert_eq!(t.classify(Path::new("/w/scenarios/a.yaml")), Changes { scenarios: true, ..Changes::default() });
        assert_eq!(t.classify(Path::new("/w/a.scenario.yaml")), Changes { scenarios: true, ..Changes::default() });
        assert!(t.classify(Path::new("/w/cascade.layout.json.tmp")).is_empty());
        assert!(t.classify(Path::new("/w/.cascade.yaml.swp")).is_empty());
        assert!(t.classify(Path::new("/other/cascade.yaml")).is_empty());
    }

    #[test]
    fn only_content_events_count() {
        assert!(is_relevant(&EventKind::Modify(ModifyKind::Data(DataChange::Content))));
        assert!(is_relevant(&EventKind::Create(CreateKind::File)));
        assert!(is_relevant(&EventKind::Remove(RemoveKind::File)));
        assert!(is_relevant(&EventKind::Access(AccessKind::Close(AccessMode::Write))));
        assert!(!is_relevant(&EventKind::Access(AccessKind::Open(AccessMode::Read))));
        assert!(!is_relevant(&EventKind::Access(AccessKind::Close(AccessMode::Read))));
    }

    #[test]
    fn events_merge_their_paths() {
        let t = targets();
        let e = event(EventKind::Modify(ModifyKind::Any), &["/w/cascade.yaml", "/w/cascade.layout.json", "/w/x"]);
        assert_eq!(classify_event(&t, &e), Changes { definition: true, sidecar: true, scenarios: false });
        let read = event(EventKind::Access(AccessKind::Read), &["/w/cascade.yaml"]);
        assert!(classify_event(&t, &read).is_empty());
    }

    #[test]
    fn merge_is_a_union() {
        let a = Changes { definition: true, ..Changes::default() };
        let b = Changes { scenarios: true, ..Changes::default() };
        assert_eq!(a.merge(b), Changes { definition: true, sidecar: false, scenarios: true });
        assert!(Changes::default().is_empty());
    }

    #[test]
    fn watcher_reports_a_write() {
        let dir = std::env::temp_dir().join(format!("cascade-watch-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let definition = dir.join("cascade.yaml");
        std::fs::write(&definition, "a").expect("write");
        let targets = WatchTargets { definition: definition.clone(), sidecar: dir.join("cascade.layout.json") };
        let (_watcher, mut rx) = FileWatcher::start(targets).expect("watcher starts");
        std::fs::write(&definition, "b").expect("write");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let got = loop {
            match rx.try_recv() {
                Ok(changes) if changes.definition => break true,
                Ok(_) => {}
                Err(TryRecvError::Closed) => break false,
                Err(TryRecvError::Empty) if std::time::Instant::now() > deadline => break false,
                Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        assert!(got, "no definition change reported");
        let _ = std::fs::remove_file(&definition);
        let _ = std::fs::remove_dir(&dir);
    }
}
