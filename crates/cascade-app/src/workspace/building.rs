//! Build mode glue: committing edit ops to the file, undo/redo, the
//! toolbar actions, drag-to-connect drops, the inspector's text fields, and
//! creating a new file.
//!
//! Every edit goes through [`Workspace::commit`]:
//! `pipeline::commit` (apply → patch → load) → atomic write → note the
//! written text as the app's own → show the new model → record the inverse
//! → select what the edit touched. Nothing is written unless every stage
//! succeeds.

use std::collections::HashMap;
use std::path::PathBuf;

use cascade_core::ElementKey;
use cascade_core::edit::EditOp;
use chrono::Local;
use gpui::{AppContext as _, Context, Entity, Subscription, Window};

use super::Workspace;
use crate::build::connect;
use crate::build::disk::{self, create_new};
use crate::build::inspector::{self, FieldId, FieldInput, FieldValue, Inspection, InspectorAction};
use crate::build::ops::{self, Planned, StateTarget};
use crate::build::pipeline::{self, Commit};
use crate::build::undo::{Direction, History};
use crate::document::Loaded;
use crate::input::{InputEvent, TextInput};
use crate::mode::AppMode;

/// Width of an inspector text field.
const FIELD_WIDTH: f32 = 196.0;

/// Build mode's state.
#[derive(Default)]
pub struct BuildUi {
    pub history: History,
    /// The last edit's error, shown in the build bar.
    pub error: Option<String>,
    /// "Comments were not preserved" was shown this session.
    pub warned_rewrite: bool,
    pub inspector: Option<InspectorUi>,
}

/// The inspector for one element of one model generation.
pub struct InspectorUi {
    pub key: ElementKey,
    pub generation: u64,
    pub inspection: Result<Inspection, String>,
    /// Text fields, in `inspection.fields` order.
    pub inputs: Vec<(FieldId, Entity<TextInput>)>,
    /// Inline errors per field (live validation or a rejected commit).
    pub errors: HashMap<FieldId, String>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    /// The loaded model, when it is safe to edit: the latest load succeeded.
    pub(crate) fn editable(&self) -> Result<Loaded, String> {
        match self.doc.state() {
            crate::document::DocState::Ready { loaded } => Ok(loaded.clone()),
            crate::document::DocState::Stale { .. } | crate::document::DocState::Failed { .. } => {
                Err("The file has errors, so Build mode is read-only until it loads again".to_owned())
            }
            crate::document::DocState::Empty => Err("Nothing is loaded yet".to_owned()),
        }
    }

    fn edit_failed(&mut self, message: String, cx: &mut Context<Self>) {
        tracing::warn!(error = %message, "edit rejected");
        self.set_status(message.clone(), true);
        self.build.error = Some(message);
        cx.notify();
    }

    /// Run `op` through the pipeline and write it. Returns the commit's
    /// inverse and touched keys.
    fn apply_and_write(&mut self, op: &EditOp) -> Result<(EditOp, Vec<ElementKey>, bool), String> {
        let loaded = self.editable()?;
        let Commit { applied, text, rewritten, analyzed } =
            pipeline::commit(&loaded.text, loaded.model.definition(), op).map_err(|e| e.to_string())?;
        let path = self.doc.path().to_owned();
        disk::write_atomic(&path, &text).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
        self.disk.note(&text);
        self.doc.apply(Ok(analyzed), Local::now());
        self.on_model_replaced();
        Ok((applied.inverse, applied.touched, rewritten))
    }

    /// After an edit: warn once about lost comments, select what changed.
    fn after_write(&mut self, touched: &[ElementKey], rewritten: bool) {
        self.build.error = None;
        if rewritten && !self.build.warned_rewrite {
            self.build.warned_rewrite = true;
            self.set_status(
                "The file was rewritten to save this edit: its comments and custom formatting were not preserved",
                true,
            );
        }
        let Some(loaded) = self.doc.loaded() else {
            return;
        };
        let present = |k: &ElementKey| loaded.model.resolve_key(k).is_some();
        match touched.iter().find(|k| present(k)) {
            Some(key) => self.view.selection = vec![key.clone()],
            None => self.view.selection.retain(|k| present(k)),
        }
        self.view.cone = None;
    }

    /// Commit a planned edit and record it for undo.
    pub(crate) fn commit(&mut self, planned: Planned, cx: &mut Context<Self>) -> bool {
        match self.apply_and_write(&planned.op) {
            Ok((inverse, touched, rewritten)) => {
                tracing::info!(edit = %planned.label, "edit saved");
                self.build.history.record(inverse, planned.label.clone());
                self.set_status(planned.label, false);
                self.after_write(&touched, rewritten);
                self.changed(cx);
                true
            }
            Err(message) => {
                self.edit_failed(message, cx);
                false
            }
        }
    }

