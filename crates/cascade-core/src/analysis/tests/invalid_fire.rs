use super::*;

const MODEL: &str = r#"
machines:
  Order:
    states: [draft, pending]
    transitions:
      - { from: draft, to: pending, on: submit, emits: [Submitted] }
  Shipment:
    states:
      - idle
      - active:
          states: [picking, packing]
    transitions:
      - { from: idle, to: active, on: start }
      - { from: active.picking, to: active.packing, on: pack }
controllers:
  Fulfillment:
    on:
      Submitted:
        - fire: Shipment.start
        - fire: Shipment.begin
        - fire: Shipment.pack
external:
  Customer: [Order.submit, Order.cancel]
"#;

#[test]
fn rule_firing_an_unaccepted_trigger_is_an_error() {
    let (model, findings) = run(MODEL);
    let invalid = of(&findings, Check::InvalidFire);
    assert_eq!(invalid.len(), 2, "{findings:#?}");

    let rule_finding = invalid
        .iter()
        .find(|f| matches!(f.detail, FindingDetail::InvalidFire { .. }))
        .unwrap_or_else(|| panic!("no rule finding: {findings:#?}"));
    assert_eq!(rule_finding.severity, Severity::Error);
    assert_eq!(
        keys(&model, &rule_finding.detail.subjects()),
        ["rule:Fulfillment/Submitted#1", "trigger:Shipment.begin"]
    );
    assert_eq!(key(&model, rule_finding.detail.primary()), "rule:Fulfillment/Submitted#1");
    assert_eq!(
        rule_finding.message,
        "Fulfillment fires Shipment.begin on Submitted, but no Shipment transition accepts `begin`"
    );
}

#[test]
fn external_dead_trigger_is_reported_under_invalid_fire() {
    let (model, findings) = run(MODEL);
    let dead = findings
        .iter()
        .find(|f| matches!(f.detail, FindingDetail::DeadExternalTrigger { .. }))
        .unwrap_or_else(|| panic!("no dead external trigger: {findings:#?}"));
    assert_eq!(dead.check(), Check::InvalidFire);
    assert_eq!(dead.severity, Severity::Error);
    assert_eq!(key(&model, dead.detail.primary()), "external:Customer");
    assert_eq!(keys(&model, &dead.detail.subjects()), ["external:Customer", "trigger:Order.cancel"]);
    assert_eq!(dead.message, "Customer can fire Order.cancel, but no Order transition accepts `cancel`");
}

#[test]
fn triggers_accepted_only_from_nested_or_other_states_are_valid() {
    // `pack` is accepted only from `active.picking`, which is not an invalid
    // fire (at most a state-dependent one).
    let (model, findings) = run(MODEL);
    let pack = element(&model, "rule:Fulfillment/Submitted#2");
    assert!(
        !of(&findings, Check::InvalidFire).iter().any(|f| f.detail.primary() == pack),
        "pack is accepted somewhere: {findings:#?}"
    );
}

#[test]
fn a_trigger_accepted_by_another_machine_does_not_count() {
    let (model, findings) = run(r#"
machines:
  A:
    states: [a0, a1]
    transitions:
      - { from: a0, to: a1, on: go, emits: [Went] }
  B:
    states: [b0]
controllers:
  C:
    on:
      Went: [{ fire: B.go }]
external:
  User: [A.go]
"#);
    let f = single(&findings, Check::InvalidFire);
    assert_eq!(
        f.detail,
        FindingDetail::InvalidFire {
            rule: match element(&model, "rule:C/Went#0") {
                ElementRef::Rule(r) => r,
                other => panic!("{other:?}"),
            },
            trigger: match element(&model, "trigger:B.go") {
                ElementRef::Trigger(t) => t,
                other => panic!("{other:?}"),
            },
        }
    );
}

#[test]
fn one_finding_per_rule_and_per_exposing_source() {
    let (model, findings) = run(r#"
machines:
  A:
    states: [a0, a1]
    transitions:
      - { from: a0, to: a1, on: go, emits: [Went] }
controllers:
  C1:
    on:
      Went: [{ fire: A.stop }]
  C2:
    on:
      Went: [{ fire: A.stop }]
external:
  User: [A.go, A.stop, A.halt]
  Admin: [A.stop]
"#);
    let primaries: Vec<String> =
        of(&findings, Check::InvalidFire).iter().map(|f| key(&model, f.detail.primary())).collect();
    assert_eq!(primaries, ["rule:C1/Went#0", "rule:C2/Went#0", "external:User", "external:User", "external:Admin"]);
    let user_triggers: Vec<String> = of(&findings, Check::InvalidFire)
        .iter()
        .filter_map(|f| match f.detail {
            FindingDetail::DeadExternalTrigger { trigger, .. } => Some(key(&model, ElementRef::Trigger(trigger))),
            _ => None,
        })
        .collect();
    assert_eq!(user_triggers, ["trigger:A.stop", "trigger:A.halt", "trigger:A.stop"]);
}

#[test]
fn valid_fires_and_sources_give_no_invalid_fire() {
    let (_, findings) = run(examples::SPEC_EXAMPLE);
    assert!(of(&findings, Check::InvalidFire).is_empty());
}
