//! History pseudo-states restore per instance.

mod common;

use common::{finals, lines, model, run};

const PLAYER: &str = r#"
machines:
  Player:
    initial: stopped
    states:
      - stopped
      - playing:
          initial: normal
          states:
            - normal
            - fast:
                initial: x2
                states: [x2, x4]
            - shallow: { kind: history }
            - deep: { kind: deep-history }
      - paused
    transitions:
      - { from: stopped, to: playing, on: play }
      - { from: stopped, to: playing.deep, on: jump }
      - { from: playing.normal, to: playing.fast, on: ff }
      - { from: playing.fast.x2, to: playing.fast.x4, on: ff }
      - { from: playing, to: paused, on: pause }
      - { from: paused, to: playing.shallow, on: resume }
      - { from: paused, to: playing.deep, on: resume_deep }
external:
  User: [Player.play, Player.jump, Player.ff, Player.pause, Player.resume, Player.resume_deep]
"#;

fn steps(triggers: &[&str]) -> String {
    let mut text = String::from("scenario: player\ninstances:\n  p1: Player\n  p2: Player\nsteps:\n");
    for trigger in triggers {
        let (target, trigger) = trigger.split_once(':').unwrap_or(("p1", trigger));
        text.push_str(&format!("  - {{ source: User, fire: Player.{trigger}, target: {target} }}\n"));
    }
    text
}

#[test]
fn shallow_history_reenters_the_active_child_by_default() {
    let model = model(PLAYER);
    let trace = run(&model, &steps(&["play", "ff", "ff", "pause", "resume"]));
    let got = lines(&model, &trace);
    assert_eq!(got[9], "p1 paused -> playing.fast.x2");
    assert_eq!(finals(&model, &trace)[0], "p1: playing.fast.x2");
}

#[test]
fn deep_history_restores_the_last_leaf() {
    let model = model(PLAYER);
    let trace = run(&model, &steps(&["play", "ff", "ff", "pause", "resume_deep"]));
    assert_eq!(lines(&model, &trace)[9], "p1 paused -> playing.fast.x4");
}

#[test]
fn history_with_nothing_recorded_enters_the_parent_by_default() {
    let model = model(PLAYER);
    let trace = run(&model, &steps(&["jump"]));
    assert_eq!(lines(&model, &trace)[1], "p1 stopped -> playing.normal");
}

#[test]
fn history_tracks_the_most_recent_visit() {
    let model = model(PLAYER);
    // Each resume restores what the latest pause left: x4, then x4 again
    // (entered shallowly as x2), then x2.
    let trace =
        run(&model, &steps(&["play", "ff", "ff", "pause", "resume_deep", "pause", "resume", "pause", "resume_deep"]));
    let got = lines(&model, &trace);
    assert_eq!(got[9], "p1 paused -> playing.fast.x4");
    assert_eq!(got[13], "p1 paused -> playing.fast.x2");
    assert_eq!(got[17], "p1 paused -> playing.fast.x2");
}

#[test]
fn history_is_per_instance() {
    let model = model(PLAYER);
    let trace =
        run(&model, &steps(&["play", "ff", "ff", "pause", "p2:play", "p2:pause", "resume_deep", "p2:resume_deep"]));
    assert_eq!(finals(&model, &trace), ["p1: playing.fast.x4", "p2: playing.normal"]);
}
