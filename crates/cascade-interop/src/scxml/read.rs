//! SCXML document → charts (and Cascade annotations).
//!
//! A document whose only top-level state is a `<parallel>` is a system: each
//! region `<state>` is a machine (or, when marked `cascade:controller`, a
//! controller). Any other document is one machine named by `<scxml name>`.
//! State names are ids with the parent's id prefix removed (`Order.draft`
//! under `Order` is `draft`), so Cascade's own export reads back unchanged.

use std::collections::HashMap;

use roxmltree::{Document, Node};

use cascade_core::PaletteColor;
use cascade_core::definition::FieldDef;
use cascade_core::expr::{Assignment, ExprContext, parse_expr, parse_field_type, parse_operand};

use crate::error::InteropError;
use crate::import::Warnings;
use crate::import::chart::{Chart, Delay, Edge, Emit, NodeIx, NodeKind, Route, TriggerSpec};
use crate::import::wiring::{
    AnnotatedController, AnnotatedEvent, AnnotatedHandler, AnnotatedRule, AnnotatedSource, Annotations, Wiring,
};
use crate::scxml::{CASCADE_NS, FORMAT, SCXML_NS};

/// What an SCXML document contributes to an import.
pub(crate) struct ReadDocument {
    pub system: Option<String>,
    pub charts: Vec<Chart>,
    pub wiring: Wiring,
}

fn invalid(location: &str, message: impl Into<String>) -> InteropError {
    InteropError::Invalid { format: FORMAT, location: location.to_owned(), message: message.into() }
}

fn unsupported(location: &str, what: impl Into<String>) -> InteropError {
    InteropError::Unsupported { format: FORMAT, location: location.to_owned(), what: what.into() }
}

/// An SCXML element (the SCXML namespace, or no namespace at all).
fn is_scxml(node: Node, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name && matches!(node.tag_name().namespace(), None | Some(SCXML_NS))
}

fn is_cascade(node: Node, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name && node.tag_name().namespace() == Some(CASCADE_NS)
}

fn cascade_attr<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attribute((CASCADE_NS, name))
}

struct PendingEdge {
    chart: usize,
    source: NodeIx,
    trigger: String,
    /// The event descriptor as written (for matching delayed sends).
    raw_event: String,
    /// A subscription (Cascade's export: an event without the machine
    /// prefix), with its filter.
    subscription: Option<Option<cascade_core::expr::Expr>>,
    sets: Vec<Assignment>,
    target: Option<String>,
    guard: Option<String>,
    emits: Vec<Emit>,
    reenter: bool,
    bounded: bool,
    location: String,
}

struct DelayedSend {
    chart: usize,
    node: NodeIx,
    event: String,
    millis: u64,
    emit: Emit,
}

struct Reader<'d, 'w> {
    doc: &'d Document<'d>,
    warnings: &'w mut Warnings,
    annotated: bool,
    charts: Vec<Chart>,
    /// Document-wide state ids → (chart, node).
    ids: HashMap<String, (usize, NodeIx)>,
    edges: Vec<PendingEdge>,
    initials: Vec<(usize, NodeIx, String, String)>,
    /// Delayed `<send>`s in `<onentry>`: a timer when a transition of the
    /// same state takes the event, else an entry emit.
    delayed: Vec<DelayedSend>,
    annotations: Annotations,
    generated: usize,
}

pub(crate) fn read(text: &str, warnings: &mut Warnings) -> Result<ReadDocument, InteropError> {
    let doc = Document::parse(text).map_err(|err| {
        let pos = err.pos();
        InteropError::Syntax { format: FORMAT, line: pos.row, col: pos.col, message: err.to_string() }
    })?;
    let root = doc.root_element();
    if !is_scxml(root, "scxml") {
        return Err(invalid("line 1", format!("the root element must be <scxml>, found <{}>", root.tag_name().name())));
    }
    let annotated = root.namespaces().any(|ns| ns.uri() == CASCADE_NS);
    let mut reader = Reader {
        doc: &doc,
        warnings,
        annotated,
        charts: Vec::new(),
        ids: HashMap::new(),
        edges: Vec::new(),
        initials: Vec::new(),
        delayed: Vec::new(),
        annotations: Annotations::default(),
        generated: 0,
    };
    let system = reader.document(root)?;
    reader.resolve()?;
    let wiring = if reader.annotated { Wiring::Annotated(reader.annotations) } else { Wiring::Synthesize };
    Ok(ReadDocument { system, charts: reader.charts, wiring })
}

