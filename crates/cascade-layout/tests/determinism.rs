//! Same input, same output, regardless of hash map iteration order in the
//! hints.

mod common;

use cascade_layout::{LayoutHints, LayoutOptions, PreviousLayout};
use common::*;

fn sample() -> Spec {
    let mut s = Spec::new().group("L0").group("L1");
    for i in 0..12 {
        let group = match i % 3 {
            0 => None,
            1 => Some(0),
            _ => Some(1),
        };
        s = match group {
            None => s.node(&format!("n{i}"), 40.0 + (i * 7 % 30) as f32, 20.0 + (i * 5 % 17) as f32),
            Some(gi) => s.node_in(&format!("n{i}"), 40.0 + (i * 7 % 30) as f32, 20.0 + (i * 5 % 17) as f32, gi),
        };
    }
    for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 0), (4, 7), (7, 10), (5, 8), (8, 11), (11, 5), (0, 6), (6, 9), (9, 3)] {
        s = s.edge(&format!("n{a}"), &format!("n{b}"));
    }
    s
}

#[test]
fn identical_inputs_give_identical_outputs() {
    let g = sample().build();
    let a = run(&g);
    let b = run(&g);
    assert_eq!(a, b);
}

#[test]
fn previous_hint_insertion_order_does_not_matter() {
    let g = sample().build();
    let r = run(&g);
    let prev = r.to_previous(&g);
    let mut entries: Vec<_> = prev.nodes.iter().map(|(k, v)| (k.clone(), *v)).collect();
    entries.sort_by(|x, y| x.0.cmp(&y.0));
    let forward = PreviousLayout { nodes: entries.iter().cloned().collect() };
    let backward = PreviousLayout { nodes: entries.iter().rev().cloned().collect() };

    let g2 = sample().node("extra", 50.0, 20.0).edge("n2", "extra").build();
    let r1 = run_with(&g2, &LayoutOptions::default(), &LayoutHints { previous: Some(forward), ..Default::default() });
    let r2 = run_with(&g2, &LayoutOptions::default(), &LayoutHints { previous: Some(backward), ..Default::default() });
    assert_eq!(r1, r2);
}
