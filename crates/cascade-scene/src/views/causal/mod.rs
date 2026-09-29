//! Causal flow view (the default): what does this set off, and what can
//! cause it?
//!
//! Draws the causal graph: external sources (rectangles, pinned to the
//! first layer), transition pills with an input port West (triggers and
//! fires arrive) and an output port East (emits leave), event tags and one
//! controller hexagon per handler. States are hidden; pills carry both
//! endpoints. Layers run left to right from the external sources, and edges
//! the layout reverses to break a causal cycle are drawn red.
//!
//! Structural filters, applied before layout (so they relayout):
//! the machine pair (matrix click), hidden machines (collapsed to a stub
//! keeping their links, counted), and hide mode (outside the focus removed,
//! cut links left as stubs). Emphasis is applied after layout.
//!
//! With `ViewState::group_by_machine` the same graph is laid out in one
//! lane per machine on shared causal columns (see [`lanes`]).

mod lanes;

use std::collections::{BTreeSet, HashMap};

use cascade_core::{CausalEdgeKind, CausalNode, EdgeIx, ElementRef, MachineId, NodeIx};
use cascade_layout::{LayerConstraint, LayoutOptions, Port, PortSide};

use crate::color::machine_styles;
use crate::emphasis::{Anchor, Interaction};
use crate::scene::{Arrow, EdgeKind, HitTarget, Scene, Stroke};
use crate::view_state::ViewKind;
use crate::views::cache::LayoutCache;
use crate::views::decorate::{Decor, FindingIndex, scene_bounds};
use crate::views::draft::{DraftEdge, DraftGraph, DraftNode, EdgeText, Meta, RealizeCtx, realize};
use crate::views::filters::{apply_hide, hidden_machines, machine_pair, pair_nodes};
use crate::views::links::cycle_edges;
use crate::views::overlays::{Placement, PlayDecor};
use crate::views::style::{Painter, bracketed};
use crate::views::{SceneError, SceneInput};
use lanes::{LaneGroups, LaneOf, LaneRules};

/// Pill ports: input West, output East.
const PORT_IN: u16 = 0;
const PORT_OUT: u16 = 1;

/// A link into or out of a hidden machine, aggregated per endpoint pair
/// and kind.
struct StubLink {
    from: usize,
    to: usize,
    stroke: Stroke,
    elements: Vec<ElementRef>,
    edges: Vec<EdgeIx>,
    target: HitTarget,
}

