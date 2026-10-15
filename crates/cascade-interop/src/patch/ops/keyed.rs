//! Small keyed mappings edited entry by entry: `sets:`, emit `with:`, typed
//! `fields:`/`payload:` (and their switch to and from the list form).
//!
//! Each change is its own step (re-parsing in between): removals, then
//! value changes, then insertions. Unchanged entries, their comments and
//! the comments between them are never touched. When the new keys keep the
//! old ones in a different order (or no key survives) the value is
//! replaced as a whole, still in place: as a flow mapping, or as block
//! lines when it was a block mapping.

use cascade_core::definition::{FieldDef, fields_are_typed};

use super::{Child, add_child, step};
use crate::patch::collection::{Value, remove_child, replace_scalar, replace_value, set_names};
use crate::patch::doc::{Doc, Entry, Kind};
use crate::patch::error::{Result, Unsupported};
use crate::patch::render::{Block, plain_or_quoted};
use crate::patch::splice::Splice;

/// Finds the entry (`key: value`) being edited in a freshly parsed text.
pub(crate) type Locate<'a> = &'a dyn for<'d> Fn(&'d Doc) -> Result<&'d Entry>;

/// One mapping entry: the key and the value's text (unquoted).
pub(crate) type Pair = (String, String);

/// `{ k: v, … }` in a flow context.
pub(crate) fn flow_mapping(pairs: &[Pair]) -> String {
    let entries: Vec<String> =
        pairs.iter().map(|(k, v)| format!("{}: {}", plain_or_quoted(k, true), plain_or_quoted(v, true))).collect();
    format!("{{ {} }}", entries.join(", "))
}

fn pair_child(key: &str, value: &str) -> Child {
    let mut block = Block::new();
    block.line(0, format!("{}: {}", plain_or_quoted(key, false), plain_or_quoted(value, false)));
    Child {
        block,
        inline: Some(format!("{}: {}", plain_or_quoted(key, true), plain_or_quoted(value, true))),
        item: false,
    }
}

/// The keys and scalar values of a mapping node, or `None` for any other
/// shape.
fn current(entry: &Entry) -> Option<Vec<(String, Option<String>)>> {
    let map = entry.value.as_map()?;
    map.entries.iter().map(|e| Some((e.key.text()?.to_owned(), e.value.text().map(str::to_owned)))).collect()
}

/// Replace the whole mapping value in place, keeping its style.
fn replace_whole(doc: &Doc, entry: &Entry, pairs: &[Pair]) -> Result<Vec<Splice>> {
    let value = &entry.value;
    if value.as_map().is_some_and(|m| !m.flow)
        && let Some(first) = value.as_map().and_then(|m| m.entries.first())
    {
        let col = doc.col(first.key.start);
        let body: Vec<String> = pairs
            .iter()
            .map(|(k, v)| format!("{}{}: {}", " ".repeat(col), plain_or_quoted(k, false), plain_or_quoted(v, false)))
            .collect();
        // Through the end of the last entry's line, so its comment goes
        // with it rather than landing on another entry.
        let lines = doc.lines();
        let last = lines.line_of(value.end.max(value.start + 1) - 1);
        let end = lines.start(last) + lines.text(doc.text(), last).len();
        return Ok(vec![Splice::replace(doc.after_colon(&entry.key)?, end, format!("\n{}", body.join("\n")))]);
    }
    replace_value(doc, entry, Value::Inline(flow_mapping(pairs)))
}

