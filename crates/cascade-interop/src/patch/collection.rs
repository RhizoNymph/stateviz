//! Generic edits on collections: insert and remove children in either
//! style, set or remove a mapping key, reconcile a list of names, convert a
//! flow collection to block form.

use super::block;
use super::doc::{Doc, Entry, Kind, Node, Span};
use super::error::{Result, Unsupported};
use super::render::{Block, Style, flow_list, free_text, plain_or_quoted, restyle};
use super::splice::Splice;

/// A value to write under a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    /// On the key's line: `key: text`.
    Inline(String),
    /// Below the key, with indentation relative to the key's column.
    Lines(Block),
}

/// Where each child of a collection starts and ends.
pub(crate) fn spans(node: &Node) -> Vec<Span> {
    match &node.kind {
        Kind::Seq(s) => s.spans(),
        Kind::Map(m) => m.spans(),
        Kind::Scalar(_) | Kind::Null => Vec::new(),
    }
}

fn check_no_comment(doc: &Doc, start: usize, end: usize) -> Result<()> {
    match doc.text().get(start..end) {
        Some(removed) if !removed.contains('#') => Ok(()),
        Some(_) => Err(Unsupported::CommentInFlow.into()),
        None => Err(Unsupported::Shape.into()),
    }
}

/// Replace a scalar's text, keeping its quoting style. Inside a padded
/// flow row (`to: paid,      on: …`) the padding after the comma shrinks or
/// grows so the next key keeps its column when it can.
pub(crate) fn replace_scalar(doc: &Doc, node: &Node, value: &str) -> Result<Splice> {
    let scalar = node.as_scalar().ok_or(Unsupported::Shape)?;
    let text = restyle(scalar, value)?;
    let rest = doc.text().get(node.end..).unwrap_or_default();
    if scalar.in_flow
        && let Some(after_comma) = rest.strip_prefix(',')
    {
        let pad = after_comma.len() - after_comma.trim_start_matches(' ').len();
        let next = after_comma[pad..].chars().next();
        let next_at = node.end + 1 + pad;
        let aligned = pad > 1 || column_shared_with_neighbour_line(doc, next_at);
        if aligned && next.is_some_and(|c| !matches!(c, '\n' | '\r' | '#')) {
            let old_len = doc.text().get(node.start..node.end).map_or(0, |t| t.chars().count());
            let new_len = text.chars().count();
            let pad = (pad + old_len).saturating_sub(new_len).max(1);
            let end = node.end + 1 + (after_comma.len() - after_comma.trim_start_matches(' ').len());
            return Ok(Splice::replace(node.start, end, format!("{text},{}", " ".repeat(pad))));
        }
    }
    Ok(Splice::replace(node.start, node.end, text))
}

/// Whether the word starting at `offset` (a key such as `on:`) starts at
/// the same column in the line above or below, padded there: the rows are
/// aligned.
fn column_shared_with_neighbour_line(doc: &Doc, offset: usize) -> bool {
    let (text, lines) = (doc.text(), doc.lines());
    let word: String = text.get(offset..).unwrap_or_default().chars().take_while(|c| !c.is_whitespace()).collect();
    if word.is_empty() {
        return false;
    }
    let line = lines.line_of(offset);
    let col = doc.col(offset);
    [line.checked_sub(1), Some(line + 1)].into_iter().flatten().filter(|&l| l < lines.count()).any(|l| {
        let neighbour: Vec<char> = lines.text(text, l).chars().collect();
        col > 1
            && neighbour.get(col - 1) == Some(&' ')
            && neighbour.get(col - 2) == Some(&' ')
            && neighbour.get(col..).is_some_and(|rest| rest.iter().take(word.chars().count()).copied().eq(word.chars()))
    })
}

// --- Flow collections -------------------------------------------------------------

/// Insert `text` as child `index` of a flow collection.
pub(crate) fn flow_insert(coll: &Node, index: usize, text: &str) -> Result<Splice> {
    let spans = spans(coll);
    match (spans.last(), spans.get(index)) {
        (None, _) => {
            let (open, close) = if coll.as_seq().is_some() { ("[", "]") } else { ("{ ", " }") };
            Ok(Splice::replace(coll.start, coll.end, format!("{open}{text}{close}")))
        }
        (Some(_), Some(at)) => Ok(Splice::insert(at.head, format!("{text}, "))),
        (Some(last), None) => Ok(Splice::insert(last.end, format!(", {text}"))),
    }
}

