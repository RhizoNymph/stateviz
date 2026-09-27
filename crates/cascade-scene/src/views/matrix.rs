//! Matrix view: which parts of the system are coupled, and how tightly?
//!
//! Machines × machines. The cell in row R, column C counts the derived
//! causal links from a transition of R to a transition of C (through an
//! event and a controller rule), like a design structure matrix. Rows and
//! columns share one order that clusters coupled machines (see
//! [`seriate`]); headers carry a hue chip, rows the number and name,
//! columns the number. Cells are shaded neutral gray by count, never by
//! hue. Clicking a cell (`HitTarget::MatrixCell`) opens the causal view
//! filtered to that pair. Hidden machines leave the matrix.

use std::cmp::Reverse;

use cascade_core::{EdgeIx, ElementRef, MachineId};
use cascade_layout::{Point, Rect};

use crate::color::{Rgba, machine_styles};
use crate::emphasis::{Anchor, Interaction};
use crate::scene::{Border, Emphasis, FontWeight, HitTarget, Label, Layer, Overlay, Scene, SceneNode, Shape, Stroke};
use crate::view_state::ViewKind;
use crate::views::SceneInput;
use crate::views::decorate::{Decor, FindingIndex, scene_bounds};
use crate::views::draft::{EdgeInfo, Meta};
use crate::views::filters::hidden_machines;
use crate::views::links::causal_links;
use crate::views::style::Painter;

/// Smallest cell side.
const CELL_MIN: f32 = 32.0;
/// Hue chip side.
const CHIP: f32 = 10.0;
/// Gap between a chip and its text, and around headers.
const GAP: f32 = 6.0;
/// Lightest and darkest shading of a non-empty cell (mix toward the text
/// color).
const SHADE_MIN: f32 = 0.12;
const SHADE_MAX: f32 = 0.7;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Header(MachineId),
    Cell { row: MachineId, column: MachineId },
}

