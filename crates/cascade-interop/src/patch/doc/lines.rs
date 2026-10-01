//! Line table over the source text: offsets, indentation, blank and comment
//! lines.

/// Byte offsets of line starts. Lines are numbered from 0 here.
#[derive(Clone, Debug)]
pub(crate) struct Lines {
    starts: Vec<usize>,
    len: usize,
}

impl Lines {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1).filter(|&s| s < text.len()));
        Self { starts, len: text.len() }
    }

    pub fn count(&self) -> usize {
        self.starts.len()
    }

    /// The line containing byte `offset` (the last line for the end offset).
    pub fn line_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|&s| s <= offset).saturating_sub(1)
    }

    pub fn start(&self, line: usize) -> usize {
        self.starts.get(line).copied().unwrap_or(self.len)
    }

    /// Start of the next line, or the end of the text.
    pub fn next_start(&self, line: usize) -> usize {
        self.starts.get(line + 1).copied().unwrap_or(self.len)
    }

    /// The text of a line without its line break.
    pub fn text<'t>(&self, text: &'t str, line: usize) -> &'t str {
        let start = self.start(line).min(text.len());
        let end = self.next_start(line).min(text.len());
        text[start..end].trim_end_matches(['\n', '\r'])
    }

    pub fn is_blank(&self, text: &str, line: usize) -> bool {
        self.text(text, line).trim().is_empty()
    }

    pub fn is_comment(&self, text: &str, line: usize) -> bool {
        self.text(text, line).trim_start().starts_with('#')
    }

    /// Leading spaces of a line.
    pub fn indent(&self, text: &str, line: usize) -> usize {
        let line = self.text(text, line);
        line.len() - line.trim_start_matches(' ').len()
    }

    /// Blank lines directly above `line`.
    pub fn blanks_before(&self, text: &str, line: usize) -> usize {
        (0..line).rev().take_while(|&l| self.is_blank(text, l)).count()
    }

    /// Blank lines directly below `line`.
    pub fn blanks_after(&self, text: &str, line: usize) -> usize {
        (line + 1..self.count()).take_while(|&l| self.is_blank(text, l)).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_lines() {
        let text = "a: 1\n\n  # c\nb: 2";
        let lines = Lines::new(text);
        assert_eq!(lines.count(), 4);
        assert_eq!(lines.line_of(0), 0);
        assert_eq!(lines.line_of(5), 1);
        assert_eq!(lines.line_of(text.len()), 3);
        assert_eq!(lines.text(text, 2), "  # c");
        assert!(lines.is_blank(text, 1));
        assert!(lines.is_comment(text, 2));
        assert_eq!(lines.indent(text, 2), 2);
        assert_eq!(lines.blanks_before(text, 2), 1);
        assert_eq!(lines.next_start(3), text.len());
    }

    #[test]
    fn trailing_newline_adds_no_line() {
        let lines = Lines::new("a\nb\n");
        assert_eq!(lines.count(), 2);
    }
}
