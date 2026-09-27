//! The matrix view: coupling counts, clustering order, shading, headers.

mod common;

use cascade_scene::{Emphasis, HitTarget, Overlay, Scene, SceneNode, ViewKind, ViewState};
use common::*;

fn matrix() -> ViewState {
    ViewState { view: ViewKind::Matrix, ..ViewState::default() }
}

fn cell<'a>(scene: &'a Scene, row: &str, column: &str) -> &'a SceneNode {
    scene
        .nodes
        .iter()
        .find(|n| matches!(&n.target, HitTarget::MatrixCell { row: r, column: c, .. } if r == row && c == column))
        .unwrap_or_else(|| panic!("no cell {row} → {column}"))
}

fn count(node: &SceneNode) -> u32 {
    match node.target {
        HitTarget::MatrixCell { count, .. } => count,
        _ => panic!("not a cell"),
    }
}

/// Row headers top to bottom.
fn row_order(scene: &Scene) -> Vec<String> {
    let mut heads: Vec<&SceneNode> = scene
        .nodes
        .iter()
        .filter(|n| matches!(&n.target, HitTarget::Element(k) if k.kind() == cascade_core::ElementKind::Machine))
        .collect();
    let left = heads.iter().map(|n| n.rect.left()).fold(f32::INFINITY, f32::min);
    heads.retain(|n| (n.rect.left() - left).abs() < 0.5);
    heads.sort_by(|a, b| a.rect.top().total_cmp(&b.rect.top()));
    heads.iter().map(|n| describe(&n.target).trim_start_matches("machine:").to_owned()).collect()
}

fn luminance(c: cascade_scene::Rgba) -> f32 {
    0.2126 * f32::from(c.r) + 0.7152 * f32::from(c.g) + 0.0722 * f32::from(c.b)
}

#[test]
fn cells_count_causal_links_from_row_to_column() {
    let fx = Fixture::new(CHAIN);
    let scene = fx.scene(&matrix());
    assert_eq!(scene.view, ViewKind::Matrix);
    let cells = scene.nodes.iter().filter(|n| matches!(n.target, HitTarget::MatrixCell { .. })).count();
    assert_eq!(cells, 9, "3 × 3");
    assert_eq!(count(cell(&scene, "A", "B")), 2, "go → start, reset → rewind");
    assert_eq!(count(cell(&scene, "B", "A")), 1);
    assert_eq!(count(cell(&scene, "B", "D")), 1);
    assert_eq!(count(cell(&scene, "D", "B")), 0);
    assert_eq!(count(cell(&scene, "A", "A")), 0);
    assert_eq!(cell(&scene, "A", "B").labels[0].text, "2");
    assert!(cell(&scene, "D", "B").labels.is_empty(), "empty cells have no text");
}

#[test]
fn shading_is_neutral_gray_by_count() {
    let fx = Fixture::new(CHAIN);
    let scene = fx.scene(&matrix());
    let (two, one, zero) = (cell(&scene, "A", "B"), cell(&scene, "B", "A"), cell(&scene, "D", "B"));
    let fill = |n: &SceneNode| n.fill.unwrap_or(fx.theme.background);
    assert!(luminance(fill(two)) < luminance(fill(one)), "more links, darker");
    assert!(luminance(fill(one)) < luminance(fill(zero)));
    let styles = cascade_scene::machine_styles(&fx.model, &fx.theme);
    for n in scene.nodes.iter().filter(|n| matches!(n.target, HitTarget::MatrixCell { .. })) {
        let c = fill(n);
        assert!(styles.iter().all(|s| s.hue != c && s.pale != c), "never a hue");
        let spread =
            [c.r, c.g, c.b].iter().max().copied().unwrap_or(0) - [c.r, c.g, c.b].iter().min().copied().unwrap_or(0);
        assert!(spread <= 16, "neutral: {c:?}");
    }
}

