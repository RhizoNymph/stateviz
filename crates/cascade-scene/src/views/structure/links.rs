//! Cross-machine links in the structure view: one edge per pair of
//! endpoints and kind, straight from the causing pill (or arrow) to the
//! caused one. Links through a controller rule are fires (dashed in the
//! target's hue), labelled with the event and controller they run through
//! ("OrderPaid › Fulfillment"); links through a machine subscription are
//! `Notify` edges (solid in the subscriber's hue), labelled with the event
//! ("PaymentCaptured").

use std::collections::HashMap;

use cascade_core::model::TriggerKind;
use cascade_core::{CausalGraph, EdgeIx, ElementRef, MachineId, Model};

use crate::emphasis::Anchor;
use crate::scene::{Arrow, EdgeKind, FontStyle, HitTarget};
use crate::views::draft::{DraftEdge, DraftGraph, EdgeText, Meta};
use crate::views::links::{causal_links, subscription_links};
use crate::views::structure::machines::{EndKind, Endpoint, PORT_EAST, PORT_NORTH, PORT_SOUTH, PORT_WEST};
use crate::views::style::Painter;

/// One drawn link: what it runs through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Through {
    Rule,
    Subscription,
}

struct Link {
    from: Endpoint,
    to: Endpoint,
    through: Through,
    target_machine: MachineId,
    /// Rules (fires) or subscription triggers (notify links).
    elements: Vec<ElementRef>,
    labels: Vec<String>,
    chains: Vec<Vec<EdgeIx>>,
}

impl Link {
    fn add(&mut self, element: ElementRef, label: String, chain: Vec<EdgeIx>) {
        if !self.labels.contains(&label) {
            self.labels.push(label);
        }
        if !self.elements.contains(&element) {
            self.elements.push(element);
        }
        self.chains.push(chain);
    }
}

/// Add the cross-lane links, then finish every stub with its link count.
pub(super) fn draft_links(
    draft: &mut DraftGraph,
    model: &Model,
    graph: &CausalGraph,
    painter: &Painter<'_>,
    endpoints: &[Option<Endpoint>],
    stubs: &[(MachineId, usize)],
    pills: bool,
) {
    let theme = painter.theme;
    let mut links: Vec<Link> = Vec::new();
    let mut at: HashMap<(usize, usize, Through), usize> = HashMap::new();
    let rule_links = causal_links(graph).into_iter().map(|l| {
        let rule = model.rule(l.rule);
        let label = format!("{} › {}", model.event(rule.event).name, model.controller(rule.controller).name);
        (l.from, l.to, Through::Rule, ElementRef::Rule(l.rule), label, l.chain.to_vec())
    });
    let subscriptions = subscription_links(graph).into_iter().map(|l| {
        let label = match model.trigger(l.trigger).kind {
            TriggerKind::Event(event) => model.event(event).name.clone(),
            TriggerKind::Command | TriggerKind::Timer(_) => model.trigger(l.trigger).name.clone(),
        };
        (l.from, l.to, Through::Subscription, ElementRef::Trigger(l.trigger), label, l.chain.to_vec())
    });
    for (from_t, to_t, through, element, label, chain) in rule_links.chain(subscriptions) {
        let (Some(Some(from)), Some(Some(to))) = (endpoints.get(from_t.index()), endpoints.get(to_t.index())) else {
            continue;
        };
        let i = *at.entry((from.node, to.node, through)).or_insert_with(|| {
            links.push(Link {
                from: *from,
                to: *to,
                through,
                target_machine: model.transition(to_t).machine,
                elements: Vec::new(),
                labels: Vec::new(),
                chains: Vec::new(),
            });
            links.len() - 1
        });
        links[i].add(element, label, chain);
    }

    let mut stub_links: HashMap<usize, u32> = HashMap::new();
    for link in links {
        let through_stub = link.from.kind == EndKind::Stub || link.to.kind == EndKind::Stub;
        for end in [link.from, link.to] {
            if end.kind == EndKind::Stub {
                *stub_links.entry(end.node).or_insert(0) += 1;
            }
        }
        let target_style = painter.machine(link.target_machine);
        let (kind, stroke) = match link.through {
            Through::Rule => (EdgeKind::Fire, painter.fire_stroke(target_style)),
            Through::Subscription => (EdgeKind::Notify, painter.notify_stroke(target_style)),
        };
        let (from_port, to_port) = if link.from.kind == EndKind::Pill && link.to.kind == EndKind::Pill {
            let (ga, gb) = (draft.nodes[link.from.node].group, draft.nodes[link.to.node].group);
            match ga.cmp(&gb) {
                // Pills side by side: East → West. Junctions sit on their
                // arrows, whose West and East ports the arrow itself uses,
                // so the link dips below the lane's row instead.
                std::cmp::Ordering::Equal if pills => (Some(PORT_EAST), Some(PORT_WEST)),
                std::cmp::Ordering::Equal => (Some(PORT_SOUTH), Some(PORT_SOUTH)),
                std::cmp::Ordering::Less => (Some(PORT_SOUTH), Some(PORT_NORTH)),
                std::cmp::Ordering::Greater => (Some(PORT_NORTH), Some(PORT_SOUTH)),
            }
        } else {
            (None, None)
        };
        let elements = link.elements;
        draft.edges.push(DraftEdge {
            from: link.from.node,
            from_port,
            to: link.to.node,
            to_port,
            kind: if through_stub { EdgeKind::StubLink } else { kind },
            stroke: if through_stub { painter.stub_link_stroke(stroke) } else { stroke },
            arrow: Arrow::End,
            label: Some(EdgeText {
                text: link.labels.join(", "),
                font_size: theme.small_font_size,
                color: theme.text,
                style: FontStyle::Normal,
            }),
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
