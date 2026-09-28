//! A span index over definition text.
//!
//! [`Doc`] is the YAML node tree of the file with exact byte ranges, built
//! from saphyr's marked nodes: scalars keep their quoting style, collections
//! their flow/block style, block sequence items the offset of their `-`.
//! saphyr's collection end markers are unreliable (block collections end
//! at the next token, flow collections before their closer), so ends are
//! recomputed here: a flow collection ends after its closing bracket, a block
//! collection where its last descendant ends. Everything the patcher edits
//! is located through this index; the line table answers questions about
//! comments, blank lines and indentation.

mod build;
mod lines;

pub(crate) use lines::Lines;

use super::error::{Result, Unsupported};

/// A parsed definition file.
#[derive(Debug)]
pub(crate) struct Doc {
    text: String,
    lines: Lines,
    root: Node,
    /// First line that is not part of the file's leading comment block.
    /// Header comments are never attached to an entry.
    header_end: usize,
}

impl Doc {
    pub fn parse(text: &str) -> Result<Self> {
        let root = build::build(text)?;
        let lines = Lines::new(text);
        let header_end =
            (0..lines.count()).find(|&l| !lines.is_blank(text, l) && !lines.is_comment(text, l)).unwrap_or(0);
        Ok(Self { text: text.to_owned(), lines, root, header_end })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn lines(&self) -> &Lines {
        &self.lines
    }

    pub fn root(&self) -> &Node {
        &self.root
    }

    pub fn header_end(&self) -> usize {
        self.header_end
    }

    /// Column of a byte offset, in characters from the start of its line.
    pub fn col(&self, offset: usize) -> usize {
        let start = self.lines.start(self.lines.line_of(offset));
        self.text.get(start..offset).map_or(0, |s| s.chars().count())
    }

    /// Whether only spaces precede `offset` on its line.
    pub fn starts_line(&self, offset: usize) -> bool {
        let start = self.lines.start(self.lines.line_of(offset));
        self.text.get(start..offset).is_some_and(|s| s.chars().all(|c| c == ' '))
    }

    /// The offset just past the `:` that follows a mapping key.
    pub fn after_colon(&self, key: &Node) -> Result<usize> {
        let rest = self.text.get(key.end..).ok_or(Unsupported::Shape)?;
        let skipped = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        match rest[skipped..].starts_with(':') {
            true => Ok(key.end + skipped + 1),
            false => Err(Unsupported::Shape.into()),
        }
    }
}

/// A YAML node with its byte range in the source.
#[derive(Debug)]
pub(crate) struct Node {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

#[derive(Debug)]
pub(crate) enum Kind {
    Scalar(Scalar),
    /// `~`, `null`, or nothing at all (then `start == end`, just past the
    /// `:` or `-` that introduced it).
    Null,
    Seq(Seq),
    Map(Map),
}

#[derive(Debug)]
pub(crate) struct Scalar {
    pub value: String,
    pub style: ScalarStyle,
    /// Written inside a flow collection, where `,[]{}` need quoting.
    pub in_flow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScalarStyle {
    /// A one-line plain scalar whose source text is its value.
    Plain,
    SingleQuoted,
    DoubleQuoted,
    /// Block scalars and multi-line plain scalars; never edited in place.
    Complex,
}

#[derive(Debug)]
pub(crate) struct Seq {
    pub flow: bool,
    pub items: Vec<Item>,
}

#[derive(Debug)]
pub(crate) struct Item {
    /// Where the item's entry starts: its `-` in a block sequence, the node
    /// itself in a flow sequence.
    pub head: usize,
    pub node: Node,
}

#[derive(Debug)]
pub(crate) struct Map {
    pub flow: bool,
    pub entries: Vec<Entry>,
}

#[derive(Debug)]
pub(crate) struct Entry {
    pub key: Node,
    pub value: Node,
}

/// Where a child entry of a collection starts and where its content ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub head: usize,
    pub end: usize,
}

impl Node {
    pub fn as_map(&self) -> Option<&Map> {
        match &self.kind {
            Kind::Map(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_seq(&self) -> Option<&Seq> {
        match &self.kind {
            Kind::Seq(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_scalar(&self) -> Option<&Scalar> {
        match &self.kind {
            Kind::Scalar(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self.kind, Kind::Null)
    }

    /// The scalar's value, if this is a scalar.
    pub fn text(&self) -> Option<&str> {
        self.as_scalar().map(|s| s.value.as_str())
    }

    /// Whether this is a flow collection (`[…]` or `{…}`).
    pub fn is_flow(&self) -> bool {
        match &self.kind {
            Kind::Seq(s) => s.flow,
            Kind::Map(m) => m.flow,
            Kind::Scalar(_) | Kind::Null => false,
        }
    }

    /// The item nodes of a sequence (empty for anything else).
    pub fn items(&self) -> Vec<&Node> {
        match &self.kind {
            Kind::Seq(s) => s.items.iter().map(|i| &i.node).collect(),
            Kind::Scalar(_) | Kind::Null | Kind::Map(_) => Vec::new(),
        }
    }

    /// Scalar items of a sequence, or the scalar itself: the shapes of
    /// "a name or a list of names".
    pub fn scalar_items(&self) -> Vec<&Node> {
        match &self.kind {
            Kind::Seq(s) => s.items.iter().map(|i| &i.node).collect(),
            Kind::Scalar(_) => vec![self],
            Kind::Null | Kind::Map(_) => Vec::new(),
        }
    }
}

impl Map {
    /// The entry with this key, and its position.
    pub fn get(&self, key: &str) -> Option<(usize, &Entry)> {
        self.entries.iter().enumerate().find(|(_, e)| e.key.text() == Some(key))
    }

    pub fn value(&self, key: &str) -> Option<&Node> {
        self.get(key).map(|(_, e)| &e.value)
    }

    pub fn spans(&self) -> Vec<Span> {
        self.entries.iter().map(|e| Span { head: e.key.start, end: e.value.end.max(e.key.end) }).collect()
    }
}

impl Seq {
    pub fn spans(&self) -> Vec<Span> {
        self.items.iter().map(|i| Span { head: i.head, end: i.node.end.max(i.head + 1) }).collect()
    }
}