/// Make the mapping held by the located entry hold exactly `new`, in
/// order. `new` must not be empty (removing the key is the caller's job).
pub(crate) fn reconcile_map(text: &str, locate: Locate<'_>, new: &[Pair]) -> Result<String> {
    let doc = Doc::parse(text)?;
    let entry = locate(&doc)?;
    let Some(old) = current(entry) else {
        return step(text, |doc, _| replace_whole(doc, locate(doc)?, new));
    };
    let old_keys: Vec<&str> = old.iter().map(|(k, _)| k.as_str()).collect();
    let new_keys: Vec<&str> = new.iter().map(|(k, _)| k.as_str()).collect();
    let kept_old: Vec<&str> = old_keys.iter().copied().filter(|k| new_keys.contains(k)).collect();
    let kept_new: Vec<&str> = new_keys.iter().copied().filter(|k| old_keys.contains(k)).collect();
    let duplicates = |keys: &[&str]| keys.iter().enumerate().any(|(i, k)| keys[..i].contains(k));
    if kept_old.is_empty() || kept_old != kept_new || duplicates(&old_keys) || duplicates(&new_keys) {
        return step(text, |doc, _| replace_whole(doc, locate(doc)?, new));
    }
    let mut text = text.to_owned();
    // Removals, last first.
    for (index, (key, _)) in old.iter().enumerate().rev() {
        if !new_keys.contains(&key.as_str()) {
            text = step(&text, |doc, _| Ok(vec![remove_child(doc, &locate(doc)?.value, index)?]))?;
        }
    }
    // Changed values.
    for (key, value) in new {
        let unchanged = old.iter().any(|(k, v)| k == key && v.as_deref() == Some(value.as_str()));
        if !kept_old.contains(&key.as_str()) || unchanged {
            continue;
        }
        text = step(&text, |doc, _| {
            let map = locate(doc)?.value.as_map().ok_or(Unsupported::Shape)?;
            let (_, entry) = map.get(key).ok_or(Unsupported::Shape)?;
            Ok(vec![replace_scalar(doc, &entry.value, value)?])
        })?;
    }
    // Insertions, after the nearest preceding key already there.
    for (position, (key, value)) in new.iter().enumerate() {
        if kept_old.contains(&key.as_str()) {
            continue;
        }
        text = step(&text, |doc, style| {
            let entry = locate(doc)?;
            let map = entry.value.as_map().ok_or(Unsupported::Shape)?;
            let at = new[..position].iter().rev().find_map(|(k, _)| map.get(k).map(|(i, _)| i + 1)).unwrap_or(0);
            add_child(doc, style, entry, at, &pair_child(key, value), false)
        })?;
    }
    Ok(text)
}

/// The text of a field's type in a mapping (`any` when untyped).
fn type_text(field: &FieldDef) -> String {
    field.ty.as_ref().map(|t| t.value.to_string()).unwrap_or_else(|| "any".to_owned())
}

/// Make a non-empty field list (`fields:` or `payload:`) hold `fields`,
/// switching between the list form (untyped) and the mapping form (typed)
/// entry by entry when the old value is a block collection.
pub(crate) fn set_field_decls(text: &str, locate: Locate<'_>, fields: &[FieldDef]) -> Result<String> {
    let names: Vec<String> = fields.iter().map(|f| f.name.value.clone()).collect();
    let doc = Doc::parse(text)?;
    let entry = locate(&doc)?;
    let block = !entry.value.is_flow();
    if fields_are_typed(fields) {
        let pairs: Vec<Pair> = fields.iter().map(|f| (f.name.value.clone(), type_text(f))).collect();
        let text = match &entry.value.kind {
            Kind::Seq(_) if block => step(text, |doc, _| list_to_mapping(doc, locate(doc)?))?,
            Kind::Seq(_) | Kind::Scalar(_) | Kind::Null => {
                return step(text, |doc, _| replace_value(doc, locate(doc)?, Value::Inline(flow_mapping(&pairs))));
            }
            Kind::Map(_) => text.to_owned(),
        };
        return reconcile_map(&text, locate, &pairs);
    }
    let text = match &entry.value.kind {
        Kind::Map(_) if block => step(text, |doc, _| mapping_to_list(doc, locate(doc)?))?,
        Kind::Map(_) => {
            let list = crate::yaml::fields_text(fields);
            return step(text, |doc, _| replace_value(doc, locate(doc)?, Value::Inline(list.clone())));
        }
        Kind::Seq(_) | Kind::Scalar(_) | Kind::Null => text.to_owned(),
    };
    step(&text, |doc, _| set_names(doc, locate(doc)?, &names))
}

/// `- name` items → `name: any` entries, in place (comments stay).
fn list_to_mapping(doc: &Doc, entry: &Entry) -> Result<Vec<Splice>> {
    let seq = entry.value.as_seq().filter(|s| !s.flow).ok_or(Unsupported::Shape)?;
    let key_col = doc.col(entry.key.start);
    seq.items
        .iter()
        .map(|item| {
            let name = item.node.text().ok_or(Unsupported::Shape)?;
            if doc.col(item.head) <= key_col || !doc.starts_line(item.head) {
                return Err(Unsupported::Shape.into());
            }
            Ok(Splice::replace(item.head, item.node.end, format!("{}: any", plain_or_quoted(name, false))))
        })
        .collect()
}

/// `name: type` entries → `- name` items, in place (comments stay).
fn mapping_to_list(doc: &Doc, entry: &Entry) -> Result<Vec<Splice>> {
    let map = entry.value.as_map().filter(|m| !m.flow).ok_or(Unsupported::Shape)?;
    map.entries
        .iter()
        .map(|e| {
            let name = e.key.text().ok_or(Unsupported::Shape)?;
            if !doc.starts_line(e.key.start) || e.value.as_scalar().is_none() {
                return Err(Unsupported::Shape.into());
            }
            Ok(Splice::replace(e.key.start, e.value.end, format!("- {}", plain_or_quoted(name, false))))
        })
        .collect()
}
