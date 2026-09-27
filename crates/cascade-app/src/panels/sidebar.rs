//! The sidebar: machine legend (entity filter) and the findings list.

use cascade_core::Severity;
use cascade_scene::machine_styles;
use gpui::{Context, div, prelude::*, px};

use super::{button, caption};
use crate::theme::{Chrome, chrome, hsla};
use crate::workspace::Workspace;

const SIDEBAR_WIDTH: f32 = 290.0;

pub fn severity_color(severity: Severity, colors: Chrome) -> gpui::Hsla {
    match severity {
        Severity::Error => colors.error,
        Severity::Warning => colors.warning,
        Severity::Info => colors.info,
    }
}

impl Workspace {
    pub(crate) fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = chrome(cx);
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .border_r_1()
            .border_color(colors.border)
            .bg(colors.panel)
            .child(self.render_legend(colors, cx))
            .child(self.render_findings(colors, cx))
    }

    fn render_legend(&mut self, colors: Chrome, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<_> = match self.doc.loaded() {
            Some(loaded) => {
                let styles = machine_styles(&loaded.model, &self.theme);
                loaded
                    .model
                    .machines()
                    .map(|(id, machine)| {
                        let name = machine.name.clone();
                        let hidden = self.view.hidden_machines.contains(&name);
                        let hue = styles.get(id.index()).map_or(colors.muted, |s| hsla(s.hue, 1.0));
                        let chip = div().flex_none().size(px(12.)).rounded(px(3.)).border_2().border_color(hue);
                        let chip = if hidden { chip } else { chip.bg(hue) };
                        let toggle_name = name.clone();
                        div()
                            .id(format!("legend-{name}"))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            .px(px(10.))
                            .py(px(3.))
                            .cursor_pointer()
                            .hover(move |s| s.bg(colors.hover))
                            .on_click(cx.listener(move |ws, _, _, cx| ws.toggle_machine(&toggle_name, cx)))
                            .child(chip)
                            .child(
                                div().flex_1().text_color(if hidden { colors.muted } else { colors.text }).child(name),
                            )
                            .when(hidden, |el| el.child(caption("hidden", colors)))
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        let any_hidden = !self.view.hidden_machines.is_empty();
        div()
            .flex()
            .flex_col()
            .flex_none()
            .max_h(px(260.))
            .border_b_1()
            .border_color(colors.border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(px(10.))
                    .py(px(6.))
                    .child(caption("MACHINES  (click to hide)", colors))
                    .when(any_hidden, |el| {
                        el.child(
                            button("show-all", "Show all", false, colors)
                                .on_click(cx.listener(|ws, _, _, cx| ws.show_all_machines(cx))),
                        )
                    }),
            )
            .child(div().id("legend-list").flex().flex_col().overflow_y_scroll().pb(px(6.)).children(rows))
    }

    fn render_findings(&mut self, colors: Chrome, cx: &mut Context<Self>) -> impl IntoElement {
        let findings = self.doc.loaded().map(|l| l.findings.clone());
        let count = findings.as_ref().map_or(0, |f| f.len());
        let rows: Vec<_> = findings
            .iter()
            .flat_map(|f| f.iter().enumerate())
            .map(|(index, finding)| {
                let color = severity_color(finding.severity, colors);
                div()
                    .id(("finding", index))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .px(px(10.))
                    .py(px(5.))
                    .border_b_1()
                    .border_color(colors.border)
                    .cursor_pointer()
                    .hover(move |s| s.bg(colors.hover))
                    .on_click(cx.listener(move |ws, _, _, cx| ws.open_finding(index, cx)))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(6.))
                            .text_size(px(11.5))
                            .child(div().text_color(color).child(finding.severity.to_string()))
                            .child(div().text_color(colors.muted).child(finding.check().code())),
                    )
                    .child(div().text_size(px(12.5)).child(finding.message.clone()))
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(div().px(px(10.)).py(px(6.)).child(caption(format!("FINDINGS  ({count})"), colors)))
            .child(
                div()
                    .id("findings-list")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_y_scroll()
                    .when(count == 0, |el| el.child(div().px(px(10.)).child(caption("No findings.", colors))))
                    .children(rows),
            )
    }
}
