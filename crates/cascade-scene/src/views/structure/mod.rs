//! Structure view: what phases does each machine have, and where do the
//! cross-machine links attach?
//!
//! Each machine is a lane (a layout group) stacked in definition order,
//! with a header naming the machine in its hue. Inside, states are laid out
//! in layers with the transition pills sitting on the edges
//! (state → pill → state).
//!
//! - **Nesting:** each expanded compound state gets a band of its own (a
//!   further layout group, drawn as a nested lane titled with the state's
//!   path) right below its machine's lane and inside its outline. The
//!   compound state itself stays a node in its parent's band (marked
//!   "▾ n states"), since transitions can leave or enter it as a whole.
//!   This keeps containment visible with a single level of layout groups.
//! - **Collapse:** a collapsed compound state keeps its node ("▸ n states"),
//!   its descendants disappear, transitions inside it disappear and
//!   transitions crossing its boundary attach to it (pills keep their real
//!   endpoints in the label). A collapsed machine becomes one node in a
//!   collapsed lane.
//! - **Cross-lane links:** one dashed edge from the causing pill to the
//!   caused pill in the target machine's hue, labelled "Event › Controller";
//!   parallel links between the same endpoints merge. Pills in different
//!   lanes connect through their South/North ports so the layout routes the
//!   link between lanes rather than through them.
//! - **Hidden machines** become a stub in a thin band of their own, and
//!   their links attach to it.
//!
//! **Arrow mode** (`ViewState::transition_pills` off, in view and edit
//! mode alike) draws each transition as one labelled state → state arrow
//! instead of a pill (see `arrows`); links and wiring attach to a point on
//! the arrow.
//!
//! **Edit mode** (the build canvas) keeps the lanes and replaces the
//! cross-lane links with the wiring (see `wiring`): event tags, controller
//! hexagons and source boxes in gutters between the lanes (see `gutters`),
//! joined to the pills by real emit, subscribe, fire and trigger edges. Connectable nodes get connect
//! handles, a machine without transitions says how to add one, and an empty
//! definition says how to start.

mod arrows;
mod edit;
mod gutters;
mod links;
mod machines;
mod selector;
mod wiring;

pub(crate) use gutters::memo::WiringMemo;

use cascade_core::ElementRef;
use cascade_layout::{LayoutOptions, Point, Rect};

use crate::color::machine_styles;
use crate::emphasis::Interaction;
use crate::play::SceneMode;
use crate::scene::{FontWeight, HitTarget, Label, Lane, Scene, Stroke};
use crate::view_state::ViewKind;
use crate::views::cache::LayoutCache;
use crate::views::decorate::{Decor, FindingIndex, scene_bounds};
use crate::views::draft::{DraftGraph, RealizeCtx, realize};
use crate::views::filters::{apply_hide, hidden_machines};
use crate::views::overlays::{Placement, PlayDecor, add_handles};
use crate::views::style::Painter;
use crate::views::{SceneError, SceneInput};

use machines::{Collapse, Drafter, MachinePlan};

/// Horizontal inset of a nested band inside its machine lane.
const BAND_INSET: f32 = 6.0;

