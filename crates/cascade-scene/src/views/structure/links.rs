//! Cross-machine links in the structure view: one dashed edge per pair of
//! endpoints, straight from the causing pill to the caused pill, labelled
//! with the event and controller it runs through ("OrderPaid ›
//! Fulfillment").

use std::collections::HashMap;

use cascade_core::{CausalGraph, EdgeIx, ElementRef, MachineId, Model, RuleId};

use crate::emphasis::Anchor;
use crate::scene::{Arrow, EdgeKind, HitTarget};
use crate::views::draft::{DraftEdge, DraftGraph, EdgeText, Meta};
use crate::views::links::causal_links;
use crate::views::structure::machines::{EndKind, Endpoint, PORT_EAST, PORT_NORTH, PORT_SOUTH, PORT_WEST};
use crate::views::style::Painter;

struct Link {
    from: Endpoint,
    to: Endpoint,
    target_machine: MachineId,
    rules: Vec<RuleId>,
    labels: Vec<String>,
    chains: Vec<Vec<EdgeIx>>,
}

/// Add the cross-lane links, then finish every stub with its link count.
pub(super) fn draft_links(
    draft: &mut DraftGraph,
    model: &Model,
    graph: &CausalGraph,
    painter: &Painter<'_>,
    endpoints: &[Option<Endpoint>],
    stubs: &[(MachineId, usize)],
) {
    let theme = painter.theme;
    let mut links: Vec<Link> = Vec::new();
    let mut at: HashMap<(usize, usize), usize> = HashMap::new();
    for link in causal_links(graph) {
        let (Some(Some(from)), Some(Some(to))) = (endpoints.get(link.from.index()), endpoints.get(link.to.index()))
        else {
            continue;
        };
        let i = *at.entry((from.node, to.node)).or_insert_with(|| {
            links.push(Link {
                from: *from,
                to: *to,
                target_machine: model.transition(link.to).machine,
                rules: Vec::new(),
                labels: Vec::new(),
                chains: Vec::new(),
            });
            links.len() - 1
        });
        let entry = &mut links[i];
        let rule = model.rule(link.rule);
        let label = format!("{} › {}", model.event(rule.event).name, model.controller(rule.controller).name);
        if !entry.labels.contains(&label) {
            entry.labels.push(label);
        }
        if !entry.rules.contains(&link.rule) {
            entry.rules.push(link.rule);
        }
        entry.chains.push(link.chain.to_vec());
    }

    let mut stub_links: HashMap<usize, u32> = HashMap::new();
    for link in links {
        let through_stub = link.from.kind == EndKind::Stub || link.to.kind == EndKind::Stub;
        for end in [link.from, link.to] {
            if end.kind == EndKind::Stub {
                *stub_links.entry(end.node).or_insert(0) += 1;
            }
        }
        let fire = painter.fire_stroke(painter.machine(link.target_machine));
        let (from_port, to_port) = if link.from.kind == EndKind::Pill && link.to.kind == EndKind::Pill {
            let (ga, gb) = (draft.nodes[link.from.node].group, draft.nodes[link.to.node].group);
            match ga.cmp(&gb) {
                std::cmp::Ordering::Equal => (Some(PORT_EAST), Some(PORT_WEST)),
                std::cmp::Ordering::Less => (Some(PORT_SOUTH), Some(PORT_NORTH)),
                std::cmp::Ordering::Greater => (Some(PORT_NORTH), Some(PORT_SOUTH)),
            }
        } else {
            (None, None)
        };
        let elements: Vec<ElementRef> = link.rules.iter().map(|&r| ElementRef::Rule(r)).collect();
        draft.edges.push(DraftEdge {
            from: link.from.node,
            from_port,
            to: link.to.node,
            to_port,
            kind: if through_stub { EdgeKind::StubLink } else { EdgeKind::Fire },
            stroke: if through_stub { painter.stub_link_stroke(fire) } else { fire },
            arrow: Arrow::End,
            label: Some(EdgeText { text: link.labels.join(", "), font_size: theme.small_font_size, color: theme.text }),
            target: elements.first().map_or(HitTarget::None, |e| HitTarget::Element(model.key_of(*e))),
            meta: Meta::new(elements, Anchor::Chains(link.chains)),
            on_cycle: false,
        });
    }

    for &(machine, node) in stubs {
        let links = stub_links.get(&node).copied().unwrap_or(0);
        let name = &model.machine(machine).name;
        let stub = &mut draft.nodes[node];
        stub.look = painter.stub(name, links, painter.machine(machine));
        stub.target = HitTarget::MachineStub { machine: name.clone(), links };
    }
}
