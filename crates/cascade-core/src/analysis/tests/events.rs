use super::*;

#[test]
fn emitted_events_without_handlers_are_unhandled() {
    let (model, findings) = run(examples::SPEC_EXAMPLE);
    let unhandled = of(&findings, Check::UnhandledEvent);
    assert_eq!(
        unhandled.iter().map(|f| key(&model, f.detail.primary())).collect::<Vec<_>>(),
        ["event:OrderCancelled", "event:Shipped"]
    );
    assert!(unhandled.iter().all(|f| f.severity == Severity::Warning));
    assert_eq!(
        unhandled[0].message,
        "OrderCancelled is emitted by Order: pending → cancelled, but no controller subscribes to it"
    );
}

const EVENTS: &str = r#"
events: [Started, Unused, Ghost, Loud]
machines:
  M:
    states: [a, b, c]
    transitions:
      - { from: a, to: b, on: go, emits: [Started] }
      - { from: b, to: c, on: go, emits: [Started] }
      - { from: c, to: a, on: reset, emits: [Loud] }
controllers:
  Watcher:
    on:
      Ghost: [{ fire: M.reset }]
  Auditor:
    on:
      Ghost: [{ fire: M.reset }]
      Loud: [{ fire: M.go }]
external:
  User: [M.go]
"#;

#[test]
fn multiple_emitters_are_summarized() {
    let (model, findings) =
        run(&EVENTS
            .replace("  Auditor:\n    on:\n      Ghost: [{ fire: M.reset }]\n      Loud: [{ fire: M.go }]\n", ""));
    let f = of(&findings, Check::UnhandledEvent)
        .into_iter()
        .find(|f| key(&model, f.detail.primary()) == "event:Started")
        .unwrap_or_else(|| panic!("{findings:#?}"));
    assert_eq!(f.message, "Started is emitted by M: a → b and 1 other transition, but no controller subscribes to it");
}

#[test]
fn declared_but_unused_events_are_not_reported() {
    let (model, findings) = run(EVENTS);
    let primaries: Vec<String> =
        of(&findings, Check::UnhandledEvent).iter().map(|f| key(&model, f.detail.primary())).collect();
    assert_eq!(primaries, ["event:Started"]);
}

#[test]
fn orphan_controllers_are_reported_per_handler() {
    let (model, findings) = run(EVENTS);
    let orphans = of(&findings, Check::OrphanController);
    assert_eq!(
        orphans.iter().map(|f| key(&model, f.detail.primary())).collect::<Vec<_>>(),
        ["handler:Watcher/Ghost", "handler:Auditor/Ghost"]
    );
    assert!(orphans.iter().all(|f| f.severity == Severity::Warning));
    assert_eq!(orphans[0].message, "Watcher subscribes to Ghost, but no transition emits it");
    assert_eq!(keys(&model, &orphans[0].detail.subjects()), ["handler:Watcher/Ghost"]);
}

#[test]
fn handled_and_emitted_events_are_fine() {
    let (_, findings) = run(r#"
machines:
  M:
    states: [a, b]
    transitions:
      - { from: a, to: b, on: go, emits: [Went] }
      - { from: b, to: a, on: back }
controllers:
  C:
    on:
      Went: [{ fire: M.back }]
external:
  User: [M.go]
"#);
    assert!(of(&findings, Check::UnhandledEvent).is_empty(), "{findings:#?}");
    assert!(of(&findings, Check::OrphanController).is_empty(), "{findings:#?}");
}
