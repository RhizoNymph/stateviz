//! The Explore panel (Play mode, `E`): the controls, the running readout,
//! the result summary and notes, and one row per outcome with its preview
//! and "Open as branch".

use cascade_sim::OrderingModel;
use gpui::{Context, div, prelude::*, px};

use super::sidebar::severity_color;
use super::{button, caption, error_line, hint, section};
use crate::commands::Command;
use crate::play::explore::{ExploreRun, Freshness};
use crate::theme::chrome;
use crate::workspace::Workspace;

const EXPLORE_WIDTH: f32 = 320.0;

impl Workspace {
    pub(crate) fn render_explore_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = chrome(cx);
        let explore = &self.play.explore;
        let controls = explore.controls;
        let running = explore.running_states();

        // --- Controls --------------------------------------------------------------
        let ordering = [OrderingModel::PerConsumer, OrderingModel::Unordered].map(|ordering| {
            button(("explore-ordering", ordering as usize), ordering.name(), controls.ordering == ordering, colors)
                .on_click(cx.listener(move |ws, _, _, cx| ws.set_explore_ordering(ordering, cx)))
        });
        let switches = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(4.))
            .child(
                button("explore-choices", "Branch on choices", controls.choices, colors)
                    .on_click(cx.listener(|ws, _, _, cx| ws.toggle_explore_choices(cx))),
            )
            .child(
                button("explore-timers", "Timers anytime", controls.timers_anytime, colors)
                    .on_click(cx.listener(|ws, _, _, cx| ws.toggle_explore_timers(cx))),
            );
        let run_row = div().flex().flex_row().items_center().gap(px(6.)).map(|row| match running {
            Some(states) => row
                .child(
                    button("explore-cancel", "Cancel", false, colors)
                        .on_click(cx.listener(|ws, _, _, cx| ws.cancel_explore(cx))),
                )
                .child(caption(format!("Exploring… {states} states"), colors)),
            None => row.child(
                button("explore-run", "Run", false, colors).on_click(cx.listener(|ws, _, _, cx| ws.run_explore(cx))),
            ),
        });
        let controls_section = section("EXPLORE ORDERINGS", colors)
            .child(caption("Every ordering of what is queued, from the session's position.", colors))
            .child(div().flex().flex_row().gap(px(4.)).children(ordering))
            .child(switches)
            .child(run_row);

        // --- Results ----------------------------------------------------------------
        let state = self.play.state.as_ref();
        let mut results = section("OUTCOMES", colors);
        match &explore.run {
            ExploreRun::Idle => {
                results = results.child(caption("Run to list the outcomes.", colors));
            }
            ExploreRun::Running { origin, controls, .. } => {
                results = results.child(caption(
                    format!("Exploring from step {} ({})…", origin.position(), controls.label()),
                    colors,
                ));
            }
            ExploreRun::Failed { origin, error } => {
                results = results
                    .child(caption(format!("Exploring from step {} failed:", origin.position()), colors))
                    .child(error_line(error.clone(), colors));
            }
            ExploreRun::Done(result) => {
                let freshness = state.map_or(Freshness::Stale, |s| result.freshness(s));
                let previewing = state.and_then(|s| explore.previewing(s));
                results = results
                    .child(div().child(result.summary.clone()))
                    .child(caption(
                        format!("From step {} ({})", result.origin.position(), result.controls.label()),
                        colors,
                    ))
                    .when(!result.exploration.complete, |el| {
                        el.child(error_line("Not every ordering was explored: the state bound was hit.", colors))
                    })
                    .children(result.exploration.notes.iter().map(|note| caption(format!("Note: {note}"), colors)))
                    .when(freshness == Freshness::Moved, |el| {
                        el.child(error_line(
                            format!(
                                "Stale: the session moved since this run. Opening an outcome goes back to step {} first; run again to explore from here.",
                                result.origin.position()
                            ),
                            colors,
                        ))
                    })
                    .when(freshness == Freshness::Stale, |el| {
                        el.child(error_line(
                            "Stale: the model changed or the session left the explored line. Run again.",
                            colors,
                        ))
                    });
                for (index, row) in result.rows.iter().enumerate() {
                    let selected = previewing == Some(index);
                    let usable = freshness.usable();
                    let lines = row.instances.iter().map(|line| div().child(line.clone()));
                    let timers = row.timers.iter().map(|t| caption(format!("timer: {t}"), colors));
                    let anomalies = row
                        .anomalies
                        .iter()
                        .map(|a| div().text_color(severity_color(a.severity, colors)).child(a.text.clone()));
                    let card = div()
                        .id(("outcome", index))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .p(px(6.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(if selected { colors.text } else { colors.border })
                        .bg(if selected { colors.active } else { colors.raised })
                        .when(usable, |el| {
                            el.cursor_pointer()
                                .hover(move |s| s.bg(colors.hover))
                                .on_click(cx.listener(move |ws, _, _, cx| ws.select_outcome(index, cx)))
                        })
                        .when(!usable, |el| el.text_color(colors.muted))
                        .child(div().flex().flex_row().child(div().flex_1().child(row.title.clone())))
                        .children(lines)
                        .when(row.same_instances, |el| el.child(caption("Instances as in outcome 1.", colors)))
                        .children(timers)
                        .children(anomalies)
                        .when(usable, |el| {
                            el.child(div().flex().flex_row().pt(px(2.)).child(
                                button(("open-outcome", index), "Open as branch", false, colors).on_click(cx.listener(
                                    move |ws, _, _, cx| {
                                        cx.stop_propagation();
                                        ws.open_outcome(index, cx);
                                    },
                                )),
                            ))
                        });
                    results = results.child(card);
                }
                if result.rows.len() > 1 {
                    results = results.child(caption(
                        "Outcome 1 lists every instance; the others only what differs. Click one to preview it.",
                        colors,
                    ));
                }
            }
        }

        div()
            .id("explore-panel")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(EXPLORE_WIDTH))
            .h_full()
            .overflow_y_scroll()
            .border_l_1()
            .border_color(colors.border)
            .bg(colors.panel)
            .text_size(px(12.5))
            .child(controls_section)
            .child(results)
            .child(
                div().px(px(10.)).py(px(8.)).child(
                    button("explore-close", hint("Close", Command::ToggleExplore), false, colors)
                        .on_click(cx.listener(|ws, _, _, cx| ws.toggle_explore(cx))),
                ),
            )
    }
}
