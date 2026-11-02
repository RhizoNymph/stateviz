//! `flowchart LR`: the causal graph.
//!
//! Representation (ids are generated, so always safe):
//!
//! | Element | Node |
//! | --- | --- |
//! | External source | parallelogram `s0[/"Customer"/]` |
//! | Transition | stadium pill `t0(["Order: pending → paid"])`, filled with its machine's hue |
//! | Event | tag `e0>"OrderPaid"]`, gray |
//! | Controller | hexagon `c0{{"Fulfillment"}}`, outline only |
//! | Trigger nothing accepts | box `u0["Shipment.restart (no transition)"]`, dashed vermillion outline |
//!
//! Edges, in this order: source → transition (solid, labelled with the
//! trigger), transition → event (dashed emit), event → controller (solid,
//! one per subscription), controller → transition (dashed fire, labelled
//! `trigger [when]`, colored with the target machine's hue), event →
//! subscribing transition (thick `==>` notify, labelled with its `where`
//! filter, machine hue), transition → timer transition (dotted delay
//! labelled `after 30s`, machine hue with a short dash). A trigger edge
//! or fire goes to every transition that accepts the trigger, as in the
//! causal graph; one that nothing accepts goes to the trigger's `u` node.
//!
//! Limits: controllers are one node each, so a controller subscribed to
//! several events merges its handlers (the fire labels name the trigger,
//! not the event); target selectors are not shown; Mermaid chooses the
//! layout, so depth from the sources is only approximated by `LR` ranking.

use std::fmt::Write as _;

use cascade_core::causal::{CausalEdgeKind, CausalGraph, CausalNode};
use cascade_core::model::TriggerKind;
use cascade_core::{Model, TransitionId, TriggerId};

use super::escape;
use super::palette::{Hue, machine_hues};

const INDENT: &str = "    ";
const EMIT_STROKE: &str = "#888888";

/// The causal graph as a Mermaid `flowchart LR`.
pub(crate) fn causal(model: &Model) -> String {
    let hues = machine_hues(model);
    let mut out = String::from("flowchart LR\n");

    // --- Nodes -------------------------------------------------------------
    for (xid, source) in model.externals() {
        let _ = writeln!(out, "{INDENT}s{}[/\"{}\"/]", xid.index(), escape::label(&source.name));
    }
    for (tid, _) in model.transitions() {
        let _ = writeln!(out, "{INDENT}t{}([\"{}\"])", tid.index(), escape::label(&model.transition_label(tid)));
    }
    for (eid, event) in model.events() {
        let _ = writeln!(out, "{INDENT}e{}>\"{}\"]", eid.index(), escape::label(&event.name));
    }
    for (cid, controller) in model.controllers() {
        let _ = writeln!(out, "{INDENT}c{}{{{{\"{}\"}}}}", cid.index(), escape::label(&controller.name));
    }
    // Triggers that something fires but no transition accepts.
    let mut missing: Vec<Option<usize>> = vec![None; model.trigger_count()];
    let mut missing_count = 0usize;
    for (trid, trigger) in model.triggers() {
        if trigger.accepted_by.is_empty() && (!trigger.fired_by.is_empty() || !trigger.sources.is_empty()) {
            let label = format!("{}.{} (no transition)", model.machine(trigger.machine).name, trigger.name);
            let _ = writeln!(out, "{INDENT}u{missing_count}[\"{}\"]", escape::label(&label));
            missing[trid.index()] = Some(missing_count);
            missing_count += 1;
        }
    }

    // --- Edges ---------------------------------------------------------------
    let mut edges = Edges::default();
    for (xid, source) in model.externals() {
        for &trigger in &source.triggers {
            let label = escape::label(&model.trigger(trigger).name);
            for target in targets(model, trigger, &missing) {
                edges.push(format!("s{} -->|\"{label}\"| {target}", xid.index()), None);
            }
        }
    }
    for (tid, transition) in model.transitions() {
        for emit in &transition.emits {
            edges.push(format!("t{} -.-> e{}", tid.index(), emit.event.index()), Some(format!("stroke:{EMIT_STROKE}")));
        }
    }
    for (_, handler) in model.handlers() {
        edges.push(format!("e{} --> c{}", handler.event.index(), handler.controller.index()), None);
    }
    for (_, rule) in model.rules() {
        let trigger = model.trigger(rule.trigger);
        let mut label = trigger.name.clone();
        if let Some(condition) = &rule.condition {
            let _ = write!(label, " [{condition}]");
        }
        let label = escape::label(&label);
        let stroke = hues.get(trigger.machine.index()).map(|h| format!("stroke:{}", h.fill));
        for target in targets(model, rule.trigger, &missing) {
            edges.push(format!("c{} -.->|\"{label}\"| {target}", rule.controller.index()), stroke.clone());
        }
    }
    // Subscriptions and timers come from the causal graph.
    let graph = CausalGraph::build(model);
    for (_, edge) in graph.edges() {
        let (CausalNode::Event(_) | CausalNode::Transition(_), CausalNode::Transition(to)) =
            (graph.node(edge.from), graph.node(edge.to))
        else {
            continue;
        };
        let hue = hues.get(model.transition(to).machine.index()).map(|h| h.fill);
        match (edge.kind, graph.node(edge.from)) {
            (CausalEdgeKind::Notify { .. }, CausalNode::Event(event)) => {
                let line = match &model.transition(to).filter {
                    Some(filter) => {
                        let label = escape::label(&format!("where {}", filter.display_filter()));
                        format!("e{} ==>|\"{label}\"| t{}", event.index(), to.index())
                    }
                    None => format!("e{} ==> t{}", event.index(), to.index()),
                };
                edges.push(line, hue.map(|h| format!("stroke:{h}")));
            }
            (CausalEdgeKind::Delay { trigger }, CausalNode::Transition(from)) => {
                let label = match model.trigger(trigger).kind {
                    TriggerKind::Timer(delay) => format!("after {delay}"),
                    TriggerKind::Command | TriggerKind::Event(_) => model.trigger(trigger).name.clone(),
                };
                let line = format!("t{} -.->|\"{}\"| t{}", from.index(), escape::label(&label), to.index());
                edges.push(line, hue.map(|h| format!("stroke:{h},stroke-dasharray:2 4")));
            }
            _ => {}
        }
    }
    for line in &edges.lines {
        let _ = writeln!(out, "{INDENT}{line}");
    }

    // --- Styles --------------------------------------------------------------
    write_styles(&mut out, model, &hues, missing_count);
    for (style, indices) in edges.styles() {
        let list: Vec<String> = indices.iter().map(ToString::to_string).collect();
        let _ = writeln!(out, "{INDENT}linkStyle {} {style}", list.join(","));
    }
    out
}

