//! New definition elements as text, in the compact style of `to_yaml`.

use cascade_core::definition::{
    ControllerDef, EventDef, ExternalDef, HandlerDef, MachineDef, RuleDef, StateDef, StateKindDef, TransitionDef,
};

use super::scalar::{flow_list, free_text, plain_or_quoted};
use super::{Block, Style};
use crate::yaml::selector;

fn values(items: &[cascade_core::Spanned<String>]) -> Vec<&str> {
    items.iter().map(|s| s.value.as_str()).collect()
}

// --- States ---------------------------------------------------------------------

pub(crate) fn has_body(state: &StateDef) -> bool {
    state.kind.value != StateKindDef::Normal || state.initial.is_some() || !state.states.is_empty()
}

fn is_kind_only(state: &StateDef) -> bool {
    state.initial.is_none() && state.states.is_empty()
}

/// Where a state is written: as a `- name` sequence item or a `name:` entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StateSlot {
    SeqItem,
    MapEntry,
}

/// A state as a sequence item (`- name`, `- name: { kind: final }`, or a
/// block body) or as a mapping entry (`name: {}`, …). Lines are relative
/// to the `-` or the key.
pub(crate) fn state_item(state: &StateDef, style: Style, slot: StateSlot) -> Block {
    let name = plain_or_quoted(&state.name.value, false);
    let (lead, body_indent) = match slot {
        StateSlot::SeqItem => ("- ", 2 + style.unit),
        StateSlot::MapEntry => ("", style.unit),
    };
    let mut block = Block::new();
    if !has_body(state) {
        match slot {
            StateSlot::SeqItem => block.line(0, format!("- {name}")),
            StateSlot::MapEntry => block.line(0, format!("{name}: {{}}")),
        }
    } else if is_kind_only(state) {
        block.line(0, format!("{lead}{name}: {{ kind: {} }}", state.kind.value.name()));
    } else {
        block.line(0, format!("{lead}{name}:"));
        block.nest(body_indent, state_body(state, style));
    }
    block
}

/// A state as a flow sequence item: `name`, or `{ name: { kind: final,
/// initial: a, states: [a, b] } }` when it has a body.
pub(crate) fn state_inline(state: &StateDef) -> String {
    let name = plain_or_quoted(&state.name.value, true);
    if !has_body(state) {
        return name;
    }
    let mut body = Vec::new();
    if state.kind.value != StateKindDef::Normal {
        body.push(format!("kind: {}", state.kind.value.name()));
    }
    if let Some(initial) = &state.initial {
        body.push(format!("initial: {}", plain_or_quoted(&initial.value, true)));
    }
    if !state.states.is_empty() {
        let items: Vec<String> = state.states.iter().map(state_inline).collect();
        body.push(format!("states: [{}]", items.join(", ")));
    }
    format!("{{ {name}: {{ {} }} }}", body.join(", "))
}

/// The body mapping of a state with children.
pub(crate) fn state_body(state: &StateDef, style: Style) -> Block {
    let mut body = Block::new();
    if state.kind.value != StateKindDef::Normal {
        body.line(0, format!("kind: {}", state.kind.value.name()));
    }
    if let Some(initial) = &state.initial {
        body.line(0, format!("initial: {}", plain_or_quoted(&initial.value, false)));
    }
    if !state.states.is_empty() {
        body.nest(0, states_entry(&state.states, style));
    }
    body
}

/// `states: [a, b]` when no state has a body, else a block list.
pub(crate) fn states_entry(states: &[StateDef], style: Style) -> Block {
    let mut block = Block::new();
    if !states.iter().any(has_body) {
        let names: Vec<&str> = states.iter().map(|s| s.name.value.as_str()).collect();
        block.line(0, format!("states: {}", flow_list(&names)));
        return block;
    }
    block.line(0, "states:");
    for state in states {
        block.nest(style.seq_offset, state_item(state, style, StateSlot::SeqItem));
    }
    block
}

// --- Transitions ------------------------------------------------------------------

/// Column layout of one-line flow transitions, relative to their `{`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RowLayout {
    /// `{ from` rather than `{from`.
    pub brace_pad: bool,
    pub to_col: Option<usize>,
    pub on_col: Option<usize>,
    pub rest_col: Option<usize>,
    /// Double-quote the guard.
    pub quote_text: bool,
}

impl Default for RowLayout {
    fn default() -> Self {
        Self { brace_pad: true, to_col: None, on_col: None, rest_col: None, quote_text: false }
    }
}

struct Parts {
    from: String,
    to: String,
    on: String,
    rest: Vec<String>,
}

