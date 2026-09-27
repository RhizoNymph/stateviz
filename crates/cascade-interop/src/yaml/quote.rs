//! When a YAML scalar can be written plain, and how to double-quote it when
//! it cannot.
//!
//! The rules are conservative: a string is written plain only when every
//! YAML 1.2 parser reads it back as the same string, in the context it is
//! written in. Everything else is double-quoted with escapes.

use std::borrow::Cow;

/// Where a scalar is written. Flow collections (`[a, b]`, `{ k: v }`) also
/// forbid the flow indicators `,[]{}` inside plain scalars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ctx {
    Block,
    Flow,
}

/// `s` as a YAML scalar: plain when that reads back as `s`, else quoted.
pub(crate) fn scalar(s: &str, ctx: Ctx) -> Cow<'_, str> {
    if is_plain_safe(s, ctx) { Cow::Borrowed(s) } else { Cow::Owned(double_quoted(s)) }
}

fn is_plain_safe(s: &str, ctx: Ctx) -> bool {
    let Some(first) = s.chars().next() else {
        return false;
    };
    if s.trim() != s || looks_like_non_string(s) {
        return false;
    }
    // Indicators that change the meaning of a scalar's first character.
    if "-?:,[]{}#&*!|>'\"%@`+.".contains(first) || first.is_ascii_digit() {
        return false;
    }
    if s.chars().any(needs_escape) {
        return false;
    }
    match ctx {
        Ctx::Flow => !s.contains([',', '[', ']', '{', '}', ':', '#']),
        Ctx::Block => !(s.contains(": ") || s.ends_with(':') || s.contains(" #")),
    }
}

/// Words the YAML 1.2 core schema reads as null or booleans. Numbers are
/// excluded by the first-character rule.
fn looks_like_non_string(s: &str) -> bool {
    matches!(s, "~" | "null" | "Null" | "NULL" | "true" | "True" | "TRUE" | "false" | "False" | "FALSE")
}

/// Characters that cannot appear raw in a YAML scalar.
fn needs_escape(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{FEFF}' | '\u{FFFE}' | '\u{FFFF}')
}

fn double_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if needs_escape(c) => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_when_safe() {
        for s in ["draft", "amount > 0", "Order.submit", "not a gift card", "a-b_c", "café", "x == y"] {
            assert_eq!(scalar(s, Ctx::Flow), s, "{s:?}");
            assert_eq!(scalar(s, Ctx::Block), s, "{s:?}");
        }
    }

    #[test]
    fn flow_indicators_only_matter_in_flow_context() {
        let s = "new Shipment with a = 1, b = 2";
        assert_eq!(scalar(s, Ctx::Block), s);
        assert_eq!(scalar(s, Ctx::Flow), "\"new Shipment with a = 1, b = 2\"");
    }

    #[test]
    fn quotes_reserved_words_numbers_and_indicators() {
        for s in [
            "",
            "true",
            "False",
            "null",
            "~",
            "12",
            "3 retries",
            "-1",
            "+1",
            ".5",
            "- item",
            "#x",
            "*alias",
            "&a",
            "!tag",
            "|",
            ">",
            "'q'",
            "\"q\"",
            "%x",
            "@x",
            "`x`",
            "?",
            "a: b",
            "a:",
            "a #b",
            " lead",
            "trail ",
        ] {
            assert!(scalar(s, Ctx::Block).starts_with('"'), "{s:?} should be quoted");
        }
    }

    #[test]
    fn escapes_quotes_backslashes_and_control_characters() {
        // Mid-string quotes and backslashes are fine in plain scalars.
        assert_eq!(scalar("say \"hi\"\\now", Ctx::Flow), "say \"hi\"\\now");
        assert_eq!(scalar("\"hi\" \\ now", Ctx::Flow), r#""\"hi\" \\ now""#);
        assert_eq!(scalar("a\nb\tc", Ctx::Block), r#""a\nb\tc""#);
        assert_eq!(scalar("bell\u{7}", Ctx::Block), r#""bell\u0007""#);
    }
}