    pub(crate) fn undo_redo(&mut self, direction: Direction, cx: &mut Context<Self>) {
        let Some(entry) = self.build.history.peek(direction).cloned() else {
            let what = match direction {
                Direction::Undo => "Nothing to undo",
                Direction::Redo => "Nothing to redo",
            };
            self.set_status(what, false);
            cx.notify();
            return;
        };
        match self.apply_and_write(&entry.op) {
            Ok((inverse, touched, rewritten)) => {
                self.build.history.complete(direction, inverse);
                self.set_status(format!("{} {}", direction.verb(), entry.label), false);
                self.after_write(&touched, rewritten);
                self.changed(cx);
            }
            Err(message) => self.edit_failed(format!("{} failed: {message}", direction.verb()), cx),
        }
    }

    /// Plan with the editable definition, or report why not.
    fn plan(
        &mut self,
        cx: &mut Context<Self>,
        plan: impl FnOnce(&cascade_core::Definition, Option<&ElementKey>) -> Result<Planned, ops::PlanError>,
    ) {
        let loaded = match self.editable() {
            Ok(loaded) => loaded,
            Err(message) => return self.edit_failed(message, cx),
        };
        let selection = self.view.selection.first().cloned();
        match plan(loaded.model.definition(), selection.as_ref()) {
            Ok(planned) => {
                self.commit(planned, cx);
            }
            Err(error) => self.edit_failed(capitalize(&error.to_string()), cx),
        }
    }

    pub(crate) fn add_machine(&mut self, cx: &mut Context<Self>) {
        self.plan(cx, |d, _| Ok(ops::add_machine(d)));
    }

    pub(crate) fn add_state(&mut self, cx: &mut Context<Self>) {
        self.plan(cx, |d, selection| ops::add_state(d, &ops::state_target(d, selection)?));
    }

    pub(crate) fn add_child_state(&mut self, cx: &mut Context<Self>) {
        self.plan(cx, |d, selection| match selection {
            Some(ElementKey::State { machine, path }) => {
                ops::add_state(d, &StateTarget { machine: machine.clone(), parent: Some(path.clone()) })
            }
            _ => Err(ops::PlanError::NeedsSelection("a state")),
        });
    }

    pub(crate) fn add_controller(&mut self, cx: &mut Context<Self>) {
        self.plan(cx, |d, _| Ok(ops::add_controller(d)));
    }

    pub(crate) fn add_external(&mut self, cx: &mut Context<Self>) {
        self.plan(cx, |d, _| Ok(ops::add_external(d)));
    }

    pub(crate) fn delete_selection(&mut self, cx: &mut Context<Self>) {
        if self.mode != AppMode::Build {
            self.set_status("Switch to Build mode (Ctrl-2) to delete elements", false);
            cx.notify();
            return;
        }
        self.plan(cx, |d, selection| match selection {
            Some(key) => ops::delete(d, key),
            None => Err(ops::PlanError::NeedsSelection("an element to delete")),
        });
    }

    pub(crate) fn delete_key(&mut self, key: ElementKey, cx: &mut Context<Self>) {
        self.plan(cx, move |d, _| ops::delete(d, &key));
    }

    /// Connect the first selected element to the second, as if dragged.
    pub(crate) fn connect_selection(&mut self, cx: &mut Context<Self>) {
        let pair = match self.view.selection.as_slice() {
            [from, to] => Some((from.clone(), to.clone())),
            _ => None,
        };
        match pair {
            Some((from, to)) => self.connect_keys(&from, &to, cx),
            None => {
                self.set_status("Shift-click two elements (from, then to) to connect them", false);
                cx.notify();
            }
        }
    }

    pub(crate) fn connect_keys(&mut self, from: &ElementKey, to: &ElementKey, cx: &mut Context<Self>) {
        let (from, to) = (from.clone(), to.clone());
        self.plan(cx, move |d, _| connect::connect(d, &from, &to));
    }

    // --- New file --------------------------------------------------------------

