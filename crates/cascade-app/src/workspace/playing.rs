//! Play mode glue: the session's inputs, actions, timeline jumps, loading
//! and saving scenarios, and keeping the session in step with the model.

use std::collections::BTreeMap;

use cascade_core::definition::TriggerRef;
use cascade_sim::{PlayAction, load_scenario_file};
use gpui::{AppContext as _, Context, Entity, Subscription};

use super::Workspace;
use super::building::capitalize;
use crate::document::scenarios;
use crate::input::{InputEvent, TextInput};
use crate::play::PlayState;
use crate::play::choices::{self, ChoicePrompt, PromptStep, SessionOracle, StepRequest};
use crate::play::clock::advance_action;
use crate::play::forms::{parse_assignments, scenario_path, scenario_text, write_scenario};

/// Play mode's state and form fields.
pub struct PlayUi {
    /// Created on first entering Play mode.
    pub state: Option<PlayState>,
    /// The machine picked in the "add instance" form.
    pub machine: Option<String>,
    pub name: Entity<TextInput>,
    pub fields: Entity<TextInput>,
    pub start: Entity<TextInput>,
    pub payload: Entity<TextInput>,
    pub scenario_name: Entity<TextInput>,
    /// The clock's "advance by" box (empty: to the next timer).
    pub advance_by: Entity<TextInput>,
    /// A step waiting for the user's decisions.
    pub prompt: Option<ChoicePrompt>,
    /// A form input that did not parse.
    pub form_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl PlayUi {
    pub fn new(cx: &mut Context<Workspace>) -> Self {
        let advance_by = cx.new(|cx| TextInput::new("by: 30s (blank: next timer)", 170.0, cx));
        // Enter advances; typing updates the button's label.
        let advance_events = cx.subscribe(&advance_by, |ws: &mut Workspace, _, event: &InputEvent, cx| match event {
            InputEvent::Submit => ws.play_advance(false, cx),
            InputEvent::Changed => cx.notify(),
            InputEvent::Cancel | InputEvent::Up | InputEvent::Down => {}
        });
        Self {
            state: None,
            machine: None,
            name: cx.new(|cx| TextInput::new("name (optional)", 130.0, cx)),
            fields: cx.new(|cx| TextInput::new("fields: orderId=1", 150.0, cx)),
            start: cx.new(|cx| TextInput::new("start state (optional)", 150.0, cx)),
            payload: cx.new(|cx| TextInput::new("payload: amount=42", 200.0, cx)),
            scenario_name: cx.new(|cx| TextInput::new("scenario name", 170.0, cx)),
            advance_by,
            prompt: None,
            form_error: None,
            _subscriptions: vec![advance_events],
        }
    }
}

impl Workspace {
    /// Make sure a session exists for the model on screen.
    pub(crate) fn ensure_play(&mut self) {
        let Some(loaded) = self.doc.loaded() else {
            return;
        };
        match &mut self.play.state {
            Some(state) => {
                if let Some(note) = state.rebase(&loaded.model, loaded.generation) {
                    self.set_status(note, true);
                }
            }
            None => self.play.state = Some(PlayState::new(&loaded.model, loaded.generation)),
        }
    }

    /// The model was replaced (reload or edit): replay the session on it.
    pub(crate) fn on_model_replaced(&mut self) {
        self.play.prompt = None;
        if self.play.state.is_some() {
            self.ensure_play();
        }
    }

    /// Perform a play action. Any open choice prompt is dropped: it was
    /// asked about the session as it was.
    pub(crate) fn play(&mut self, action: PlayAction, cx: &mut Context<Self>) {
        self.play.prompt = None;
        self.perform(vec![action]);
        self.changed(cx);
    }

    /// Perform `actions` in order, stopping at the first failure.
    fn perform(&mut self, actions: Vec<PlayAction>) -> bool {
        self.ensure_play();
        let (Some(loaded), Some(state)) = (self.doc.loaded().cloned(), self.play.state.as_mut()) else {
            self.set_status("Nothing to play: no model is loaded", true);
            return false;
        };
        for action in actions {
            if let Err(error) = state.act(&loaded.model, action) {
                tracing::warn!(%error, "play action failed");
                self.set_status(capitalize(&error.to_string()), true);
                return false;
            }
        }
        true
    }

