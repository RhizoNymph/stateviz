//! Whole-line edits of block collection entries.
//!
//! A block entry owns:
//! - the comment lines directly above it at its own column (no blank line
//!   in between; the file's header comment block is never owned),
//! - its own lines up to where its content ends, and
//! - comment lines below that are indented deeper than the entry (they are
//!   inside its body), up to the next line at its column or shallower.
//!
//! Removing an entry deletes exactly those lines plus one side of the blank
//! lines that separate it from its siblings; inserting an entry mirrors
//! that, so an insertion followed by a removal restores the text.

use super::doc::{Doc, Span};
use super::error::{Result, Unsupported};
use super::splice::Splice;

/// The lines an entry owns, as a byte range of whole lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Region {
    pub start: usize,
    pub end: usize,
    pub first_line: usize,
    pub last_line: usize,
}

/// The region of a block entry whose head (key or `-`) starts its line.
pub(crate) fn region(doc: &Doc, span: Span) -> Result<Region> {
    if !doc.starts_line(span.head) {
        return Err(Unsupported::SharedLine.into());
    }
    let (text, lines) = (doc.text(), doc.lines());
    let col = doc.col(span.head);
    let head_line = lines.line_of(span.head);
    let mut first = head_line;
    while first > doc.header_end()
        && first > 0
        && lines.is_comment(text, first - 1)
        && lines.indent(text, first - 1) == col
    {
        first -= 1;
    }
    let last = last_line(doc, span, col);
    Ok(Region { start: lines.start(first), end: lines.next_start(last), first_line: first, last_line: last })
}

/// The last line an entry owns: its content, then deeper comment lines.
pub(crate) fn last_line(doc: &Doc, span: Span, col: usize) -> usize {
    let (text, lines) = (doc.text(), doc.lines());
    let mut last = lines.line_of(span.end.max(span.head + 1) - 1);
    let mut next = last + 1;
    while next < lines.count() {
        if lines.is_blank(text, next) {
            next += 1;
        } else if lines.is_comment(text, next) && lines.indent(text, next) > col {
            last = next;
            next += 1;
        } else {
            break;
        }
    }
    last
}

/// Offset just past the last line an entry owns (for inserting after it).
pub(crate) fn end_of(doc: &Doc, span: Span) -> usize {
    let col = doc.col(span.head);
    doc.lines().next_start(last_line(doc, span, col))
}

/// Whether siblings are separated by blank lines, judged from the first
/// two; `default` when there are fewer.
pub(crate) fn separated(doc: &Doc, siblings: &[Span], default: bool) -> Result<bool> {
    match siblings.get(1) {
        Some(&second) => {
            let r = region(doc, second)?;
            Ok(doc.lines().blanks_before(doc.text(), r.first_line) > 0)
        }
        None => Ok(default),
    }
}

/// Delete sibling `index` with its comments and one side of its blank-line
/// separation. The caller handles removing the only entry.
pub(crate) fn remove(doc: &Doc, siblings: &[Span], index: usize) -> Result<Splice> {
    let &span = siblings.get(index).ok_or(Unsupported::Shape)?;
    let r = region(doc, span)?;
    let (text, lines) = (doc.text(), doc.lines());
    let before = lines.blanks_before(text, r.first_line);
    let after = lines.blanks_after(text, r.last_line);
    let has_prev = index > 0;
    let has_next = index + 1 < siblings.len();
    let (mut start, mut end) = (r.start, r.end);
    if has_prev && before > 0 && (!has_next || after > 0) {
        start = lines.start(r.first_line - before);
    } else if !has_prev && has_next && after > 0 {
        end = lines.next_start(r.last_line + after);
    }
    Ok(Splice::delete(start, end))
}

/// Insert `block` (whole lines, each ending in a line break, already
/// indented) as sibling `index`.
pub(crate) fn insert(doc: &Doc, siblings: &[Span], index: usize, block: &str, separate: bool) -> Result<Splice> {
    let sep = if separate { "\n" } else { "" };
    if index >= siblings.len() {
        let &last = siblings.last().ok_or(Unsupported::Shape)?;
        let at = end_of(doc, last);
        let lead = if at == doc.text().len() && !doc.text().ends_with('\n') { "\n" } else { "" };
        return Ok(Splice::insert(at, format!("{lead}{sep}{block}")));
    }
    let r = region(doc, siblings[index])?;
    Ok(Splice::insert(r.start, format!("{block}{sep}")))
}

/// Insert whole lines right after the line containing `offset`.
pub(crate) fn insert_after_line(doc: &Doc, offset: usize, block: &str) -> Splice {
    let lines = doc.lines();
    let at = lines.next_start(lines.line_of(offset));
    let lead = if at == doc.text().len() && !doc.text().ends_with('\n') { "\n" } else { "" };
    Splice::insert(at, format!("{lead}{block}"))
}