/// Node ids a trigger leads to: every accepting transition, or the
/// trigger's placeholder node.
fn targets(model: &Model, trigger: TriggerId, missing: &[Option<usize>]) -> Vec<String> {
    let accepted: &[TransitionId] = &model.trigger(trigger).accepted_by;
    if accepted.is_empty() {
        missing.get(trigger.index()).copied().flatten().map(|u| format!("u{u}")).into_iter().collect()
    } else {
        accepted.iter().map(|t| format!("t{}", t.index())).collect()
    }
}

/// Edge lines with their link styles, in output order (Mermaid addresses
/// links by index in `linkStyle`).
#[derive(Default)]
struct Edges {
    lines: Vec<String>,
    styles: Vec<Option<String>>,
}

impl Edges {
    fn push(&mut self, line: String, style: Option<String>) {
        self.lines.push(line);
        self.styles.push(style);
    }

    /// Edge indices grouped by style, styles in order of first use.
    fn styles(&self) -> Vec<(&str, Vec<usize>)> {
        let mut groups: Vec<(&str, Vec<usize>)> = Vec::new();
        for (index, style) in self.styles.iter().enumerate() {
            let Some(style) = style.as_deref() else { continue };
            match groups.iter_mut().find(|(s, _)| *s == style) {
                Some((_, indices)) => indices.push(index),
                None => groups.push((style, vec![index])),
            }
        }
        groups
    }
}

/// `classDef` and `class` lines. Style lines never end with `;`: Mermaid
/// would read `#hex;` as an entity code.
fn write_styles(out: &mut String, model: &Model, hues: &[Hue], missing_count: usize) {
    for (mid, machine) in model.machines() {
        let Some(hue) = hues.get(mid.index()) else { continue };
        let _ = writeln!(
            out,
            "{INDENT}classDef machine{} fill:{},stroke:{},color:{}",
            mid.index(),
            hue.fill,
            hue.fill,
            hue.text
        );
        if !machine.transitions.is_empty() {
            let members: Vec<String> = machine.transitions.iter().map(|t| format!("t{}", t.index())).collect();
            let _ = writeln!(out, "{INDENT}class {} machine{}", members.join(","), mid.index());
        }
    }
    let groups = [
        ("cascadeSource", "fill:#ffffff,stroke:#555555,color:#333333", 's', model.external_count()),
        ("cascadeEvent", "fill:#eeeeee,stroke:#888888,color:#333333", 'e', model.event_count()),
        ("cascadeController", "fill:none,stroke:#333333,stroke-width:2px,color:#333333", 'c', model.controller_count()),
        ("cascadeMissing", "fill:#ffffff,stroke:#D55E00,stroke-dasharray:4,color:#333333", 'u', missing_count),
    ];
    for (class, style, prefix, count) in groups {
        if count == 0 {
            continue;
        }
        let _ = writeln!(out, "{INDENT}classDef {class} {style}");
        let members: Vec<String> = (0..count).map(|i| format!("{prefix}{i}")).collect();
        let _ = writeln!(out, "{INDENT}class {} {class}", members.join(","));
    }
}
