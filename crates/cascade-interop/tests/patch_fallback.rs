//! The shapes `patch_text` does not edit in place. They fall back to
//! re-emitting the file with `to_yaml` (`rewritten: true`) once
//! `edit::apply` provides the edited definition; before that, they report
//! `PatchError::NotImplemented`. Either way nothing wrong is written.

mod patch_common;

use cascade_core::edit::EditOp;
use cascade_core::load_str;
use cascade_interop::{PatchError, Patched, patch_text};
use patch_common::*;

fn assert_falls_back(text: &str, op: &EditOp) {
    match patch_text(text, op) {
        Err(PatchError::NotImplemented) => {}
        Ok(Patched { text: out, rewritten: true }) => {
            if let Err(err) = load_str(&out) {
                panic!("the rewrite does not resolve:\n{err}\n---\n{out}");
            }
        }
        other => panic!("expected a fallback for {op:?}, got {other:?}"),
    }
}

#[test]
fn a_block_scalar_is_never_edited_in_place() {
    let text = "machines:\n  A:\n    states: [x, y]\n    transitions:\n      - from: x\n        to: y\n        on: go\n        guard: |\n          n > 0\n";
    let mut t = guarded(transition(&["x"], "y", "go"), "n > 1");
    t.bounded = false;
    assert_falls_back(text, &EditOp::UpdateTransition { machine: "A".into(), index: 0, transition: t });
}

#[test]
fn a_single_rule_block_mapping_is_not_turned_into_a_list() {
    let text = "machines:\n  A:\n    states: [x, y]\n    transitions:\n      - { from: x, to: y, on: go, emits: [E] }\ncontrollers:\n  C:\n    on:\n      E:\n        fire: A.go\n        when: sometimes\n";
    assert_falls_back(
        text,
        &EditOp::AddRule { controller: "C".into(), event: "E".into(), rule: rule("A.go", None, None), index: None },
    );
}

#[test]
fn tagged_nodes_are_left_to_the_rewrite() {
    let text = "machines:\n  A:\n    color: !paint blue\n    states: [x, y]\n";
    assert_falls_back(text, &EditOp::SetMachineDomain { machine: "A".into(), domain: Some("core".into()) });
}

#[test]
fn a_flow_list_with_comments_inside_is_not_restructured() {
    let text = "machines:\n  A:\n    states: [x,   # first\n      y]\n    transitions: []\n";
    assert_falls_back(
        text,
        &EditOp::AddState { machine: "A".into(), parent: None, state: final_state("z"), index: None },
    );
}
