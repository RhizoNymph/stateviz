//! The visual encoding: how each kind of element and link looks.
//!
//! Hue means entity and nothing else. Machine-owned things (transition
//! pills, states, lanes, stubs, transition arrows, fires into a machine)
//! take the machine's hue; events, controllers, external sources, emits and
//! subscriptions are neutral. Selection, focus and search change only
//! outline weight (see `decorate`), never color.

use cascade_core::MachineId;
use cascade_layout::{Point, Size};

use crate::color::{MachineStyle, Rgba, Theme, style_of};
use crate::scene::{Border, Dash, FontWeight, Shape, Stroke};
use crate::text::TextMeasure;

/// Horizontal text padding inside a node.
pub(crate) const PAD_X: f32 = 12.0;
/// Vertical text padding inside a node.
pub(crate) const PAD_Y: f32 = 6.0;
/// Corner radius of states.
pub(crate) const STATE_RADIUS: f32 = 8.0;
/// Width of the initial state's thick left border.
pub(crate) const INITIAL_BORDER: f32 = 5.0;
/// Corner radius of stubs and notes.
pub(crate) const STUB_RADIUS: f32 = 6.0;

/// A label positioned relative to its node's top-left corner.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RelLabel {
    pub text: String,
    pub offset: Point,
    pub font_size: f32,
    pub color: Rgba,
    pub weight: FontWeight,
}

/// A circle drawn with a node (the ring around a history marker), relative
/// to the node's top-left corner.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RelCircle {
    pub center: Point,
    pub radius: f32,
    pub stroke: Stroke,
}

/// Everything about a node's appearance except its position.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeLook {
    pub shape: Shape,
    pub size: Size,
    pub fill: Option<Rgba>,
    pub stroke: Stroke,
    pub border: Border,
    pub labels: Vec<RelLabel>,
    pub circles: Vec<RelCircle>,
}

/// One line of text to lay out in a node.
pub(crate) struct Line {
    pub text: String,
    pub font_size: f32,
    pub color: Rgba,
    pub weight: FontWeight,
}

/// How a state is marked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StateMark {
    Normal,
    Final,
    History { deep: bool },
}

/// Text sizing plus the theme and machine styles: everything needed to turn
/// an element into a [`NodeLook`].
pub(crate) struct Painter<'a> {
    pub theme: &'a Theme,
    pub measure: &'a dyn TextMeasure,
    pub styles: Vec<MachineStyle>,
}