impl<'d> Reader<'d, '_> {
    fn location(&self, node: Node) -> String {
        let pos = self.doc.text_pos_at(node.range().start);
        format!("line {} <{}>", pos.row, node.tag_name().name())
    }

    fn document(&mut self, root: Node<'d, 'd>) -> Result<Option<String>, InteropError> {
        for child in root.children().filter(Node::is_element) {
            if is_cascade(child, "event") {
                self.cascade_event(child)?;
            } else if is_cascade(child, "source") {
                self.cascade_source(child)?;
            }
        }
        let top: Vec<Node> = root
            .children()
            .filter(|n| is_scxml(*n, "state") || is_scxml(*n, "parallel") || is_scxml(*n, "final"))
            .collect();
        if let [parallel] = top.as_slice()
            && is_scxml(*parallel, "parallel")
        {
            self.system_regions(*parallel)?;
            let system = cascade_attr(root, "system").or_else(|| root.attribute("name"));
            return Ok(system.map(str::to_owned));
        }
        let name = root.attribute("name").unwrap_or("Machine");
        self.machine(root, name)?;
        Ok(cascade_attr(root, "system").map(str::to_owned))
    }

    fn system_regions(&mut self, parallel: Node<'d, 'd>) -> Result<(), InteropError> {
        for region in parallel.children().filter(Node::is_element) {
            let location = self.location(region);
            match region.tag_name().name() {
                "state" if self.annotated && cascade_attr(region, "controller").is_some() => self.controller(region)?,
                "state" => {
                    let name =
                        region.attribute("id").ok_or_else(|| invalid(&location, "a machine region needs an `id`"))?;
                    self.machine(region, name)?;
                }
                "parallel" => return Err(unsupported(&location, "parallel states inside a machine")),
                "final" | "history" => {
                    return Err(invalid(&location, "each region of the top-level <parallel> must be a <state>"));
                }
                "datamodel" | "script" => {}
                other => self.warnings.ignored(location, format!("<{other}> directly inside the top-level <parallel>")),
            }
        }
        Ok(())
    }

    /// A machine: `element` is `<scxml>` (single machine) or a region.
    fn machine(&mut self, element: Node<'d, 'd>, name: &str) -> Result<(), InteropError> {
        let location = self.location(element);
        let index = self.charts.len();
        let mut chart = Chart::new(name.to_owned(), location.clone());
        if self.annotated {
            if let Some(color) = cascade_attr(element, "color") {
                chart.attrs.color =
                    Some(color.parse::<PaletteColor>().map_err(|err| invalid(&location, err.to_string()))?);
            }
            chart.attrs.domain = cascade_attr(element, "domain").map(str::to_owned);
            chart.attrs.fields = cascade_attr(element, "fields")
                .map(|f| f.split_whitespace().map(FieldDef::untyped).collect())
                .unwrap_or_default();
            chart.attrs.fields.extend(typed_fields(element, &location)?);
        }
        self.charts.push(chart);
        if !is_scxml(element, "scxml") {
            self.register(element, index, NodeIx::ROOT)?;
        }
        if let Some(initial) = element.attribute("initial") {
            self.initials.push((index, NodeIx::ROOT, initial.to_owned(), location));
        }
        self.children(index, NodeIx::ROOT, element, name)
    }

    fn register(&mut self, element: Node, chart: usize, node: NodeIx) -> Result<(), InteropError> {
        if let Some(id) = element.attribute("id")
            && self.ids.insert(id.to_owned(), (chart, node)).is_some()
        {
            return Err(invalid(&self.location(element), format!("duplicate id `{id}`")));
        }
        Ok(())
    }

    /// The children of a state-like element. `parent_id` is stripped from
    /// child ids to get local names.
    fn children(
        &mut self,
        chart: usize,
        parent: NodeIx,
        element: Node<'d, 'd>,
        parent_id: &str,
    ) -> Result<(), InteropError> {
        let machine = self.charts[chart].name().to_owned();
        for child in element.children().filter(Node::is_element) {
            if !matches!(child.tag_name().namespace(), None | Some(SCXML_NS)) {
                continue;
            }
            let location = self.location(child);
            match child.tag_name().name() {
                name @ ("state" | "final" | "history") => {
                    let kind = match name {
                        "state" => NodeKind::Normal,
                        "final" => NodeKind::Final,
                        _ => NodeKind::History { deep: child.attribute("type") == Some("deep") },
                    };
                    let local = match child.attribute("id") {
                        Some(id) => id.strip_prefix(&format!("{parent_id}.")).unwrap_or(id).to_owned(),
                        None => {
                            self.generated += 1;
                            let generated = format!("state{}", self.generated);
                            self.warnings.approximated(
                                location.as_str(),
                                "a state without an `id`",
                                format!("named `{generated}`; no transition can target it"),
                            );
                            generated
                        }
                    };
                    let ix = self.charts[chart].add_child(parent, local.clone(), location.clone(), kind);
                    self.register(child, chart, ix)?;
                    if let Some(initial) = child.attribute("initial") {
                        self.initials.push((chart, ix, initial.to_owned(), location.clone()));
                    }
                    if kind == NodeKind::Normal || kind == NodeKind::Final {
                        let own_id = child.attribute("id").map_or(local, str::to_owned);
                        self.children(chart, ix, child, &own_id)?;
                    } else if child.children().any(|n| is_scxml(n, "transition")) {
                        self.warnings.ignored(location, "the default transition of a history state");
                    }
                }
                "parallel" => return Err(unsupported(&location, "parallel states")),
                "transition" => {
                    if is_scxml(element, "scxml") {
                        self.warnings.ignored(location, "a <transition> directly inside <scxml>");
                    } else {
                        self.transition(chart, parent, child, &machine)?;
                    }
                }
                "initial" => {
                    let target = child
                        .children()
                        .find(|n| is_scxml(*n, "transition"))
                        .and_then(|t| t.attribute("target"))
                        .ok_or_else(|| invalid(&location, "<initial> needs a <transition target>"))?;
                    self.initials.push((chart, parent, target.to_owned(), location));
                }
                "onentry" => {
                    let emits = self.entry_actions(chart, parent, child);
                    self.charts[chart].node_mut(parent).entry.extend(emits);
                }
                "onexit" => {
                    let emits = self.executable(child);
                    self.charts[chart].node_mut(parent).exit.extend(emits);
                }
                "invoke" => {
                    self.warnings.ignored(location, "<invoke> (its done and error events are ordinary events here)")
                }
                "datamodel" | "donedata" | "script" => {}
                other => self.warnings.ignored(location, format!("unknown element <{other}>")),
            }
        }
        Ok(())
    }

    fn transition(&mut self, chart: usize, source: NodeIx, element: Node, machine: &str) -> Result<(), InteropError> {
        let location = self.location(element);
        let Some(events) = element.attribute("event").filter(|e| !e.trim().is_empty()) else {
            return Err(unsupported(&location, "eventless transitions (a <transition> without `event`)"));
        };
        let target = match element.attribute("target").map(|t| t.split_whitespace().collect::<Vec<_>>()) {
            None => None,
            Some(ids) => match ids.as_slice() {
                [] => None,
                [one] => Some((*one).to_owned()),
                _ => return Err(unsupported(&location, "multiple transition targets (they need parallel states)")),
            },
        };
        let emits = self.executable(element);
        let sets = self.assigns(element);
        for token in events.split_whitespace() {
            let descriptor = token.trim_end_matches('*').trim_end_matches('.');
            let stripped = descriptor.strip_prefix(&format!("{machine}."));
            let trigger = stripped.unwrap_or(descriptor);
            if trigger.is_empty() {
                self.warnings.ignored(location.as_str(), format!("the wildcard event descriptor `{token}`"));
                continue;
            }
            // Cascade's export names triggers `Machine.trigger`; an event
            // without the prefix is a subscription.
            let subscription = match (self.annotated, stripped) {
                (true, None) => Some(match cascade_attr(element, "where") {
                    Some(text) => Some(
                        parse_expr(text, ExprContext::Filter)
                            .map_err(|err| invalid(&location, format!("invalid `cascade:where` `{text}`: {err}")))?,
                    ),
                    None => None,
                }),
                _ => None,
            };
            let guard = match &subscription {
                Some(_) if cascade_attr(element, "where").is_some() => cascade_attr(element, "guard"),
                _ => element.attribute("cond"),
            };
            self.edges.push(PendingEdge {
                chart,
                source,
                trigger: trigger.to_owned(),
                raw_event: descriptor.to_owned(),
                subscription,
                sets: sets.clone(),
                target: target.clone(),
                guard: guard.map(str::to_owned),
                emits: emits.clone(),
                reenter: element.attribute("type") != Some("internal"),
                bounded: cascade_attr(element, "bounded") == Some("true"),
                location: location.clone(),
            });
        }
        Ok(())
    }

    /// `<onentry>` content: emits, except delayed `<send>`s with a static
    /// event and delay, which wait to see whether they are timers.
    fn entry_actions(&mut self, chart: usize, node: NodeIx, element: Node) -> Vec<Emit> {
        let mut immediate = Vec::new();
        for child in element.children().filter(Node::is_element) {
            let delay = child.attribute("delay").and_then(parse_delay);
            match (is_scxml(child, "send"), child.attribute("event"), delay) {
                (true, Some(event), Some(millis)) if child.attribute("target").is_none() => {
                    let with = self.params(child);
                    let emit = Emit { event: event.to_owned(), route: Route::All, with };
                    self.delayed.push(DelayedSend { chart, node, event: event.to_owned(), millis, emit });
                }
                _ => immediate.extend(self.action(child)),
            }
        }
        immediate
    }

    /// `<assign location expr>` children of a transition as `sets`: kept
    /// when `expr` is a Cascade operand (in a foreign document, only one
    /// reading `event.x` or `self.x`), else ignored with a warning.
    fn assigns(&mut self, element: Node) -> Vec<Assignment> {
        let mut out = Vec::new();
        for child in element.children().filter(|n| is_scxml(*n, "assign")) {
            let location = self.location(child);
            let (Some(field), Some(expr)) = (child.attribute("location"), child.attribute("expr")) else {
                self.warnings.ignored(location, "an <assign> without a static `location` and `expr`");
                continue;
            };
            match self.operand(expr) {
                Some(value) if cascade_core::parse::grammar::is_valid_name(field) => {
                    out.push(Assignment::new(field, value));
                }
                _ => {
                    self.warnings.ignored(location, format!("an <assign> to `{field}` of `{expr}` (not a field copy)"))
                }
            }
        }
        out
    }

    /// `<param name expr>` children of a `<send>` as emit `with:`, read
    /// like [`Self::assigns`].
    fn params(&mut self, send: Node) -> Vec<Assignment> {
        let mut out = Vec::new();
        for child in send.children().filter(|n| is_scxml(*n, "param")) {
            let location = self.location(child);
            let (Some(name), Some(expr)) = (child.attribute("name"), child.attribute("expr")) else {
                self.warnings.ignored(location, "a <param> without a `name` and `expr`");
                continue;
            };
            match self.operand(expr) {
                Some(value) if cascade_core::parse::grammar::is_valid_name(name) => {
                    out.push(Assignment::new(name, value));
                }
                _ => self.warnings.ignored(location, format!("a <param> `{name}` of `{expr}` (not a field copy)")),
            }
        }
        out
    }

    /// An `expr` as a Cascade operand: any operand in Cascade's own export,
    /// only `event.x`/`self.x` in a foreign document.
    fn operand(&self, expr: &str) -> Option<cascade_core::expr::Operand> {
        let operand = parse_operand(expr.trim()).ok()?;
        (self.annotated || !operand.is_literal()).then_some(operand)
    }

    /// Events raised or sent by executable content, including inside
    /// `<if>`/`<foreach>` blocks.
    fn executable(&mut self, element: Node) -> Vec<Emit> {
        let mut out = Vec::new();
        for child in element.children().filter(Node::is_element) {
            out.extend(self.action(child));
        }
        out
    }

    /// The events one executable element raises or sends.
    fn action(&mut self, child: Node) -> Vec<Emit> {
        if !matches!(child.tag_name().namespace(), None | Some(SCXML_NS)) {
            return Vec::new();
        }
        match child.tag_name().name() {
            "raise" => match child.attribute("event") {
                Some(event) => vec![Emit::new(event, Route::All)],
                None => {
                    self.warnings.ignored(self.location(child), "a <raise> without an `event`");
                    Vec::new()
                }
            },
            "send" => match child.attribute("event") {
                Some(event) => {
                    let route = match child.attribute("target") {
                        None | Some("#_internal") => Route::All,
                        Some("#_parent") => Route::Others,
                        Some(t) => match t.strip_prefix("#_scxml_").or_else(|| t.strip_prefix("#_")) {
                            Some(id) => Route::Machine(id.to_owned()),
                            None => Route::Others,
                        },
                    };
                    let with = self.params(child);
                    vec![Emit { event: event.to_owned(), route, with }]
                }
                None => {
                    self.warnings.ignored(self.location(child), "a <send> without a static `event`");
                    Vec::new()
                }
            },
            "if" | "elseif" | "else" | "foreach" => self.executable(child),
            _ => Vec::new(),
        }
    }

    // --- Cascade annotations ---------------------------------------------------------

    fn cascade_event(&mut self, element: Node) -> Result<(), InteropError> {
        let location = self.location(element);
        let name = element.attribute("name").ok_or_else(|| invalid(&location, "<cascade:event> needs a `name`"))?;
        let mut payload: Vec<FieldDef> = element
            .attribute("payload")
            .map(|p| p.split_whitespace().map(FieldDef::untyped).collect())
            .unwrap_or_default();
        payload.extend(typed_fields(element, &location)?);
        self.annotations.events.push(AnnotatedEvent { name: name.to_owned(), payload, location });
        Ok(())
    }

    fn cascade_source(&mut self, element: Node) -> Result<(), InteropError> {
        let location = self.location(element);
        let name = element.attribute("name").ok_or_else(|| invalid(&location, "<cascade:source> needs a `name`"))?;
        let triggers = element
            .attribute("fires")
            .unwrap_or_default()
            .split_whitespace()
            .map(|t| split_trigger(t, &location))
            .collect::<Result<Vec<_>, _>>()?;
        self.annotations.sources.push(AnnotatedSource { name: name.to_owned(), triggers, location });
        Ok(())
    }

    /// A `cascade:controller` region: one transition per subscribed event,
    /// one `<send>` per rule (inside `<if cond>` for a rule with `when`).
    fn controller(&mut self, region: Node) -> Result<(), InteropError> {
        let location = self.location(region);
        let name = cascade_attr(region, "controller").unwrap_or_default().to_owned();
        let mut handlers: Vec<AnnotatedHandler> = Vec::new();
        for transition in region.children().filter(|n| is_scxml(*n, "transition")) {
            let at = self.location(transition);
            let event = transition
                .attribute("event")
                .map(str::trim)
                .filter(|e| !e.is_empty())
                .ok_or_else(|| invalid(&at, "a controller transition needs an `event`"))?;
            let mut rules = Vec::new();
            for child in transition.children().filter(Node::is_element) {
                if is_scxml(child, "send") {
                    rules.push(self.rule(child, None)?);
                } else if is_scxml(child, "if") {
                    let when = child.attribute("cond").map(str::to_owned);
                    for send in child.children().filter(|n| is_scxml(*n, "send")) {
                        rules.push(self.rule(send, when.clone())?);
                    }
                }
            }
            match handlers.iter_mut().find(|h| h.event == event) {
                Some(handler) => handler.rules.extend(rules),
                None => handlers.push(AnnotatedHandler { event: event.to_owned(), rules }),
            }
        }
        self.annotations.controllers.push(AnnotatedController { name, location, handlers });
        Ok(())
    }

    fn rule(&mut self, send: Node, when: Option<String>) -> Result<AnnotatedRule, InteropError> {
        let location = self.location(send);
        let event = send.attribute("event").ok_or_else(|| invalid(&location, "a rule <send> needs an `event`"))?;
        let (machine, trigger) = split_trigger(event, &location)?;
        Ok(AnnotatedRule {
            machine,
            trigger,
            target: cascade_attr(send, "target").map(str::to_owned),
            when,
            bounded: cascade_attr(send, "bounded") == Some("true"),
            location,
        })
    }

    // --- Resolution --------------------------------------------------------------------

    fn lookup(&self, chart: usize, id: &str, location: &str) -> Result<NodeIx, InteropError> {
        match self.ids.get(id) {
            Some(&(c, ix)) if c == chart => Ok(ix),
            Some(_) => Err(unsupported(location, format!("a transition or initial state `{id}` in another machine"))),
            None => Err(invalid(location, format!("no state has the id `{id}`"))),
        }
    }

    fn resolve(&mut self) -> Result<(), InteropError> {
        for (chart, node, raw, location) in std::mem::take(&mut self.initials) {
            let ids: Vec<&str> = raw.split_whitespace().collect();
            let [id] = ids.as_slice() else {
                return Err(unsupported(&location, "several initial states (they need parallel states)"));
            };
            let target = self.lookup(chart, id, &location)?;
            let c = &self.charts[chart];
            if target == node || !c.is_descendant_or_self(target, node) {
                return Err(invalid(&location, format!("initial state `{id}` is not inside this state")));
            }
            self.charts[chart].node_mut(node).initial = Some(target);
        }
        let delayed = std::mem::take(&mut self.delayed);
        let mut timers_used = vec![false; delayed.len()];
        for edge in std::mem::take(&mut self.edges) {
            let target = match &edge.target {
                None => None,
                Some(id) => Some(self.lookup(edge.chart, id, &edge.location)?),
            };
            let timer = delayed
                .iter()
                .position(|d| d.chart == edge.chart && d.node == edge.source && d.event == edge.raw_event);
            let trigger = match (timer, edge.subscription) {
                (Some(i), _) => {
                    timers_used[i] = true;
                    TriggerSpec::After(Delay::Millis(delayed[i].millis))
                }
                (None, Some(filter)) => TriggerSpec::Subscription { event: edge.trigger, filter },
                (None, None) => TriggerSpec::Event(edge.trigger),
            };
            self.charts[edge.chart].node_mut(edge.source).edges.push(Edge {
                trigger,
                target,
                guard: edge.guard,
                sets: edge.sets,
                emits: edge.emits,
                reenter: edge.reenter,
                bounded: edge.bounded,
                location: edge.location,
            });
        }
        // Delayed sends no transition of their state takes are plain emits.
        for (send, used) in delayed.into_iter().zip(timers_used) {
            if !used {
                self.charts[send.chart].node_mut(send.node).entry.push(send.emit);
            }
        }
        Ok(())
    }
}

