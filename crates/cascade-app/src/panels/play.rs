//! Play mode's panel on the right: session, instances, trigger palette,
//! clock, queue (with the choice prompt and per-consumer delivery), last
//! action, timeline and saving as a scenario.

use cascade_core::ElementRef;
use gpui::{Context, div, prelude::*, px};

use super::{button, caption, error_line, hint, muted_button, section};
use crate::commands::Command;
use crate::play::clock::{advance_label, timer_rows};
use crate::play::forms::group_fires;
use crate::play::queue::queue_rows;
use crate::play::timeline::{acting_forks, branch_label, chips};
use crate::theme::chrome;
use crate::workspace::Workspace;

const PLAY_WIDTH: f32 = 340.0;

impl Workspace {
    pub(crate) fn render_play_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = chrome(cx);
        let panel = div()
            .id("play-panel")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(PLAY_WIDTH))
            .h_full()
            .overflow_y_scroll()
            .border_l_1()
            .border_color(colors.border)
            .bg(colors.panel)
            .text_size(px(12.5));
        let (Some(loaded), Some(state)) = (self.doc.loaded().cloned(), self.play.state.as_ref()) else {
            return panel.child(section("PLAY", colors).child(div().child("Load a definition to play it.")));
        };
        let model = &loaded.model;
        let session = state.session();
        let stale = state.generation() != loaded.generation;

        // --- Session -----------------------------------------------------------
        let scenario_chips = self.scenarios.iter().map(|s| {
            let id = s.id.clone();
            button(format!("play-from-{id}"), id.clone(), false, colors)
                .on_click(cx.listener(move |ws, _, _, cx| ws.load_play_scenario(id.clone(), cx)))
        });
        let session_section = section("SESSION", colors)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(6.))
                    .child(
                        button("play-restart", "New session", false, colors)
                            .on_click(cx.listener(|ws, _, _, cx| ws.restart_play(cx))),
                    )
                    .children(scenario_chips),
            )
            .when(self.scenarios.is_empty(), |el| el.child(caption("No scenarios to start from yet.", colors)))
            .when_some(state.error.clone(), |el, e| el.child(error_line(e, colors)))
            .when_some(state.replay_note.clone(), |el, note| el.child(error_line(note, colors)))
            .when(stale, |el| {
                el.child(error_line("The session belongs to an older model; it replays on the next change.", colors))
            })
            .when_some(self.play.form_error.clone(), |el, e| el.child(error_line(e, colors)));

        // --- Instances ------------------------------------------------------------
        let instances = if stale { Vec::new() } else { session.instances() };
        let instance_rows = instances.iter().map(|i| {
            let name = i.name.clone();
            let state_path = model.label_of(ElementRef::State(i.state));
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .child(div().flex_none().w(px(56.)).child(i.name.clone()))
                .child(div().flex_1().text_color(colors.muted).child(state_path))
                .child(
                    button(format!("remove-{name}"), "Remove", false, colors)
                        .on_click(cx.listener(move |ws, _, _, cx| ws.remove_instance(name.clone(), cx))),
                )
        });
        let machine_chips = model.machines().map(|(_, m)| {
            let name = m.name.clone();
            let active = self.play.machine.as_deref() == Some(name.as_str());
            button(format!("pick-{name}"), name.clone(), active, colors)
                .on_click(cx.listener(move |ws, _, _, cx| ws.pick_play_machine(name.clone(), cx)))
        });
        let instances_section = section(format!("INSTANCES  ({})", instances.len()), colors)
            .children(instance_rows)
            .when(instances.is_empty(), |el| el.child(caption("No instances yet: add one below.", colors)))
            .child(div().flex().flex_row().flex_wrap().gap(px(4.)).children(machine_chips))
            .child(div().flex().flex_row().gap(px(6.)).child(self.play.name.clone()).child(self.play.start.clone()))
            .child(
                div().flex().flex_row().gap(px(6.)).child(self.play.fields.clone()).child(
                    button("add-instance", "Add instance", false, colors)
                        .on_click(cx.listener(|ws, _, _, cx| ws.add_instance(cx))),
                ),
            );

        // --- Palette ----------------------------------------------------------------
        let groups = if stale { Vec::new() } else { group_fires(session.available_fires(model)) };
        let mut palette = section("TRIGGERS", colors);
        if groups.is_empty() {
            palette =
                palette.child(caption("Nothing to fire: add an instance of a machine a source can trigger.", colors));
        }
        for (group_index, (source, fires)) in groups.into_iter().enumerate() {
            let mut buttons = Vec::with_capacity(fires.len());
            for (index, fire) in fires.into_iter().enumerate() {
                let label = format!("{} → {}", fire.trigger.trigger, fire.target);
                let id = ("fire", group_index * 10_000 + index);
                let (source, trigger, target) = (fire.source.clone(), fire.trigger.clone(), fire.target.clone());
                let el = if fire.accepted { button(id, label, false, colors) } else { muted_button(id, label, colors) };
                buttons.push(el.on_click(cx.listener(move |ws, _, _, cx| {
                    ws.fire(source.clone(), trigger.clone(), target.clone(), cx);
                })));
            }
            palette = palette
                .child(div().text_color(colors.muted).child(source))
                .child(div().flex().flex_row().flex_wrap().gap(px(4.)).children(buttons));
        }
        palette = palette.child(self.play.payload.clone()).child(caption(
            "Dashed triggers are not accepted in the target's state; firing one records a drop.",
            colors,
        ));

        // --- Clock ------------------------------------------------------------------------
        let now = session.now();
        let timers = if stale { Vec::new() } else { timer_rows(model, now, &session.timers()) };
        let timer_list = timers.iter().map(|t| {
            div()
                .flex()
                .flex_row()
                .gap(px(6.))
                .child(div().flex_none().w(px(56.)).child(t.instance.clone()))
                .child(div().flex_1().text_color(colors.muted).child(t.trigger.clone()))
                .child(div().flex_none().child(t.due_text()))
        });
        let advance_text = self.play.advance_by.read(cx).text().to_owned();
        let clock = section(format!("CLOCK  now {now}"), colors)
            .children(timer_list)
            .when(timers.is_empty(), |el| el.child(caption("No timers armed.", colors)))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(6.))
                    .child(
                        button("advance-next", hint("Next timer", Command::PlayAdvance), false, colors)
                            .when(timers.is_empty(), |b| b.text_color(colors.muted))
                            .on_click(cx.listener(|ws, _, _, cx| ws.play_advance(true, cx))),
                    )
                    .child(self.play.advance_by.clone())
                    .child(
                        button("advance-by", advance_label(&advance_text), false, colors)
                            .on_click(cx.listener(|ws, _, _, cx| ws.play_advance(false, cx))),
                    ),
            );

        // --- Queue ----------------------------------------------------------------------
        let pending = if stale { Vec::new() } else { session.pending() };
        let rows = queue_rows(&pending);
        let queue_rows_el = rows.iter().enumerate().map(|(index, row)| {
            let request = row.request.clone();
            let consumers = row.consumers.iter().enumerate().map(|(n, item)| {
                let request = item.request.clone();
                button(("consumer", index * 1_000 + n), item.label.clone(), false, colors)
                    .on_click(cx.listener(move |ws, _, _, cx| ws.request_step(request.clone(), cx)))
            });
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .id(("pending", index))
                        .flex()
                        .flex_row()
                        .gap(px(6.))
                        .px(px(4.))
                        .py(px(2.))
                        .rounded(px(3.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(colors.hover))
                        .on_click(cx.listener(move |ws, _, _, cx| ws.request_step(request.clone(), cx)))
                        .child(div().flex_none().w(px(28.)).text_color(colors.muted).child(row.position.clone()))
                        .child(div().flex_1().child(row.label.clone())),
                )
                .when_some(row.waiting.clone(), |el, waiting| {
                    el.child(
                        div()
                            .pl(px(38.))
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .items_center()
                            .gap(px(4.))
                            .child(caption(waiting, colors))
                            .children(consumers),
                    )
                })
        });
        let prompt = self.play.prompt.as_ref().map(|prompt| {
            let options = prompt.point.options.iter().enumerate().map(|(n, option)| {
                let value = option.clone();
                let label = if n == 0 { format!("{option}  (default)") } else { option.clone() };
                button(("choose", n), label, false, colors)
                    .on_click(cx.listener(move |ws, _, _, cx| ws.pick_choice(value.clone(), cx)))
            });
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .p(px(6.))
                .rounded(px(4.))
                .border_1()
                .border_color(colors.text)
                .bg(colors.raised)
                .child(div().child(prompt.title()))
                .child(
                    div().flex().flex_row().flex_wrap().gap(px(4.)).children(options).child(
                        button("choose-cancel", "Cancel", false, colors)
                            .on_click(cx.listener(|ws, _, _, cx| ws.cancel_choice(cx))),
                    ),
                )
                .when(!prompt.answers.is_empty(), |el| {
                    el.child(caption(format!("so far: {}", prompt.answers.join(", ")), colors))
                })
                .child(caption("Space takes the default.", colors))
        });
        let decisions = if stale { Vec::new() } else { session.pending_decisions().to_vec() };
        let queue = section(format!("QUEUE  ({})", pending.len()), colors)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.))
                    .child(
                        button("step", "Step  SPACE", false, colors)
                            .on_click(cx.listener(|ws, _, _, cx| ws.play_step(None, cx))),
                    )
                    .child(
                        button("run", "Run until quiet  R", false, colors)
                            .on_click(cx.listener(|ws, _, _, cx| ws.play_run(cx))),
                    ),
            )
            .children(prompt)
            .children(queue_rows_el)
            .when(pending.is_empty(), |el| el.child(caption("The queue is empty.", colors)))
            .when(!pending.is_empty(), |el| {
                el.child(caption("Click an item to deliver it out of order; → delivers to one consumer.", colors))
            })
            .when(!decisions.is_empty(), |el| {
                el.child(caption(format!("Pending decisions: {}", decisions.join(", ")), colors))
            });

        // --- Last action ------------------------------------------------------------------
        let steps = if stale { Vec::new() } else { state.last_steps(model) };
        let last = section("LAST ACTION", colors)
            .children(steps.iter().map(|line| div().child(line.clone())))
            .when(steps.is_empty(), |el| el.child(caption("Nothing happened yet.", colors)));

        // --- Timeline ------------------------------------------------------------------------
        let timeline = session.timeline();
        let chip_row = chips(timeline).into_iter().map(|chip| {
            let position = chip.position;
            let el = if chip.applied {
                button(("seek", position), chip.label, chip.current, colors)
            } else {
                muted_button(("seek", position), chip.label, colors)
            };
            el.on_click(cx.listener(move |ws, _, _, cx| ws.seek(position, cx)))
        });
        let branches = timeline.branches.iter().enumerate().map(|(index, branch)| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .child(div().flex_1().child(branch_label(index, branch)))
                .child(
                    button(("branch", index), "Switch", false, colors)
                        .on_click(cx.listener(move |ws, _, _, cx| ws.switch_branch(index, cx))),
                )
        });
        let timeline_section = section("TIMELINE", colors)
            .child(div().flex().flex_row().flex_wrap().gap(px(4.)).children(chip_row))
            .when(acting_forks(timeline), |el| {
                el.child(caption("Acting now keeps the later actions as a branch.", colors))
            })
            .children(branches);

        // --- Save -----------------------------------------------------------------------------
        let save = section("SAVE AS SCENARIO", colors).child(
            div().flex().flex_row().gap(px(6.)).child(self.play.scenario_name.clone()).child(
                button("save-scenario", "Save", false, colors)
                    .on_click(cx.listener(|ws, _, _, cx| ws.save_scenario(cx))),
            ),
        );

        panel
            .child(session_section)
            .child(instances_section)
            .child(palette)
            .child(clock)
            .child(queue)
            .child(last)
            .child(timeline_section)
            .child(save)
    }
}