pub(super) fn build(input: &SceneInput<'_>, interaction: &Interaction) -> Scene {
    let model = input.model;
    let theme = input.theme;
    let painter = Painter { theme, measure: input.measure, styles: machine_styles(model, theme) };
    let mut scene = Scene::empty(ViewKind::Matrix, theme.background);

    let hidden = hidden_machines(model, &input.view.hidden_machines);
    let machines: Vec<MachineId> = model.machine_ids().filter(|m| !hidden.contains(m)).collect();
    if machines.is_empty() {
        scene.notes.push("Every machine is hidden.".to_owned());
        return scene;
    }
    let n = machines.len();
    let slot = |m: MachineId| machines.iter().position(|&x| x == m);

    // Links and their causal chains per (row, column) slot.
    let mut chains: Vec<Vec<Vec<Vec<EdgeIx>>>> = vec![vec![Vec::new(); n]; n];
    for link in causal_links(input.graph) {
        let (from, to) = (model.transition(link.from).machine, model.transition(link.to).machine);
        if let (Some(r), Some(c)) = (slot(from), slot(to)) {
            chains[r][c].push(link.chain.to_vec());
        }
    }
    let counts: Vec<Vec<u32>> =
        chains.iter().map(|row| row.iter().map(|c| u32::try_from(c.len()).unwrap_or(u32::MAX)).collect()).collect();
    let order = seriate(&counts);
    let max = counts.iter().flatten().copied().max().unwrap_or(0);

    // Geometry.
    let font = theme.font_size;
    let small = theme.small_font_size;
    let digits = |v: usize| v.to_string();
    let cell = CELL_MIN
        .max(painter.text_width(&max.to_string(), small) + 2.0 * GAP)
        .max(painter.text_width(&digits(n), small) + CHIP + 3.0 * GAP);
    let row_labels: Vec<String> =
        order.iter().enumerate().map(|(i, &s)| format!("{} {}", i + 1, model.machine(machines[s]).name)).collect();
    let row_text_w = row_labels.iter().map(|l| painter.text_width(l, font)).fold(0.0, f32::max);
    let row_header_w = CHIP + 3.0 * GAP + row_text_w;
    let col_header_h = CHIP + painter.line_height(small) + 3.0 * GAP;
    let grid = Point::new(row_header_w, col_header_h);

    let mut metas: Vec<Meta> = Vec::new();
    let mut roles: Vec<Role> = Vec::new();
    let mut owners: Vec<Option<usize>> = Vec::new();
    let header_stroke = Stroke::solid(theme.rule, 0.0);
    for (i, &s) in order.iter().enumerate() {
        let m = machines[s];
        let key = model.key_of(ElementRef::Machine(m));
        let hue = painter.machine(m).hue;
        let meta = Meta::new(vec![ElementRef::Machine(m)], Anchor::Free);
        let offset = i as f32 * cell;

        // Row header: chip, then "n Name".
        let rect = Rect::new(0.0, grid.y + offset, row_header_w, cell);
        let text_y = rect.center().y - painter.line_height(font) / 2.0;
        let label = Label {
            text: row_labels[i].clone(),
            origin: Point::new(CHIP + 2.0 * GAP, text_y),
            font_size: font,
            color: theme.text,
            weight: FontWeight::Normal,
        };
        push_node(&mut scene, HitTarget::Element(key.clone()), rect, None, header_stroke, vec![label]);
        metas.push(meta.clone());
        roles.push(Role::Header(m));
        let owner = scene.nodes.len() - 1;
        push_chip(&mut scene, &mut owners, Point::new(GAP, rect.center().y - CHIP / 2.0), hue, &key, owner);

        // Column header: chip over the number.
        let rect = Rect::new(grid.x + offset, 0.0, cell, col_header_h);
        let number = (i + 1).to_string();
        let label = Label {
            origin: Point::new(rect.center().x - painter.text_width(&number, small) / 2.0, GAP + CHIP + GAP),
            text: number,
            font_size: small,
            color: theme.text,
            weight: FontWeight::Normal,
        };
        push_node(&mut scene, HitTarget::Element(key.clone()), rect, None, header_stroke, vec![label]);
        metas.push(meta);
        roles.push(Role::Header(m));
        let owner = scene.nodes.len() - 1;
        push_chip(&mut scene, &mut owners, Point::new(rect.center().x - CHIP / 2.0, GAP), hue, &key, owner);
    }

    for (r, &rs) in order.iter().enumerate() {
        for (c, &cs) in order.iter().enumerate() {
            let (row, column) = (machines[rs], machines[cs]);
            let count = counts[rs][cs];
            let rect = Rect::new(grid.x + c as f32 * cell, grid.y + r as f32 * cell, cell, cell);
            let shade = shade(theme.background, theme.text, count, max);
            let text_color = if count > 0 && shade_t(count, max) > 0.45 { theme.background } else { theme.text };
            let labels = if count == 0 {
                Vec::new()
            } else {
                let text = count.to_string();
                vec![Label {
                    origin: Point::new(
                        rect.center().x - painter.text_width(&text, small) / 2.0,
                        rect.center().y - painter.line_height(small) / 2.0,
                    ),
                    text,
                    font_size: small,
                    color: text_color,
                    weight: FontWeight::Normal,
                }]
            };
            let target = HitTarget::MatrixCell {
                row: model.machine(row).name.clone(),
                column: model.machine(column).name.clone(),
                count,
            };
            push_node(&mut scene, target, rect, Some(shade), Stroke::solid(theme.rule, 1.0), labels);
            let anchor = if count == 0 { Anchor::Free } else { Anchor::Chains(chains[rs][cs].clone()) };
            metas.push(Meta::new(Vec::new(), anchor));
            roles.push(Role::Cell { row, column });
        }
    }

    let decor = Decor { model, theme, interaction, findings: FindingIndex::new(&[]), diff: input.diff };
    decor.apply(&mut scene, &metas, &Vec::<EdgeInfo>::new(), &owners);

    // Selection by weight: the machine pair's cell, and the headers of every
    // selected element's machine.
    let pair = input
        .view
        .machine_pair
        .as_ref()
        .and_then(|(a, b)| Some((model.machine_by_name(a)?, model.machine_by_name(b)?)));
    let selected_machines: Vec<MachineId> =
        interaction.selected().iter().filter_map(|e| model.machine_of(*e)).collect();
    for (node, role) in scene.nodes.iter_mut().zip(&roles) {
        let selected = match *role {
            Role::Cell { row, column } => pair == Some((row, column)),
            Role::Header(m) => selected_machines.contains(&m),
        };
        if selected {
            node.emphasis = Emphasis::Selected;
            node.opacity = 1.0;
            node.stroke.width = theme.selected_stroke_width;
            if matches!(role, Role::Header(_)) {
                node.stroke.color = theme.text_muted;
            }
        }
    }
    scene.bounds = scene_bounds(&scene, input.measure);
    scene.notes = interaction.notes().to_vec();
    scene
}

fn push_node(scene: &mut Scene, target: HitTarget, rect: Rect, fill: Option<Rgba>, stroke: Stroke, labels: Vec<Label>) {
    scene.nodes.push(SceneNode {
        target,
        shape: Shape::Rect,
        rect,
        fill,
        stroke,
        border: Border::Single,
        labels,
        badge: None,
        opacity: 1.0,
        emphasis: Emphasis::Normal,
        diff: None,
    });
}

