//! Causal lanes (`ViewState::group_by_machine`): the causal view with one
//! lane per machine on shared causal columns.
//!
//! Every causal node goes into one lane, by these rules (looked up in the
//! model, so filters and hide mode never move a node to another lane):
//!
//! | Node | Lane |
//! | --- | --- |
//! | Transition | its machine |
//! | Hidden machine's stub | that machine |
//! | Event | the machine of the first transition, in definition order, that emits it |
//! | Handler | the machine its first rule fires into |
//! | External source | the machine of its first trigger |
//! | Anything else (an event nothing emits, a handler without rules, a source without triggers) | "Unattached", the last lane |
//!
//! The layout runs with `LayoutOptions::shared_layers`, so a node's column
//! is its causal layer in every lane and every forward arrow points right.

use std::collections::BTreeSet;

use cascade_core::{CausalNode, ElementRef, EventId, MachineId, Model};
use cascade_layout::{Insets, Point, Rect};

use crate::emphasis::Interaction;
use crate::scene::{FontWeight, HitTarget, Label, Lane, Scene, Stroke};
use crate::views::SceneInput;
use crate::views::draft::{DraftGraph, DraftGroup};
use crate::views::structure::machine_lane;
use crate::views::style::Painter;

/// Layout group key of the lane holding nodes with no machine.
const UNATTACHED_KEY: &str = "lane:unattached";
/// Title of that lane.
const UNATTACHED_TITLE: &str = "Unattached";

/// The lane a causal node belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LaneOf {
    Machine(MachineId),
    Unattached,
}

/// Which lane every node kind goes to.
pub(super) struct LaneRules {
    /// Machine of the first transition emitting each event.
    emitter: Vec<Option<MachineId>>,
}

impl LaneRules {
    pub(super) fn new(model: &Model) -> Self {
        let mut emitter: Vec<Option<MachineId>> = vec![None; model.event_count()];
        for (_, t) in model.transitions() {
            for &ev in &t.emits {
                if let Some(slot) = emitter.get_mut(ev.index())
                    && slot.is_none()
                {
                    *slot = Some(t.machine);
                }
            }
        }
        Self { emitter }
    }

    fn event(&self, ev: EventId) -> LaneOf {
        self.emitter.get(ev.index()).copied().flatten().map_or(LaneOf::Unattached, LaneOf::Machine)
    }

    /// The lane of a causal node.
    pub(super) fn of(&self, model: &Model, node: CausalNode) -> LaneOf {
        let machine = match node {
            CausalNode::Transition(t) => Some(model.transition(t).machine),
            CausalNode::Event(ev) => return self.event(ev),
            CausalNode::Handler(h) => {
                model.handler(h).rules.first().map(|&r| model.trigger(model.rule(r).trigger).machine)
            }
            CausalNode::External(x) => model.external(x).triggers.first().map(|&t| model.trigger(t).machine),
        };
        machine.map_or(LaneOf::Unattached, LaneOf::Machine)
    }
}

/// The draft groups of the lanes: one per machine in definition order,
/// then the unattached lane. Groups left without nodes are dropped by the
/// layout step.
pub(super) struct LaneGroups {
    /// Draft group of every machine, by machine index.
    machines: Vec<usize>,
    unattached: usize,
}

impl LaneGroups {
    pub(super) fn add(draft: &mut DraftGraph, model: &Model, painter: &Painter<'_>) -> Self {
        let header = painter.line_height(painter.theme.font_size) + 10.0;
        let padding = Insets { top: 8.0, right: 16.0, bottom: 12.0, left: 16.0 };
        let machines = model
            .machine_ids()
            .map(|m| {
                let key = model.key_of(ElementRef::Machine(m)).to_string();
                draft.add_group(DraftGroup { key, padding, header })
            })
            .collect();
        let unattached = draft.add_group(DraftGroup { key: UNATTACHED_KEY.to_owned(), padding, header });
        Self { machines, unattached }
    }

    pub(super) fn group(&self, lane: LaneOf) -> usize {
        match lane {
            LaneOf::Machine(m) => self.machines.get(m.index()).copied().unwrap_or(self.unattached),
            LaneOf::Unattached => self.unattached,
        }
    }

    /// Draw a lane for every group that holds nodes, in stacking order. A
    /// hidden machine's lane (it holds the machine's stub) is marked
    /// collapsed; a selected machine's lane gets the selected outline.
    pub(super) fn draw(
        &self,
        scene: &mut Scene,
        input: &SceneInput<'_>,
        painter: &Painter<'_>,
        interaction: &Interaction,
        hidden: &BTreeSet<MachineId>,
        group_rect: impl Fn(usize) -> Option<Rect>,
    ) {
        let model = input.model;
        let selected: Vec<_> = interaction.selected().iter().map(|e| model.key_of(*e)).collect();
        for (m, &group) in model.machine_ids().zip(&self.machines) {
            let Some(rect) = group_rect(group) else { continue };
            let mut lane = machine_lane(input, painter, m, rect, hidden.contains(&m));
            if matches!(&lane.target, HitTarget::Element(key) if selected.contains(key)) {
                lane.stroke.width = input.theme.selected_stroke_width;
            }
            scene.lanes.push(lane);
        }
        if let Some(rect) = group_rect(self.unattached) {
            scene.lanes.push(unattached_lane(painter, rect));
        }
    }
}

/// The neutral lane for nodes that belong to no machine.
fn unattached_lane(painter: &Painter<'_>, rect: Rect) -> Lane {
    let theme = painter.theme;
    Lane {
        target: HitTarget::None,
        rect,
        fill: theme.neutral.mix(theme.background, 0.95),
        stroke: Stroke::dashed(theme.rule, 1.0),
        title: Label {
            text: UNATTACHED_TITLE.to_owned(),
            origin: Point::new(rect.left() + 12.0, rect.top() + 5.0),
            font_size: theme.font_size,
            color: theme.text_muted,
            weight: FontWeight::Bold,
        },
        opacity: 1.0,
        collapsed: false,
    }
}