/// `<cascade:field name type/>` children: typed fields.
fn typed_fields(element: Node, location: &str) -> Result<Vec<FieldDef>, InteropError> {
    element
        .children()
        .filter(|n| is_cascade(*n, "field"))
        .map(|field| {
            let name = field.attribute("name").ok_or_else(|| invalid(location, "<cascade:field> needs a `name`"))?;
            let ty = field.attribute("type").unwrap_or("any");
            let ty = parse_field_type(ty)
                .map_err(|err| invalid(location, format!("invalid type `{ty}` of field `{name}`: {err}")))?;
            Ok(FieldDef::typed(name, ty))
        })
        .collect()
}

/// An SCXML delay (`30s`, `1.5s`, `250ms`) in milliseconds, when it is a
/// whole positive number of them.
fn parse_delay(text: &str) -> Option<u64> {
    let text = text.trim();
    let (number, scale) = match text.strip_suffix("ms") {
        Some(n) => (n, 1.0),
        None => (text.strip_suffix('s')?, 1000.0),
    };
    if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let millis = number.parse::<f64>().ok()? * scale;
    (millis >= 1.0 && millis.fract() == 0.0 && millis < u64::MAX as f64).then_some(millis as u64)
}

/// `Machine.trigger` → (machine, trigger).
fn split_trigger(text: &str, location: &str) -> Result<(String, String), InteropError> {
    match text.split_once('.') {
        Some((machine, trigger)) if !machine.is_empty() && !trigger.is_empty() => {
            Ok((machine.to_owned(), trigger.to_owned()))
        }
        _ => Err(invalid(location, format!("`{text}` is not `Machine.trigger`"))),
    }
}
