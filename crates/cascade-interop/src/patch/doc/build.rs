//! saphyr marked nodes → [`Node`] with byte offsets and recomputed ends.

use saphyr::{LoadableYamlNode, MarkedYamlOwned, ScalarOwned, YamlDataOwned};

use super::{Entry, Item, Kind, Map, Node, Scalar, ScalarStyle, Seq};
use crate::patch::error::{Result, Unsupported};

pub(super) fn build(text: &str) -> Result<Node> {
    let documents = MarkedYamlOwned::load_from_str(text).map_err(|_| Unsupported::Unparseable)?;
    let [root] = documents.as_slice() else {
        return Err(Unsupported::NotOneDocument.into());
    };
    Builder::new(text).node(root, false)
}

/// Length of a quoted scalar starting at the beginning of `text`, up to and
/// including its closing quote.
fn closing_quote(text: &str, quote: char) -> Option<usize> {
    let mut chars = text.char_indices().skip(1).peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' if quote == '"' => {
                chars.next();
            }
            c if c == quote => {
                if quote == '\'' && chars.peek().is_some_and(|&(_, n)| n == '\'') {
                    chars.next();
                } else {
                    return Some(i + c.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

struct Builder<'t> {
    text: &'t str,
    /// Byte offset of every char index (saphyr markers count chars), plus
    /// the end of the text.
    byte_of: Vec<usize>,
}

impl<'t> Builder<'t> {
    fn new(text: &'t str) -> Self {
        let mut byte_of: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        byte_of.push(text.len());
        Self { text, byte_of }
    }

    fn byte(&self, char_index: usize) -> Result<usize> {
        self.byte_of.get(char_index).copied().ok_or_else(|| Unsupported::Shape.into())
    }

    fn node(&self, node: &MarkedYamlOwned, in_flow: bool) -> Result<Node> {
        let start = self.byte(node.span.start.index())?;
        let end = self.byte(node.span.end.index())?;
        if self.text[start..].starts_with(['&', '!', '*']) {
            return Err(Unsupported::TagsOrAnchors.into());
        }
        match &node.data {
            YamlDataOwned::Mapping(mapping) => {
                let flow = self.text[start..].starts_with('{');
                let mut entries = Vec::with_capacity(mapping.len());
                for (key, value) in mapping {
                    let key = self.node(key, in_flow || flow)?;
                    if !matches!(key.kind, Kind::Scalar(_)) {
                        return Err(Unsupported::ComplexKey.into());
                    }
                    let mut value = self.node(value, in_flow || flow)?;
                    if value.is_null() && value.start == value.end {
                        // An implicit null sits right after the key's colon.
                        let at = self.after_colon(key.end).unwrap_or(key.end);
                        value.start = at;
                        value.end = at;
                    }
                    entries.push(Entry { key, value });
                }
                let last = entries.last().map(|e| e.value.end.max(e.key.end));
                let end = self.collection_end(start, flow, last, '}')?;
                Ok(Node { start, end, kind: Kind::Map(Map { flow, entries }) })
            }
            YamlDataOwned::Sequence(sequence) => {
                let flow = self.text[start..].starts_with('[');
                let mut items = Vec::with_capacity(sequence.len());
                for item in sequence {
                    let node = self.node(item, in_flow || flow)?;
                    let head = if flow { node.start } else { self.dash_before(node.start)? };
                    items.push(Item { head, node });
                }
                let last = items.last().map(|i| i.node.end.max(i.head + 1));
                let end = self.collection_end(start, flow, last, ']')?;
                Ok(Node { start, end, kind: Kind::Seq(Seq { flow, items }) })
            }
            YamlDataOwned::Value(ScalarOwned::Null) => Ok(Node { start, end, kind: Kind::Null }),
            YamlDataOwned::Value(ScalarOwned::String(s)) if s.is_empty() && start == end => {
                Ok(Node { start, end, kind: Kind::Null })
            }
            YamlDataOwned::Value(value) => {
                let value = match value {
                    ScalarOwned::String(s) => s.clone(),
                    ScalarOwned::Integer(i) => i.to_string(),
                    ScalarOwned::FloatingPoint(f) => f.to_string(),
                    ScalarOwned::Boolean(b) => b.to_string(),
                    ScalarOwned::Null => String::new(),
                };
                Ok(self.scalar(start, end, value, in_flow))
            }
            YamlDataOwned::Representation(value, _, _) => Ok(self.scalar(start, end, value.clone(), in_flow)),
            YamlDataOwned::Tagged(_, _) | YamlDataOwned::Alias(_) => Err(Unsupported::TagsOrAnchors.into()),
            YamlDataOwned::BadValue => Err(Unsupported::Shape.into()),
        }
    }

    fn scalar(&self, start: usize, end: usize, value: String, in_flow: bool) -> Node {
        // saphyr ends quoted scalars after any trailing blanks and comment;
        // end them at their closing quote instead.
        let end = match self.text[start..].chars().next() {
            Some(quote @ ('"' | '\'')) => closing_quote(&self.text[start..], quote).map_or(end, |len| start + len),
            _ => end,
        };
        let source = &self.text[start..end];
        let style = if source.starts_with('"') {
            ScalarStyle::DoubleQuoted
        } else if source.starts_with('\'') {
            ScalarStyle::SingleQuoted
        } else if source == value && !source.contains('\n') {
            ScalarStyle::Plain
        } else {
            ScalarStyle::Complex
        };
        Node { start, end, kind: Kind::Scalar(Scalar { value, style, in_flow }) }
    }

    /// Offset just past the `:` after a key ending at `key_end`.
    fn after_colon(&self, key_end: usize) -> Option<usize> {
        let rest = self.text.get(key_end..)?;
        let skipped = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        rest[skipped..].starts_with(':').then_some(key_end + skipped + 1)
    }

    /// The `-` of a block sequence item starting at `item_start`.
    fn dash_before(&self, item_start: usize) -> Result<usize> {
        let before = self.text.get(..item_start).ok_or(Unsupported::Shape)?;
        let trimmed = before.trim_end_matches([' ', '\t', '\n', '\r']);
        match trimmed.ends_with('-') {
            true => Ok(trimmed.len() - 1),
            false => Err(Unsupported::Shape.into()),
        }
    }

    /// A flow collection ends after its closer; a block collection where its
    /// last child ends.
    fn collection_end(&self, start: usize, flow: bool, last_child_end: Option<usize>, closer: char) -> Result<usize> {
        if !flow {
            return last_child_end.ok_or_else(|| Unsupported::Shape.into());
        }
        let from = last_child_end.unwrap_or(start + 1).max(start + 1);
        let mut chars = self.text.get(from..).ok_or(Unsupported::FlowScan)?.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                ' ' | '\t' | '\n' | '\r' | ',' => {}
                '#' => {
                    for (_, c) in chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                }
                c if c == closer => return Ok(from + i + 1),
                _ => return Err(Unsupported::FlowScan.into()),
            }
        }
        Err(Unsupported::FlowScan.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(node: &Node) -> &Map {
        node.as_map().expect("a mapping")
    }

    #[test]
    fn scalars_keep_exact_ranges_and_styles() {
        let text = "a: plain\nb: \"dq\"\nc: 'it''s'\nd: é…x\n";
        let root = build(text).expect("builds");
        let entries = &map(&root).entries;
        let spans: Vec<&str> = entries.iter().map(|e| &text[e.value.start..e.value.end]).collect();
        assert_eq!(spans, ["plain", "\"dq\"", "'it''s'", "é…x"]);
        let styles: Vec<ScalarStyle> = entries.iter().filter_map(|e| e.value.as_scalar()).map(|s| s.style).collect();
        assert_eq!(
            styles,
            [ScalarStyle::Plain, ScalarStyle::DoubleQuoted, ScalarStyle::SingleQuoted, ScalarStyle::Plain]
        );
    }

    #[test]
    fn quoted_scalars_end_at_their_closing_quote() {
        let text = "a: { k: \"v \\\" w\" }\nb: 'it''s'   # c\nc: plain   # c\nd: [\"x\" ]\n";
        let root = build(text).expect("builds");
        let entries = &map(&root).entries;
        let k = &map(&entries[0].value).entries[0].value;
        assert_eq!(&text[k.start..k.end], "\"v \\\" w\"");
        let ranges: Vec<&str> = entries[1..3].iter().map(|e| &text[e.value.start..e.value.end]).collect();
        assert_eq!(ranges, ["'it''s'", "plain"]);
        let x = &entries[3].value.as_seq().expect("seq").items[0].node;
        assert_eq!(&text[x.start..x.end], "\"x\"");
    }

    #[test]
    fn flow_collections_end_after_their_closer() {
        let text = "a: [x, \"y\"]  # c\nb: { k: v, }\nc: []\nd: {}\n";
        let root = build(text).expect("builds");
        let ranges: Vec<&str> = map(&root).entries.iter().map(|e| &text[e.value.start..e.value.end]).collect();
        assert_eq!(ranges, ["[x, \"y\"]", "{ k: v, }", "[]", "{}"]);
    }

    #[test]
    fn block_collections_end_at_their_last_content() {
        let text = "a:\n  - x\n  - y   # trailing\n\n  # after\nb:\n";
        let root = build(text).expect("builds");
        let a = &map(&root).entries[0].value;
        assert_eq!(&text[a.start..a.end], "- x\n  - y");
        let seq = a.as_seq().expect("seq");
        assert_eq!(seq.items[1].head, text.find("- y").expect("dash"));
        let b = &map(&root).entries[1].value;
        assert!(b.is_null());
        assert_eq!(b.start, text.len() - 1);
    }

    #[test]
    fn rejects_anchors_and_tags() {
        assert!(build("a: &x 1\nb: *x\n").is_err());
        assert!(build("a: !tag 1\n").is_err());
    }
}