    /// Ask where to create a new definition, create it and open it.
    pub(crate) fn new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.doc.path().parent().map_or_else(|| PathBuf::from("."), ToOwned::to_owned);
        let answer = cx.prompt_for_new_path(&dir, Some("cascade.yaml"));
        cx.spawn_in(window, async move |this, cx| {
            let chosen = match answer.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) => return,
                Ok(Err(error)) => {
                    tracing::warn!(%error, "file dialog failed");
                    let message = format!("Cannot ask for a file name: {error}");
                    let _ = this.update(cx, |ws, cx| ws.edit_failed(message, cx));
                    return;
                }
                Err(_) => return,
            };
            if let Err(error) = this.update_in(cx, |ws, window, cx| ws.create_and_open(chosen, window, cx)) {
                tracing::debug!(%error, "workspace gone before the new file was created");
            }
        })
        .detach();
    }

    fn create_and_open(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = create_new(&path) {
            return self.edit_failed(error.to_string(), cx);
        }
        match std::fs::canonicalize(&path) {
            Ok(path) => {
                self.open_path(path, window, cx);
                self.set_mode(AppMode::Build, cx);
            }
            Err(error) => self.edit_failed(format!("Cannot open {}: {error}", path.display()), cx),
        }
    }

    // --- Inspector ---------------------------------------------------------------

    /// Rebuild the inspector when the selection or the model changed.
    pub(crate) fn sync_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.view.selection.first().cloned();
        let generation = self.doc.generation();
        let current = self.build.inspector.as_ref().map(|i| (&i.key, i.generation));
        if current == key.as_ref().map(|k| (k, generation)) {
            return;
        }
        let (Some(key), Some(loaded)) = (key, self.doc.loaded().cloned()) else {
            self.build.inspector = None;
            return;
        };
        let inspection = inspector::inspect(loaded.model.definition(), &key).map_err(|e| capitalize(&e.to_string()));
        let mut inputs = Vec::new();
        let mut subscriptions = Vec::new();
        if let Ok(inspection) = &inspection {
            for field in &inspection.fields {
                let FieldInput::Text(value) = &field.input else {
                    continue;
                };
                let id = field.id;
                let value = value.clone();
                let input = cx.new(|cx| {
                    let mut input = TextInput::new(id.hint(), FIELD_WIDTH, cx);
                    input.set_text(value, cx);
                    input
                });
                subscriptions.push(cx.subscribe_in(&input, window, move |ws, _, event: &InputEvent, window, cx| {
                    ws.on_field_event(id, *event, window, cx);
                }));
                inputs.push((id, input));
            }
        }
        self.build.inspector = Some(InspectorUi {
            key,
            generation,
            inspection,
            inputs,
            errors: HashMap::new(),
            _subscriptions: subscriptions,
        });
    }

    fn field_input(&self, id: FieldId) -> Option<Entity<TextInput>> {
        self.build.inspector.as_ref()?.inputs.iter().find(|(f, _)| *f == id).map(|(_, input)| input.clone())
    }

    fn on_field_event(&mut self, id: FieldId, event: InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.field_input(id) else {
            return;
        };
        match event {
            InputEvent::Changed => {
                let text = input.read(cx).text().to_owned();
                if let Some(ui) = &mut self.build.inspector {
                    match inspector::validate(id, &text) {
                        Ok(()) => ui.errors.remove(&id),
                        Err(message) => ui.errors.insert(id, message),
                    };
                }
                cx.notify();
            }
            InputEvent::Submit => {
                let text = input.read(cx).text().to_owned();
                self.commit_field(id, FieldValue::Text(text), cx);
                if id == FieldId::ControllerAddHandler
                    && let Some(input) = self.field_input(id)
                {
                    input.update(cx, |i, cx| i.set_text("", cx));
                }
            }
            InputEvent::Cancel => window.focus(&self.focus, cx),
            InputEvent::Up | InputEvent::Down => {}
        }
    }

    /// Commit one inspector field.
    pub(crate) fn commit_field(&mut self, id: FieldId, value: FieldValue, cx: &mut Context<Self>) {
        let Some(key) = self.build.inspector.as_ref().map(|i| i.key.clone()) else {
            return;
        };
        let loaded = match self.editable() {
            Ok(loaded) => loaded,
            Err(message) => return self.edit_failed(message, cx),
        };
        let result = match inspector::field_op(loaded.model.definition(), &key, id, &value) {
            Ok(None) => {
                self.set_status(format!("{}: no change", id.label()), false);
                Ok(())
            }
            Ok(Some(planned)) => {
                if self.commit(planned, cx) {
                    Ok(())
                } else {
                    Err(self.build.error.clone().unwrap_or_default())
                }
            }
            Err(error) => Err(capitalize(&error.to_string())),
        };
        if let Some(ui) = &mut self.build.inspector {
            match result {
                Ok(()) => ui.errors.remove(&id),
                Err(message) => ui.errors.insert(id, message),
            };
        }
        cx.notify();
    }

    pub(crate) fn inspector_action(&mut self, action: InspectorAction, cx: &mut Context<Self>) {
        let Some(key) = self.build.inspector.as_ref().map(|i| i.key.clone()) else {
            return;
        };
        match action {
            InspectorAction::AddChildState => {
                self.view.selection = vec![key];
                self.add_child_state(cx);
            }
            InspectorAction::Delete => self.delete_key(key, cx),
        }
    }
}

/// Error messages start lowercase in the libraries; the status bar wants a
/// sentence.
pub fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capitalize_makes_sentences() {
        assert_eq!(capitalize("select a state first"), "Select a state first");
        assert_eq!(capitalize(""), "");
        assert_eq!(capitalize("État"), "État");
    }
}