/// Remove child `index` of a flow collection with one adjacent comma.
pub(crate) fn flow_remove(doc: &Doc, coll: &Node, index: usize) -> Result<Splice> {
    let spans = spans(coll);
    let (start, end) = match (index.checked_sub(1), spans.get(index), spans.get(index + 1)) {
        (_, None, _) => return Err(Unsupported::Shape.into()),
        (None, Some(_), None) => {
            let empty = if coll.as_seq().is_some() { "[]" } else { "{}" };
            check_no_comment(doc, coll.start, coll.end)?;
            return Ok(Splice::replace(coll.start, coll.end, empty));
        }
        (_, Some(this), Some(next)) => (this.head, next.head),
        (Some(prev), Some(this), None) => (spans[prev].end, this.end),
    };
    check_no_comment(doc, start, end)?;
    Ok(Splice::delete(start, end))
}

// --- Either style -----------------------------------------------------------------------

/// Remove child `index` of a collection. Removing the only child of a block
/// collection is the caller's business (it has no empty block form).
pub(crate) fn remove_child(doc: &Doc, coll: &Node, index: usize) -> Result<Splice> {
    if coll.is_flow() {
        return flow_remove(doc, coll, index);
    }
    let spans = spans(coll);
    if spans.len() < 2 {
        return Err(Unsupported::EmptyBlockMapping.into());
    }
    let span = spans.get(index).ok_or(Unsupported::Shape)?;
    if !doc.starts_line(span.head) {
        // The first key of a mapping that is a sequence item: `- a: 1`.
        let next = spans.get(index + 1).ok_or(Unsupported::SharedLine)?;
        return Ok(Splice::delete(span.head, next.head));
    }
    block::remove(doc, &spans, index)
}

/// The column of a block collection's children.
pub(crate) fn child_col(doc: &Doc, coll: &Node) -> Result<usize> {
    spans(coll).first().map(|s| doc.col(s.head)).ok_or_else(|| Unsupported::Shape.into())
}

// --- Mapping keys -------------------------------------------------------------------------

/// Set `key` in a mapping to `value`, or remove it (`None`). A new key goes
/// before the first present key that follows it in `order`, else after the
/// last one that precedes it.
pub(crate) fn set_key(
    doc: &Doc,
    map_node: &Node,
    key: &str,
    value: Option<Value>,
    order: &[&str],
) -> Result<Vec<Splice>> {
    let map = map_node.as_map().ok_or(Unsupported::Shape)?;
    match (map.get(key), value) {
        (Some((_, entry)), Some(value)) => replace_value(doc, entry, value),
        (Some((index, _)), None) => Ok(vec![remove_child(doc, map_node, index)?]),
        (None, None) => Ok(Vec::new()),
        (None, Some(value)) => insert_key(doc, map_node, key, value, order).map(|s| vec![s]),
    }
}

/// Set a scalar key, keeping the old value's quoting style, or remove it.
pub(crate) fn set_scalar_key(
    doc: &Doc,
    map_node: &Node,
    key: &str,
    value: Option<&str>,
    order: &[&str],
) -> Result<Vec<Splice>> {
    let map = map_node.as_map().ok_or(Unsupported::Shape)?;
    match (map.get(key), value) {
        (Some((_, entry)), Some(value)) if entry.value.as_scalar().is_some() => {
            Ok(vec![replace_scalar(doc, &entry.value, value)?])
        }
        (_, value) => set_key(doc, map_node, key, value.map(|v| Value::Inline(plain_or_quoted(v, map.flow))), order),
    }
}

/// Like [`set_scalar_key`] for free text: a new value follows the file's
/// quoting habit for free text.
pub(crate) fn set_text_key(
    doc: &Doc,
    map_node: &Node,
    key: &str,
    value: Option<&str>,
    order: &[&str],
) -> Result<Vec<Splice>> {
    let map = map_node.as_map().ok_or(Unsupported::Shape)?;
    match (map.get(key), value) {
        (None, Some(value)) => {
            let quote = Style::detect(doc).quote_text;
            set_key(doc, map_node, key, Some(Value::Inline(free_text(value, map.flow, quote))), order)
        }
        (_, value) => set_scalar_key(doc, map_node, key, value, order),
    }
}

