//! Source locations in the definition file, used for diagnostics and
//! click-to-source.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A position in a source file. Both fields are 1-based; `Pos::default()`
/// (0:0) marks "no position", for elements that were not parsed from text
/// (imports, synthesized diff ghosts).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
}

impl Pos {
    pub const fn new(line: u32, col: u32) -> Self {
        Self { line, col }
    }

    pub const fn is_known(self) -> bool {
        self.line > 0
    }
}

/// A half-open range `[start, end)` in a source file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start: Pos,
    pub end: Pos,
}

impl SourceSpan {
    pub const fn new(start: Pos, end: Pos) -> Self {
        Self { start, end }
    }

    /// A span with no source location.
    pub const fn unknown() -> Self {
        Self { start: Pos::new(0, 0), end: Pos::new(0, 0) }
    }

    pub const fn is_known(self) -> bool {
        self.start.is_known()
    }

    /// The first line of the span, if known. This is the line click-to-source
    /// jumps to.
    pub fn line(self) -> Option<u32> {
        self.is_known().then_some(self.start.line)
    }
}

impl fmt::Display for SourceSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_known() { write!(f, "{}:{}", self.start.line, self.start.col) } else { f.write_str("?:?") }
    }
}

/// A value together with the span it was parsed from.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Spanned<T> {
    pub value: T,
    pub span: SourceSpan,
}

impl<T> Spanned<T> {
    pub const fn new(value: T, span: SourceSpan) -> Self {
        Self { value, span }
    }

    /// Wrap a value that has no source location.
    pub const fn synthetic(value: T) -> Self {
        Self { value, span: SourceSpan::unknown() }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned { value: f(self.value), span: self.span }
    }

    pub fn as_ref(&self) -> Spanned<&T> {
        Spanned { value: &self.value, span: self.span }
    }
}

impl Spanned<String> {
    pub fn as_str(&self) -> &str {
        &self.value
    }
}
