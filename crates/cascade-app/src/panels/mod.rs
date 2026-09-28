//! The app chrome around the canvas. Each module renders one area of the
//! workspace from its state; all state changes go through `Workspace`
//! methods.

mod build;
mod overlays;
mod play;
mod sidebar;
mod status;
mod toolbar;

use gpui::{Div, ElementId, SharedString, Stateful, div, prelude::*, px};

use crate::theme::Chrome;

/// A small push button; `active` shows a pressed state by weight and
/// background, never by hue.
pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, active: bool, colors: Chrome) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(8.))
        .py(px(3.))
        .rounded(px(4.))
        .border_1()
        .border_color(if active { colors.text } else { colors.border })
        .bg(if active { colors.active } else { colors.raised })
        .hover(move |s| s.bg(colors.hover))
        .cursor_pointer()
        .text_size(px(12.))
        .child(label.into())
}

/// A muted caption.
pub fn caption(text: impl Into<SharedString>, colors: Chrome) -> Div {
    div().flex_none().text_size(px(11.5)).text_color(colors.muted).child(text.into())
}

/// A thin vertical separator for toolbars.
pub fn separator(colors: Chrome) -> Div {
    div().flex_none().w(px(1.)).h(px(18.)).mx(px(4.)).bg(colors.border)
}

/// A button that works but reads as unavailable: muted text on a dashed
/// border (a disabled palette trigger still records a drop).
pub fn muted_button(id: impl Into<ElementId>, label: impl Into<SharedString>, colors: Chrome) -> Stateful<Div> {
    button(id, label, false, colors).text_color(colors.muted).border_dashed()
}

/// A titled section of a side panel.
pub fn section(title: impl Into<SharedString>, colors: Chrome) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .px(px(10.))
        .py(px(8.))
        .border_b_1()
        .border_color(colors.border)
        .child(caption(title, colors))
}

/// An inline error line.
pub fn error_line(text: impl Into<SharedString>, colors: Chrome) -> Div {
    div().text_size(px(11.5)).text_color(colors.error).child(text.into())
}