fn insert_key(doc: &Doc, map_node: &Node, key: &str, value: Value, order: &[&str]) -> Result<Splice> {
    let map = map_node.as_map().ok_or(Unsupported::Shape)?;
    let rank = |k: &str| order.iter().position(|o| *o == k);
    let own = rank(key).unwrap_or(order.len());
    let ranked: Vec<(usize, usize)> =
        map.entries.iter().enumerate().filter_map(|(i, e)| e.key.text().and_then(rank).map(|r| (i, r))).collect();
    // Before the first key that follows it in canonical order, else after
    // the last one that precedes it: this keeps an author's own order.
    let following = ranked.iter().filter(|(_, r)| *r > own).map(|(i, _)| *i).min();
    let preceding = ranked.iter().filter(|(_, r)| *r < own).map(|(i, _)| *i).max();
    let spans = map.spans();
    if map.flow {
        let Value::Inline(text) = value else {
            return Err(Unsupported::Shape.into());
        };
        let index = following.or(preceding.map(|i| i + 1)).unwrap_or(0);
        return flow_insert(map_node, index, &format!("{key}: {text}"));
    }
    let &first = spans.first().ok_or(Unsupported::Shape)?;
    let col = doc.col(first.head);
    let lines = match value {
        Value::Inline(text) => format!("{}{key}: {text}\n", " ".repeat(col)),
        Value::Lines(block) => format!("{}{key}:\n{}", " ".repeat(col), block.at(col)),
    };
    let before = following.map(|i| spans[i]).filter(|s| doc.starts_line(s.head));
    match (before, preceding) {
        (Some(span), _) => Ok(Splice::insert(block::region(doc, span)?.start, lines)),
        (None, Some(i)) => Ok(Splice::insert(block::end_of(doc, spans[i]), lines)),
        (None, None) if doc.starts_line(first.head) => Ok(Splice::insert(block::region(doc, first)?.start, lines)),
        (None, None) => Ok(Splice::insert(block::end_of(doc, first), lines)),
    }
}

/// Replace an entry's value.
pub(crate) fn replace_value(doc: &Doc, entry: &Entry, value: Value) -> Result<Vec<Splice>> {
    let old = &entry.value;
    let implicit = old.is_null() && old.start == old.end;
    let block_value = !implicit && matches!(old.kind, Kind::Seq(_) | Kind::Map(_)) && !old.is_flow();
    match value {
        Value::Inline(text) if implicit => Ok(vec![Splice::insert(old.start, format!(" {text}"))]),
        Value::Inline(text) if block_value => {
            Ok(vec![Splice::replace(doc.after_colon(&entry.key)?, old.end, format!(" {text}"))])
        }
        Value::Inline(text) => Ok(vec![Splice::replace(old.start, old.end, text)]),
        Value::Lines(lines) => {
            let col = doc.col(entry.key.start);
            let body = lines.at(col);
            if block_value {
                let body = body.trim_end_matches('\n');
                return Ok(vec![Splice::replace(doc.after_colon(&entry.key)?, old.end, format!("\n{body}"))]);
            }
            let mut splices = Vec::new();
            if !implicit {
                check_no_comment(doc, old.start, old.end)?;
                splices.push(Splice::delete(doc.after_colon(&entry.key)?, old.end));
            }
            splices.push(block::insert_after_line(doc, old.end.max(entry.key.end), &body));
            Ok(splices)
        }
    }
}

// --- Lists of names ---------------------------------------------------------------------------

