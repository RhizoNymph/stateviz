//! What the user can do: commands, selection, pins, search, diff, links,
//! click-to-source, and keeping up with files on disk.

use std::path::PathBuf;
use std::time::Duration;

use cascade_core::{Check, ElementKey};
use cascade_scene::{ViewKind, load_sidecar, save_sidecar};
use chrono::Local;
use futures::StreamExt;
use gpui::{AppContext as _, ClipboardItem, Context, Focusable, Window};

use super::{Status, Workspace};
use crate::build::disk::{DiskChange, DiskSync};
use crate::build::undo::Direction;
use crate::commands::{self, Command, HostEffect, Outcome};
use crate::diffmode::refs_from_fields;
use crate::document::{Document, Reread, load_definition, reread, scenarios};
use crate::editor;
use crate::input::InputEvent;
use crate::link;
use crate::locate;
use crate::mode::AppMode;
use crate::viewport;
use crate::watch::{Changes, DEBOUNCE, FileWatcher, WatchTargets};

/// Search results shown in the dropdown.
const SEARCH_LIMIT: usize = 12;
/// How often a spawned editor process is checked for exit.
const REAP_INTERVAL: Duration = Duration::from_millis(500);

/// Views where dragging a node pins it (the others have fixed layouts).
pub fn pins_supported(view: ViewKind) -> bool {
    matches!(view, ViewKind::Causal | ViewKind::Structure)
}

impl Workspace {
    pub(crate) fn set_status(&mut self, text: impl Into<String>, is_error: bool) {
        self.status_serial += 1;
        self.status = Some(Status { text: text.into(), is_error, serial: self.status_serial });
    }

    // --- Files ---------------------------------------------------------------

    /// Load synchronously at startup (and when opening another file) so
    /// the first frame has content.
    pub(super) fn load_initial(&mut self) {
        let result = match reread(self.doc.path(), DiskSync::default()) {
            Reread::Changed { text, result } => {
                if let Some(text) = &text {
                    self.disk.note(text);
                }
                *result
            }
            // Unreachable with nothing known; treat it as an empty read.
            Reread::Unchanged => load_definition(self.doc.path()),
        };
        if let Err(failure) = &result {
            tracing::warn!(path = %self.doc.path().display(), problems = failure.count(), "definition does not load");
        }
        self.doc.apply(result, Local::now());
        self.reload_sidecar();
        self.scenarios = scenarios::discover(self.doc.path());
        if let Some(loaded) = self.doc.loaded() {
            tracing::info!(
                path = %self.doc.path().display(),
                machines = loaded.model.machine_count(),
                findings = loaded.findings.len(),
                scenarios = self.scenarios.len(),
                "definition loaded"
            );
            let unknown = link::unknown_keys(&self.view, &loaded.model).len();
            if unknown > 0 {
                self.set_status(format!("{unknown} linked element(s) are not in this definition"), false);
            }
        }
    }

