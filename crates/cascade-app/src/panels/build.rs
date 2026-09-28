//! Build mode chrome: the build bar (add, delete, connect, undo/redo, new
//! file) and the inspector panel on the right.

use gpui::{Context, div, prelude::*, px};

use super::{button, caption, error_line, section, separator};
use crate::build::connect;
use crate::build::inspector::{FieldInput, FieldValue, InspectorAction};
use crate::build::undo::Direction;
use crate::commands::{Command, key_hint};
use crate::mode::AppMode;
use crate::theme::chrome;
use crate::workspace::Workspace;

const INSPECTOR_WIDTH: f32 = 320.0;

fn hint(label: &str, command: Command) -> String {
    match key_hint(command) {
        Some(key) => format!("{label}  {}", key.to_uppercase()),
        None => label.to_owned(),
    }
}

impl Workspace {
    /// The second toolbar row, in Build mode only.
    pub(crate) fn render_build_bar(&mut self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        if self.mode != AppMode::Build {
            return None;
        }
        let colors = chrome(cx);
        let read_only = self.editable().err();
        let history = &self.build.history;
        let undo = history.peek(Direction::Undo).map(|e| e.label.clone());
        let redo = history.peek(Direction::Redo).map(|e| e.label.clone());
        let (can_undo, can_redo) = (history.can(Direction::Undo), history.can(Direction::Redo));
        let undo_count = history.len(Direction::Undo);
        let can_connect = matches!(self.view.selection.as_slice(), [from, to] if connect::can_connect(from, to));
        let bar = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .py(px(5.))
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.panel)
            .child(caption("Build", colors))
            .child(
                button("add-machine", "+ Machine", false, colors)
                    .on_click(cx.listener(|ws, _, _, cx| ws.add_machine(cx))),
            )
            .child(button("add-state", "+ State", false, colors).on_click(cx.listener(|ws, _, _, cx| ws.add_state(cx))))
            .child(
                button("add-controller", "+ Controller", false, colors)
                    .on_click(cx.listener(|ws, _, _, cx| ws.add_controller(cx))),
            )
            .child(
                button("add-source", "+ Source", false, colors)
                    .on_click(cx.listener(|ws, _, _, cx| ws.add_external(cx))),
            )
            .child(separator(colors))
            .child(
                button("connect", "Connect selection", can_connect, colors)
                    .on_click(cx.listener(|ws, _, _, cx| ws.connect_selection(cx))),
            )
            .child(
                button("delete", hint("Delete", Command::DeleteSelection), false, colors)
                    .on_click(cx.listener(|ws, _, _, cx| ws.delete_selection(cx))),
            )
            .child(separator(colors))
            .child(
                button("undo", hint("Undo", Command::Undo), false, colors)
                    .when(!can_undo, |b| b.text_color(colors.muted))
                    .on_click(cx.listener(|ws, _, _, cx| ws.undo_redo(Direction::Undo, cx))),
            )
            .child(
                button("redo", hint("Redo", Command::Redo), false, colors)
                    .when(!can_redo, |b| b.text_color(colors.muted))
                    .on_click(cx.listener(|ws, _, _, cx| ws.undo_redo(Direction::Redo, cx))),
            )
            .when_some(undo, |el, label| el.child(caption(format!("undo ({undo_count}): {label}"), colors)))
            .when_some(redo, |el, label| el.child(caption(format!("redo: {label}"), colors)))
            .child(separator(colors))
            .child(
                button("new-file", hint("New file…", Command::NewFile), false, colors)
                    .on_click(cx.listener(|ws, _, window, cx| ws.new_file(window, cx))),
            )
            .child(div().flex_1())
            .child(caption("Drag from a handle to connect · shift-click two elements, then Connect", colors));
        let status = match (read_only, &self.build.error) {
            (Some(message), _) => Some(message),
            (None, Some(error)) => Some(error.clone()),
            (None, None) => None,
        };
        Some(div().flex().flex_col().child(bar).when_some(status, |el, message| {
            el.child(
                div()
                    .px(px(12.))
                    .py(px(4.))
                    .border_b_1()
                    .border_l_4()
                    .border_color(colors.error)
                    .bg(colors.raised)
                    .text_size(px(12.))
                    .text_color(colors.error)
                    .child(message),
            )
        }))
    }

    /// The inspector for the selected element.
    pub(crate) fn render_inspector(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = chrome(cx);
        let panel = div()
            .id("inspector")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(INSPECTOR_WIDTH))
            .h_full()
            .overflow_y_scroll()
            .border_l_1()
            .border_color(colors.border)
            .bg(colors.panel)
            .text_size(px(12.5));
        let Some(ui) = &self.build.inspector else {
            return panel.child(
                section("INSPECTOR", colors)
                    .child(div().child("Select an element to edit it."))
                    .child(caption("Add machines, states, controllers and sources with the Build bar.", colors)),
            );
        };
        let inspection = match &ui.inspection {
            Ok(inspection) => inspection.clone(),
            Err(message) => {
                return panel.child(section("INSPECTOR", colors).child(error_line(message.clone(), colors)));
            }
        };
        let mut body = section("INSPECTOR", colors).child(div().text_size(px(13.5)).child(inspection.title.clone()));
        for field in &inspection.fields {
            let id = field.id;
            let error = ui.errors.get(&id).cloned();
            let control = match &field.input {
                FieldInput::Text(_) => match ui.inputs.iter().find(|(f, _)| *f == id) {
                    Some((_, input)) => div().child(input.clone()),
                    None => div(),
                },
                FieldInput::Choice { value, options } => {
                    div().flex().flex_row().flex_wrap().gap(px(4.)).children(options.iter().map(|(option, label)| {
                        let option_value = option.clone();
                        button(format!("choice-{id:?}-{option}"), label.clone(), option == value, colors).on_click(
                            cx.listener(move |ws, _, _, cx| {
                                ws.commit_field(id, FieldValue::Text(option_value.clone()), cx)
                            }),
                        )
                    }))
                }
                FieldInput::Toggle(on) => {
                    let next = !on;
                    div().child(
                        button(format!("toggle-{id:?}"), if *on { "yes" } else { "no" }, *on, colors).on_click(
                            cx.listener(move |ws, _, _, cx| ws.commit_field(id, FieldValue::Toggle(next), cx)),
                        ),
                    )
                }
            };
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            .child(div().flex_none().w(px(84.)).text_color(colors.muted).child(id.label()))
                            .child(control),
                    )
                    .when_some(error, |el, e| el.child(div().pl(px(92.)).child(error_line(e, colors)))),
            );
        }
        if inspection.fields.iter().any(|f| matches!(f.input, FieldInput::Text(_))) {
            body = body.child(caption("Enter applies a field · Esc returns to the canvas", colors));
        }
        for note in &inspection.notes {
            body = body.child(div().text_color(colors.muted).child(note.clone()));
        }
        if !inspection.actions.is_empty() {
            body = body.child(div().flex().flex_row().gap(px(6.)).children(inspection.actions.iter().map(|action| {
                let action = *action;
                let label = match action {
                    InspectorAction::AddChildState => "+ Child state",
                    InspectorAction::Delete => "Delete",
                };
                button(format!("action-{action:?}"), label, false, colors)
                    .on_click(cx.listener(move |ws, _, _, cx| ws.inspector_action(action, cx)))
            })));
        }
        let mut panel = panel.child(body);
        if let Some((title, items)) = &inspection.list {
            let rows = items.iter().enumerate().map(|(index, item)| {
                let select = item.key.clone();
                let remove = item.key.clone();
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .id(("inspector-item", index))
                            .flex_1()
                            .px(px(4.))
                            .py(px(2.))
                            .rounded(px(3.))
                            .cursor_pointer()
                            .hover(move |s| s.bg(colors.hover))
                            .on_click(cx.listener(move |ws, _, _, cx| ws.select_key(select.clone(), cx)))
                            .child(item.label.clone()),
                    )
                    .child(
                        button(("inspector-remove", index), "Remove", false, colors)
                            .on_click(cx.listener(move |ws, _, _, cx| ws.delete_key(remove.clone(), cx))),
                    )
            });
            let mut list = section(title.to_uppercase(), colors).children(rows);
            if items.is_empty() {
                list = list.child(caption("None yet.", colors));
            }
            panel = panel.child(list);
        }
        panel
    }
}