fn parts(t: &TransitionDef, in_flow: bool, quote: bool) -> Parts {
    let from = match t.from.as_slice() {
        [one] => plain_or_quoted(&one.value, true),
        many => flow_list(&values(many)),
    };
    let mut rest = Vec::new();
    if let Some(guard) = &t.guard {
        rest.push(format!("guard: {}", free_text(&guard.value, in_flow, quote)));
    }
    if !t.emits.is_empty() {
        rest.push(format!("emits: {}", flow_list(&values(&t.emits))));
    }
    if t.bounded {
        rest.push("bounded: true".to_owned());
    }
    Parts {
        from: format!("from: {from}"),
        to: format!("to: {}", plain_or_quoted(&t.to.value, in_flow)),
        on: format!("on: {}", plain_or_quoted(&t.on.value, in_flow)),
        rest,
    }
}

fn pad_to(text: &mut String, col: Option<usize>) {
    let len = text.chars().count();
    let target = col.filter(|&c| c > len).unwrap_or(len + 1);
    text.extend(std::iter::repeat_n(' ', target - len));
}

/// `{ from: a, to: b, on: go, emits: [E] }`, padded to `layout`'s columns
/// where the text fits.
pub(crate) fn transition_row(t: &TransitionDef, layout: &RowLayout) -> String {
    let p = parts(t, true, layout.quote_text);
    let mut row = String::from(if layout.brace_pad { "{ " } else { "{" });
    row.push_str(&p.from);
    row.push(',');
    pad_to(&mut row, layout.to_col);
    row.push_str(&p.to);
    row.push(',');
    pad_to(&mut row, layout.on_col);
    row.push_str(&p.on);
    if !p.rest.is_empty() {
        row.push(',');
        pad_to(&mut row, layout.rest_col);
        row.push_str(&p.rest.join(", "));
    }
    row.push_str(if layout.brace_pad { " }" } else { "}" });
    row
}

/// A transition as a block mapping sequence item; `key_offset` is the
/// column of its keys relative to the `-`.
pub(crate) fn transition_block(t: &TransitionDef, key_offset: usize, quote: bool) -> Block {
    let p = parts(t, false, quote);
    let mut block = Block::new();
    let gap = " ".repeat(key_offset.saturating_sub(1).max(1));
    block.line(0, format!("-{gap}{}", p.from));
    block.line(key_offset, p.to);
    block.line(key_offset, p.on);
    for extra in p.rest {
        block.line(key_offset, extra);
    }
    block
}

/// Rows for a new machine, aligned the way `to_yaml` aligns them.
fn aligned_rows(transitions: &[TransitionDef], quote: bool) -> Vec<String> {
    let all: Vec<Parts> = transitions.iter().map(|t| parts(t, true, quote)).collect();
    let width = |f: &dyn Fn(&Parts) -> usize, only_with_rest: bool| {
        all.iter().filter(|p| !only_with_rest || !p.rest.is_empty()).map(f).max().unwrap_or(0)
    };
    let from_w = width(&|p| p.from.chars().count() + 1, false);
    let to_w = width(&|p| p.to.chars().count() + 1, false);
    let on_w = width(&|p| p.on.chars().count() + 1, true);
    let layout = RowLayout {
        brace_pad: true,
        to_col: Some(2 + from_w + 1),
        on_col: Some(2 + from_w + 1 + to_w + 1),
        rest_col: Some(2 + from_w + 1 + to_w + 1 + on_w + 1),
        quote_text: quote,
    };
    transitions.iter().map(|t| transition_row(t, &layout)).collect()
}

// --- Machines ---------------------------------------------------------------------

/// A machine entry (`Name:` and its body), relative to the key.
pub(crate) fn machine(m: &MachineDef, style: Style) -> Block {
    let mut block = Block::new();
    block.line(0, format!("{}:", plain_or_quoted(&m.name.value, false)));
    let mut body = Block::new();
    if let Some(color) = &m.color {
        body.line(0, format!("color: {}", color.value.name()));
    }
    if let Some(domain) = &m.domain {
        body.line(0, format!("domain: {}", plain_or_quoted(&domain.value, false)));
    }
    if let Some(initial) = &m.initial {
        body.line(0, format!("initial: {}", plain_or_quoted(&initial.value, false)));
    }
    if !m.fields.is_empty() {
        body.line(0, format!("fields: {}", flow_list(&values(&m.fields))));
    }
    body.nest(0, states_entry(&m.states, style));
    if !m.transitions.is_empty() {
        body.line(0, "transitions:");
        for row in aligned_rows(&m.transitions, style.quote_text) {
            body.line(style.seq_offset, format!("- {row}"));
        }
    }
    block.nest(style.unit, body);
    block
}

// --- Events and external sources ------------------------------------------------------

