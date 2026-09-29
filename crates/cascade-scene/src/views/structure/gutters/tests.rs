use super::order::{WiringNode, order};
use super::*;

/// Gutters around three machines, the second with a nested band:
///
/// ```text
/// 0 gutter 0
/// 1 A
/// 2 gutter 1
/// 3 B (top)
/// 4 B (nested band)
/// 5 gutter 2
/// 6 C
/// 7 gutter 3
/// ```
fn stack() -> Stack {
    Stack::new(&[true, false, true, false, false, true, false, true])
}

const A: usize = 1;
const B_TOP: usize = 3;
const B_BAND: usize = 4;
const C: usize = 6;

fn at(position: usize, column: f32) -> LaneEnd {
    LaneEnd { position, column }
}

#[test]
fn lanes_between_counts_only_lane_groups() {
    let s = stack();
    assert_eq!(s.gutter_count(), 4);
    assert_eq!(s.group(Gutter(2)), Some(5));
    assert_eq!(s.lanes_between(0, 1), 0, "gutter 0 borders A");
    assert_eq!(s.lanes_between(2, 3), 0);
    assert_eq!(s.lanes_between(2, B_BAND), 1, "B's top lies between gutter 1 and B's band");
    assert_eq!(s.lanes_between(0, 7), 4);
    assert_eq!(s.lanes_between(7, 0), 4, "symmetric");
    assert_eq!(s.lanes_between(3, 3), 0);
}

#[test]
fn an_event_sits_between_its_emitter_and_the_lane_its_handler_fires_into() {
    let wires = Wires {
        events: vec![EventWires { emitters: vec![at(A, 1.0)], handled_into: vec![vec![at(B_TOP, 1.0)]] }],
        controllers: vec![ControllerWires { fires: vec![at(B_TOP, 1.0)], events: vec![0] }],
        sources: vec![],
    };
    let a = assign(&stack(), &wires);
    assert_eq!(a.events, [Gutter(1)]);
    assert_eq!(a.controllers, [Gutter(1)], "with its event, right above the lane it fires into");
}

#[test]
fn an_unhandled_event_goes_below_its_emitter() {
    let wires =
        Wires { events: vec![EventWires { emitters: vec![at(C, 1.0)], handled_into: vec![] }], ..Wires::default() };
    assert_eq!(assign(&stack(), &wires).events, [Gutter(3)]);
}

#[test]
fn nested_bands_count_as_lanes() {
    // Emitted from B's nested band: only gutter 2 borders it.
    let wires = Wires {
        events: vec![EventWires { emitters: vec![at(B_BAND, 1.0)], handled_into: vec![] }],
        ..Wires::default()
    };
    assert_eq!(assign(&stack(), &wires).events, [Gutter(2)]);
    // Emitted from B's top lane: gutter 1 borders it, gutter 2 does not.
    let wires =
        Wires { events: vec![EventWires { emitters: vec![at(B_TOP, 1.0)], handled_into: vec![] }], ..Wires::default() };
    assert_eq!(assign(&stack(), &wires).events, [Gutter(1)]);
}

#[test]
fn fewer_corridors_beat_shorter_detours() {
    // Two emitters in A and one in C: gutter 1 needs one corridor (to C),
    // gutter 3 two (to A).
    let wires = Wires {
        events: vec![EventWires { emitters: vec![at(A, 0.0), at(A, 2.0), at(C, 1.0)], handled_into: vec![] }],
        ..Wires::default()
    };
    assert_eq!(assign(&stack(), &wires).events, [Gutter(1)]);
}

#[test]
fn a_controller_sits_above_the_lane_it_fires_into_near_its_events() {
    // Fires into C; its event is emitted from A and placed in gutter 1.
    let wires = Wires {
        events: vec![EventWires { emitters: vec![at(A, 0.0)], handled_into: vec![vec![at(C, 0.0)]] }],
        controllers: vec![
            ControllerWires { fires: vec![at(C, 0.0)], events: vec![0] },
            ControllerWires { fires: vec![at(C, 0.0)], events: vec![] },
        ],
        sources: vec![],
    };
    let a = assign(&stack(), &wires);
    // The event's two wires cost one corridor in gutter 1 or 2; the lower
    // one wins the tie.
    assert_eq!(a.events, [Gutter(2)]);
    assert_eq!(a.controllers, [Gutter(2), Gutter(2)], "right above C, with the event");
}

