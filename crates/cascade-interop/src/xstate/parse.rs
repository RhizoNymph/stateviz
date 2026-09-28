//! One XState v5 machine config → [`Chart`].
//!
//! State nodes are read in document order; targets and `initial` values are
//! resolved once the whole tree (and every `id`) is known, following
//! XState's rules: `"sibling.child"` is relative to the source's parent,
//! `".child"` to the source itself, `"#id.child"` to the node with that id.
//! At the machine root a plain `"child"` is accepted as `".child"`.

use std::collections::HashMap;

use crate::error::InteropError;
use crate::import::Warnings;
use crate::import::chart::{Chart, Delay, Edge, Emit, NodeIx, NodeKind, TriggerSpec};
use crate::xstate::FORMAT;
use crate::xstate::actions::{emits, guard_text};
use crate::xstate::json::Json;

/// Keys XState accepts on a state node that carry nothing Cascade models.
const IGNORED_KEYS: &[&str] = &[
    "$schema",
    "actions",
    "actors",
    "context",
    "data",
    "delays",
    "description",
    "guards",
    "input",
    "key",
    "meta",
    "order",
    "output",
    "params",
    "predictableActionArguments",
    "preserveActionOrder",
    "schemas",
    "strict",
    "systemId",
    "tags",
    "tsTypes",
    "types",
    "version",
];

/// Keys of a transition object that carry nothing Cascade models.
const IGNORED_TRANSITION_KEYS: &[&str] = &["description", "meta", "eventType", "source", "order"];

fn invalid(location: &str, message: impl Into<String>) -> InteropError {
    InteropError::Invalid { format: FORMAT, location: location.to_owned(), message: message.into() }
}

fn unsupported(location: &str, what: impl Into<String>) -> InteropError {
    InteropError::Unsupported { format: FORMAT, location: location.to_owned(), what: what.into() }
}

struct PendingEdge {
    source: NodeIx,
    trigger: TriggerSpec,
    targets: Vec<String>,
    guard: Option<String>,
    emits: Vec<Emit>,
    reenter: bool,
    location: String,
}

struct Parser<'a> {
    chart: Chart,
    emit_prefix: &'a str,
    warnings: &'a mut Warnings,
    ids: HashMap<String, NodeIx>,
    root_id: String,
    edges: Vec<PendingEdge>,
    initials: Vec<(NodeIx, String, String)>,
}

/// Parse one machine config. `key` is its name in a `machines` map; the
/// machine is named by its `id`, else the key, else `Machine`.
pub(crate) fn machine(
    key: Option<&str>,
    config: &Json,
    location: String,
    emit_prefix: &str,
    warnings: &mut Warnings,
) -> Result<Chart, InteropError> {
    let Some(entries) = config.as_object() else {
        return Err(invalid(&location, format!("a machine config must be an object, found {}", config.kind())));
    };
    let id = config.get("id").and_then(Json::as_str);
    let name = id.or(key).unwrap_or("Machine").to_owned();
    let mut parser = Parser {
        chart: Chart::new(name, location.clone()),
        emit_prefix,
        warnings,
        ids: HashMap::new(),
        root_id: id.unwrap_or("(machine)").to_owned(),
        edges: Vec::new(),
        initials: Vec::new(),
    };
    parser.node(NodeIx::ROOT, entries, &location)?;
    parser.resolve()?;
    Ok(parser.chart)
}