pub(super) fn build(
    input: &SceneInput<'_>,
    interaction: &Interaction,
    cache: &mut LayoutCache,
) -> Result<Scene, SceneError> {
    let model = input.model;
    let graph = input.graph;
    let theme = input.theme;
    let painter = Painter { theme, measure: input.measure, styles: machine_styles(model, theme) };
    let mut notes: Vec<String> = interaction.notes().to_vec();

    let pair = machine_pair(model, input.view.machine_pair.as_ref(), &mut notes);
    let included = pair_nodes(model, graph, pair);
    let hidden = hidden_machines(model, &input.view.hidden_machines);
    let on_cycle = cycle_edges(graph);
    let grouped = input.view.group_by_machine;
    let rules = grouped.then(|| LaneRules::new(model));

    let mut draft = DraftGraph::default();
    // The lane of every draft node, for causal lanes.
    let mut lane_of: Vec<LaneOf> = Vec::new();
    let mut slot: Vec<Option<usize>> = vec![None; graph.node_count()];
    let mut stubs: Vec<(MachineId, usize, Vec<NodeIx>)> = Vec::new();
    for (ix, node) in graph.nodes() {
        if !included[ix.index()] {
            continue;
        }
        if let CausalNode::Transition(t) = node {
            let machine = model.transition(t).machine;
            if hidden.contains(&machine) {
                let at = match stubs.iter().position(|(m, _, _)| *m == machine) {
                    Some(at) => at,
                    None => {
                        let n = draft.add_node(stub_node(input, &painter, machine));
                        lane_of.push(LaneOf::Machine(machine));
                        stubs.push((machine, n, Vec::new()));
                        stubs.len() - 1
                    }
                };
                stubs[at].2.push(ix);
                slot[ix.index()] = Some(stubs[at].1);
                continue;
            }
        }
        slot[ix.index()] = Some(draft.add_node(causal_node(input, &painter, ix, node, grouped)));
        lane_of.push(rules.as_ref().map_or(LaneOf::Unattached, |r| r.of(model, node)));
    }
    // Hidden machines with nothing in the graph still get a stub, unless a
    // pair filter excludes them.
    for &machine in &hidden {
        let in_scope = pair.is_none_or(|p| p.contains(&machine));
        if in_scope && !stubs.iter().any(|(m, _, _)| *m == machine) {
            let n = draft.add_node(stub_node(input, &painter, machine));
            lane_of.push(LaneOf::Machine(machine));
            stubs.push((machine, n, Vec::new()));
        }
    }
    let stub_slots: BTreeSet<usize> = stubs.iter().map(|(_, n, _)| *n).collect();

    let mut stub_links: Vec<StubLink> = Vec::new();
    let mut stub_link_at: HashMap<(usize, usize, EdgeKind), usize> = HashMap::new();
    for (e, edge) in graph.edges() {
        let (Some(from), Some(to)) = (slot[edge.from.index()], slot[edge.to.index()]) else { continue };
        let (kind, stroke, target, elements, label) = link_style(input, &painter, edge.kind, edge.to);
        if stub_slots.contains(&from) || stub_slots.contains(&to) {
            let at = *stub_link_at.entry((from, to, kind)).or_insert_with(|| {
                stub_links.push(StubLink {
                    from,
                    to,
                    stroke: painter.stub_link_stroke(stroke),
                    elements: Vec::new(),
                    edges: Vec::new(),
                    target: target.clone(),
                });
                stub_links.len() - 1
            });
            let link = &mut stub_links[at];
            link.edges.push(e);
            for el in elements {
                if !link.elements.contains(&el) {
                    link.elements.push(el);
                }
            }
            continue;
        }
        draft.edges.push(DraftEdge {
            from,
            from_port: (kind == EdgeKind::Emit).then_some(PORT_OUT),
            to,
            to_port: matches!(kind, EdgeKind::Trigger | EdgeKind::Fire).then_some(PORT_IN),
            kind,
            stroke,
            arrow: Arrow::End,
            label: label.map(|text| EdgeText { text, font_size: theme.small_font_size, color: theme.text }),
            target,
            meta: Meta::new(elements, Anchor::Chains(vec![vec![e]])),
            on_cycle: on_cycle[e.index()],
        });
    }

    // Stub labels count the links actually drawn to them.
    for (machine, n, members) in &stubs {
        let links = stub_links.iter().filter(|l| l.from == *n || l.to == *n).count();
        let links = u32::try_from(links).unwrap_or(u32::MAX);
        let name = &model.machine(*machine).name;
        let node = &mut draft.nodes[*n];
        node.look = painter.stub(name, links, painter.machine(*machine));
        node.target = HitTarget::MachineStub { machine: name.clone(), links };
        node.meta.anchor = Anchor::Nodes(members.clone());
    }
    for link in stub_links {
        let count = link.edges.len();
        draft.edges.push(DraftEdge {
            from: link.from,
            from_port: None,
            to: link.to,
            to_port: None,
            kind: EdgeKind::StubLink,
            stroke: link.stroke,
            arrow: Arrow::End,
            label: (count > 1).then(|| EdgeText {
                text: format!("×{count}"),
                font_size: theme.small_font_size,
                color: theme.text_muted,
            }),
            target: link.target,
            meta: Meta::new(link.elements, Anchor::Chains(link.edges.into_iter().map(|e| vec![e]).collect())),
            on_cycle: false,
        });
    }

    if draft.nodes.is_empty() {
        notes.push("Nothing to draw: no transitions, events, controllers or sources in view.".to_owned());
    }

    let lane_groups = grouped.then(|| {
        let groups = LaneGroups::add(&mut draft, model, &painter);
        for (node, lane) in draft.nodes.iter_mut().zip(&lane_of) {
            node.group = Some(groups.group(*lane));
        }
        groups
    });

    let cuts = apply_hide(&mut draft, interaction);
    let ctx = RealizeCtx {
        view: ViewKind::Causal,
        theme,
        measure: input.measure,
        sidecar: input.sidecar,
        options: LayoutOptions { shared_layers: grouped, ..LayoutOptions::default() },
    };
    let realized = realize(draft, cuts, &ctx, cache)?;
    let mut scene = realized.scene;
    let decor = Decor { model, theme, interaction, findings: FindingIndex::new(input.findings), diff: input.diff };
    decor.apply(&mut scene, &realized.nodes, &realized.edges, &realized.overlay_owner);
    if let Some(groups) = &lane_groups {
        let group_rect = |g: usize| realized.groups.get(g).copied().flatten();
        groups.draw(&mut scene, input, &painter, interaction, &hidden, group_rect);
    }
    if let Some(play) = input.play {
        let play_decor = PlayDecor { model, painter: &painter };
        play_decor.apply(&mut scene, &realized.nodes, &realized.edges, play, Placement::Transitions);
    }
    scene.bounds = scene_bounds(&scene, input.measure);
    scene.notes = notes;
    Ok(scene)
}