#[test]
fn a_source_sits_above_the_topmost_lane_it_triggers() {
    let wires = Wires {
        sources: vec![
            SourceWires { triggers: vec![at(A, 0.0)] },
            SourceWires { triggers: vec![at(C, 0.0), at(C, 3.0)] },
            // Gutters 1 and 2 both cost one corridor: the upper one wins.
            SourceWires { triggers: vec![at(A, 0.0), at(C, 0.0)] },
            // Gutter 2 crosses only B's nested band to reach B's top.
            SourceWires { triggers: vec![at(B_TOP, 0.0), at(C, 0.0)] },
        ],
        ..Wires::default()
    };
    assert_eq!(assign(&stack(), &wires).sources, [Gutter(0), Gutter(2), Gutter(1), Gutter(2)]);
}

#[test]
fn nodes_without_wires_go_last() {
    let wires = Wires {
        events: vec![EventWires::default()],
        controllers: vec![ControllerWires { fires: vec![], events: vec![0] }, ControllerWires::default()],
        sources: vec![SourceWires::default()],
    };
    let a = assign(&stack(), &wires);
    assert_eq!(a.events, [Gutter(3)]);
    assert_eq!(a.controllers, [Gutter(3), Gutter(3)]);
    assert_eq!(a.sources, [Gutter(3)]);
}

#[test]
fn an_unemitted_event_joins_its_controller() {
    let wires = Wires {
        events: vec![EventWires::default()],
        controllers: vec![ControllerWires { fires: vec![at(A, 0.0)], events: vec![0] }],
        sources: vec![],
    };
    let a = assign(&stack(), &wires);
    assert_eq!(a.controllers, [Gutter(0)]);
    assert_eq!(a.events, [Gutter(0)]);
}

#[test]
fn rows_follow_the_median_column_of_their_pills() {
    let wires = Wires {
        events: vec![
            EventWires { emitters: vec![at(A, 5.0)], handled_into: vec![] },
            EventWires { emitters: vec![at(A, 1.0), at(A, 9.0), at(A, 2.0)], handled_into: vec![] },
            EventWires::default(),
        ],
        controllers: vec![],
        sources: vec![SourceWires { triggers: vec![at(A, 3.0)] }],
    };
    let assignment = Assignment { events: vec![Gutter(1); 3], controllers: vec![], sources: vec![Gutter(1)] };
    let rows = order(&wires, &assignment);
    assert_eq!(
        rows[&Gutter(1)],
        [WiringNode::Event(1), WiringNode::Source(0), WiringNode::Event(0), WiringNode::Event(2)],
        "medians 2, 3, 5, then the event without pills"
    );
}

#[test]
fn a_controller_follows_its_last_event_in_the_gutter() {
    let wires = Wires {
        events: vec![
            EventWires { emitters: vec![at(A, 1.0)], handled_into: vec![] },
            EventWires { emitters: vec![at(A, 4.0)], handled_into: vec![] },
            EventWires { emitters: vec![at(A, 8.0)], handled_into: vec![] },
        ],
        controllers: vec![
            // Fires far left, but handles events 0 and 1 here.
            ControllerWires { fires: vec![at(B_TOP, 0.0)], events: vec![0, 1] },
            // Handles an event in another gutter only: keyed by its fires.
            ControllerWires { fires: vec![at(B_TOP, 6.0)], events: vec![2] },
        ],
        sources: vec![],
    };
    let assignment = Assignment {
        events: vec![Gutter(1), Gutter(1), Gutter(0)],
        controllers: vec![Gutter(1), Gutter(1)],
        sources: vec![],
    };
    let rows = order(&wires, &assignment);
    assert_eq!(
        rows[&Gutter(1)],
        [WiringNode::Event(0), WiringNode::Event(1), WiringNode::Controller(0), WiringNode::Controller(1)]
    );
    assert_eq!(rows[&Gutter(0)], [WiringNode::Event(2)]);
}

#[test]
fn medians() {
    assert_eq!(order::median(&[]), None);
    assert_eq!(order::median(&[at(A, 3.0)]), Some(3.0));
    assert_eq!(order::median(&[at(A, 4.0), at(A, 1.0)]), Some(2.5));
}
