//! Rendering new text in the file's own style.
//!
//! New entries are written like `crate::to_yaml` writes them (flow lists
//! for names, one-line flow transitions, block rules), but indented with
//! the file's indentation unit and sequence offset, and scalars quoted only
//! when needed. Existing scalars that change keep their quoting style.

mod elements;
mod scalar;

pub(crate) use elements::{
    RowLayout, StateSlot, controller, event_entry, external_entry, handler, has_body, machine, rule_block, rule_flow,
    state_body, state_inline, state_item, transition_block, transition_row,
};
pub(crate) use scalar::{flow_list, free_text, plain_or_quoted, restyle};

use super::doc::{Doc, Kind, Node, ScalarStyle};

/// Lines with indentation relative to wherever they are placed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Block {
    lines: Vec<(usize, String)>,
}

impl Block {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn line(&mut self, indent: usize, text: impl Into<String>) {
        self.lines.push((indent, text.into()));
    }

    /// Append `other` indented by `by`.
    pub fn nest(&mut self, by: usize, other: Block) {
        self.lines.extend(other.lines.into_iter().map(|(i, t)| (i + by, t)));
    }

    /// The first line's text, when the block is one line at indent 0.
    pub fn single_line(&self) -> Option<&str> {
        match self.lines.as_slice() {
            [(0, text)] => Some(text),
            _ => None,
        }
    }

    /// The block as the text of a sequence item that replaces an existing
    /// item node: placed at the item's `-` column `col`, without the first
    /// line's indentation and `- `, and without the final line break.
    pub fn as_item_text(&self, col: usize) -> String {
        let text = self.at(col);
        let text = text.get(col..).unwrap_or_default();
        text.strip_prefix("- ").unwrap_or(text).trim_end_matches('\n').to_owned()
    }

    /// The lines at column `base`, each ending in a line break.
    pub fn at(&self, base: usize) -> String {
        let mut out = String::new();
        for (indent, text) in &self.lines {
            out.extend(std::iter::repeat_n(' ', base + indent));
            out.push_str(text);
            out.push('\n');
        }
        out
    }
}

/// Indentation habits of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Style {
    /// Indentation of a nested mapping relative to its key.
    pub unit: usize,
    /// Column of a block sequence's `-` relative to its key (0 for
    /// "indentless" sequences).
    pub seq_offset: usize,
    /// Whether free text (`guard:`, `when:`) is written double-quoted.
    pub quote_text: bool,
}

impl Style {
    /// Detect from the file: the unit from the first nested block mapping,
    /// the sequence offset from the first block sequence under a key.
    pub fn detect(doc: &Doc) -> Self {
        let unit = find_unit(doc, doc.root()).filter(|&u| u > 0).unwrap_or(2);
        let seq_offset = find_seq_offset(doc, doc.root()).unwrap_or(unit);
        let (mut quoted, mut plain) = (0, 0);
        count_free_text(doc.root(), &mut quoted, &mut plain);
        Self { unit, seq_offset, quote_text: quoted > plain }
    }
}

/// Count double-quoted and plain `guard:` / `when:` values.
fn count_free_text(node: &Node, quoted: &mut usize, plain: &mut usize) {
    match &node.kind {
        Kind::Map(map) => {
            for entry in &map.entries {
                if matches!(entry.key.text(), Some("guard" | "when"))
                    && let Some(scalar) = entry.value.as_scalar()
                {
                    match scalar.style {
                        ScalarStyle::DoubleQuoted => *quoted += 1,
                        ScalarStyle::Plain => *plain += 1,
                        ScalarStyle::SingleQuoted | ScalarStyle::Complex => {}
                    }
                }
                count_free_text(&entry.value, quoted, plain);
            }
        }
        Kind::Seq(seq) => seq.items.iter().for_each(|i| count_free_text(&i.node, quoted, plain)),
        Kind::Scalar(_) | Kind::Null => {}
    }
}

fn find_unit(doc: &Doc, node: &Node) -> Option<usize> {
    let map = node.as_map().filter(|m| !m.flow)?;
    for entry in &map.entries {
        if let Some(inner) = entry.value.as_map().filter(|m| !m.flow)
            && let Some(first) = inner.entries.first()
            && doc.starts_line(first.key.start)
        {
            return doc.col(first.key.start).checked_sub(doc.col(entry.key.start));
        }
    }
    map.entries.iter().find_map(|e| find_unit(doc, &e.value))
}

fn find_seq_offset(doc: &Doc, node: &Node) -> Option<usize> {
    match &node.kind {
        Kind::Map(map) if !map.flow => map.entries.iter().find_map(|entry| {
            if let Some(seq) = entry.value.as_seq().filter(|s| !s.flow)
                && let Some(first) = seq.items.first()
            {
                return doc.col(first.head).checked_sub(doc.col(entry.key.start));
            }
            find_seq_offset(doc, &entry.value)
        }),
        Kind::Seq(seq) if !seq.flow => seq.items.iter().find_map(|i| find_seq_offset(doc, &i.node)),
        _ => None,
    }
}
