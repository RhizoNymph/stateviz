//! A transition's `emits:` list, item by item: plain names (`- OrderPaid`)
//! and shaped emits (`{ event: Receipt, with: { total: self.amount } }`,
//! flow or block). Unchanged items keep their text and comments; a changed
//! item keeps its position and trailing comment; `with:` mappings are
//! reconciled entry by entry.

use cascade_core::definition::EmitDef;

use super::keyed::{self, Locate, Pair, flow_mapping};
use super::{Child, add_child, step};
use crate::patch::collection::{remove_child, replace_scalar, set_key};
use crate::patch::doc::{Doc, Entry, Node};
use crate::patch::error::{Result, Unsupported};
use crate::patch::render::{Block, plain_or_quoted};
use crate::patch::splice::Splice;

fn pairs(emit: &EmitDef) -> Vec<Pair> {
    emit.with.iter().map(|a| (a.value.field.clone(), a.value.value.to_string())).collect()
}

/// An emit as a sequence item's text.
fn item_text(emit: &EmitDef, in_flow: bool) -> String {
    let event = plain_or_quoted(&emit.event.value, in_flow);
    if emit.with.is_empty() {
        event
    } else {
        format!("{{ event: {}, with: {} }}", plain_or_quoted(&emit.event.value, true), flow_mapping(&pairs(emit)))
    }
}

fn same(a: &EmitDef, b: &EmitDef) -> bool {
    a.event.value == b.event.value && pairs(a) == pairs(b)
}

/// Make the located `emits:` sequence hold `new` (non-empty), when the
/// old value is a sequence; `Unsupported` otherwise.
pub(crate) fn reconcile(text: &str, locate: Locate<'_>, old: &[EmitDef], new: &[EmitDef]) -> Result<String> {
    {
        let doc = Doc::parse(text)?;
        let seq = locate(&doc)?.value.as_seq().ok_or(Unsupported::Shape)?;
        if seq.items.len() != old.len() {
            return Err(Unsupported::Shape.into());
        }
    }
    let prefix = old.iter().zip(new).take_while(|(a, b)| same(a, b)).count();
    let suffix = old[prefix..].iter().rev().zip(new[prefix..].iter().rev()).take_while(|(a, b)| same(a, b)).count();
    let old_mid = prefix..old.len() - suffix;
    let new_mid = prefix..new.len() - suffix;
    let paired = old_mid.len().min(new_mid.len());
    let mut text = text.to_owned();
    for offset in 0..paired {
        let index = prefix + offset;
        text = update_item(&text, locate, index, &old[index], &new[new_mid.start + offset])?;
    }
    for index in (old_mid.start + paired..old_mid.end).rev() {
        text = step(&text, |doc, _| Ok(vec![remove_child(doc, &locate(doc)?.value, index)?]))?;
    }
    // Each new item goes to its own position in `new`: everything before it
    // already matches.
    for (index, emit) in new.iter().enumerate().take(new_mid.end).skip(new_mid.start + paired) {
        text = step(&text, |doc, style| {
            let mut block = Block::new();
            block.line(0, format!("- {}", item_text(emit, false)));
            let child = Child { block, inline: Some(item_text(emit, true)), item: true };
            add_child(doc, style, locate(doc)?, index, &child, false)
        })?;
    }
    Ok(text)
}

/// The item node at `index` of the located sequence.
fn item<'d>(doc: &'d Doc, locate: Locate<'_>, index: usize) -> Result<&'d Node> {
    let seq = locate(doc)?.value.as_seq().ok_or(Unsupported::Shape)?;
    seq.items.get(index).map(|i| &i.node).ok_or_else(|| Unsupported::Shape.into())
}