    /// Switch the window to another definition file (after "New").
    pub(crate) fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        tracing::info!(path = %path.display(), "opening");
        self.watcher = None;
        self._watch_task = None;
        self.reload_task = None;
        self.sidecar_path = cascade_scene::sidecar_path(&path);
        self.sidecar = cascade_scene::LayoutSidecar::default();
        self.doc = Document::new(path);
        self.disk.reset();
        self.builder.reset();
        self.build = super::building::BuildUi::default();
        self.play.state = None;
        self.view.selection.clear();
        self.view.cone = None;
        self.view.scenario = None;
        self.view.race = None;
        self.view.viewport = None;
        self.view.hidden_machines.clear();
        self.parked_viewports.clear();
        self.invalidate_traces();
        self.load_initial();
        self.start_watching(cx);
        window.set_window_title(&super::window_title(self.doc.path()));
        self.set_status(format!("Opened {}", self.doc.path().display()), false);
        self.changed(cx);
    }

    /// Switch between View, Build and Play.
    pub(crate) fn set_mode(&mut self, mode: AppMode, cx: &mut Context<Self>) {
        if mode == self.mode {
            return;
        }
        tracing::info!(mode = mode.label(), "mode");
        let view = mode.entry_view(self.view.view);
        self.switch_view(view);
        self.mode = mode;
        match mode {
            AppMode::Play => self.ensure_play(),
            AppMode::Build => {
                if let Err(message) = self.editable() {
                    self.set_status(message, true);
                }
            }
            AppMode::View => {}
        }
        self.changed(cx);
    }

    pub(super) fn start_watching(&mut self, cx: &mut Context<Self>) {
        let targets = WatchTargets { definition: self.doc.path().to_owned(), sidecar: self.sidecar_path.clone() };
        match FileWatcher::start(targets) {
            Ok((watcher, mut rx)) => {
                self.watcher = Some(watcher);
                self._watch_task = Some(cx.spawn(async move |this, cx| {
                    while let Some(first) = rx.next().await {
                        cx.background_executor().timer(DEBOUNCE).await;
                        let mut changes = first;
                        while let Ok(more) = rx.try_recv() {
                            changes = changes.merge(more);
                        }
                        if this.update(cx, |ws, cx| ws.on_files_changed(changes, cx)).is_err() {
                            break;
                        }
                    }
                }));
            }
            Err(error) => {
                tracing::warn!(%error, "live reload unavailable");
                self.set_status(format!("Live reload unavailable: {error}"), true);
            }
        }
    }

    fn on_files_changed(&mut self, changes: Changes, cx: &mut Context<Self>) {
        tracing::debug!(?changes, "files changed");
        if changes.definition {
            self.start_reload(cx);
        }
        if changes.sidecar && self.reload_sidecar() {
            self.changed(cx);
        }
        if changes.scenarios {
            if let Some(watcher) = &mut self.watcher {
                watcher.refresh_scenario_dir();
            }
            self.scenarios = scenarios::discover(self.doc.path());
            self.invalidate_traces();
            self.changed(cx);
        }
    }

    /// Re-read the definition on a background thread. The app's own saves
    /// come back unchanged and are ignored (keeping the undo history); any
    /// other change reloads and clears the history.
    pub(crate) fn start_reload(&mut self, cx: &mut Context<Self>) {
        let path = self.doc.path().to_owned();
        let known = self.disk;
        let work = cx.background_spawn(async move { reread(&path, known) });
        self.reload_task = Some(cx.spawn(async move |this, cx| {
            let reread = work.await;
            if let Err(error) = this.update(cx, |ws, cx| ws.finish_reload(reread, cx)) {
                tracing::debug!(%error, "workspace gone before the reload finished");
            }
        }));
    }

    fn finish_reload(&mut self, reread: Reread, cx: &mut Context<Self>) {
        let (text, result) = match reread {
            Reread::Unchanged => {
                tracing::debug!("definition unchanged (own write)");
                return;
            }
            Reread::Changed { text, result } => (text, *result),
        };
        match &text {
            Some(text) => {
                // Re-check against what is known now: another save may have
                // happened while this read was in flight.
                if self.disk.classify(text) == DiskChange::Own {
                    tracing::debug!("definition unchanged (own write)");
                    return;
                }
                self.disk.note(text);
            }
            // Unreadable (e.g. mid-rename): whatever comes back next is new.
            None => self.disk.reset(),
        }
        match &result {
            Ok(_) => tracing::info!(path = %self.doc.path().display(), "definition reloaded"),
            Err(failure) => tracing::warn!(problems = failure.count(), "reload failed; keeping last good model"),
        }
        let reloaded = result.is_ok();
        self.doc.apply(result, Local::now());
        if !self.build.history.is_empty() {
            self.build.history.clear();
            self.set_status("The file changed on disk; the undo history was cleared", false);
        }
        if reloaded {
            self.on_model_replaced();
        }
        self.changed(cx);
    }

    /// Re-read the pins sidecar; true when it changed.
    fn reload_sidecar(&mut self) -> bool {
        match load_sidecar(&self.sidecar_path) {
            Ok(sidecar) => {
                self.sidecar_error = None;
                if sidecar == self.sidecar {
                    return false;
                }
                self.sidecar = sidecar;
                true
            }
            Err(error) => {
                tracing::warn!(%error, "cannot load pins");
                self.sidecar_error = Some(error.to_string());
                false
            }
        }
    }

    // --- Commands ------------------------------------------------------------

    pub(crate) fn run_command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        let before = self.view.view;
        match commands::reduce(command, &mut self.view, &mut self.cone_depth) {
            Outcome::Unchanged => {
                if matches!(command, Command::ConeForward | Command::ConeBackward) {
                    self.set_status("Select an element first, then press F or B", false);
                    cx.notify();
                }
            }
            Outcome::ViewChanged => {
                if self.view.view != before {
                    // `reduce` switched views; move the viewports along.
                    let after = self.view.view;
                    self.view.view = before;
                    self.switch_view(after);
                }
                if command == Command::TogglePills {
                    self.save_settings(cx);
                }
                if self.view.search.is_none() && !self.search.input.read(cx).text().is_empty() {
                    // Esc cleared the search highlight: clear the box to match.
                    self.search.input.update(cx, |input, cx| input.set_text("", cx));
                }
                self.changed(cx);
            }
            Outcome::Host(effect) => self.host_effect(effect, window, cx),
        }
    }

    fn host_effect(&mut self, effect: HostEffect, window: &mut Window, cx: &mut Context<Self>) {
        match effect {
            HostEffect::Zoom(factor) => {
                if let (Some(vp), Some(canvas)) = (self.effective_viewport(), self.canvas_rect()) {
                    self.view.viewport = Some(viewport::zoom_about(vp, canvas, canvas.center(), factor));
                    cx.notify();
                }
            }
            HostEffect::Fit => {
                self.view.viewport = None;
                cx.notify();
            }
            HostEffect::Pan(dx, dy) => {
                if let Some(vp) = self.effective_viewport() {
                    self.view.viewport = Some(viewport::pan_by(vp, dx, dy));
                    cx.notify();
                }
            }
            HostEffect::FocusSearch => {
                let handle = self.search.input.focus_handle(cx);
                window.focus(&handle, cx);
                self.search.open = !self.search.hits.is_empty();
                cx.notify();
            }
            HostEffect::CopyLink => self.copy_link(cx),
            HostEffect::PasteLink => self.paste_link(window, cx),
            HostEffect::ToggleTheme => self.toggle_theme(window, cx),
            HostEffect::OpenSource => match self.view.selection.first().cloned() {
                Some(key) => self.open_source(&key, cx),
                None => {
                    self.set_status("Select an element to open its source", false);
                    cx.notify();
                }
            },
            HostEffect::UnpinSelection => {
                let keys = self.view.selection.clone();
                if keys.is_empty() {
                    self.set_status("Select a pinned node to unpin it", false);
                    cx.notify();
                }
                for key in keys {
                    self.unpin(&key, cx);
                }
            }
            HostEffect::Reload => {
                self.start_reload(cx);
                if self.reload_sidecar() {
                    self.changed(cx);
                }
                self.scenarios = scenarios::discover(self.doc.path());
                self.invalidate_traces();
                self.set_status("Reloading…", false);
                self.changed(cx);
            }
            HostEffect::Quit => cx.quit(),
            HostEffect::SetMode(mode) => self.set_mode(mode, cx),
            HostEffect::Undo => self.undo_redo(Direction::Undo, cx),
            HostEffect::Redo => self.undo_redo(Direction::Redo, cx),
            HostEffect::DeleteSelection => self.delete_selection(cx),
            HostEffect::NewFile => self.new_file(window, cx),
            HostEffect::PlayStep => self.play_step(None, cx),
            HostEffect::PlayRun => self.play_run(cx),
        }
    }

    // --- Selection -------------------------------------------------------------

    pub(crate) fn select_key(&mut self, key: ElementKey, cx: &mut Context<Self>) {
        if commands::select(&mut self.view, key) {
            self.changed(cx);
        }
    }

    pub(crate) fn add_key(&mut self, key: ElementKey, cx: &mut Context<Self>) {
        commands::add_to_selection(&mut self.view, key);
        if commands::is_path_query(&self.view) {
            self.set_status("Path query: every causal path between the two transitions", false);
        }
        self.changed(cx);
    }

    pub(crate) fn toggle_machine(&mut self, machine: &str, cx: &mut Context<Self>) {
        commands::toggle_machine(&mut self.view, machine);
        self.changed(cx);
    }

    pub(crate) fn show_all_machines(&mut self, cx: &mut Context<Self>) {
        self.view.hidden_machines.clear();
        self.changed(cx);
    }

    pub(crate) fn open_pair(&mut self, row: String, column: String, cx: &mut Context<Self>) {
        self.switch_view(ViewKind::Causal);
        commands::open_pair(&mut self.view, row, column);
        self.changed(cx);
    }

    pub(crate) fn show_view(&mut self, view: ViewKind, cx: &mut Context<Self>) {
        if self.switch_view(view) {
            self.changed(cx);
        }
    }

    // --- Findings ----------------------------------------------------------------

    /// Focus a finding: select its primary element and centre it in the
    /// causal view; a race candidate opens both orderings in the trace view
    /// when a scenario exists.
    pub(crate) fn open_finding(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(loaded) = self.doc.loaded().cloned() else {
            return;
        };
        let Some(finding) = loaded.findings.get(index).cloned() else {
            return;
        };
        let key = loaded.model.key_of(finding.detail.primary());
        commands::select(&mut self.view, key);
        if finding.check() == Check::RaceCandidate
            && let Some(race) = locate::race_index(&loaded.findings, &finding)
        {
            let scenario = self.view.scenario.clone().or_else(|| self.scenarios.first().map(|s| s.id.clone()));
            match scenario {
                Some(scenario) => {
                    self.view.scenario = Some(scenario);
                    self.view.race = Some(race);
                    self.switch_view(ViewKind::Trace);
                    self.changed(cx);
                    return;
                }
                None => self
                    .set_status("Replaying a race needs a scenario file (scenarios/*.yaml or *.scenario.yaml)", false),
            }
        }
        self.switch_view(ViewKind::Causal);
        self.changed(cx);
        self.ensure_scene();
        match locate::locate_finding(&self.scene, &loaded.model, &finding) {
            Some(rect) => self.center_rect(rect),
            None => self.set_status("That finding has nothing drawn in this view", false),
        }
        cx.notify();
    }

    // --- Trace view ----------------------------------------------------------------

    pub(crate) fn pick_scenario(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.view.scenario = id;
        self.view.race = None;
        self.changed(cx);
    }

    pub(crate) fn clear_race(&mut self, cx: &mut Context<Self>) {
        self.view.race = None;
        self.changed(cx);
    }

    // --- Pins ------------------------------------------------------------------

    pub(crate) fn pin(&mut self, key: ElementKey, top_left: cascade_layout::Point, cx: &mut Context<Self>) {
        let view = self.view.view;
        if !pins_supported(view) {
            return;
        }
        self.sidecar.pin(view, key.clone(), top_left);
        self.save_pins();
        tracing::info!(%key, x = top_left.x, y = top_left.y, "pinned");
        self.changed(cx);
    }

    pub(crate) fn unpin(&mut self, key: &ElementKey, cx: &mut Context<Self>) {
        let view = self.view.view;
        let pinned = self.sidecar.pins_for(view).any(|(k, _)| k == key);
        if !pinned {
            self.set_status(format!("{key} is not pinned"), false);
            cx.notify();
            return;
        }
        self.sidecar.unpin(view, key);
        self.save_pins();
        self.set_status(format!("Unpinned {key}"), false);
        self.changed(cx);
    }

    fn save_pins(&mut self) {
        if let Err(error) = save_sidecar(&self.sidecar_path, &self.sidecar) {
            tracing::warn!(%error, "cannot save pins");
            self.set_status(format!("Cannot save pins: {error}"), true);
        }
    }

    // --- Click to source ---------------------------------------------------------

    pub(crate) fn open_source(&mut self, key: &ElementKey, cx: &mut Context<Self>) {
        let Some(loaded) = self.doc.loaded() else {
            return;
        };
        let Some(element) = loaded.model.resolve_key(key) else {
            self.set_status(format!("{key} is not in the working tree definition"), false);
            cx.notify();
            return;
        };
        let span = loaded.model.span_of(element);
        let file = self.doc.path().to_owned();
        let command = match editor::command_for(editor::Location::from_span(&file, span)) {
            Ok(command) => command,
            Err(error) => {
                self.set_status(error.to_string(), true);
                cx.notify();
                return;
            }
        };
        match editor::spawn(&command) {
            Ok(mut child) => {
                tracing::info!(command = %command.display(), "opened source");
                self.set_status(format!("Opened {}", command.display()), false);
                // Reap the editor launcher when it exits, without blocking.
                cx.spawn(async move |_, cx| {
                    while let Ok(None) = child.try_wait() {
                        cx.background_executor().timer(REAP_INTERVAL).await;
                    }
                })
                .detach();
            }
            Err(error) => {
                tracing::warn!(%error, command = %command.display(), "cannot start editor");
                self.set_status(format!("Cannot run `{}`: {error}", command.program), true);
            }
        }
        cx.notify();
    }

    // --- Links -----------------------------------------------------------------

    fn copy_link(&mut self, cx: &mut Context<Self>) {
        let link = link::link_for(&self.view, self.effective_viewport());
        cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
        tracing::info!(%link, "copied view link");
        self.set_status(format!("Copied {link}"), false);
        cx.notify();
    }

    fn paste_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
        match link::parse_pasted(&text) {
            Ok(state) => {
                self.parked_viewports.clear();
                self.cone_depth = state.cone.map_or(self.cone_depth, |c| c.depth);
                let search = state.search.clone().unwrap_or_default();
                let (base, head) = state
                    .diff
                    .as_ref()
                    .map(|d| (d.base.clone(), d.head.clone().unwrap_or_default()))
                    .unwrap_or_default();
                self.view = state;
                self.search.input.update(cx, |i, cx| i.set_text(search, cx));
                self.search.open = false;
                self.diff_base.update(cx, |i, cx| i.set_text(base, cx));
                self.diff_head.update(cx, |i, cx| i.set_text(head, cx));
                let unknown = self.doc.loaded().map_or(0, |l| link::unknown_keys(&self.view, &l.model).len());
                if unknown > 0 {
                    self.set_status(format!("Opened link; {unknown} element(s) are not in this definition"), false);
                } else {
                    self.set_status("Opened view link", false);
                }
                window.focus(&self.focus, cx);
                self.changed(cx);
            }
            Err(error) => {
                self.set_status(format!("Clipboard has no view link: {error}"), true);
                cx.notify();
            }
        }
    }

    // --- Search ------------------------------------------------------------------

    pub(crate) fn on_search_event(&mut self, event: InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputEvent::Changed => {
                let query = self.search.input.read(cx).text().to_owned();
                self.search.hits = match self.doc.loaded() {
                    Some(loaded) => cascade_core::search::search(&loaded.model, &query, SEARCH_LIMIT),
                    None => Vec::new(),
                };
                self.search.cursor = 0;
                // Only while typing: a pasted link also sets the text.
                let typing = self.search.input.focus_handle(cx).is_focused(window);
                self.search.open = typing && !self.search.hits.is_empty();
                let query = (!query.trim().is_empty()).then_some(query);
                if query != self.view.search {
                    self.view.search = query;
                    self.changed(cx);
                } else {
                    cx.notify();
                }
            }
            InputEvent::Submit => self.focus_search_hit(self.search.cursor, window, cx),
            InputEvent::Cancel => {
                self.search.open = false;
                window.focus(&self.focus, cx);
                cx.notify();
            }
            InputEvent::Down => {
                if !self.search.hits.is_empty() {
                    self.search.open = true;
                    self.search.cursor = (self.search.cursor + 1) % self.search.hits.len();
                    cx.notify();
                }
            }
            InputEvent::Up => {
                if !self.search.hits.is_empty() {
                    self.search.open = true;
                    let n = self.search.hits.len();
                    self.search.cursor = (self.search.cursor + n - 1) % n;
                    cx.notify();
                }
            }
        }
    }

    /// Select and centre search result `index`, then hand the keyboard back
    /// to the canvas so F/B work right away.
    pub(crate) fn focus_search_hit(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(loaded) = self.doc.loaded() else {
            return;
        };
        let Some(hit) = self.search.hits.get(index) else {
            self.set_status("No match", false);
            cx.notify();
            return;
        };
        let key = loaded.model.key_of(hit.element);
        let label = hit.label.clone();
        self.search.open = false;
        commands::select(&mut self.view, key.clone());
        self.changed(cx);
        if !self.center_key(&key) && self.view.view != ViewKind::Causal {
            self.switch_view(ViewKind::Causal);
            self.changed(cx);
            self.center_key(&key);
        }
        self.set_status(format!("Selected {label}"), false);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    // --- Diff --------------------------------------------------------------------

    pub(crate) fn on_diff_field_event(&mut self, event: InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputEvent::Submit => self.start_diff(window, cx),
            InputEvent::Cancel => window.focus(&self.focus, cx),
            InputEvent::Changed | InputEvent::Up | InputEvent::Down => {}
        }
    }

    pub(crate) fn start_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = self.diff_base.read(cx).text().to_owned();
        let head = self.diff_head.read(cx).text().to_owned();
        match refs_from_fields(&base, &head) {
            Some(refs) => {
                let against = refs.head.clone().unwrap_or_else(|| "the working tree".to_owned());
                self.set_status(format!("Comparing {} with {against}", refs.base), false);
                self.view.diff = Some(refs);
                window.focus(&self.focus, cx);
                self.changed(cx);
            }
            None => {
                self.set_status("Enter a base git ref to compare against", false);
                let handle = self.diff_base.focus_handle(cx);
                window.focus(&handle, cx);
                cx.notify();
            }
        }
    }

    pub(crate) fn exit_diff(&mut self, cx: &mut Context<Self>) {
        self.view.diff = None;
        self.set_status("Diff mode off", false);
        self.changed(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_only_in_laid_out_views() {
        assert!(pins_supported(ViewKind::Causal));
        assert!(pins_supported(ViewKind::Structure));
        assert!(!pins_supported(ViewKind::Trace));
        assert!(!pins_supported(ViewKind::Matrix));
    }
}