    fn in_play(&mut self, what: &str, cx: &mut Context<Self>) -> bool {
        if self.mode == crate::mode::AppMode::Play {
            return true;
        }
        self.set_status(format!("Switch to Play mode (Ctrl-3) to {what}"), false);
        cx.notify();
        false
    }

    /// Space, or clicking a queue item: deliver it, asking first for the
    /// decisions it needs. With a prompt open, Space takes its first
    /// option (the default).
    pub(crate) fn play_step(&mut self, choice: Option<u32>, cx: &mut Context<Self>) {
        if !self.in_play("step the queue", cx) {
            return;
        }
        if choice.is_none()
            && let Some(first) = self.play.prompt.as_ref().and_then(|p| p.point.options.first().cloned())
        {
            return self.pick_choice(first, cx);
        }
        self.request_step(StepRequest { choice, consumer: None }, cx);
    }

    /// Deliver a queue item (to one consumer or all), through the choice
    /// prompt.
    pub(crate) fn request_step(&mut self, request: StepRequest, cx: &mut Context<Self>) {
        self.play.prompt = None;
        self.ensure_play();
        let (Some(loaded), Some(state)) = (self.doc.loaded().cloned(), self.play.state.as_ref()) else {
            return;
        };
        let oracle = SessionOracle { session: state.session(), model: &loaded.model };
        let next = choices::begin(&oracle, request);
        self.after_prompt_step(next, cx);
    }

    /// Answer the open prompt.
    pub(crate) fn pick_choice(&mut self, option: String, cx: &mut Context<Self>) {
        let Some(prompt) = self.play.prompt.take() else {
            return;
        };
        let (Some(loaded), Some(state)) = (self.doc.loaded().cloned(), self.play.state.as_ref()) else {
            return;
        };
        let oracle = SessionOracle { session: state.session(), model: &loaded.model };
        let next = choices::pick(&oracle, prompt, &option);
        self.after_prompt_step(next, cx);
    }

    pub(crate) fn cancel_choice(&mut self, cx: &mut Context<Self>) {
        if self.play.prompt.take().is_some() {
            self.set_status("Step cancelled: nothing was delivered", false);
        }
        cx.notify();
    }

    fn after_prompt_step(&mut self, next: Result<PromptStep, String>, cx: &mut Context<Self>) {
        match next {
            Ok(PromptStep::Ask(prompt)) => {
                self.set_status(format!("{}: pick an option in the queue panel", prompt.title()), false);
                self.play.prompt = Some(prompt);
                cx.notify();
            }
            Ok(PromptStep::Go(actions)) => {
                self.perform(actions);
                self.changed(cx);
            }
            Err(message) => {
                tracing::warn!(error = %message, "cannot prepare the step");
                self.set_status(capitalize(&message), true);
                cx.notify();
            }
        }
    }

    pub(crate) fn play_run(&mut self, cx: &mut Context<Self>) {
        if !self.in_play("run the queue", cx) {
            return;
        }
        self.play(PlayAction::RunUntilQuiet, cx);
    }

    /// Advance the clock by the typed duration, or to the next timer when
    /// the box is empty (`T` always goes to the next timer).
    pub(crate) fn play_advance(&mut self, next_timer: bool, cx: &mut Context<Self>) {
        if !self.in_play("advance the clock", cx) {
            return;
        }
        let text = if next_timer { String::new() } else { self.play.advance_by.read(cx).text().to_owned() };
        match advance_action(&text) {
            Ok(action) => {
                self.play.form_error = None;
                self.play(action, cx);
            }
            Err(message) => {
                self.play.form_error = Some(message);
                cx.notify();
            }
        }
    }

    pub(crate) fn pick_play_machine(&mut self, machine: String, cx: &mut Context<Self>) {
        self.play.machine = Some(machine);
        cx.notify();
    }

    /// The "add instance" form.
    pub(crate) fn add_instance(&mut self, cx: &mut Context<Self>) {
        let Some(machine) = self.play.machine.clone() else {
            self.play.form_error = Some("Pick a machine first".to_owned());
            cx.notify();
            return;
        };
        let name = self.play.name.read(cx).text().trim().to_owned();
        let start = self.play.start.read(cx).text().trim().to_owned();
        let fields = match parse_assignments(self.play.fields.read(cx).text()) {
            Ok(fields) => fields,
            Err(message) => {
                self.play.form_error = Some(format!("Fields: {message}"));
                cx.notify();
                return;
            }
        };
        self.play.form_error = None;
        let action = PlayAction::AddInstance {
            name: (!name.is_empty()).then_some(name),
            machine,
            fields,
            state: (!start.is_empty()).then_some(start),
        };
        self.play(action, cx);
        if self.play.state.as_ref().is_some_and(|s| s.error.is_none()) {
            self.play.name.update(cx, |i, cx| i.set_text("", cx));
        }
    }

