//! Scenario spans use the same 1-based columns as `cascade_core` spans.

use cascade_core::span::Pos;
use cascade_sim::parse_scenario;

#[test]
fn step_columns_are_one_based() {
    let text = "scenario: s\nsteps:\n  - { source: Clock, fire: Order.timeout }\n";
    let scenario = parse_scenario(text).expect("parses");
    // `Clock` starts at the 15th character of line 3.
    assert_eq!(scenario.steps[0].source.span.start, Pos::new(3, 15));
}