impl Parser<'_> {
    fn node(&mut self, ix: NodeIx, entries: &[(String, Json)], location: &str) -> Result<(), InteropError> {
        let mut kind_name: Option<&str> = None;
        let mut deep: Option<bool> = None;
        let mut children: Option<(&Json, String)> = None;
        let mut transitions: Vec<(&str, &Json, String)> = Vec::new();

        for (key, value) in entries {
            let at = format!("{location}.{key}");
            match key.as_str() {
                "id" => {
                    let Some(id) = value.as_str() else {
                        return Err(invalid(&at, "`id` must be a string"));
                    };
                    if self.ids.insert(id.to_owned(), ix).is_some() {
                        return Err(invalid(&at, format!("duplicate state id `{id}`")));
                    }
                }
                "type" => match value.as_str() {
                    Some(t) => kind_name = Some(t),
                    None => return Err(invalid(&at, "`type` must be a string")),
                },
                "history" => match value.as_str() {
                    Some("deep") => deep = Some(true),
                    Some("shallow") => deep = Some(false),
                    _ => return Err(invalid(&at, "`history` must be \"shallow\" or \"deep\"")),
                },
                "initial" => {
                    let target = match value {
                        Json::String(s) => s.clone(),
                        Json::Object(_) => match value.get("target") {
                            Some(Json::String(s)) => s.clone(),
                            Some(Json::Array(items)) if items.len() == 1 => items[0]
                                .as_str()
                                .map(str::to_owned)
                                .ok_or_else(|| invalid(&at, "`initial.target` must be a string"))?,
                            _ => return Err(invalid(&at, "`initial` must be a state name or { target }")),
                        },
                        _ => return Err(invalid(&at, "`initial` must be a state name or { target }")),
                    };
                    self.initials.push((ix, target, at));
                }
                "states" => children = Some((value, at)),
                "on" | "after" | "invoke" | "onDone" => transitions.push((key.as_str(), value, at)),
                "always" => return Err(unsupported(&at, "eventless `always` transitions")),
                "entry" => {
                    let list = emits(value, self.emit_prefix, &at, self.warnings)?;
                    self.chart.node_mut(ix).entry = list;
                }
                "exit" => {
                    let list = emits(value, self.emit_prefix, &at, self.warnings)?;
                    self.chart.node_mut(ix).exit = list;
                }
                "target" => self.warnings.ignored(at, "the default target of a history state"),
                k if IGNORED_KEYS.contains(&k) => {}
                k => self.warnings.ignored(at, format!("unknown key `{k}`")),
            }
        }

        let kind = match kind_name {
            None | Some("atomic" | "compound") => NodeKind::Normal,
            Some("final") => NodeKind::Final,
            Some("history") => NodeKind::History { deep: deep.unwrap_or(false) },
            Some("parallel") => return Err(unsupported(location, "parallel states (`type: \"parallel\"`)")),
            Some(other) => return Err(invalid(location, format!("unknown state type `{other}`"))),
        };
        if ix == NodeIx::ROOT && kind != NodeKind::Normal {
            return Err(invalid(location, "the machine itself cannot be a final or history state"));
        }
        if deep.is_some() && !matches!(kind, NodeKind::History { .. }) {
            self.warnings.ignored(format!("{location}.history"), "`history` on a state that is not a history state");
        }
        self.chart.node_mut(ix).kind = kind;

        if let Some((value, at)) = children {
            self.children(ix, kind, value, &at)?;
        }
        for (key, value, at) in transitions {
            match key {
                "on" => self.on(ix, value, &at)?,
                "after" => self.after(ix, value, &at)?,
                "invoke" => self.invoke(ix, value, &at)?,
                _ => {
                    if ix == NodeIx::ROOT {
                        self.warnings.ignored(at, "`onDone` on the machine itself");
                    } else {
                        self.transitions(ix, TriggerSpec::StateDone, value, &at)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn children(&mut self, ix: NodeIx, kind: NodeKind, value: &Json, location: &str) -> Result<(), InteropError> {
        let Some(entries) = value.as_object() else {
            return Err(invalid(location, format!("`states` must be an object, found {}", value.kind())));
        };
        if kind != NodeKind::Normal && !entries.is_empty() {
            return Err(invalid(location, "final and history states cannot have child states"));
        }
        for (key, config) in entries {
            let at = format!("{location}.{key}");
            let Some(child_entries) = config.as_object() else {
                return Err(invalid(&at, format!("a state node must be an object, found {}", config.kind())));
            };
            let child = self.chart.add_child(ix, key.clone(), at.clone(), NodeKind::Normal);
            self.node(child, child_entries, &at)?;
        }
        Ok(())
    }

    fn on(&mut self, ix: NodeIx, value: &Json, location: &str) -> Result<(), InteropError> {
        match value {
            Json::Null => Ok(()),
            Json::Object(entries) => {
                for (event, config) in entries {
                    let at = format!("{location}.{event}");
                    if event == "*" {
                        self.warnings.ignored(at, "a wildcard (`*`) transition");
                    } else if event.is_empty() {
                        return Err(invalid(&at, "an event name cannot be empty"));
                    } else {
                        self.transitions(ix, TriggerSpec::Event(event.clone()), config, &at)?;
                    }
                }
                Ok(())
            }
            other => Err(invalid(location, format!("`on` must be an object, found {}", other.kind()))),
        }
    }

    fn after(&mut self, ix: NodeIx, value: &Json, location: &str) -> Result<(), InteropError> {
        let Some(entries) = value.as_object() else {
            return Err(invalid(location, format!("`after` must be an object, found {}", value.kind())));
        };
        for (delay, config) in entries {
            let spec = match delay.trim().parse::<u64>() {
                Ok(ms) => Delay::Millis(ms),
                Err(_) => Delay::Named(delay.clone()),
            };
            self.transitions(ix, TriggerSpec::After(spec), config, &format!("{location}.{delay}"))?;
        }
        Ok(())
    }

    fn invoke(&mut self, ix: NodeIx, value: &Json, location: &str) -> Result<(), InteropError> {
        let invocations: Vec<(&Json, String)> = match value {
            Json::Array(items) => items.iter().enumerate().map(|(i, v)| (v, format!("{location}[{i}]"))).collect(),
            other => vec![(other, location.to_owned())],
        };
        for (invocation, at) in invocations {
            if invocation.as_object().is_none() {
                return Err(invalid(&at, format!("an invocation must be an object, found {}", invocation.kind())));
            }
            let actor = invocation
                .get("id")
                .and_then(Json::as_str)
                .or_else(|| match invocation.get("src") {
                    Some(Json::String(src)) => Some(src.as_str()),
                    Some(src @ Json::Object(_)) => src.get("type").and_then(Json::as_str),
                    _ => None,
                })
                .unwrap_or("invoke")
                .to_owned();
            if let Some(done) = invocation.get("onDone") {
                self.transitions(ix, TriggerSpec::ActorDone(actor.clone()), done, &format!("{at}.onDone"))?;
            }
            if let Some(error) = invocation.get("onError") {
                self.transitions(ix, TriggerSpec::ActorError(actor.clone()), error, &format!("{at}.onError"))?;
            }
            if invocation.get("onSnapshot").is_some() {
                self.warnings.ignored(format!("{at}.onSnapshot"), "snapshot transitions of an invoked actor");
            }
        }
        Ok(())
    }

    /// A transition config: a target string, a transition object, or an
    /// array of those (guarded alternatives, in priority order).
    fn transitions(
        &mut self,
        source: NodeIx,
        trigger: TriggerSpec,
        value: &Json,
        location: &str,
    ) -> Result<(), InteropError> {
        match value {
            Json::Null => {
                self.warnings.ignored(location, "a forbidden (null) transition");
                Ok(())
            }
            Json::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    let at = format!("{location}[{i}]");
                    if matches!(item, Json::Array(_) | Json::Null) {
                        return Err(invalid(&at, "expected a target string or a transition object"));
                    }
                    self.transition(source, trigger.clone(), item, &at)?;
                }
                Ok(())
            }
            single => self.transition(source, trigger, single, location),
        }
    }

    fn transition(
        &mut self,
        source: NodeIx,
        trigger: TriggerSpec,
        value: &Json,
        location: &str,
    ) -> Result<(), InteropError> {
        let mut edge = PendingEdge {
            source,
            trigger,
            targets: Vec::new(),
            guard: None,
            emits: Vec::new(),
            reenter: false,
            location: location.to_owned(),
        };
        match value {
            Json::String(target) => edge.targets.push(target.clone()),
            Json::Object(entries) => {
                for (key, v) in entries {
                    let at = format!("{location}.{key}");
                    match key.as_str() {
                        "target" => {
                            edge.targets = match v {
                                Json::Null => Vec::new(),
                                Json::String(s) => vec![s.clone()],
                                Json::Array(items) => items
                                    .iter()
                                    .map(|t| t.as_str().map(str::to_owned))
                                    .collect::<Option<Vec<_>>>()
                                    .ok_or_else(|| invalid(&at, "targets must be strings"))?,
                                other => {
                                    return Err(invalid(
                                        &at,
                                        format!("`target` must be a string, found {}", other.kind()),
                                    ));
                                }
                            };
                        }
                        "guard" | "cond" => edge.guard = Some(guard_text(v, &at)?),
                        "actions" => edge.emits = emits(v, self.emit_prefix, &at, self.warnings)?,
                        "reenter" => edge.reenter = matches!(v, Json::Bool(true)),
                        "internal" => edge.reenter = matches!(v, Json::Bool(false)),
                        "in" => self.warnings.ignored(at, "an `in` state guard"),
                        k if IGNORED_TRANSITION_KEYS.contains(&k) => {}
                        k => self.warnings.ignored(at, format!("unknown transition key `{k}`")),
                    }
                }
            }
            other => {
                return Err(invalid(
                    location,
                    format!("expected a target string or a transition object, found {}", other.kind()),
                ));
            }
        }
        self.edges.push(edge);
        Ok(())
    }

    // --- Resolution ------------------------------------------------------------------

    fn resolve(&mut self) -> Result<(), InteropError> {
        for (ix, raw, location) in std::mem::take(&mut self.initials) {
            let target = if raw.starts_with('#') {
                self.lookup(ix, &raw, &location)?
            } else {
                self.walk(ix, raw.split('.'), &raw, &location)?
            };
            if target == ix || !self.chart.is_descendant_or_self(target, ix) {
                return Err(invalid(&location, format!("initial state `{raw}` is not inside this state")));
            }
            self.chart.node_mut(ix).initial = Some(target);
        }
        for edge in std::mem::take(&mut self.edges) {
            let target = match edge.targets.as_slice() {
                [] => None,
                [one] => Some(self.lookup(edge.source, one, &edge.location)?),
                _ => {
                    return Err(unsupported(&edge.location, "multiple transition targets (they need parallel states)"));
                }
            };
            self.chart.node_mut(edge.source).edges.push(Edge {
                trigger: edge.trigger,
                target,
                guard: edge.guard,
                emits: edge.emits,
                reenter: edge.reenter,
                bounded: false,
                location: edge.location,
            });
        }
        Ok(())
    }

    /// Resolve a target string from `source`.
    fn lookup(&self, source: NodeIx, raw: &str, location: &str) -> Result<NodeIx, InteropError> {
        if let Some(body) = raw.strip_prefix('#') {
            if let Some(&ix) = self.ids.get(body) {
                return Ok(ix);
            }
            let mut segments = body.split('.');
            let first = segments.next().unwrap_or_default();
            let base = match self.ids.get(first) {
                Some(&ix) => ix,
                None if first == self.root_id => NodeIx::ROOT,
                None => return Err(invalid(location, format!("no state has the id `{first}` (in target `{raw}`)"))),
            };
            return self.walk(base, segments, raw, location);
        }
        if let Some(relative) = raw.strip_prefix('.') {
            return self.walk(source, relative.split('.'), raw, location);
        }
        let base = self.chart.node(source).parent.unwrap_or(NodeIx::ROOT);
        self.walk(base, raw.split('.'), raw, location)
    }

    fn walk<'s>(
        &self,
        base: NodeIx,
        segments: impl Iterator<Item = &'s str>,
        raw: &str,
        location: &str,
    ) -> Result<NodeIx, InteropError> {
        let mut current = base;
        for segment in segments {
            if segment.is_empty() {
                return Err(invalid(location, format!("target `{raw}` has an empty path segment")));
            }
            let found = self.chart.node(current).children.iter().copied().find(|&c| self.chart.node(c).name == segment);
            current = found.ok_or_else(|| {
                invalid(
                    location,
                    format!(
                        "`{}` has no child state `{segment}` (in target `{raw}`)",
                        self.chart.display_path(current)
                    ),
                )
            })?;
        }
        Ok(current)
    }
}
