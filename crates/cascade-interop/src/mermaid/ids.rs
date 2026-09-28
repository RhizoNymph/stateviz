//! Mermaid node ids for the state diagram.
//!
//! State diagram ids are global (two composites cannot both contain a state
//! `idle`), may not contain `-`, `.` or whitespace, and must not be a
//! keyword. Ids are built from the machine name and state path, sanitized to
//! `[A-Za-z0-9_]`, and made unique with a `_2`, `_3`, … suffix.

use std::collections::HashSet;

/// Words the state diagram or flowchart grammars treat specially, compared
/// case-insensitively.
const KEYWORDS: &[&str] = &[
    "as",
    "bt",
    "choice",
    "class",
    "classdef",
    "click",
    "concurrent",
    "default",
    "direction",
    "end",
    "flowchart",
    "fork",
    "graph",
    "hide",
    "join",
    "left",
    "linkstyle",
    "lr",
    "note",
    "of",
    "over",
    "right",
    "rl",
    "state",
    "statediagram",
    "style",
    "subgraph",
    "tb",
    "td",
    "acctitle",
    "accdescr",
];

/// Replace every character outside `[A-Za-z0-9_]` with `_`; never empty and
/// never starting with a digit.
pub(super) fn sanitize(text: &str) -> String {
    let mut out: String = text.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// Hands out unique ids.
#[derive(Debug, Default)]
pub(super) struct IdAllocator {
    used: HashSet<String>,
}

impl IdAllocator {
    /// A fresh id based on `base` (sanitized, keyword-safe, unique).
    pub(super) fn allocate(&mut self, base: &str) -> String {
        let mut base = sanitize(base);
        if KEYWORDS.contains(&base.to_ascii_lowercase().as_str()) {
            base.push('_');
        }
        if self.used.insert(base.clone()) {
            return base;
        }
        let mut n = 2u32;
        loop {
            let candidate = format!("{base}_{n}");
            if self.used.insert(candidate.clone()) {
                return candidate;
            }
            n = n.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_to_safe_characters() {
        assert_eq!(sanitize("Order-Flow"), "Order_Flow");
        assert_eq!(sanitize("état"), "_tat");
        assert_eq!(sanitize("2fast"), "_2fast");
        assert_eq!(sanitize(""), "_");
    }

    #[test]
    fn allocates_unique_keyword_safe_ids() {
        let mut ids = IdAllocator::default();
        assert_eq!(ids.allocate("A_b"), "A_b");
        assert_eq!(ids.allocate("A.b"), "A_b_2");
        assert_eq!(ids.allocate("A-b"), "A_b_3");
        assert_eq!(ids.allocate("note"), "note_");
        assert_eq!(ids.allocate("End"), "End_");
        assert_eq!(ids.allocate("note"), "note__2");
    }
}
