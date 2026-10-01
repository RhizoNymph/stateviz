//! `ViewState::group_by_machine` in `cascade://` links: `lanes=1`, left out
//! when off.

use cascade_scene::{ViewKind, ViewLinkError, ViewState};

#[test]
fn lanes_are_left_out_when_off() {
    assert_eq!(ViewState::default().to_link(), "cascade://causal");
    assert!(!ViewState::from_link("cascade://causal").expect("parses").group_by_machine);
}

#[test]
fn lanes_round_trip() {
    let state = ViewState { group_by_machine: true, ..ViewState::default() };
    let link = state.to_link();
    assert_eq!(link, "cascade://causal?lanes=1");
    assert_eq!(ViewState::from_link(&link), Ok(state));
}

#[test]
fn lanes_round_trip_with_other_parameters() {
    let state = ViewState {
        view: ViewKind::Causal,
        selection: vec!["transition:Order:pending->paid@capture_ok".parse().expect("key")],
        hidden_machines: ["Payment".to_owned()].into_iter().collect(),
        group_by_machine: true,
        search: Some("paid".to_owned()),
        ..ViewState::default()
    };
    let link = state.to_link();
    assert!(link.contains("lanes=1"), "{link}");
    assert_eq!(ViewState::from_link(&link), Ok(state));
}

#[test]
fn lanes_zero_parses_as_off() {
    assert_eq!(ViewState::from_link("cascade://causal?lanes=0"), Ok(ViewState::default()));
}

#[test]
fn other_lane_values_are_rejected() {
    for bad in ["cascade://causal?lanes=yes", "cascade://causal?lanes=", "cascade://causal?lanes=2"] {
        assert!(matches!(ViewState::from_link(bad), Err(ViewLinkError::InvalidValue { param: "lanes", .. })), "{bad}");
    }
}

#[test]
fn lanes_survive_in_other_views() {
    // The flag belongs to the view state, so switching views keeps it.
    let state = ViewState { view: ViewKind::Structure, group_by_machine: true, ..ViewState::default() };
    assert_eq!(ViewState::from_link(&state.to_link()), Ok(state));
}