/// The draft node of a causal node. In causal lanes a pill leaves out its
/// machine, which its lane names.
fn causal_node(
    input: &SceneInput<'_>,
    painter: &Painter<'_>,
    ix: NodeIx,
    node: CausalNode,
    grouped: bool,
) -> DraftNode {
    let model = input.model;
    let element = node.element();
    let key = model.key_of(element);
    let (look, layer, ports, meta) = match node {
        CausalNode::External(x) => (
            painter.external(model.external(x).name.clone()),
            LayerConstraint::First,
            Vec::new(),
            Meta::new(vec![element], Anchor::Nodes(vec![ix])),
        ),
        CausalNode::Transition(t) => {
            let tr = model.transition(t);
            let label = if grouped {
                format!("{} → {}", model.state(tr.from).path, model.state(tr.to).path)
            } else {
                model.transition_label(t)
            };
            (
                painter.pill(label, model.trigger(tr.trigger).name.clone(), painter.machine(tr.machine)),
                LayerConstraint::Free,
                vec![Port { side: PortSide::West }, Port { side: PortSide::East }],
                Meta::new(vec![element], Anchor::Nodes(vec![ix])),
            )
        }
        CausalNode::Event(ev) => (
            painter.tag(model.event(ev).name.clone()),
            LayerConstraint::Free,
            Vec::new(),
            Meta::new(vec![element], Anchor::Nodes(vec![ix])),
        ),
        CausalNode::Handler(h) => {
            let handler = model.handler(h);
            let mut meta =
                Meta::new(vec![element, ElementRef::Controller(handler.controller)], Anchor::Nodes(vec![ix]));
            meta.badge_elements = handler.rules.iter().map(|&r| ElementRef::Rule(r)).collect();
            (
                painter.hexagon(model.controller(handler.controller).name.clone()),
                LayerConstraint::Free,
                Vec::new(),
                meta,
            )
        }
    };
    DraftNode { key: key.to_string(), group: None, layer, ports, look, target: HitTarget::Element(key), meta }
}

/// A placeholder stub; its look and target are finished once its links
/// are counted.
fn stub_node(input: &SceneInput<'_>, painter: &Painter<'_>, machine: MachineId) -> DraftNode {
    let name = &input.model.machine(machine).name;
    let key = input.model.key_of(ElementRef::Machine(machine));
    DraftNode {
        key: key.to_string(),
        group: None,
        layer: LayerConstraint::Free,
        ports: Vec::new(),
        look: painter.stub(name, 0, painter.machine(machine)),
        target: HitTarget::MachineStub { machine: name.clone(), links: 0 },
        meta: Meta::new(vec![ElementRef::Machine(machine)], Anchor::Nodes(Vec::new())),
    }
}

/// Kind, stroke, hit target, elements and label of one causal edge.
fn link_style(
    input: &SceneInput<'_>,
    painter: &Painter<'_>,
    kind: CausalEdgeKind,
    to: NodeIx,
) -> (EdgeKind, Stroke, HitTarget, Vec<ElementRef>, Option<String>) {
    let model = input.model;
    let guard_of = |node: NodeIx| match input.graph.node(node) {
        CausalNode::Transition(t) => model.transition(t).guard.clone(),
        _ => None,
    };
    match kind {
        CausalEdgeKind::Trigger { trigger } => {
            let el = ElementRef::Trigger(trigger);
            (
                EdgeKind::Trigger,
                painter.trigger_stroke(),
                HitTarget::Element(model.key_of(el)),
                vec![el],
                bracketed(guard_of(to).as_deref()),
            )
        }
        CausalEdgeKind::Emit => (EdgeKind::Emit, painter.emit_stroke(), HitTarget::None, Vec::new(), None),
        CausalEdgeKind::Subscribe => {
            (EdgeKind::Subscribe, painter.subscribe_stroke(), HitTarget::None, Vec::new(), None)
        }
        CausalEdgeKind::Fire { rule } => {
            let el = ElementRef::Rule(rule);
            let r = model.rule(rule);
            let target_machine = model.trigger(r.trigger).machine;
            let parts: Vec<String> =
                [bracketed(r.condition.as_deref()), bracketed(guard_of(to).as_deref())].into_iter().flatten().collect();
            (
                EdgeKind::Fire,
                painter.fire_stroke(painter.machine(target_machine)),
                HitTarget::Element(model.key_of(el)),
                vec![el],
                (!parts.is_empty()).then(|| parts.join(" ")),
            )
        }
    }
}
