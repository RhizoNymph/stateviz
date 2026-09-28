//! Finding definition elements in the span index.

use cascade_core::edit::EditError;

use crate::patch::doc::{Doc, Entry, Kind, Node};
use crate::patch::error::{Result, SurgeryError, Unsupported, not_found};

/// Canonical key orders, as `to_yaml` writes them. New keys go after the
/// last present key that precedes them here.
pub(crate) const ROOT_ORDER: &[&str] = &["system", "machines", "events", "controllers", "external"];
pub(crate) const MACHINE_ORDER: &[&str] = &["color", "domain", "initial", "fields", "states", "transitions"];
pub(crate) const STATE_ORDER: &[&str] = &["kind", "initial", "states"];
pub(crate) const TRANSITION_ORDER: &[&str] = &["from", "to", "on", "guard", "emits", "bounded"];
pub(crate) const RULE_ORDER: &[&str] = &["fire", "target", "when", "bounded"];

/// The root mapping.
pub(crate) fn root(doc: &Doc) -> Result<&Node> {
    let root = doc.root();
    root.as_map().map(|_| root).ok_or_else(|| Unsupported::Shape.into())
}

/// A top-level section (`machines:`, `events:`, …), if present.
pub(crate) fn section<'d>(doc: &'d Doc, key: &str) -> Result<Option<&'d Entry>> {
    Ok(root(doc)?.as_map().and_then(|m| m.get(key)).map(|(_, e)| e))
}

/// Entry `name` of a mapping node, with its position.
pub(crate) fn named<'d>(node: &'d Node, name: &str, what: &'static str) -> Result<(usize, &'d Entry)> {
    let map = node.as_map().ok_or_else(|| not_found(what, name))?;
    map.get(name).ok_or_else(|| not_found(what, name))
}

/// The machines mapping node.
pub(crate) fn machines(doc: &Doc) -> Result<&Entry> {
    section(doc, "machines")?.ok_or_else(|| Unsupported::Shape.into())
}

/// Machine `name`: its position and entry.
pub(crate) fn machine<'d>(doc: &'d Doc, name: &str) -> Result<(usize, &'d Entry)> {
    named(&machines(doc)?.value, name, "machine")
}

/// Controller `name`: its position and entry.
pub(crate) fn controller<'d>(doc: &'d Doc, name: &str) -> Result<(usize, &'d Entry)> {
    let section = section(doc, "controllers")?.ok_or_else(|| not_found("controller", name))?;
    named(&section.value, name, "controller")
}

/// The `on:` entry of a controller body.
pub(crate) fn handlers<'d>(doc: &'d Doc, controller_name: &str) -> Result<&'d Entry> {
    let (_, entry) = controller(doc, controller_name)?;
    let body = entry.value.as_map().ok_or(Unsupported::Shape)?;
    body.get("on").map(|(_, e)| e).ok_or_else(|| Unsupported::Shape.into())
}

/// Handler `event` of a controller.
pub(crate) fn handler<'d>(doc: &'d Doc, controller_name: &str, event: &str) -> Result<(usize, &'d Entry)> {
    named(&handlers(doc, controller_name)?.value, event, "handler")
}

/// The rule nodes of a handler value: a list of rules or one rule mapping.
pub(crate) fn rules(value: &Node) -> Vec<&Node> {
    match &value.kind {
        Kind::Seq(seq) => seq.items.iter().map(|i| &i.node).collect(),
        Kind::Map(_) => vec![value],
        Kind::Scalar(_) | Kind::Null => Vec::new(),
    }
}

/// A key of a mapping node that must be there.
pub(crate) fn require<'d>(node: &'d Node, key: &str) -> Result<&'d Entry> {
    node.as_map().and_then(|m| m.get(key)).map(|(_, e)| e).ok_or_else(|| Unsupported::Shape.into())
}

// --- States ---------------------------------------------------------------------------

/// One state as written in a `states:` collection.
#[derive(Debug)]
pub(crate) struct StateRef<'d> {
    /// The `states:` entry holding it.
    pub list: &'d Entry,
    /// Position among its siblings.
    pub index: usize,
    /// The name scalar (a sequence item, or a mapping key).
    pub name: &'d Node,
    /// The whole sequence item (a scalar or a single-key mapping); `None` in
    /// mapping form, where the state is an entry of `list`.
    pub item: Option<&'d Node>,
    /// The body after the name, if written (may be null or `{}`).
    pub body: Option<&'d Node>,
}

impl<'d> StateRef<'d> {
    /// The body mapping, when the state has a non-empty one.
    pub fn body_map(&self) -> Option<&'d Node> {
        self.body.filter(|b| b.as_map().is_some_and(|m| !m.entries.is_empty()))
    }

    /// The `states:` entry of the body, if the state has children written.
    pub fn children(&self) -> Option<&'d Entry> {
        self.body.and_then(|b| b.as_map()).and_then(|m| m.get("states")).map(|(_, e)| e)
    }
}

/// The states of a `states:` entry, in order.
pub(crate) fn states_of(list: &Entry) -> Result<Vec<StateRef<'_>>> {
    let mut found = Vec::new();
    match &list.value.kind {
        Kind::Seq(seq) => {
            for (index, item) in seq.items.iter().enumerate() {
                let (name, body) = match &item.node.kind {
                    Kind::Scalar(_) => (&item.node, None),
                    Kind::Map(m) => match m.entries.as_slice() {
                        [entry] => (&entry.key, Some(&entry.value)),
                        _ => return Err(Unsupported::Shape.into()),
                    },
                    Kind::Null | Kind::Seq(_) => return Err(Unsupported::Shape.into()),
                };
                found.push(StateRef { list, index, name, item: Some(&item.node), body });
            }
        }
        Kind::Map(map) => {
            for (index, entry) in map.entries.iter().enumerate() {
                found.push(StateRef { list, index, name: &entry.key, item: None, body: Some(&entry.value) });
            }
        }
        Kind::Null => {}
        Kind::Scalar(_) => return Err(Unsupported::Shape.into()),
    }
    Ok(found)
}

/// State `path` (dotted) of a machine body.
pub(crate) fn state<'d>(machine_body: &'d Node, machine: &str, path: &str) -> Result<StateRef<'d>> {
    let missing = || SurgeryError::Edit(EditError::NotFound { what: "state", name: format!("{machine}.{path}") });
    let mut list = require(machine_body, "states").map_err(|_| missing())?;
    let mut segments = path.split('.').peekable();
    while let Some(segment) = segments.next() {
        let found = states_of(list)?.into_iter().find(|s| s.name.text() == Some(segment)).ok_or_else(missing)?;
        if segments.peek().is_none() {
            return Ok(found);
        }
        list = found.children().ok_or_else(missing)?;
    }
    Err(missing())
}