    pub(crate) fn remove_instance(&mut self, name: String, cx: &mut Context<Self>) {
        self.play(PlayAction::RemoveInstance { name }, cx);
    }

    /// A palette button: fire `trigger` from `source` at `target`, with the
    /// payload field's assignments.
    pub(crate) fn fire(&mut self, source: String, trigger: TriggerRef, target: String, cx: &mut Context<Self>) {
        let payload: BTreeMap<String, String> = match parse_assignments(self.play.payload.read(cx).text()) {
            Ok(payload) => payload,
            Err(message) => {
                self.play.form_error = Some(format!("Payload: {message}"));
                cx.notify();
                return;
            }
        };
        self.play.form_error = None;
        self.play(PlayAction::Fire { source, trigger, target, payload }, cx);
    }

    pub(crate) fn seek(&mut self, position: usize, cx: &mut Context<Self>) {
        self.play.prompt = None;
        let (Some(loaded), Some(state)) = (self.doc.loaded().cloned(), self.play.state.as_mut()) else {
            return;
        };
        if let Err(error) = state.seek(&loaded.model, position) {
            self.set_status(capitalize(&error.to_string()), true);
        }
        self.changed(cx);
    }

    pub(crate) fn switch_branch(&mut self, index: usize, cx: &mut Context<Self>) {
        self.play.prompt = None;
        let (Some(loaded), Some(state)) = (self.doc.loaded().cloned(), self.play.state.as_mut()) else {
            return;
        };
        if let Err(error) = state.switch_branch(&loaded.model, index) {
            self.set_status(capitalize(&error.to_string()), true);
        }
        self.changed(cx);
    }

    /// Throw the session away and start empty.
    pub(crate) fn restart_play(&mut self, cx: &mut Context<Self>) {
        self.play.prompt = None;
        self.play.state = None;
        self.ensure_play();
        self.set_status("New play session", false);
        self.changed(cx);
    }

    /// Start play from a scenario file.
    pub(crate) fn load_play_scenario(&mut self, id: String, cx: &mut Context<Self>) {
        self.play.prompt = None;
        let Some(loaded) = self.doc.loaded().cloned() else {
            return;
        };
        let Some(file) = self.scenarios.iter().find(|s| s.id == id).cloned() else {
            self.set_status(format!("No scenario file named `{id}`"), true);
            cx.notify();
            return;
        };
        let started = load_scenario_file(&file.path).map_err(|e| e.to_string()).and_then(|scenario| {
            PlayState::from_scenario(&loaded.model, loaded.generation, &scenario).map_err(|e| e.to_string())
        });
        match started {
            Ok(state) => {
                self.play.state = Some(state);
                self.set_status(format!("Playing from scenario {id}"), false);
            }
            Err(message) => {
                tracing::warn!(error = %message, scenario = %id, "cannot start play from scenario");
                self.set_status(format!("Cannot play {id}: {message}"), true);
            }
        }
        self.changed(cx);
    }

    /// Save the session so far as `scenarios/<name>.yaml`.
    pub(crate) fn save_scenario(&mut self, cx: &mut Context<Self>) {
        let name = self.play.scenario_name.read(cx).text().trim().to_owned();
        let Some(state) = &self.play.state else {
            return;
        };
        let saved = scenario_path(self.doc.path(), &name).and_then(|path| {
            let text = scenario_text(state.session(), &name)?;
            write_scenario(&path, &text)?;
            Ok(path)
        });
        match saved {
            Ok(path) => {
                tracing::info!(path = %path.display(), "scenario saved");
                self.set_status(format!("Saved {}", path.display()), false);
                if let Some(watcher) = &mut self.watcher {
                    watcher.refresh_scenario_dir();
                }
                self.scenarios = scenarios::discover(self.doc.path());
                self.invalidate_traces();
            }
            Err(error) => {
                tracing::warn!(%error, "cannot save scenario");
                self.set_status(capitalize(&error.to_string()), true);
            }
        }
        self.changed(cx);
    }
}
