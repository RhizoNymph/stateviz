//! Floating panels over the canvas: search, scene notes, trace controls.
//! Each occludes the canvas beneath it so clicks do not fall through.

use cascade_core::ElementKind;
use cascade_scene::ViewKind;
use gpui::{Context, Window, div, prelude::*, px};

use super::{button, caption};
use crate::locate::race_finding;
use crate::theme::chrome;
use crate::trace::TraceRun;
use crate::workspace::Workspace;

fn kind_label(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::Machine => "machine",
        ElementKind::State => "state",
        ElementKind::Transition => "transition",
        ElementKind::Trigger => "trigger",
        ElementKind::Event => "event",
        ElementKind::Controller => "controller",
        ElementKind::Handler => "handler",
        ElementKind::Rule => "rule",
        ElementKind::External => "source",
    }
}

impl Workspace {
    /// The search box (top left) and its results.
    pub(crate) fn render_search(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = chrome(cx);
        let open = self.search.open && !self.search.hits.is_empty();
        let rows: Vec<_> = if open {
            self.search
                .hits
                .iter()
                .enumerate()
                .map(|(index, hit)| {
                    let active = index == self.search.cursor;
                    div()
                        .id(("search-hit", index))
                        .flex()
                        .flex_row()
                        .gap(px(8.))
                        .px(px(8.))
                        .py(px(3.))
                        .cursor_pointer()
                        .when(active, |el| el.bg(colors.active))
                        .hover(move |s| s.bg(colors.hover))
                        .on_click(cx.listener(move |ws, _, window, cx| ws.focus_search_hit(index, window, cx)))
                        .child(
                            div().flex_none().w(px(70.)).text_color(colors.muted).child(kind_label(hit.element.kind())),
                        )
                        .child(div().flex_1().child(hit.label.clone()))
                })
                .collect()
        } else {
            Vec::new()
        };
        div().absolute().top(px(8.)).left(px(8.)).flex().flex_col().occlude().child(self.search.input.clone()).when(
            open,
            |el| {
                el.child(
                    div()
                        .mt(px(2.))
                        .w(px(360.))
                        .py(px(4.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.panel)
                        .text_size(px(12.5))
                        .children(rows)
                        .child(
                            div()
                                .px(px(8.))
                                .pt(px(3.))
                                .child(caption("↑↓ to choose · Enter to focus · Esc to close", colors)),
                        ),
                )
            },
        )
    }

    /// The scene's notes (e.g. "not implemented yet"): centred when the
    /// scene is empty, otherwise in the bottom-left corner.
    pub(crate) fn render_notes(&mut self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let colors = chrome(cx);
        let notes = self.scene.notes.clone();
        if notes.is_empty() {
            return None;
        }
        let empty = self.scene.nodes.is_empty() && self.scene.lanes.is_empty() && self.scene.edges.is_empty();
        let body = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(12.))
            .py(px(8.))
            .rounded(px(6.))
            .border_1()
            .border_color(colors.border)
            .bg(colors.panel)
            .occlude()
            .children(notes.into_iter().map(|n| div().child(n)));
        Some(if empty {
            div().absolute().top_0().left_0().size_full().flex().items_center().justify_center().child(body)
        } else {
            div().absolute().bottom(px(8.)).left(px(8.)).child(body)
        })
    }

    /// Scenario and race picker (top right), in the trace view only.
    pub(crate) fn render_trace_controls(&mut self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        if self.view.view != ViewKind::Trace {
            return None;
        }
        let colors = chrome(cx);
        let selected = self.view.scenario.clone();
        let chips: Vec<_> = self
            .scenarios
            .iter()
            .map(|s| {
                let id = s.id.clone();
                let active = selected.as_deref() == Some(id.as_str());
                button(format!("scenario-{id}"), id.clone(), active, colors)
                    .on_click(cx.listener(move |ws, _, _, cx| ws.pick_scenario(Some(id.clone()), cx)))
            })
            .collect();
        let race = self.view.race.and_then(|r| {
            let loaded = self.doc.loaded()?;
            race_finding(&loaded.findings, r).map(|f| (r, f.message.clone()))
        });
        let state = match self.trace_request().map(|r| (self.trace_run.request() == Some(&r), r)) {
            None => Some(("Pick a scenario to simulate.".to_owned(), false)),
            Some((true, _)) => match &self.trace_run {
                TraceRun::Running { .. } => Some(("Simulating…".to_owned(), false)),
                TraceRun::Failed { error, .. } => Some((error.clone(), true)),
                TraceRun::Done { traces, .. } => {
                    let steps: usize = traces.iter().map(|t| t.steps.len()).sum();
                    Some((format!("{} trace(s), {steps} step(s)", traces.len()), false))
                }
                TraceRun::Idle => None,
            },
            Some((false, _)) => Some(("Waiting for the model…".to_owned(), false)),
        };
        Some(
            div()
                .absolute()
                .top(px(8.))
                .right(px(8.))
                .max_w(px(460.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .px(px(10.))
                .py(px(8.))
                .rounded(px(6.))
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel)
                .occlude()
                .child(caption("SCENARIO", colors))
                .when(chips.is_empty(), |el| {
                    el.child(
                        div().text_size(px(12.)).child(
                            "No scenario files. Add scenarios/*.yaml or *.scenario.yaml next to the definition.",
                        ),
                    )
                })
                .child(div().flex().flex_row().flex_wrap().gap(px(6.)).children(chips))
                .when_some(race, |el, (index, message)| {
                    el.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            .child(div().flex_1().text_size(px(12.)).child(format!("Race #{index}: {message}")))
                            .child(
                                button("race-clear", "Single run", false, colors)
                                    .on_click(cx.listener(|ws, _, _, cx| ws.clear_race(cx))),
                            ),
                    )
                })
                .when_some(state, |el, (text, is_error)| {
                    el.child(
                        div()
                            .text_size(px(12.))
                            .text_color(if is_error { colors.error } else { colors.muted })
                            .child(text),
                    )
                }),
        )
    }
}
