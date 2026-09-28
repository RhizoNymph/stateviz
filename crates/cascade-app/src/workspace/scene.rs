//! Deriving the scene, and the background work it depends on.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use cascade_core::diff::ModelDiff;
use cascade_core::{CausalGraph, ElementKey, Finding, Model};
use cascade_layout::Rect;
use cascade_scene::{MonoMeasure, Scene, SceneInput, ViewKind, Viewport};
use cascade_sim::Trace;
use gpui::{AppContext as _, Context};

use super::Workspace;
use crate::diffmode::{self, DiffRequest, DiffRun};
use crate::locate::race_finding;
use crate::trace::{self, TraceRequest, TraceRun};
use crate::viewport::{self, ScreenRect};

impl Workspace {
    /// Something the scene depends on changed: start any background work the
    /// new state needs and rebuild on the next frame.
    pub(crate) fn changed(&mut self, cx: &mut Context<Self>) {
        self.ensure_trace(cx);
        self.ensure_diff(cx);
        self.scene_dirty = true;
        cx.notify();
    }

    /// Rebuild now if anything changed (handlers that need the new scene
    /// immediately, e.g. to centre an element).
    pub(crate) fn ensure_scene(&mut self) {
        if self.scene_dirty {
            self.rebuild_scene();
        }
    }

    pub(crate) fn rebuild_scene(&mut self) {
        self.scene_dirty = false;
        let Some(loaded) = self.doc.loaded().cloned() else {
            let mut scene = Scene::empty(self.view.view, self.theme.background);
            scene.notes.push("No model loaded yet.".to_owned());
            self.scene = Rc::new(scene);
            return;
        };
        let trace_request = self.trace_request();
        let traces: &[Trace] = match &trace_request {
            Some(request) => self.trace_run.traces_for(request),
            None => &[],
        };
        let diff_display = self
            .view
            .diff
            .as_ref()
            .filter(|_| diffmode::applies_to(self.view.view))
            .and_then(|refs| self.diff_run.display_for(refs))
            .cloned();
        let (model, graph, findings, diff): (&Model, &CausalGraph, &[Finding], Option<&ModelDiff>) = match &diff_display
        {
            Some(display) => (&display.model, &display.graph, &[], Some(&display.diff)),
            None => (&loaded.model, &loaded.graph, &loaded.findings, None),
        };
        let measure = MonoMeasure::default();
        let input = SceneInput {
            model,
            graph,
            findings,
            view: &self.view,
            theme: &self.theme,
            measure: &measure,
            sidecar: &self.sidecar,
            traces,
            mode: cascade_scene::SceneMode::View,
            play: None,
            diff,
        };
        let started = Instant::now();
        match self.builder.build(&input) {
            Ok(scene) => {
                tracing::debug!(
                    view = %scene.view,
                    nodes = scene.nodes.len(),
                    edges = scene.edges.len(),
                    elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
                    "scene built"
                );
                self.scene = Rc::new(scene);
                self.scene_error = None;
            }
            Err(error) => {
                tracing::warn!(%error, view = %self.view.view, "scene build failed");
                self.scene_error = Some(error.to_string());
                // Keep the last scene of the same view; never show another
                // view's scene under this view's name.
                if self.scene.view != self.view.view {
                    let mut scene = Scene::empty(self.view.view, self.theme.background);
                    scene.notes.push(format!("The {} view could not be built.", self.view.view));
                    self.scene = Rc::new(scene);
                }
            }
        }
    }

    // --- Viewport ----------------------------------------------------------

    /// The viewport on screen: the stored one, or a fit of the scene.
    pub(crate) fn effective_viewport(&self) -> Option<Viewport> {
        self.view.viewport.or_else(|| self.canvas.bounds.map(|c| viewport::fit(self.scene.bounds, c)))
    }

    pub(crate) fn canvas_rect(&self) -> Option<ScreenRect> {
        self.canvas.bounds
    }

    /// Centre `rect` (scene coordinates) on the canvas.
    pub(crate) fn center_rect(&mut self, rect: Rect) {
        let Some(canvas) = self.canvas_rect() else {
            self.view.viewport = Some(Viewport { center: rect.center(), zoom: 1.0 });
            return;
        };
        let current = self.effective_viewport().unwrap_or(Viewport { center: rect.center(), zoom: 1.0 });
        self.view.viewport = Some(viewport::center_on(current, canvas, rect));
    }

    /// Centre `key` in the current scene; false when it is not drawn.
    pub(crate) fn center_key(&mut self, key: &ElementKey) -> bool {
        self.ensure_scene();
        let located = match self.doc.loaded() {
            Some(loaded) => crate::locate::locate_with_fallback(&self.scene, &loaded.model, key),
            None => crate::locate::locate_key(&self.scene, key),
        };
        match located {
            Some(rect) => {
                self.center_rect(rect);
                true
            }
            None => false,
        }
    }