/// Make an entry's value (a name, a list of names, or nothing) hold exactly
/// `new`, touching only the items that change.
pub(crate) fn set_names(doc: &Doc, entry: &Entry, new: &[String]) -> Result<Vec<Splice>> {
    let value = &entry.value;
    match &value.kind {
        Kind::Scalar(_) if new.len() == 1 => Ok(vec![replace_scalar(doc, value, &new[0])?]),
        Kind::Scalar(_) | Kind::Null => replace_value(doc, entry, Value::Inline(flow_list(new))),
        Kind::Map(_) => Err(Unsupported::Shape.into()),
        Kind::Seq(seq) => {
            let old: Vec<&str> = seq
                .items
                .iter()
                .map(|i| i.node.text().ok_or_else(|| Unsupported::Shape.into()))
                .collect::<Result<_>>()?;
            let (lo, ln) = (old.len(), new.len());
            let prefix = old.iter().zip(new).take_while(|(a, b)| **a == b.as_str()).count();
            let suffix = old[prefix..]
                .iter()
                .rev()
                .zip(new[prefix..].iter().rev())
                .take_while(|(a, b)| **a == b.as_str())
                .count();
            let (old_mid, new_mid) = (prefix..lo - suffix, &new[prefix..ln - suffix]);
            if old_mid.len() == new_mid.len() {
                return old_mid.zip(new_mid).map(|(i, v)| replace_scalar(doc, &seq.items[i].node, v)).collect();
            }
            if new.is_empty() {
                return match seq.flow {
                    true => Ok(vec![Splice::replace(value.start, value.end, "[]")]),
                    false => replace_value(doc, entry, Value::Inline("[]".to_owned())),
                };
            }
            let spans = seq.spans();
            if seq.flow {
                flow_reconcile(doc, value, &spans, old_mid, new_mid).map(|s| vec![s])
            } else {
                block_reconcile(doc, &spans, old_mid, new_mid)
            }
        }
    }
}

fn flow_reconcile(
    doc: &Doc,
    coll: &Node,
    spans: &[Span],
    old_mid: std::ops::Range<usize>,
    new_mid: &[String],
) -> Result<Splice> {
    let items: Vec<String> = new_mid.iter().map(|v| plain_or_quoted(v, true)).collect();
    let text = items.join(", ");
    if old_mid.is_empty() {
        return flow_insert(coll, old_mid.start, &text);
    }
    let (start, end) = if !new_mid.is_empty() {
        (spans[old_mid.start].head, spans[old_mid.end - 1].end)
    } else if old_mid.end < spans.len() {
        (spans[old_mid.start].head, spans[old_mid.end].head)
    } else {
        let before = old_mid.start.checked_sub(1).ok_or(Unsupported::Shape)?;
        (spans[before].end, spans[old_mid.end - 1].end)
    };
    check_no_comment(doc, start, end)?;
    Ok(Splice::replace(start, end, text))
}

fn block_reconcile(
    doc: &Doc,
    spans: &[Span],
    old_mid: std::ops::Range<usize>,
    new_mid: &[String],
) -> Result<Vec<Splice>> {
    let &first = spans.first().ok_or(Unsupported::Shape)?;
    let col = doc.col(first.head);
    let lines: String =
        new_mid.iter().map(|v| format!("{}- {}\n", " ".repeat(col), plain_or_quoted(v, false))).collect();
    let mut splices = Vec::new();
    for i in old_mid.clone() {
        let region = block::region(doc, spans[i])?;
        splices.push(Splice::delete(region.start, region.end));
    }
    if !lines.is_empty() {
        let at = match spans.get(old_mid.start) {
            Some(&span) => block::region(doc, span)?.start,
            None => block::end_of(doc, *spans.last().ok_or(Unsupported::Shape)?),
        };
        splices.push(Splice::insert(at, lines));
    }
    Ok(splices)
}

// --- Style conversion ------------------------------------------------------------------------

/// Rewrite a one-line flow sequence value as a block sequence, keeping each
/// item's text: `states: [a, b]` → `states:` / `- a` / `- b`. Any comment
/// after the closing bracket stays on the key's line.
pub(crate) fn flow_seq_to_block(doc: &Doc, entry: &Entry, style: Style) -> Result<Vec<Splice>> {
    let value = &entry.value;
    let seq = value.as_seq().filter(|s| s.flow).ok_or(Unsupported::Shape)?;
    check_no_comment(doc, value.start, value.end)?;
    let col = doc.col(entry.key.start) + style.seq_offset;
    let mut lines = String::new();
    for item in &seq.items {
        let source = doc.text().get(item.node.start..item.node.end).ok_or(Unsupported::Shape)?;
        if source.contains('\n') {
            return Err(Unsupported::Shape.into());
        }
        lines.push_str(&format!("{}- {source}\n", " ".repeat(col)));
    }
    Ok(vec![Splice::delete(doc.after_colon(&entry.key)?, value.end), block::insert_after_line(doc, value.end, &lines)])
}

/// Replace an empty flow collection value (`[]`, `{}`) or nothing with
/// block lines below the key.
pub(crate) fn empty_to_block(doc: &Doc, entry: &Entry, lines: Block) -> Result<Vec<Splice>> {
    replace_value(doc, entry, Value::Lines(lines))
}
