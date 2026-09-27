//! The diagnostics banner and the status bar.

use cascade_core::Severity;
use gpui::{Context, div, prelude::*, px};

use super::caption;
use crate::diffmode::DiffRun;
use crate::document::DocState;
use crate::theme::{Chrome, chrome};
use crate::workspace::Workspace;

/// Diagnostics shown before "…and N more".
const BANNER_LINES: usize = 8;

fn time(at: chrono::DateTime<chrono::Local>) -> String {
    at.format("%H:%M:%S").to_string()
}

fn banner_block(title: String, lines: Vec<String>, colors: Chrome) -> gpui::Div {
    let more = lines.len().saturating_sub(BANNER_LINES);
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .px(px(12.))
        .py(px(6.))
        .border_b_1()
        .border_l_4()
        .border_color(colors.error)
        .bg(colors.raised)
        .child(div().text_color(colors.error).child(title))
        .children(lines.into_iter().take(BANNER_LINES).map(|l| div().text_size(px(12.)).child(l)))
        .when(more > 0, |el| el.child(caption(format!("…and {more} more"), colors)))
}

impl Workspace {
    /// Load diagnostics, pin and diff problems, above the canvas.
    pub(crate) fn render_banner(&mut self, cx: &mut Context<Self>) -> Vec<gpui::Div> {
        let colors = chrome(cx);
        let mut blocks = Vec::new();
        match self.doc.state() {
            DocState::Failed { failure, at } => blocks.push(banner_block(
                format!("{} does not load ({}):", self.doc.path().display(), time(*at)),
                failure.lines(),
                colors,
            )),
            DocState::Stale { failure, at, last_good } => blocks.push(banner_block(
                format!("Reload failed at {}; showing the version loaded at {}:", time(*at), time(last_good.loaded_at)),
                failure.lines(),
                colors,
            )),
            DocState::Empty | DocState::Ready { .. } => {}
        }
        if let Some(error) = &self.sidecar_error {
            blocks.push(banner_block("Pins sidecar:".to_owned(), vec![error.clone()], colors));
        }
        if let Some(error) = &self.scene_error {
            blocks.push(banner_block(format!("The {} view failed:", self.view.view), vec![error.clone()], colors));
        }
        if let Some(refs) = &self.view.diff
            && let Some(error) = self.diff_run.error_for(refs)
        {
            blocks.push(banner_block(
                format!("Diff against {} failed:", refs.base),
                error.lines().map(str::to_owned).collect(),
                colors,
            ));
        }
        blocks
    }

    pub(crate) fn render_status_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = chrome(cx);
        let mut parts: Vec<(String, gpui::Hsla)> = vec![(self.doc.path().display().to_string(), colors.muted)];
        match self.doc.state() {
            DocState::Empty => parts.push(("loading…".to_owned(), colors.muted)),
            DocState::Ready { loaded } => parts.push((format!("loaded {}", time(loaded.loaded_at)), colors.muted)),
            DocState::Stale { at, failure, .. } => {
                parts.push((format!("reload failed {} · {} diagnostic(s)", time(*at), failure.count()), colors.error));
            }
            DocState::Failed { failure, .. } => {
                parts.push((format!("{} diagnostic(s)", failure.count()), colors.error));
            }
        }
        if let Some(loaded) = self.doc.loaded() {
            let (e, w, i) =
                (loaded.count(Severity::Error), loaded.count(Severity::Warning), loaded.count(Severity::Info));
            let color = if e > 0 {
                colors.error
            } else if w > 0 {
                colors.warning
            } else {
                colors.muted
            };
            parts.push((format!("{e} error(s) · {w} warning(s) · {i} info"), color));
        }
        if let Some(refs) = &self.view.diff {
            let head = refs.head.clone().unwrap_or_else(|| "working tree".to_owned());
            let state = match &self.diff_run {
                DiffRun::Loading { .. } => "loading…".to_owned(),
                DiffRun::Ready { display, .. } => display.summary(),
                DiffRun::Failed { .. } => "failed".to_owned(),
                DiffRun::Off => String::new(),
            };
            parts.push((format!("diff {}..{head} {state}", refs.base), colors.text));
        }
        if let Some(label) = self.selection_label() {
            parts.push((label, colors.text));
        }
        let zoom = self.effective_viewport().map_or(100.0, |v| v.zoom * 100.0);
        let status = self.status.clone();
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(14.))
            .px(px(10.))
            .py(px(4.))
            .border_t_1()
            .border_color(colors.border)
            .bg(colors.panel)
            .text_size(px(11.5))
            .children(parts.into_iter().map(|(text, color)| div().flex_none().text_color(color).child(text)))
            .child(div().flex_1())
            .children(status.map(|s| {
                div()
                    .flex_none()
                    .max_w(px(520.))
                    .overflow_hidden()
                    .text_color(if s.is_error { colors.error } else { colors.text })
                    .child(s.text)
            }))
            .child(div().flex_none().text_color(colors.muted).child(format!("{zoom:.0}%")))
    }

    /// "Selected: Order: pending → paid" (and the second selection).
    fn selection_label(&self) -> Option<String> {
        let loaded = self.doc.loaded()?;
        let labels: Vec<String> = self
            .view
            .selection
            .iter()
            .map(|k| loaded.model.resolve_key(k).map_or_else(|| k.to_string(), |e| loaded.model.label_of(e)))
            .collect();
        if labels.is_empty() {
            return None;
        }
        let cone = match self.view.cone {
            Some(c) => format!(
                " · {} cone, depth {}",
                match c.direction {
                    cascade_core::Direction::Forward => "forward",
                    cascade_core::Direction::Backward => "backward",
                },
                crate::commands::depth_label(c.depth)
            ),
            None => String::new(),
        };
        Some(format!("Selected: {}{cone}", labels.join("  ⇢  ")))
    }
}
