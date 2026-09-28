//! An indenting line writer for P source.

/// Four-space indented P source, built line by line.
#[derive(Debug, Default)]
pub(super) struct Writer {
    out: String,
    depth: usize,
}

impl Writer {
    pub(super) fn line(&mut self, text: &str) {
        for _ in 0..self.depth {
            self.out.push_str("    ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// A single-line `//` comment; newlines in `text` become spaces.
    pub(super) fn comment(&mut self, text: &str) {
        self.line(&format!("// {}", one_line(text)));
    }

    pub(super) fn blank(&mut self) {
        self.out.push('\n');
    }

    /// A line ending in `{`; the lines after it are indented one level more.
    pub(super) fn open(&mut self, text: &str) {
        self.line(text);
        self.depth += 1;
    }

    /// Close the innermost block with `}`.
    pub(super) fn close(&mut self) {
        self.depth = self.depth.saturating_sub(1);
        self.line("}");
    }

    /// Close the innermost block and open the next on one line, as in
    /// `} else {`.
    pub(super) fn reopen(&mut self, text: &str) {
        self.depth = self.depth.saturating_sub(1);
        self.open(text);
    }

    pub(super) fn finish(self) -> String {
        self.out
    }
}

/// Free text (guards, conditions, selectors) on one line.
pub(super) fn one_line(text: &str) -> String {
    text.chars().map(|c| if matches!(c, '\n' | '\r' | '\t') { ' ' } else { c }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indents_blocks() {
        let mut w = Writer::default();
        w.open("machine M {");
        w.open("if ($) {");
        w.line("goto a;");
        w.reopen("} else {");
        w.comment("two\nlines");
        w.close();
        w.close();
        assert_eq!(
            w.finish(),
            "machine M {\n    if ($) {\n        goto a;\n    } else {\n        // two lines\n    }\n}\n"
        );
    }
}