#[test]
fn headers_have_hue_chips_and_numbers() {
    let fx = Fixture::new(CHAIN);
    let scene = fx.scene(&matrix());
    let styles = cascade_scene::machine_styles(&fx.model, &fx.theme);
    let chips: Vec<_> = scene
        .overlays
        .iter()
        .filter_map(|o| match o {
            Overlay::Rect { fill: Some(f), target, .. } => Some((*f, target.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(chips.len(), 6, "a chip for each row and column header");
    for (i, name) in ["A", "B", "D"].iter().enumerate() {
        let t = target(&format!("machine:{name}"));
        let mine: Vec<_> = chips.iter().filter(|(_, ct)| *ct == t).collect();
        assert_eq!(mine.len(), 2);
        assert!(mine.iter().all(|(f, _)| *f == styles[i].hue));
    }
    let heads: Vec<&SceneNode> = scene.nodes.iter().filter(|n| n.target == target("machine:A")).collect();
    assert_eq!(heads.len(), 2, "row and column header");
    assert!(heads.iter().any(|n| n.labels.iter().any(|l| l.text.ends_with("A"))));
}

#[test]
fn rows_and_columns_are_sorted_so_coupled_machines_cluster() {
    let fx = Fixture::new(
        r#"
machines:
  A:
    states: [x, y]
    transitions:
      - { from: x, to: y, on: a1, emits: [EA] }
      - { from: y, to: x, on: a2 }
  B:
    states: [x, y]
    transitions:
      - { from: x, to: y, on: b1, emits: [EB] }
      - { from: y, to: x, on: b2 }
  C:
    states: [x, y, z]
    transitions:
      - { from: x, to: y, on: c1 }
      - { from: y, to: z, on: c1 }
      - { from: z, to: x, on: c2, emits: [EC] }
  D:
    states: [x, y]
    transitions:
      - { from: x, to: y, on: d1 }
controllers:
  AC:
    on:
      EA: [{ fire: C.c1 }]
      EC: [{ fire: A.a2 }]
  BD:
    on:
      EB: [{ fire: D.d1 }, { fire: B.b2 }]
"#,
    );
    let scene = fx.scene(&matrix());
    assert_eq!(count(cell(&scene, "A", "C")), 2);
    assert_eq!(count(cell(&scene, "C", "A")), 1);
    assert_eq!(count(cell(&scene, "B", "D")), 1);
    assert_eq!(count(cell(&scene, "B", "B")), 1, "self coupling");
    assert_eq!(row_order(&scene), ["A", "C", "B", "D"]);
    // Columns follow the same order.
    let a_col = cell(&scene, "A", "A").rect.left();
    let c_col = cell(&scene, "A", "C").rect.left();
    let b_col = cell(&scene, "A", "B").rect.left();
    assert!(a_col < c_col && c_col < b_col);
    // Deterministic.
    let again = fx.scene(&matrix());
    assert_eq!(format!("{:?}", again.nodes), format!("{:?}", scene.nodes));
}

#[test]
fn hidden_machines_leave_the_matrix() {
    let fx = Fixture::new(CHAIN);
    let state = ViewState { hidden_machines: ["D".to_owned()].into_iter().collect(), ..matrix() };
    let scene = fx.scene(&state);
    assert_eq!(scene.nodes.iter().filter(|n| matches!(n.target, HitTarget::MatrixCell { .. })).count(), 4);
    assert!(scene.nodes.iter().all(|n| !matches!(&n.target, HitTarget::MatrixCell { column, .. } if column == "D")));
}

#[test]
fn machine_pair_and_selection_are_emphasised_by_weight() {
    let fx = Fixture::new(CHAIN);
    let state = ViewState {
        machine_pair: Some(("A".into(), "B".into())),
        selection: vec![k("transition:D:d0->d1@note")],
        ..matrix()
    };
    let scene = fx.scene(&state);
    let pair = cell(&scene, "A", "B");
    assert_eq!(pair.emphasis, Emphasis::Selected);
    assert_eq!(pair.stroke.width, fx.theme.selected_stroke_width);
    assert_eq!(cell(&scene, "B", "A").emphasis, Emphasis::Normal);
    let d_heads: Vec<_> = scene.nodes.iter().filter(|n| n.target == target("machine:D")).collect();
    assert!(d_heads.iter().all(|n| n.emphasis == Emphasis::Selected), "the selection's machine");
    assert!(scene.nodes.iter().filter(|n| n.target == target("machine:A")).all(|n| n.emphasis == Emphasis::Normal));
}

#[test]
fn cells_outside_a_cone_are_dimmed() {
    let fx = Fixture::new(CHAIN);
    let state = ViewState {
        selection: vec![k("transition:A:a0->a1@go")],
        cone: Some(cascade_scene::ConeFocus { direction: cascade_core::Direction::Forward, depth: Some(1) }),
        ..matrix()
    };
    let scene = fx.scene(&state);
    assert_eq!(cell(&scene, "A", "B").emphasis, Emphasis::Focused, "go → start is in the cone");
    assert_eq!(cell(&scene, "B", "D").emphasis, Emphasis::Dimmed);
    assert_eq!(cell(&scene, "D", "B").emphasis, Emphasis::Normal, "empty cells are context");
}
