//! Synthetic graphs shaped like the structure view's build canvas: machine
//! lanes of states and transition pills stacked top to bottom, and the
//! wiring (events and controllers) either in gutter groups between the
//! lanes or in one band below them all.
//!
//! Pills have the build canvas's ports (West in, East out, North, South in,
//! South out), events take emits on North and subscribe from East, and
//! controllers take subscriptions on West and fire from North into a pill's
//! South port.

#![allow(dead_code)]

use cascade_layout::{
    EdgeEnd, GroupId, Insets, LayoutEdge, LayoutGraph, LayoutGroup, LayoutNode, NodeId, Port, PortSide, Size,
};

use super::common::Lcg;

/// Where the events and controllers go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wiring {
    /// One gutter group after each lane: an event goes to the gutter below
    /// the lane that emits it, a controller next to its first event.
    Gutters,
    /// Every event and controller in one group below all lanes (today's
    /// build canvas).
    Band,
}

pub const PORT_WEST: u16 = 0;
pub const PORT_EAST: u16 = 1;
pub const PORT_NORTH: u16 = 2;
pub const PORT_SOUTH: u16 = 3;
pub const PORT_SOUTH_OUT: u16 = 4;

fn pill_ports() -> Vec<Port> {
    [PortSide::West, PortSide::East, PortSide::North, PortSide::South, PortSide::South]
        .into_iter()
        .map(|side| Port { side })
        .collect()
}

fn lane(g: &mut LayoutGraph, key: String) -> GroupId {
    g.add_group(LayoutGroup { key, padding: Insets { top: 8.0, right: 16.0, bottom: 12.0, left: 16.0 }, header: 24.0 })
}

/// Which edges of a canvas graph are wiring (emit, subscribe, fire).
pub struct Canvas {
    pub graph: LayoutGraph,
    pub lanes: Vec<GroupId>,
    /// Fire edges (controller North → pill South), labelled or not.
    pub fires: Vec<cascade_layout::EdgeId>,
    /// Emit edges (pill South out → event North).
    pub emits: Vec<cascade_layout::EdgeId>,
}