fn update_item(text: &str, locate: Locate<'_>, index: usize, old: &EmitDef, new: &EmitDef) -> Result<String> {
    let doc = Doc::parse(text)?;
    let node = item(&doc, locate, index)?;
    let in_flow = doc_in_flow(&doc, locate)?;
    match (node.as_map(), new.with.is_empty()) {
        // A name, staying a name or becoming a flow mapping.
        (None, true) => {
            step(text, |doc, _| Ok(vec![replace_scalar(doc, item(doc, locate, index)?, &new.event.value)?]))
        }
        (None, false) => step(text, |doc, _| {
            let node = item(doc, locate, index)?;
            node.as_scalar().ok_or(Unsupported::Shape)?;
            Ok(vec![Splice::replace(node.start, node.end, item_text(new, in_flow))])
        }),
        // A mapping becoming a name: keep the first line's comment.
        (Some(_), true) => step(text, |doc, _| {
            let node = item(doc, locate, index)?;
            Ok(vec![collapse(doc, node, &plain_or_quoted(&new.event.value, in_flow))?])
        }),
        (Some(_), false) => {
            let mut text = text.to_owned();
            if old.event.value != new.event.value {
                text = step(&text, |doc, _| {
                    let node = item(doc, locate, index)?;
                    let event = node.as_map().and_then(|m| m.value("event")).ok_or(Unsupported::Shape)?;
                    Ok(vec![replace_scalar(doc, event, &new.event.value)?])
                })?;
            }
            let with = pairs(new);
            if pairs(old) != with {
                let has_with = |doc: &Doc| {
                    item(doc, locate, index).is_ok_and(|n| n.as_map().and_then(|m| m.get("with")).is_some())
                };
                if has_with(&Doc::parse(&text)?) {
                    text = keyed::reconcile_map(&text, &|doc| with_entry(doc, locate, index), &with)?;
                } else {
                    let value = crate::patch::collection::Value::Inline(flow_mapping(&with));
                    text = step(&text, |doc, _| {
                        set_key(doc, item(doc, locate, index)?, "with", Some(value.clone()), &["event", "with"])
                    })?;
                }
            }
            Ok(text)
        }
    }
}

/// The `with:` entry of a shaped emit.
fn with_entry<'d>(doc: &'d Doc, locate: Locate<'_>, index: usize) -> Result<&'d Entry> {
    let node = item(doc, locate, index)?;
    node.as_map().and_then(|m| m.get("with")).map(|(_, e)| e).ok_or_else(|| Unsupported::Shape.into())
}

/// Whether the located sequence is a flow sequence.
fn doc_in_flow(doc: &Doc, locate: Locate<'_>) -> Result<bool> {
    Ok(locate(doc)?.value.is_flow())
}

/// Replace a (possibly multi-line) item node with `text`, keeping the
/// comment that ends the item's first line.
fn collapse(doc: &Doc, node: &Node, text: &str) -> Result<Splice> {
    let lines = doc.lines();
    let first = lines.line_of(node.start);
    let last = lines.line_of(node.end.max(node.start + 1) - 1);
    if first == last {
        return Ok(Splice::replace(node.start, node.end, text));
    }
    let line = lines.text(doc.text(), first);
    let comment = trailing_comment(line).unwrap_or_default();
    let end = lines.start(last) + lines.text(doc.text(), last).len();
    Ok(Splice::replace(node.start, end, format!("{text}{comment}")))
}

/// The comment ending a line, with the spaces before it.
fn trailing_comment(line: &str) -> Option<&str> {
    let mut quote: Option<char> = None;
    let mut prev = ' ';
    for (i, c) in line.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if (c == '"' || c == '\'') && matches!(prev, ' ' | '[' | '{' | ',' | ':') => quote = Some(c),
            None if c == '#' && prev == ' ' => {
                let start = line[..i].trim_end_matches(' ').len();
                return Some(&line[start..]);
            }
            None => {}
        }
        prev = c;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::trailing_comment;

    #[test]
    fn finds_comments_outside_quotes() {
        assert_eq!(trailing_comment("  event: Receipt   # shaped"), Some("   # shaped"));
        assert_eq!(trailing_comment("  event: \"a # b\""), None);
        assert_eq!(trailing_comment("  event: a#b"), None);
    }
}