/// `Name: { payload: [a] }` or `Name: {}`.
pub(crate) fn event_entry(e: &EventDef) -> String {
    let name = plain_or_quoted(&e.name.value, false);
    if e.payload.is_empty() {
        format!("{name}: {{}}")
    } else {
        format!("{name}: {{ payload: {} }}", flow_list(&values(&e.payload)))
    }
}

/// `Name: [Machine.trigger, …]`.
pub(crate) fn external_entry(x: &ExternalDef) -> String {
    let triggers: Vec<String> = x.triggers.iter().map(|t| t.value.to_string()).collect();
    format!("{}: {}", plain_or_quoted(&x.name.value, false), flow_list(&triggers))
}

// --- Controllers ------------------------------------------------------------------------

fn rule_parts(r: &RuleDef, in_flow: bool, quote: bool) -> Vec<String> {
    let mut parts = vec![format!("fire: {}", plain_or_quoted(&r.fire.value.to_string(), in_flow))];
    if let Some(target) = &r.target {
        parts.push(format!("target: {}", plain_or_quoted(&selector(&target.value), in_flow)));
    }
    if let Some(when) = &r.when {
        parts.push(format!("when: {}", free_text(&when.value, in_flow, quote)));
    }
    if r.bounded {
        parts.push("bounded: true".to_owned());
    }
    parts
}

/// A rule as a block mapping sequence item; `key_offset` is the column of
/// its keys relative to the `-`.
pub(crate) fn rule_block(r: &RuleDef, key_offset: usize, quote: bool) -> Block {
    let mut block = Block::new();
    let gap = " ".repeat(key_offset.saturating_sub(1).max(1));
    for (i, part) in rule_parts(r, false, quote).into_iter().enumerate() {
        match i {
            0 => block.line(0, format!("-{gap}{part}")),
            _ => block.line(key_offset, part),
        }
    }
    block
}

/// `{ fire: A.go, when: … }`.
pub(crate) fn rule_flow(r: &RuleDef, brace_pad: bool, quote: bool) -> String {
    let inner = rule_parts(r, true, quote).join(", ");
    if brace_pad { format!("{{ {inner} }}") } else { format!("{{{inner}}}") }
}

/// A handler entry (`Event:` and its rules), relative to the key.
pub(crate) fn handler(h: &HandlerDef, style: Style) -> Block {
    let event = plain_or_quoted(&h.event.value, false);
    let mut block = Block::new();
    if h.rules.is_empty() {
        block.line(0, format!("{event}: []"));
        return block;
    }
    block.line(0, format!("{event}:"));
    for rule in &h.rules {
        block.nest(style.seq_offset, rule_block(rule, 2, style.quote_text));
    }
    block
}

/// A controller entry (`Name:`, `on:` and its handlers), relative to the key.
pub(crate) fn controller(c: &ControllerDef, style: Style) -> Block {
    let mut block = Block::new();
    block.line(0, format!("{}:", plain_or_quoted(&c.name.value, false)));
    if c.on.is_empty() {
        block.line(style.unit, "on: {}");
        return block;
    }
    block.line(style.unit, "on:");
    for h in &c.on {
        block.nest(style.unit * 2, handler(h, style));
    }
    block
}

#[cfg(test)]
mod tests {
    use cascade_core::Spanned;
    use cascade_core::span::SourceSpan;

    use super::*;

    fn s(v: &str) -> Spanned<String> {
        Spanned::synthetic(v.to_owned())
    }

    fn t(from: &str, to: &str, on: &str, emits: &[&str]) -> TransitionDef {
        TransitionDef {
            from: vec![s(from)],
            to: s(to),
            on: s(on),
            guard: None,
            emits: emits.iter().map(|e| s(e)).collect(),
            bounded: false,
            span: SourceSpan::unknown(),
        }
    }

    #[test]
    fn rows_pad_to_the_layout_when_they_fit() {
        let layout =
            RowLayout { brace_pad: true, to_col: Some(17), on_col: Some(31), rest_col: None, quote_text: false };
        assert_eq!(
            transition_row(&t("paid", "draft", "refund", &[]), &layout),
            "{ from: paid,    to: draft,    on: refund }"
        );
        assert_eq!(
            transition_row(&t("a_very_long_name", "b", "go", &["E"]), &layout),
            "{ from: a_very_long_name, to: b, on: go, emits: [E] }"
        );
        assert_eq!(transition_row(&t("a", "b", "go", &[]), &RowLayout::default()), "{ from: a, to: b, on: go }");
    }

    #[test]
    fn new_machine_rows_align_like_to_yaml() {
        let rows = aligned_rows(&[t("draft", "pending", "submit", &[]), t("pending", "paid", "ok", &["Paid"])], false);
        assert_eq!(
            rows,
            ["{ from: draft,   to: pending, on: submit }", "{ from: pending, to: paid,    on: ok, emits: [Paid] }"]
        );
    }
}