/// A canvas with `lanes` machines. `seed` varies the machines and wiring.
pub fn canvas(lanes: usize, wiring: Wiring, seed: u64) -> Canvas {
    let mut rng = Lcg::new(seed);
    let mut g = LayoutGraph::new();
    let mut lane_ids = Vec::new();
    let mut gutters = Vec::new();
    for i in 0..lanes {
        lane_ids.push(lane(&mut g, format!("lane{i}")));
        if wiring == Wiring::Gutters {
            gutters.push(lane(&mut g, format!("gutter{i}")));
        }
    }
    let band = (wiring == Wiring::Band).then(|| lane(&mut g, "wiring".to_string()));

    // Machines: a main path of states with a pill per transition, plus a
    // few side transitions.
    let mut pills: Vec<Vec<NodeId>> = Vec::new();
    for (i, &group) in lane_ids.iter().enumerate() {
        let states: Vec<NodeId> = (0..4 + rng.below(4))
            .map(|s| {
                let w = 60.0 + 8.0 * rng.below(8) as f32;
                g.add_node(LayoutNode::new(format!("s{i}_{s}"), Size::new(w, 24.0)).in_group(group)).expect("state")
            })
            .collect();
        let mut transitions: Vec<(usize, usize)> = (0..states.len() - 1).map(|s| (s, s + 1)).collect();
        for _ in 0..1 + rng.below(3) {
            let a = rng.below(states.len() as u32) as usize;
            let b = rng.below(states.len() as u32) as usize;
            if a != b {
                transitions.push((a, b));
            }
        }
        let mut lane_pills = Vec::new();
        for (t, (a, b)) in transitions.into_iter().enumerate() {
            let w = 110.0 + 8.0 * rng.below(10) as f32;
            let pill = g
                .add_node(
                    LayoutNode::new(format!("p{i}_{t}"), Size::new(w, 36.0)).in_group(group).with_ports(pill_ports()),
                )
                .expect("pill");
            let guard = rng.chance(20).then(|| Size::new(90.0, 12.0));
            let mut into = LayoutEdge::new(EdgeEnd::node(states[a]), EdgeEnd::port(pill, PORT_WEST));
            if let Some(label) = guard {
                into = into.with_label(label);
            }
            g.add_edge(into).expect("into pill");
            g.add_edge(LayoutEdge::new(EdgeEnd::port(pill, PORT_EAST), EdgeEnd::node(states[b]))).expect("out");
            lane_pills.push(pill);
        }
        pills.push(lane_pills);
    }

    // Wiring: events emitted by one lane, controllers subscribing to one or
    // two events and firing into nearby (sometimes distant) lanes.
    let mut emits = Vec::new();
    let mut fires = Vec::new();
    let mut events: Vec<(NodeId, usize)> = Vec::new();
    for (i, lane_pills) in pills.iter().enumerate() {
        for k in 0..1 + rng.below(3) {
            let group = band.unwrap_or_else(|| gutters[i]);
            let event = g
                .add_node(
                    LayoutNode::new(format!("e{i}_{k}"), Size::new(100.0 + 8.0 * rng.below(6) as f32, 24.0))
                        .in_group(group)
                        .with_ports(vec![Port { side: PortSide::East }, Port { side: PortSide::North }]),
                )
                .expect("event");
            let pill = lane_pills[rng.below(lane_pills.len() as u32) as usize];
            emits.push(
                g.add_edge(LayoutEdge::new(EdgeEnd::port(pill, PORT_SOUTH_OUT), EdgeEnd::port(event, 1)))
                    .expect("emit"),
            );
            events.push((event, i));
        }
    }
    let controllers = events.len() * 2 / 3 + 1;
    for c in 0..controllers {
        let (first, home) = events[rng.below(events.len() as u32) as usize];
        let handlers = 1 + rng.below(2);
        let group = band.unwrap_or_else(|| gutters[home]);
        let controller = g
            .add_node(
                LayoutNode::new(format!("c{c}"), Size::new(130.0, 30.0 + 12.0 * handlers as f32))
                    .in_group(group)
                    .with_ports(vec![Port { side: PortSide::West }, Port { side: PortSide::North }]),
            )
            .expect("controller");
        g.add_edge(LayoutEdge::new(EdgeEnd::port(first, 0), EdgeEnd::port(controller, 0))).expect("subscribe");
        if handlers > 1 {
            let (other, _) = events[rng.below(events.len() as u32) as usize];
            if other != first {
                g.add_edge(LayoutEdge::new(EdgeEnd::port(other, 0), EdgeEnd::port(controller, 0))).expect("subscribe");
            }
        }
        for _ in 0..1 + rng.below(3) {
            // Mostly a neighbouring lane, sometimes anywhere.
            let target_lane = if rng.chance(70) {
                (home + rng.below(2) as usize).min(lanes - 1)
            } else {
                rng.below(lanes as u32) as usize
            };
            let lane_pills = &pills[target_lane];
            let pill = lane_pills[rng.below(lane_pills.len() as u32) as usize];
            let mut fire = LayoutEdge::new(EdgeEnd::port(controller, 1), EdgeEnd::port(pill, PORT_SOUTH));
            if rng.chance(40) {
                fire = fire.with_label(Size::new(120.0 + 8.0 * rng.below(8) as f32, 12.0));
            }
            fires.push(g.add_edge(fire).expect("fire"));
        }
    }
    Canvas { graph: g, lanes: lane_ids, fires, emits }
}

/// A plain SVG drawing of a layout (groups, nodes, routes, label boxes), for
/// looking at benchmark layouts.
pub fn to_svg(graph: &LayoutGraph, result: &cascade_layout::LayoutResult) -> String {
    use std::fmt::Write as _;
    let b = result.bounds;
    let mut out = String::new();
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="{} {} {} {}">"#,
        b.size.width + 40.0,
        b.size.height + 40.0,
        b.left() - 20.0,
        b.top() - 20.0,
        b.size.width + 40.0,
        b.size.height + 40.0
    );
    for (id, g) in graph.groups() {
        let r = result.group(id);
        let _ = writeln!(
            out,
            r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#f4f6f8" stroke="#99a"/><rect x="{}" y="{}" width="{}" height="{}" fill="#dde"/>"##,
            r.left(),
            r.top(),
            r.size.width,
            r.size.height,
            r.left(),
            r.top(),
            r.size.width,
            g.padding.top + g.header
        );
    }
    for (id, _) in graph.nodes() {
        let r = result.node(id).rect;
        let _ = writeln!(
            out,
            r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#fff" stroke="#333"/>"##,
            r.left(),
            r.top(),
            r.size.width,
            r.size.height
        );
    }
    let palette = ["#c33", "#36c", "#393", "#c80", "#909", "#088", "#666"];
    for (id, _) in graph.edges() {
        let route = result.edge(id);
        let pts: Vec<String> = route.points.iter().map(|p| format!("{},{}", p.x, p.y)).collect();
        let _ = writeln!(
            out,
            r#"<polyline points="{}" fill="none" stroke="{}" stroke-width="1.2"/>"#,
            pts.join(" "),
            palette[id.index() % palette.len()]
        );
        if let Some(l) = route.label {
            let _ = writeln!(
                out,
                r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#ffd" fill-opacity="0.8" stroke="#aa0"/>"##,
                l.left(),
                l.top(),
                l.size.width,
                l.size.height
            );
        }
    }
    out.push_str("</svg>\n");
    out
}