impl Painter<'_> {
    pub fn machine(&self, machine: MachineId) -> MachineStyle {
        style_of(&self.styles, machine)
    }

    pub fn line_height(&self, font_size: f32) -> f32 {
        self.measure.line_height(font_size)
    }

    pub fn text_width(&self, text: &str, font_size: f32) -> f32 {
        self.measure.width(text, font_size)
    }

    pub fn line(&self, text: impl Into<String>, color: Rgba) -> Line {
        Line { text: text.into(), font_size: self.theme.font_size, color, weight: FontWeight::Normal }
    }

    pub fn small(&self, text: impl Into<String>, color: Rgba) -> Line {
        Line { text: text.into(), font_size: self.theme.small_font_size, color, weight: FontWeight::Normal }
    }

    /// Width and height of a block of lines.
    pub fn block(&self, lines: &[Line]) -> Size {
        let width = lines.iter().map(|l| self.text_width(&l.text, l.font_size)).fold(0.0, f32::max);
        let height = lines.iter().map(|l| self.line_height(l.font_size)).sum();
        Size::new(width, height)
    }

    /// Lines centered horizontally in `[inset_left, width - inset_right]`
    /// and vertically in `height`.
    pub fn centered(&self, lines: Vec<Line>, size: Size, inset_left: f32, inset_right: f32) -> Vec<RelLabel> {
        let block = self.block(&lines);
        let mut y = (size.height - block.height) / 2.0;
        let span = size.width - inset_left - inset_right;
        lines
            .into_iter()
            .map(|l| {
                let w = self.text_width(&l.text, l.font_size);
                let label = RelLabel {
                    offset: Point::new(inset_left + (span - w) / 2.0, y),
                    text: l.text,
                    font_size: l.font_size,
                    color: l.color,
                    weight: l.weight,
                };
                y += self.line_height(label.font_size);
                label
            })
            .collect()
    }

    fn boxed(
        &self,
        lines: Vec<Line>,
        pad_x: f32,
        extra_left: f32,
        extra_right: f32,
        min_width: f32,
    ) -> (Size, Vec<RelLabel>) {
        let block = self.block(&lines);
        let size = Size::new(
            (block.width + 2.0 * pad_x + extra_left + extra_right).max(min_width),
            block.height + 2.0 * PAD_Y,
        );
        let labels = self.centered(lines, size, extra_left, extra_right);
        (size, labels)
    }

    /// The outline of a hue-filled shape: a darker (light theme) or lighter
    /// (dark theme) variant of the hue, so outline weight stays visible.
    pub fn hue_outline(&self, style: MachineStyle) -> Rgba {
        style.hue.mix(self.theme.text, 0.45)
    }

    /// Transition: a pill in the machine's full hue, `before → after` with
    /// the trigger underneath.
    pub fn pill(&self, primary: String, trigger: String, style: MachineStyle) -> NodeLook {
        let lines = vec![self.line(primary, style.on_hue), self.small(trigger, style.on_hue)];
        let block = self.block(&lines);
        let height = block.height + 2.0 * PAD_Y;
        let pad = PAD_X.max(height * 0.35);
        let (size, labels) = self.boxed(lines, pad, 0.0, 0.0, 0.0);
        NodeLook {
            shape: Shape::Pill,
            size,
            fill: Some(style.hue),
            stroke: Stroke::solid(self.hue_outline(style), self.theme.stroke_width),
            border: Border::Single,
            labels,
            circles: Vec::new(),
        }
    }

    /// Event: a neutral gray tag.
    pub fn tag(&self, text: String) -> NodeLook {
        let lines = vec![self.line(text, self.theme.text)];
        let point = self.line_height(self.theme.font_size) / 2.0 + PAD_Y;
        let (size, labels) = self.boxed(lines, PAD_X, 0.0, point, 0.0);
        NodeLook {
            shape: Shape::Tag,
            size,
            fill: Some(self.theme.neutral.mix(self.theme.background, 0.75)),
            stroke: Stroke::solid(self.theme.neutral, self.theme.stroke_width),
            border: Border::Single,
            labels,
            circles: Vec::new(),
        }
    }

    /// Controller (one hexagon per handler): dark neutral outline, no fill.
    pub fn hexagon(&self, text: String) -> NodeLook {
        let lines = vec![self.line(text, self.theme.text)];
        let point = (self.line_height(self.theme.font_size) / 2.0 + PAD_Y) * 0.8;
        let (size, labels) = self.boxed(lines, PAD_X, point, point, 0.0);
        NodeLook {
            shape: Shape::Hexagon,
            size,
            fill: None,
            stroke: Stroke::solid(self.theme.controller, self.theme.stroke_width),
            border: Border::Single,
            labels,
            circles: Vec::new(),
        }
    }

    /// External source: a plain rectangle with a neutral outline.
    pub fn external(&self, text: String) -> NodeLook {
        let lines = vec![self.line(text, self.theme.text)];
        let (size, labels) = self.boxed(lines, PAD_X, 0.0, 0.0, 0.0);
        NodeLook {
            shape: Shape::Rect,
            size,
            fill: None,
            stroke: Stroke::solid(self.theme.external, self.theme.stroke_width),
            border: Border::Single,
            labels,
            circles: Vec::new(),
        }
    }

    /// State: a rounded rectangle with the machine's pale fill. Initial
    /// states get a thick left border, final states a double border, history
    /// states a circled `H` / `H*`. `detail` is a small second line.
    pub fn state(
        &self,
        name: String,
        detail: Option<String>,
        style: MachineStyle,
        initial: bool,
        mark: StateMark,
    ) -> NodeLook {
        let mut lines = vec![self.line(name, self.theme.text)];
        if let Some(d) = detail {
            lines.push(self.small(d, self.theme.text_muted));
        }
        let marker = match mark {
            StateMark::History { deep: false } => Some("H"),
            StateMark::History { deep: true } => Some("H*"),
            StateMark::Normal | StateMark::Final => None,
        };
        let small = self.theme.small_font_size;
        let ring = marker.map(|m| (self.text_width(m, small).max(self.line_height(small)) / 2.0 + 2.0, m));
        let left = if initial { INITIAL_BORDER } else { 0.0 } + ring.map_or(0.0, |(r, _)| 2.0 * r + 4.0);
        let (size, mut labels) = self.boxed(lines, PAD_X, left, 0.0, 48.0);
        let mut circles = Vec::new();
        if let Some((radius, text)) = ring {
            let cx = if initial { INITIAL_BORDER } else { 0.0 } + PAD_X / 2.0 + radius;
            let center = Point::new(cx, size.height / 2.0);
            circles.push(RelCircle { center, radius, stroke: Stroke::solid(self.theme.text, 1.0) });
            labels.push(RelLabel {
                text: text.to_owned(),
                offset: Point::new(
                    center.x - self.text_width(text, small) / 2.0,
                    center.y - self.line_height(small) / 2.0,
                ),
                font_size: small,
                color: self.theme.text,
                weight: FontWeight::Bold,
            });
        }
        NodeLook {
            shape: Shape::RoundedRect { radius: STATE_RADIUS },
            size,
            fill: Some(style.pale),
            stroke: Stroke::solid(style.hue, self.theme.stroke_width),
            border: match (mark, initial) {
                (_, true) => Border::ThickLeft(INITIAL_BORDER),
                (StateMark::Final, false) => Border::Double,
                _ => Border::Single,
            },
            labels,
            circles,
        }
    }

    /// A hidden machine: a dashed stub in the machine's hue, labelled with
    /// its link count ("Payment, 4 links").
    pub fn stub(&self, machine: &str, links: u32, style: MachineStyle) -> NodeLook {
        let lines = vec![self.line(stub_label(machine, links), self.theme.text)];
        let (size, labels) = self.boxed(lines, PAD_X, 0.0, 0.0, 0.0);
        NodeLook {
            shape: Shape::Stub,
            size,
            fill: Some(style.pale),
            stroke: Stroke::dashed(style.hue, self.theme.stroke_width),
            border: Border::Single,
            labels,
            circles: Vec::new(),
        }
    }

    /// A collapsed machine in the structure view.
    pub fn collapsed_machine(&self, machine: &str, detail: String, style: MachineStyle) -> NodeLook {
        let lines = vec![
            Line {
                text: machine.to_owned(),
                font_size: self.theme.font_size,
                color: self.theme.text,
                weight: FontWeight::Bold,
            },
            self.small(detail, self.theme.text_muted),
        ];
        let (size, labels) = self.boxed(lines, PAD_X, 0.0, 0.0, 0.0);
        NodeLook {
            shape: Shape::RoundedRect { radius: STATE_RADIUS },
            size,
            fill: Some(style.pale),
            stroke: Stroke::solid(style.hue, self.theme.stroke_width),
            border: Border::Double,
            labels,
            circles: Vec::new(),
        }
    }

    // --- Links ------------------------------------------------------------

    /// State → state within a machine: solid, machine hue.
    pub fn transition_stroke(&self, style: MachineStyle) -> Stroke {
        Stroke::solid(style.hue, self.theme.stroke_width)
    }

    /// Transition → event: dashed gray.
    pub fn emit_stroke(&self) -> Stroke {
        Stroke::dashed(self.theme.neutral, self.theme.stroke_width)
    }

    /// Event → controller: solid gray.
    pub fn subscribe_stroke(&self) -> Stroke {
        Stroke::solid(self.theme.neutral, self.theme.stroke_width)
    }

    /// Controller → transition: dashed in the target machine's hue.
    pub fn fire_stroke(&self, target: MachineStyle) -> Stroke {
        Stroke::dashed(target.hue, self.theme.stroke_width)
    }

    /// External source → transition: solid, the external sources' neutral.
    pub fn trigger_stroke(&self) -> Stroke {
        Stroke::solid(self.theme.external, self.theme.stroke_width)
    }

    /// A link rerouted to a hidden machine's stub: dotted, keeping the
    /// color of the link it stands for.
    pub fn stub_link_stroke(&self, original: Stroke) -> Stroke {
        Stroke { dash: Dash::Dotted, ..original }
    }
}

