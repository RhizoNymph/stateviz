//! Label escaping with Mermaid entity codes.
//!
//! Mermaid replaces `#name;` / `#123;` sequences with HTML entities before
//! parsing, so every character that could end a label, start a comment
//! (`%%`), open a shape or be read as markup is written as an entity code.
//! `#` itself is always escaped, so user text can never form an entity by
//! accident.

/// Escape text for a quoted label (state descriptions, node and edge
/// labels) or front matter-free text. Newlines and tabs become spaces.
pub(super) fn label(text: &str) -> String {
    escape(text, false)
}

/// Escape the text after ` : ` in a state diagram transition, which ends at
/// a `:` or `;`, so colons are escaped too.
pub(super) fn transition_label(text: &str) -> String {
    escape(text, true)
}

fn escape(text: &str, colon: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '#' => out.push_str("#35;"),
            ';' => out.push_str("#59;"),
            '"' => out.push_str("#quot;"),
            '<' => out.push_str("#60;"),
            '>' => out.push_str("#62;"),
            '{' => out.push_str("#123;"),
            '}' => out.push_str("#125;"),
            '%' => out.push_str("#37;"),
            '|' => out.push_str("#124;"),
            '`' => out.push_str("#96;"),
            '&' => out.push_str("#38;"),
            ':' if colon => out.push_str("#58;"),
            '\n' | '\r' | '\t' => out.push(' '),
            other => out.push(other),
        }
    }
    out
}

/// A YAML double-quoted scalar for the front matter `title:`.
pub(super) fn yaml_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' | '\r' | '\t' => out.push(' '),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_every_special_character() {
        assert_eq!(label(r#"a > 0; "b" # c %% {d} <e> |f| `g` & h"#), {
            "a #62; 0#59; #quot;b#quot; #35; c #37;#37; #123;d#125; #60;e#62; #124;f#124; #96;g#96; #38; h"
        });
        assert_eq!(label("x: y"), "x: y");
        assert_eq!(transition_label("x: y"), "x#58; y");
        assert_eq!(label("two\nlines\tand tab"), "two lines and tab");
        assert_eq!(label("amount → paid [ok]"), "amount → paid [ok]");
    }

    #[test]
    fn yaml_strings_are_double_quoted() {
        assert_eq!(yaml_string(r#"Shop: "v2" \ x"#), r#""Shop: \"v2\" \\ x""#);
        assert_eq!(yaml_string("a\nb"), r#""a b""#);
    }
}
