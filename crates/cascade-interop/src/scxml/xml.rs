//! Writing XML by hand: escaping and an indenting element writer.

use std::fmt::Write as _;

/// Whether XML 1.0 allows `c` in a document at all (even as a character
/// reference).
fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && !matches!(c, '\u{FFFE}' | '\u{FFFF}'))
}

/// `text` escaped for a double-quoted attribute value. Whitespace other
/// than spaces is written as character references so attribute-value
/// normalization keeps it; characters XML cannot carry become U+FFFD.
pub(crate) fn attr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            c if is_xml_char(c) => out.push(c),
            _ => out.push('\u{FFFD}'),
        }
    }
    out
}

/// `text` safe inside `<!-- … -->`: no `--`, no trailing `-`.
pub(crate) fn comment(text: &str) -> String {
    let mut out: String = text.chars().map(|c| if is_xml_char(c) { c } else { '\u{FFFD}' }).collect();
    while out.contains("--") {
        out = out.replace("--", "- -");
    }
    if out.ends_with('-') {
        out.push(' ');
    }
    out
}

/// An indenting writer for elements with attributes and no text content.
pub(crate) struct XmlWriter {
    out: String,
    depth: usize,
}

impl XmlWriter {
    pub fn new() -> Self {
        Self { out: String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"), depth: 0 }
    }

    fn indent(&mut self) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
    }

    fn open_tag(&mut self, name: &str, attrs: &[(&str, &str)]) {
        self.indent();
        self.out.push('<');
        self.out.push_str(name);
        for (key, value) in attrs {
            let _ = write!(self.out, " {key}=\"{}\"", attr(value));
        }
    }

    /// `<name attrs>`, increasing the indentation.
    pub fn start(&mut self, name: &str, attrs: &[(&str, &str)]) {
        self.open_tag(name, attrs);
        self.out.push_str(">\n");
        self.depth += 1;
    }

    /// `</name>`, decreasing the indentation.
    pub fn end(&mut self, name: &str) {
        self.depth = self.depth.saturating_sub(1);
        self.indent();
        let _ = writeln!(self.out, "</{name}>");
    }

    /// `<name attrs/>`.
    pub fn empty(&mut self, name: &str, attrs: &[(&str, &str)]) {
        self.open_tag(name, attrs);
        self.out.push_str("/>\n");
    }

    /// `<!-- text -->`, one line per input line.
    pub fn comment(&mut self, text: &str) {
        for line in text.lines() {
            self.indent();
            let _ = writeln!(self.out, "<!-- {} -->", comment(line));
        }
    }

    pub fn finish(self) -> String {
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_attribute_values() {
        assert_eq!(attr(r#"a < b && c > "d""#), "a &lt; b &amp;&amp; c &gt; &quot;d&quot;");
        assert_eq!(attr("line\nbreak\ttab"), "line&#10;break&#9;tab");
        assert_eq!(attr("bell\u{7}"), "bell\u{FFFD}");
        assert_eq!(attr("it's ✓"), "it's ✓");
    }

    #[test]
    fn comments_never_contain_double_dashes() {
        assert_eq!(comment("a -- b --- c"), "a - - b - - - c");
        assert_eq!(comment("ends-"), "ends- ");
    }

    #[test]
    fn writer_indents_nested_elements() {
        let mut w = XmlWriter::new();
        w.start("a", &[("x", "1")]);
        w.empty("b", &[]);
        w.comment("note");
        w.end("a");
        assert_eq!(
            w.finish(),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<a x=\"1\">\n  <b/>\n  <!-- note -->\n</a>\n"
        );
    }
}
