//! The root view: owns the document, the view state and everything derived
//! from them, and lays out the toolbar, sidebar, canvas and status bar.
//!
//! State lives here; the scene is derived. Any change goes through
//! [`Workspace::changed`], which starts background work the new state needs
//! (simulation, diff) and marks the scene dirty; `render` rebuilds it once
//! per frame at most.

pub mod actions;
pub mod building;
mod canvas_events;
mod operations;
pub mod playing;
mod scene;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cascade_core::search::SearchHit;
use cascade_scene::{HitTarget, LayoutSidecar, Scene, SceneBuilder, Theme, ThemeMode, ViewKind, ViewState, Viewport};
use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, SharedString, Subscription, Task, Window,
    WindowAppearance, div, prelude::*, px,
};

use crate::build::disk::DiskSync;
use crate::diffmode::DiffRun;
use crate::document::Document;
use crate::document::scenarios::ScenarioFile;
use crate::gesture::Gesture;
use crate::input::{InputEvent, TextInput};
use crate::mode::AppMode;
use crate::theme::{ActiveChrome, Chrome, ThemeChoice, chrome, scene_theme};
use crate::trace::TraceRun;
use crate::viewport::ScreenRect;
use crate::watch::FileWatcher;

use self::building::BuildUi;
use self::playing::PlayUi;

/// `Cascade — cascade.yaml`.
pub fn window_title(path: &Path) -> String {
    format!(
        "Cascade — {}",
        path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
    )
}

/// A transient message in the status bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub text: String,
    pub is_error: bool,
    /// Distinguishes messages, so an expiry timer clears only its own.
    pub serial: u64,
}

/// How long status messages stay up.
const STATUS_TTL: std::time::Duration = std::time::Duration::from_secs(6);
const ERROR_STATUS_TTL: std::time::Duration = std::time::Duration::from_secs(15);

/// Pointer state of the canvas.
#[derive(Debug, Default)]
pub struct CanvasState {
    /// Where the canvas was painted last frame (window coordinates).
    pub bounds: Option<ScreenRect>,
    pub gesture: Gesture,
    pub hover: Option<HitTarget>,
}

/// The search box and its results.
pub struct SearchState {
    pub input: Entity<TextInput>,
    pub hits: Vec<SearchHit>,
    /// Highlighted result.
    pub cursor: usize,
    pub open: bool,
}

