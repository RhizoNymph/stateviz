//! Instance markers: a chip per instance (`o1`) in its machine's hue.
//!
//! - **Structure view:** on the instance's current state. When that state
//!   is not drawn, on the nearest drawn ancestor (a collapsed compound
//!   state), else on the collapsed machine's node or the hidden machine's
//!   stub.
//! - **Causal view** (no states): on every transition pill leaving the
//!   current state or one of its ancestors, i.e. what the instance can do
//!   next. In a dead end (nothing leaves) the chip goes hollow on the pills
//!   entering the state instead, reading "arrived here". A hidden machine's
//!   instances sit on its stub.
//!
//! Chips sit on the node's top edge, left to right in marker order.

use std::collections::HashMap;

use cascade_core::{ElementRef, Model, StateId, TransitionId};
use cascade_layout::Point;

use crate::play::PlayMarker;
use crate::scene::{FontWeight, HitTarget, Scene, Stroke};
use crate::views::overlays::PlayDecor;
use crate::views::overlays::chip::{self, ChipStyle};

/// Gap between neighbouring chips.
const CHIP_GAP: f32 = 3.0;
/// How far a chip rises above the node's top edge, as a share of its height.
const RISE: f32 = 0.6;

/// Where markers attach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placement {
    /// On states (structure view).
    States,
    /// On transition pills (causal view).
    Transitions,
}

/// Filled for "is here / can go", hollow for "arrived, nothing leaves".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fill {
    Solid,
    Hollow,
}

struct Pending<'m> {
    marker: &'m PlayMarker,
    machine: cascade_core::MachineId,
    fill: Fill,
}

pub(super) fn draw(decor: &PlayDecor<'_>, scene: &mut Scene, markers: &[PlayMarker], place: Placement) {
    let model = decor.model;
    let mut index: HashMap<&HitTarget, usize> = HashMap::new();
    for (i, node) in scene.nodes.iter().enumerate() {
        index.entry(&node.target).or_insert(i);
    }
    let stub = |name: &str| {
        scene.nodes.iter().position(|n| matches!(&n.target, HitTarget::MachineStub { machine, .. } if machine == name))
    };

    // Chips per node, in order of first appearance.
    let mut order: Vec<usize> = Vec::new();
    let mut chips: HashMap<usize, Vec<Pending<'_>>> = HashMap::new();
    for marker in markers {
        let Some(ElementRef::State(state)) = model.resolve_key(&marker.state) else { continue };
        let machine = model.state(state).machine;
        let name = &model.machine(machine).name;
        let spots: Vec<(usize, Fill)> = match place {
            Placement::States => state_node(model, &index, state)
                .or_else(|| index.get(&HitTarget::Element(model.key_of(ElementRef::Machine(machine)))).copied())
                .or_else(|| stub(name))
                .map(|n| (n, Fill::Solid))
                .into_iter()
                .collect(),
            Placement::Transitions => match stub(name) {
                Some(n) => vec![(n, Fill::Solid)],
                None => pill_nodes(model, &index, state),
            },
        };
        for (node, fill) in spots {
            chips
                .entry(node)
                .or_insert_with(|| {
                    order.push(node);
                    Vec::new()
                })
                .push(Pending { marker, machine, fill });
        }
    }

    let painter = decor.painter;
    for node in order {
        let Some(list) = chips.remove(&node) else { continue };
        let (rect, target, opacity) = {
            let n = &scene.nodes[node];
            (n.rect, n.target.clone(), n.opacity)
        };
        let mut x = rect.left() + (rect.size.height / 2.0).min(12.0);
        for p in list {
            let size = chip::size(painter, &p.marker.instance);
            let style = painter.machine(p.machine);
            let look = match p.fill {
                Fill::Solid => ChipStyle {
                    fill: style.hue,
                    stroke: Stroke::solid(painter.hue_outline(style), 1.0),
                    text: style.on_hue,
                    weight: FontWeight::Bold,
                },
                Fill::Hollow => ChipStyle {
                    fill: painter.theme.background,
                    stroke: Stroke::solid(style.hue, painter.theme.stroke_width),
                    text: painter.theme.text,
                    weight: FontWeight::Bold,
                },
            };
            let at = Point::new(x, rect.top() - size.height * RISE);
            chip::push(scene, painter, at, &p.marker.instance, &look, target.clone(), opacity);
            x += size.width + CHIP_GAP;
        }
    }
}

/// The node drawn for `state`: itself, else its nearest drawn ancestor.
fn state_node(model: &Model, index: &HashMap<&HitTarget, usize>, state: StateId) -> Option<usize> {
    std::iter::once(state)
        .chain(model.ancestors(state))
        .find_map(|s| index.get(&HitTarget::Element(model.key_of(ElementRef::State(s)))).copied())
}

/// Pills leaving `state` (or an ancestor), or, when nothing leaves, hollow
/// on the pills entering it.
fn pill_nodes(model: &Model, index: &HashMap<&HitTarget, usize>, state: StateId) -> Vec<(usize, Fill)> {
    let transitions = &model.machine(model.state(state).machine).transitions;
    let leaving: Vec<TransitionId> =
        transitions.iter().copied().filter(|&t| model.is_ancestor_or_self(model.transition(t).from, state)).collect();
    let (chosen, fill) = if leaving.is_empty() {
        let entering =
            transitions.iter().copied().filter(|&t| model.is_ancestor_or_self(model.transition(t).to, state)).collect();
        (entering, Fill::Hollow)
    } else {
        (leaving, Fill::Solid)
    };
    let mut nodes: Vec<(usize, Fill)> = Vec::new();
    for t in chosen {
        if let Some(&n) = index.get(&HitTarget::Element(model.key_of(ElementRef::Transition(t))))
            && !nodes.iter().any(|(m, _)| *m == n)
        {
            nodes.push((n, fill));
        }
    }
    nodes
}