/// "Payment, 4 links" / "Payment, 1 link".
pub(crate) fn stub_label(machine: &str, links: u32) -> String {
    let noun = if links == 1 { "link" } else { "links" };
    format!("{machine}, {links} {noun}")
}

/// `[guard]`, or `None` for an empty guard.
pub(crate) fn bracketed(text: Option<&str>) -> Option<String> {
    text.map(str::trim).filter(|t| !t.is_empty()).map(|t| format!("[{t}]"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::MonoMeasure;

    #[test]
    fn stub_labels_count_links() {
        assert_eq!(stub_label("Payment", 4), "Payment, 4 links");
        assert_eq!(stub_label("Payment", 1), "Payment, 1 link");
        assert_eq!(bracketed(Some(" amount > 0 ")), Some("[amount > 0]".to_owned()));
        assert_eq!(bracketed(Some("  ")), None);
    }

    #[test]
    fn labels_fit_inside_their_nodes() {
        let theme = Theme::light();
        let measure = MonoMeasure::default();
        let painter = Painter { theme: &theme, measure: &measure, styles: Vec::new() };
        let style = crate::color::style_for(cascade_core::PaletteColor::Blue, 0, &theme);
        for look in [
            painter.pill("Order: pending → paid".into(), "capture_ok".into(), style),
            painter.tag("OrderPaid".into()),
            painter.hexagon("Fulfillment".into()),
            painter.external("Customer".into()),
            painter.state("waiting".into(), Some("▾ 2 states".into()), style, true, StateMark::History { deep: true }),
            painter.stub("Payment", 4, style),
        ] {
            for label in &look.labels {
                let w = measure.width(&label.text, label.font_size);
                assert!(label.offset.x >= 0.0 && label.offset.x + w <= look.size.width + 0.01, "{label:?} in {look:?}");
                assert!(label.offset.y >= 0.0, "{label:?}");
            }
        }
    }
}