fn push_chip(
    scene: &mut Scene,
    owners: &mut Vec<Option<usize>>,
    at: Point,
    hue: Rgba,
    key: &cascade_core::ElementKey,
    owner: usize,
) {
    scene.overlays.push(Overlay::Rect {
        rect: Rect::new(at.x, at.y, CHIP, CHIP),
        fill: Some(hue),
        stroke: None,
        radius: 2.0,
        opacity: 1.0,
        layer: Layer::Over,
        target: HitTarget::Element(key.clone()),
    });
    owners.push(Some(owner));
}

/// How dark a cell is, in `[0, SHADE_MAX]`; 0 for an empty cell.
fn shade_t(count: u32, max: u32) -> f32 {
    if count == 0 || max == 0 {
        return 0.0;
    }
    SHADE_MIN + (SHADE_MAX - SHADE_MIN) * (count as f32 / max as f32)
}

/// Neutral shading: the background mixed toward a gray of the text's
/// lightness.
fn shade(background: Rgba, text: Rgba, count: u32, max: u32) -> Rgba {
    let gray = {
        let l = ((u16::from(text.r) + u16::from(text.g) + u16::from(text.b)) / 3) as u8;
        Rgba::rgb(l, l, l)
    };
    background.mix(gray, shade_t(count, max))
}

/// One order for rows and columns that clusters coupled machines.
///
/// Coupling is symmetric (`w[i][j] = count[i][j] + count[j][i]`, diagonal
/// ignored). Greedy: start from the most coupled machine; then repeatedly
/// take, among machines coupled to those already placed, the one most
/// coupled to the last placed (ties: to all placed, then total coupling);
/// when none is coupled, start a new cluster from the most coupled
/// remaining machine. Remaining ties go to definition order, so the result
/// is deterministic. Returns indices into `counts`.
pub(crate) fn seriate(counts: &[Vec<u32>]) -> Vec<usize> {
    let n = counts.len();
    let get = |i: usize, j: usize| counts.get(i).and_then(|r| r.get(j)).copied().unwrap_or(0);
    let w = |i: usize, j: usize| if i == j { 0 } else { get(i, j).saturating_add(get(j, i)) };
    let total: Vec<u32> = (0..n).map(|i| (0..n).map(|j| w(i, j)).sum()).collect();
    let mut placed = vec![false; n];
    let mut order: Vec<usize> = Vec::with_capacity(n);
    while order.len() < n {
        let to_placed = |c: usize| order.iter().map(|&p| w(p, c)).sum::<u32>();
        let remaining = (0..n).filter(|&c| !placed[c]);
        let next = match order.last() {
            Some(&last) if remaining.clone().any(|c| to_placed(c) > 0) => remaining
                .filter(|&c| to_placed(c) > 0)
                .max_by_key(|&c| (w(last, c), to_placed(c), total[c], Reverse(c))),
            _ => remaining.max_by_key(|&c| (total[c], Reverse(c))),
        };
        let Some(next) = next else { break };
        placed[next] = true;
        order.push(next);
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seriation_clusters_and_is_deterministic() {
        // 0–2 strongly coupled, 1–3 weakly, 4 alone.
        let mut counts = vec![vec![0u32; 5]; 5];
        counts[0][2] = 3;
        counts[2][0] = 1;
        counts[3][1] = 1;
        assert_eq!(seriate(&counts), [0, 2, 1, 3, 4]);
        assert_eq!(seriate(&counts), seriate(&counts));
        assert!(seriate(&[]).is_empty());
    }

    #[test]
    fn chains_follow_the_strongest_neighbour() {
        // A chain 0 - 1 - 2 - 3 with a strong 1–3 shortcut.
        let mut counts = vec![vec![0u32; 4]; 4];
        counts[0][1] = 1;
        counts[1][2] = 1;
        counts[2][3] = 1;
        counts[1][3] = 5;
        let order = seriate(&counts);
        assert_eq!(order[0], 1, "most coupled first");
        assert_eq!(order[1], 3, "then its strongest neighbour");
        assert_eq!(order.len(), 4);
    }

    #[test]
    fn shading_is_gray_and_monotone() {
        let bg = Rgba::hex(0xFFFFFF);
        let text = Rgba::hex(0x1F2328);
        assert_eq!(shade(bg, text, 0, 5), bg);
        let one = shade(bg, text, 1, 5);
        let five = shade(bg, text, 5, 5);
        assert!(five.r < one.r);
        assert_eq!((one.r, one.g), (one.g, one.b));
    }
}