    /// Switch views, parking the old view's viewport and restoring the new
    /// one's. Returns whether the view changed.
    pub(crate) fn switch_view(&mut self, view: ViewKind) -> bool {
        if self.view.view == view {
            return false;
        }
        self.parked_viewports.insert(self.view.view, self.view.viewport);
        self.view.view = view;
        self.view.viewport = self.parked_viewports.get(&view).copied().flatten();
        if view == ViewKind::Trace && self.view.scenario.is_none() {
            self.view.scenario = self.scenarios.first().map(|s| s.id.clone());
        }
        true
    }

    // --- Traces ------------------------------------------------------------

    /// What the trace view needs, if it is showing a scenario.
    pub(crate) fn trace_request(&self) -> Option<TraceRequest> {
        if self.view.view != ViewKind::Trace {
            return None;
        }
        let scenario = self.view.scenario.clone()?;
        Some(TraceRequest { scenario, race: self.view.race, generation: self.doc.generation() })
    }

    fn ensure_trace(&mut self, cx: &mut Context<Self>) {
        let Some(request) = self.trace_request() else {
            return;
        };
        if !self.trace_run.needs_run(&request) {
            return;
        }
        let Some(loaded) = self.doc.loaded() else {
            return;
        };
        let Some(file) = self.scenarios.iter().find(|s| s.id == request.scenario).cloned() else {
            let error = format!("no scenario file named `{}`", request.scenario);
            self.trace_run = TraceRun::Failed { request, error };
            return;
        };
        let race = match request.race {
            None => None,
            Some(index) => match race_finding(&loaded.findings, index) {
                Some(finding) => Some(finding.detail.clone()),
                None => {
                    let error = trace::TraceError::NoSuchRace(index).to_string();
                    self.trace_run = TraceRun::Failed { request, error };
                    return;
                }
            },
        };
        let model = Arc::clone(&loaded.model);
        self.trace_run = TraceRun::Running { request: request.clone() };
        let work = cx.background_spawn(async move { trace::run(&model, &file.id, &file.path, race.as_ref()) });
        self.trace_task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            if let Err(error) = this.update(cx, |ws, cx| ws.finish_trace(request, result, cx)) {
                tracing::debug!(%error, "workspace gone before the trace finished");
            }
        }));
    }

    fn finish_trace(
        &mut self,
        request: TraceRequest,
        result: Result<Vec<Trace>, trace::TraceError>,
        cx: &mut Context<Self>,
    ) {
        if self.trace_run.request() != Some(&request) {
            return;
        }
        self.trace_run = match result {
            Ok(traces) => {
                tracing::info!(scenario = %request.scenario, traces = traces.len(), "simulation finished");
                TraceRun::Done { request, traces: Arc::new(traces) }
            }
            Err(error) => {
                tracing::warn!(%error, scenario = %request.scenario, "simulation failed");
                TraceRun::Failed { request, error: error.to_string() }
            }
        };
        self.scene_dirty = true;
        cx.notify();
    }

    /// Forget the trace run so the next change reruns it (scenario files
    /// changed on disk).
    pub(crate) fn invalidate_traces(&mut self) {
        self.trace_run = TraceRun::Idle;
        self.trace_task = None;
    }

    // --- Diff --------------------------------------------------------------

    pub(crate) fn diff_request(&self) -> Option<DiffRequest> {
        self.view.diff.clone().map(|refs| DiffRequest::new(refs, self.doc.generation()))
    }

    fn ensure_diff(&mut self, cx: &mut Context<Self>) {
        let Some(request) = self.diff_request() else {
            if !matches!(self.diff_run, DiffRun::Off) {
                self.diff_run = DiffRun::Off;
                self.diff_task = None;
            }
            return;
        };
        if !self.diff_run.needs_run(&request) {
            return;
        }
        let previous = self.view.diff.as_ref().and_then(|refs| self.diff_run.display_for(refs)).cloned();
        let working = self.doc.loaded().map(|l| Arc::clone(&l.model));
        let file = self.doc.path().to_owned();
        let refs = request.refs.clone();
        self.diff_run = DiffRun::Loading { request: request.clone(), previous };
        let work = cx.background_spawn(async move { diffmode::compute(&file, &refs, working) });
        self.diff_task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            if let Err(error) = this.update(cx, |ws, cx| ws.finish_diff(request, result, cx)) {
                tracing::debug!(%error, "workspace gone before the diff finished");
            }
        }));
    }

    fn finish_diff(
        &mut self,
        request: DiffRequest,
        result: Result<diffmode::DiffDisplay, diffmode::DiffError>,
        cx: &mut Context<Self>,
    ) {
        if self.diff_run.request() != Some(&request) {
            return;
        }
        self.diff_run = match result {
            Ok(merged) => {
                let summary = merged.summary();
                tracing::info!(base = %request.refs.base, %summary, "diff ready");
                DiffRun::Ready { request, display: Arc::new(merged) }
            }
            Err(error) => {
                tracing::warn!(%error, base = %request.refs.base, "diff failed");
                DiffRun::Failed { request, error: error.to_string() }
            }
        };
        self.scene_dirty = true;
        cx.notify();
    }
}