pub struct Workspace {
    pub(crate) focus: FocusHandle,
    pub(crate) doc: Document,
    pub(crate) sidecar_path: PathBuf,
    pub(crate) sidecar: LayoutSidecar,
    pub(crate) sidecar_error: Option<String>,
    pub(crate) scenarios: Vec<ScenarioFile>,
    pub(crate) view: ViewState,
    /// Viewports of the views not on screen, restored when switching back.
    pub(crate) parked_viewports: HashMap<ViewKind, Option<Viewport>>,
    /// Cone depth setting; survives turning the cone off.
    pub(crate) cone_depth: Option<u32>,
    pub(crate) theme_choice: ThemeChoice,
    pub(crate) theme: Theme,
    pub(crate) builder: SceneBuilder,
    pub(crate) scene: Rc<Scene>,
    pub(crate) scene_error: Option<String>,
    pub(crate) scene_dirty: bool,
    pub(crate) trace_run: TraceRun,
    pub(crate) diff_run: DiffRun,
    pub(crate) canvas: CanvasState,
    pub(crate) search: SearchState,
    pub(crate) diff_base: Entity<TextInput>,
    pub(crate) diff_head: Entity<TextInput>,
    pub(crate) status: Option<Status>,
    pub(crate) status_serial: u64,
    status_timer_for: Option<u64>,
    status_task: Option<Task<()>>,
    pub(crate) mono: SharedString,
    /// View, Build or Play.
    pub(crate) mode: AppMode,
    /// What the app last read from or wrote to the definition file.
    pub(crate) disk: DiskSync,
    pub(crate) build: BuildUi,
    pub(crate) play: PlayUi,
    pub(crate) watcher: Option<FileWatcher>,
    reload_task: Option<Task<()>>,
    trace_task: Option<Task<()>>,
    diff_task: Option<Task<()>>,
    _watch_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

fn system_mode(window: &Window) -> ThemeMode {
    match window.appearance() {
        WindowAppearance::Dark | WindowAppearance::VibrantDark => ThemeMode::Dark,
        WindowAppearance::Light | WindowAppearance::VibrantLight => ThemeMode::Light,
    }
}

impl Workspace {
    /// Open `path` (absolute) with `view` as the initial view state, in
    /// `mode`.
    pub fn new(
        path: PathBuf,
        view: ViewState,
        mode: AppMode,
        mono: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let theme = scene_theme(system_mode(window));
        cx.set_global(ActiveChrome(Chrome::from_theme(&theme)));

        let search_input = cx.new(|cx| TextInput::new("Search  ( / )", 220.0, cx));
        let diff_base = cx.new(|cx| TextInput::new("base ref", 110.0, cx));
        let diff_head = cx.new(|cx| TextInput::new("head (working tree)", 130.0, cx));
        if let Some(diff) = &view.diff {
            let (base, head) = (diff.base.clone(), diff.head.clone().unwrap_or_default());
            diff_base.update(cx, |i, cx| i.set_text(base, cx));
            diff_head.update(cx, |i, cx| i.set_text(head, cx));
        }
        if let Some(q) = &view.search {
            let q = q.clone();
            search_input.update(cx, |i, cx| i.set_text(q, cx));
        }

        let subscriptions = vec![
            cx.subscribe_in(&search_input, window, |ws, _, event: &InputEvent, window, cx| {
                ws.on_search_event(*event, window, cx);
            }),
            cx.subscribe_in(&diff_base, window, |ws, _, event: &InputEvent, window, cx| {
                ws.on_diff_field_event(*event, window, cx);
            }),
            cx.subscribe_in(&diff_head, window, |ws, _, event: &InputEvent, window, cx| {
                ws.on_diff_field_event(*event, window, cx);
            }),
            cx.observe_window_appearance(window, |ws, window, cx| ws.sync_theme(window, cx)),
        ];

        let sidecar_path = cascade_scene::sidecar_path(&path);
        let mut ws = Self {
            focus: cx.focus_handle(),
            doc: Document::new(path),
            sidecar_path,
            sidecar: LayoutSidecar::default(),
            sidecar_error: None,
            scenarios: Vec::new(),
            parked_viewports: HashMap::new(),
            cone_depth: view.cone.and_then(|c| c.depth),
            view,
            theme_choice: ThemeChoice::System,
            scene: Rc::new(Scene::empty(ViewKind::Causal, theme.background)),
            theme,
            builder: SceneBuilder::new(),
            scene_error: None,
            scene_dirty: true,
            trace_run: TraceRun::Idle,
            diff_run: DiffRun::Off,
            canvas: CanvasState::default(),
            search: SearchState { input: search_input, hits: Vec::new(), cursor: 0, open: false },
            diff_base,
            diff_head,
            status: None,
            status_serial: 0,
            status_timer_for: None,
            status_task: None,
            mono,
            mode: AppMode::View,
            disk: DiskSync::default(),
            build: BuildUi::default(),
            play: PlayUi::new(cx),
            watcher: None,
            reload_task: None,
            trace_task: None,
            diff_task: None,
            _watch_task: None,
            _subscriptions: subscriptions,
        };
        ws.load_initial();
        ws.start_watching(cx);
        ws.set_mode(mode, cx);
        ws.changed(cx);
        window.focus(&ws.focus, cx);
        ws
    }

    /// Resolve the theme from the choice and the window appearance.
    pub(crate) fn sync_theme(&mut self, window: &Window, cx: &mut Context<Self>) {
        let mode = self.theme_choice.resolve(system_mode(window));
        if mode != self.theme.mode {
            self.theme = scene_theme(mode);
            cx.set_global(ActiveChrome(Chrome::from_theme(&self.theme)));
            self.scene_dirty = true;
            cx.notify();
        }
    }

    /// Clear the current status message after a while, unless a newer one
    /// replaced it.
    fn schedule_status_expiry(&mut self, cx: &mut Context<Self>) {
        let Some(status) = &self.status else {
            return;
        };
        if self.status_timer_for == Some(status.serial) {
            return;
        }
        let serial = status.serial;
        let ttl = if status.is_error { ERROR_STATUS_TTL } else { STATUS_TTL };
        self.status_timer_for = Some(serial);
        self.status_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ttl).await;
            let cleared = this.update(cx, |ws, cx| {
                if ws.status.as_ref().is_some_and(|s| s.serial == serial) {
                    ws.status = None;
                    cx.notify();
                }
            });
            if let Err(error) = cleared {
                tracing::debug!(%error, "workspace gone before the status expired");
            }
        }));
    }

    pub(crate) fn toggle_theme(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.theme_choice = self.theme_choice.next(system_mode(window));
        self.set_status(self.theme_choice.label(), false);
        self.sync_theme(window, cx);
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.scene_dirty {
            self.rebuild_scene();
        }
        if self.mode == AppMode::Build {
            self.sync_inspector(window, cx);
        }
        self.schedule_status_expiry(cx);
        let colors = chrome(cx);
        let root = div()
            .key_context("Workspace")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .text_color(colors.text)
            .text_size(px(13.));
        let side_panel = match self.mode {
            AppMode::View => None,
            AppMode::Build => Some(self.render_inspector(cx).into_any_element()),
            AppMode::Play => Some(self.render_play_panel(cx).into_any_element()),
        };
        actions::register(root, cx)
            .child(self.render_toolbar(window, cx))
            .children(self.render_build_bar(cx))
            .children(self.render_banner(cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(cx))
                    .child(self.render_canvas_area(window, cx))
                    .children(side_panel),
            )
            .child(self.render_status_bar(cx))
    }
}