pub(super) fn build(
    input: &SceneInput<'_>,
    interaction: &Interaction,
    cache: &mut LayoutCache,
    memo: &mut WiringMemo,
) -> Result<Scene, SceneError> {
    let model = input.model;
    let theme = input.theme;
    let painter = Painter { theme, measure: input.measure, styles: machine_styles(model, theme) };
    let hidden = hidden_machines(model, &input.view.hidden_machines);
    let collapse = Collapse::resolve(model, &input.view.collapsed);
    let edit = input.mode == SceneMode::Edit;
    let pills = input.view.transition_pills;
    let drafter = Drafter { model, graph: input.graph, painter: &painter, collapse: &collapse, edit, pills };

    let mut draft = DraftGraph::default();
    let mut endpoints = vec![None; model.transition_count()];
    let mut plans = Vec::with_capacity(model.machine_count());
    // Edit mode: a gutter above every machine's groups and one below all.
    let mut gutters = Vec::new();
    for m in model.machine_ids() {
        if edit {
            gutters.push(draft.add_group(wiring::gutter_group(wiring::gutter_key_above(&model.machine(m).name))));
        }
        let plan = if hidden.contains(&m) {
            drafter.hidden(&mut draft, m, &mut endpoints)
        } else if collapse.machine(m) {
            drafter.collapsed(&mut draft, m, &mut endpoints)
        } else {
            drafter.expanded(&mut draft, m, &mut endpoints)
        };
        plans.push(plan);
    }
    if edit {
        gutters.push(draft.add_group(wiring::gutter_group(wiring::LAST_GUTTER_KEY.to_owned())));
    }
    let stubs: Vec<_> = plans
        .iter()
        .filter_map(|p| match p {
            MachinePlan::Hidden { machine, stub } => Some((*machine, *stub)),
            MachinePlan::Collapsed { .. } | MachinePlan::Expanded { .. } => None,
        })
        .collect();
    if edit {
        let wiring = wiring::Wiring { model, graph: input.graph, painter: &painter };
        wiring.draft(&mut draft, &endpoints, &gutters, &stubs, memo);
    } else {
        links::draft_links(&mut draft, model, input.graph, &painter, &endpoints, &stubs, pills);
    }

    let cuts = apply_hide(&mut draft, interaction);
    let ctx = RealizeCtx {
        view: ViewKind::Structure,
        theme,
        measure: input.measure,
        sidecar: input.sidecar,
        options: LayoutOptions {
            layer_spacing: if pills { 48.0 } else { arrows::LAYER_SPACING },
            align_across_groups: true,
            ..LayoutOptions::default()
        },
    };
    let realized = realize(draft, cuts, &ctx, cache)?;
    let mut scene = realized.scene;
    let decor = Decor {
        model,
        theme,
        interaction,
        findings: FindingIndex::for_mode(input.findings, input.mode),
        diff: input.diff,
    };
    decor.apply(&mut scene, &realized.nodes, &realized.edges, &realized.overlay_owner);
    let (mut node_metas, mut edge_infos) = (realized.nodes, realized.edges);
    if !pills {
        arrows::fold(&mut scene, &mut node_metas, &mut edge_infos, &painter);
    }

    let group_rect = |g: usize| realized.groups.get(g).copied().flatten();
    for plan in &plans {
        match plan {
            MachinePlan::Hidden { .. } => {}
            MachinePlan::Collapsed { machine, group } => {
                if let Some(rect) = group_rect(*group) {
                    scene.lanes.push(machine_lane(input, &painter, *machine, rect, true));
                }
            }
            MachinePlan::Expanded { machine, top, bands } => {
                let rects: Vec<Rect> =
                    std::iter::once(*top).chain(bands.iter().map(|(_, g)| *g)).filter_map(group_rect).collect();
                let Some(rect) = rects.iter().copied().reduce(|a, b| a.union(&b)) else { continue };
                scene.lanes.push(machine_lane(input, &painter, *machine, rect, false));
                for (state, band) in bands {
                    if let Some(rect) = group_rect(*band) {
                        scene.lanes.push(band_lane(input, &painter, *machine, *state, rect));
                    }
                }
            }
        }
    }
    edit::gutter_lanes(&mut scene, &painter, &gutters, group_rect);
    // A selected machine or compound state shows on its lane by weight.
    let selected: Vec<_> = interaction.selected().iter().map(|e| model.key_of(*e)).collect();
    for lane in &mut scene.lanes {
        if matches!(&lane.target, HitTarget::Element(key) if selected.contains(key)) {
            lane.stroke.width = theme.selected_stroke_width;
        }
    }
    let mut notes = interaction.notes().to_vec();
    if edit {
        edit::hints(&mut scene, model, &painter, &plans, &mut notes);
        add_handles(&mut scene, theme);
    }
    if let Some(play) = input.play {
        let play_decor = PlayDecor { model, painter: &painter };
        play_decor.apply(&mut scene, &node_metas, &edge_infos, play, Placement::States);
    }
    scene.bounds = scene_bounds(&scene, input.measure);
    scene.notes = notes;
    Ok(scene)
}

pub(crate) fn machine_lane(
    input: &SceneInput<'_>,
    painter: &Painter<'_>,
    machine: cascade_core::MachineId,
    rect: Rect,
    collapsed: bool,
) -> Lane {
    let theme = input.theme;
    let style = painter.machine(machine);
    Lane {
        target: HitTarget::Element(input.model.key_of(ElementRef::Machine(machine))),
        rect,
        fill: style.hue.mix(theme.background, 0.94),
        stroke: Stroke::solid(style.hue.mix(theme.background, 0.4), 1.0),
        title: Label {
            text: input.model.machine(machine).name.clone(),
            origin: Point::new(rect.left() + 12.0, rect.top() + 5.0),
            font_size: theme.font_size,
            color: style.hue,
            weight: FontWeight::Bold,
        },
        opacity: 1.0,
        collapsed,
    }
}

fn band_lane(
    input: &SceneInput<'_>,
    painter: &Painter<'_>,
    machine: cascade_core::MachineId,
    state: cascade_core::StateId,
    rect: Rect,
) -> Lane {
    let theme = input.theme;
    let style = painter.machine(machine);
    // Inset from the machine lane's sides so the band reads as inside it.
    let inset = BAND_INSET.min(rect.size.width / 4.0);
    let rect = Rect::new(rect.left() + inset, rect.top(), rect.size.width - 2.0 * inset, rect.size.height);
    Lane {
        target: HitTarget::Element(input.model.key_of(ElementRef::State(state))),
        rect,
        fill: style.hue.mix(theme.background, 0.9),
        stroke: Stroke::dashed(style.hue.mix(theme.background, 0.3), 1.0),
        title: Label {
            text: input.model.state(state).path.clone(),
            origin: Point::new(rect.left() + 10.0, rect.top() + 4.0),
            font_size: theme.small_font_size,
            color: style.hue,
            weight: FontWeight::Normal,
        },
        opacity: 1.0,
        collapsed: false,
    }
}
