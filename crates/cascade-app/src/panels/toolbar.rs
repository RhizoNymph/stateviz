//! The toolbar: view tabs, cone controls, diff controls, link and theme
//! buttons.

use cascade_core::Direction;
use cascade_scene::{OutsideFocus, ViewKind};
use gpui::{Context, Window, div, prelude::*, px};

use super::{button, caption, separator};
use crate::commands::{Command, depth_label, key_hint};
use crate::theme::chrome;
use crate::workspace::Workspace;

fn tab_label(view: ViewKind) -> &'static str {
    match view {
        ViewKind::Causal => "Causal",
        ViewKind::Structure => "Structure",
        ViewKind::Trace => "Trace",
        ViewKind::Matrix => "Matrix",
    }
}

fn with_key(label: &str, command: Command) -> String {
    match key_hint(command) {
        Some(key) => format!("{label}  {}", key.to_uppercase()),
        None => label.to_owned(),
    }
}

impl Workspace {
    pub(crate) fn render_toolbar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = chrome(cx);
        let cone = self.view.cone;
        let forward = cone.is_some_and(|c| c.direction == Direction::Forward);
        let backward = cone.is_some_and(|c| c.direction == Direction::Backward);
        let hide = self.view.outside == OutsideFocus::Hide;
        let in_diff = self.view.diff.is_some();

        let tabs = ViewKind::ALL.into_iter().map(|view| {
            button(
                format!("tab-{view}"),
                with_key(tab_label(view), Command::ShowView(view)),
                self.view.view == view,
                colors,
            )
            .on_click(cx.listener(move |ws, _, _, cx| ws.show_view(view, cx)))
        });

        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .py(px(6.))
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.panel)
            .children(tabs)
            .child(separator(colors))
            .child(caption("Cone", colors))
            .child(
                button("cone-forward", with_key("Forward", Command::ConeForward), forward, colors)
                    .on_click(cx.listener(|ws, _, window, cx| ws.run_command(Command::ConeForward, window, cx))),
            )
            .child(
                button("cone-backward", with_key("Backward", Command::ConeBackward), backward, colors)
                    .on_click(cx.listener(|ws, _, window, cx| ws.run_command(Command::ConeBackward, window, cx))),
            )
            .child(caption("Depth", colors))
            .child(
                button("depth-less", "−", false, colors)
                    .on_click(cx.listener(|ws, _, window, cx| ws.run_command(Command::DepthLess, window, cx))),
            )
            .child(div().flex_none().min_w(px(18.)).text_size(px(12.)).child(depth_label(self.cone_depth)))
            .child(
                button("depth-more", "+", false, colors)
                    .on_click(cx.listener(|ws, _, window, cx| ws.run_command(Command::DepthMore, window, cx))),
            )
            .child(
                button("outside", if hide { "Hide  H" } else { "Dim  H" }, hide, colors)
                    .on_click(cx.listener(|ws, _, window, cx| ws.run_command(Command::ToggleOutside, window, cx))),
            )
            .child(separator(colors))
            .child(caption("Diff", colors))
            .child(self.diff_base.clone())
            .child(self.diff_head.clone())
            .child(
                button("diff-compare", "Compare", in_diff, colors)
                    .on_click(cx.listener(|ws, _, window, cx| ws.start_diff(window, cx))),
            )
            .when(in_diff, |el| {
                el.child(
                    button("diff-exit", "Exit diff", false, colors)
                        .on_click(cx.listener(|ws, _, _, cx| ws.exit_diff(cx))),
                )
            })
            .child(separator(colors))
            .child(
                button("copy-link", "Copy link", false, colors)
                    .on_click(cx.listener(|ws, _, window, cx| ws.run_command(Command::CopyLink, window, cx))),
            )
            .child(
                button("paste-link", "Paste link", false, colors)
                    .on_click(cx.listener(|ws, _, window, cx| ws.run_command(Command::PasteLink, window, cx))),
            )
            .child(
                button("fit", with_key("Fit", Command::FitView), false, colors)
                    .on_click(cx.listener(|ws, _, window, cx| ws.run_command(Command::FitView, window, cx))),
            )
            .child(
                button("theme", self.theme_choice.label(), false, colors)
                    .on_click(cx.listener(|ws, _, window, cx| ws.run_command(Command::ToggleTheme, window, cx))),
            )
    }
}
