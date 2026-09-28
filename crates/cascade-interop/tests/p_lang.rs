//! P language skeleton export.

use std::collections::{BTreeMap, BTreeSet};

use cascade_core::{Model, load_str};
use cascade_interop::{ExportFormat, export};

const SPEC_EXAMPLE: &str = include_str!("../../../examples/order-fulfillment/cascade.yaml");

const WITH_PAYLOADS: &str = r#"
machines:
  Order:
    initial: draft
    fields: [orderId]
    states: [draft, pending, paid, cancelled]
    transitions:
      - { from: draft, to: pending, on: submit }
      - { from: pending, to: paid, on: capture_ok, emits: [OrderPaid] }
      - { from: pending, to: cancelled, on: timeout, emits: [OrderCancelled] }
  Shipment:
    fields: [orderId]
    states: [idle, picking, shipped]
    transitions:
      - { from: idle, to: picking, on: start }
events:
  OrderPaid: { payload: [orderId, amount] }
  OrderCancelled: {}
controllers:
  Fulfillment:
    on:
      OrderPaid:
        - fire: Shipment.start
          target: Shipment where orderId == event.orderId
        - fire: Shipment.start
          target: all Shipment where orderId == event.orderId
          when: "express\ndelivery"
        - fire: Shipment.start
          target: new Shipment with orderId = event.orderId
external:
  Customer: [Order.submit]
  Clock: [Order.timeout]
"#;

const NESTED: &str = r#"
machines:
  Job:
    initial: queued
    states:
      - queued
      - running:
          initial: fetching
          states:
            - fetching
            - computing
            - hist: { kind: history }
      - done: { kind: final }
    transitions:
      - { from: queued, to: running, on: start }
      - { from: running, to: queued, on: cancel }
      - { from: running.computing, to: done, on: cancel }
      - { from: running.fetching, to: running.computing, on: fetched }
      - { from: queued, to: running.hist, on: resume }
      - { from: queued, to: done, on: go, guard: "x > 0" }
      - { from: queued, to: running, on: go }
      - { from: queued, to: queued, on: poke, guard: "a\nb" }
"#;

fn load(text: &str) -> Model {
    match load_str(text) {
        Ok(model) => model,
        Err(err) => panic!("expected the definition to load:\n{err}"),
    }
}

fn p(text: &str) -> String {
    export(ExportFormat::P, &load(text)).expect("p export")
}

fn lines(text: &str) -> Vec<&str> {
    text.lines().map(str::trim).collect()
}

fn has_line(text: &str, line: &str) -> bool {
    lines(text).contains(&line)
}

/// The text of `machine <name> { … }`, found by brace matching.
fn machine_block<'a>(text: &'a str, name: &str) -> &'a str {
    let head = format!("machine {name} {{");
    let start = text.find(&head).unwrap_or_else(|| panic!("no machine {name} in\n{text}"));
    let mut depth = 0usize;
    for (offset, c) in text[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &text[start..start + offset + 1];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced machine {name}");
}

/// The text of `state <name> { … }` inside a machine block.
fn state_block<'a>(machine: &'a str, name: &str) -> &'a str {
    let head = format!("state {name} {{");
    let start = machine.find(&head).unwrap_or_else(|| panic!("no state {name} in\n{machine}"));
    let mut depth = 0usize;
    for (offset, c) in machine[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &machine[start..start + offset + 1];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced state {name}");
}

fn ident_after<'a>(text: &'a str, keyword: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find(keyword) {
        let before_ok = pos == 0 || !rest[..pos].ends_with(|c: char| c.is_alphanumeric() || c == '_');
        let after = &rest[pos + keyword.len()..];
        if before_ok {
            let ident: &str = after.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().unwrap_or("");
            if !ident.is_empty() {
                out.push(ident);
            }
        }
        rest = after;
    }
    out
}

/// A light structural check: comments stripped, braces balance, every
/// `goto` names a state of its machine, every sent, deferred or ignored
/// event is declared, and every machine in the test declaration exists.
fn check_structure(text: &str) {
    let code: String = text
        .lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let open = code.matches('{').count();
    let close = code.matches('}').count();
    assert_eq!(open, close, "unbalanced braces in\n{text}");

    let events: BTreeSet<&str> = ident_after(&code, "event ").into_iter().collect();
    let machines: Vec<&str> = ident_after(&code, "machine ").into_iter().filter(|m| *m != "machine").collect();
    assert!(!machines.is_empty());
    let mut all_states = BTreeMap::new();
    for machine in &machines {
        let block = machine_block(&code, machine);
        let states: BTreeSet<&str> = ident_after(block, "state ").into_iter().collect();
        assert!(block.contains("start state "), "machine {machine} has no start state");
        for target in ident_after(block, "goto ") {
            assert!(states.contains(target), "machine {machine}: goto {target} is not a declared state\n{text}");
        }
        for line in block.lines().map(str::trim) {
            if let Some(rest) = line.strip_prefix("send ") {
                let event = rest.split(',').nth(1).map(|e| e.trim().trim_end_matches(';')).unwrap_or_default();
                assert!(events.contains(event), "machine {machine}: sends undeclared event {event:?}");
            }
            for keyword in ["defer ", "ignore "] {
                if let Some(rest) = line.strip_prefix(keyword) {
                    for event in rest.trim_end_matches(';').split(',') {
                        assert!(events.contains(event.trim()), "machine {machine}: {keyword}{event} undeclared");
                    }
                }
            }
            if let Some(rest) = line.strip_prefix("on ") {
                let event = rest.split_whitespace().next().unwrap_or_default();
                assert!(events.contains(event), "machine {machine}: handles undeclared event {event}");
            }
        }
        all_states.insert(*machine, states);
    }
    let test_line = code.lines().find(|l| l.starts_with("test ")).expect("test declaration");
    let members = test_line.split('{').nth(1).and_then(|r| r.split('}').next()).expect("module list");
    for member in members.split(',') {
        assert!(machines.contains(&member.trim()), "test lists unknown machine {member}");
    }
}

// --- Declarations ------------------------------------------------------------

#[test]
fn declares_registry_wiring_and_trigger_events() {
    let out = p(SPEC_EXAMPLE);
    assert!(out.starts_with("//"), "header comment first:\n{out}");
    assert!(has_line(&out, "type tRegistry = map[string, machine];"), "{out}");
    assert!(has_line(&out, "event eWire: tRegistry;"), "{out}");
    for event in ["eOrder_submit", "eOrder_capture_ok", "eOrder_timeout", "eShipment_start", "eShipment_handoff"] {
        assert!(has_line(&out, &format!("event {event};")), "missing {event} in\n{out}");
    }
    for event in ["OrderPaid", "OrderCancelled", "Shipped"] {
        assert!(has_line(&out, &format!("event {event};")), "missing {event} in\n{out}");
    }
    check_structure(&out);
}

#[test]
fn declares_payload_types_for_declared_payloads() {
    let out = p(WITH_PAYLOADS);
    assert!(has_line(&out, "type tOrderPaid = (orderId: any, amount: any);"), "{out}");
    assert!(has_line(&out, "event OrderPaid: tOrderPaid;"), "{out}");
    assert!(has_line(&out, "event OrderCancelled;"), "{out}");
    assert!(!out.contains("tOrderCancelled"), "{out}");
    check_structure(&out);
}

#[test]
fn keyword_and_invalid_names_become_safe_identifiers() {
    let text = r#"
machines:
  machine:
    states: [state, start, draft-2, registry, Wiring]
    transitions:
      - { from: state, to: start, on: goto, emits: [event] }
      - { from: start, to: draft-2, on: send }
  TestDriver:
    states: [a]
controllers:
  on:
    on:
      event: { fire: machine.send }
"#;
    let out = p(text);
    assert!(out.contains("machine machine_ {"), "{out}");
    assert!(out.contains("state state_ {"), "{out}");
    assert!(out.contains("state start_ {"), "{out}");
    assert!(out.contains("state draft_2 {"), "{out}");
    // Generated names inside a machine are reserved too.
    assert!(out.contains("state registry_2 {"), "{out}");
    assert!(out.contains("state Wiring_2 {"), "{out}");
    assert!(has_line(&out, "event event_;"), "{out}");
    // The user's TestDriver machine does not clash with the generated driver.
    assert!(out.contains("machine TestDriver {"), "{out}");
    assert!(out.contains("machine TestDriver_2 {"), "{out}");
    assert!(out.contains("machine on_ {"), "{out}");
    check_structure(&out);
}

// --- Machines -------------------------------------------------------------------

#[test]
fn machines_wire_then_enter_the_initial_state() {
    let out = p(SPEC_EXAMPLE);
    let order = machine_block(&out, "Order");
    assert!(order.contains("var registry: tRegistry;"), "{order}");
    let wiring = state_block(order, "Wiring");
    assert!(order.contains("start state Wiring {"), "{order}");
    assert!(has_line(wiring, "defer eOrder_submit, eOrder_capture_ok, eOrder_timeout;"), "{wiring}");
    assert!(has_line(wiring, "on eWire goto draft with (r: tRegistry) {"), "{wiring}");
    assert!(has_line(wiring, "registry = r;"), "{wiring}");
}

#[test]
fn unguarded_transitions_are_gotos_and_emits_are_sends() {
    let out = p(WITH_PAYLOADS);
    let order = machine_block(&out, "Order");
    let draft = state_block(order, "draft");
    assert!(has_line(draft, "on eOrder_submit goto pending;"), "{draft}");
    assert!(has_line(draft, "ignore eOrder_capture_ok, eOrder_timeout;"), "{draft}");
    let pending = state_block(order, "pending");
    assert!(has_line(pending, "on eOrder_capture_ok goto paid with {"), "{pending}");
    assert!(has_line(pending, r#"send registry["Fulfillment"], OrderPaid, default(tOrderPaid);"#), "{pending}");
    assert!(has_line(pending, "// emits OrderCancelled: no controller subscribes"), "{pending}");
    assert!(has_line(pending, "ignore eOrder_submit;"), "{pending}");
    check_structure(&out);
}

#[test]
fn sends_without_payload_when_undeclared() {
    let out = p(SPEC_EXAMPLE);
    let order = machine_block(&out, "Order");
    let pending = state_block(order, "pending");
    assert!(has_line(pending, r#"send registry["Fulfillment"], OrderPaid;"#), "{pending}");
}

#[test]
fn final_states_ignore_every_trigger() {
    let out = p(NESTED);
    let job = machine_block(&out, "Job");
    let done = state_block(job, "done");
    assert!(has_line(done, "ignore eJob_start, eJob_cancel, eJob_fetched, eJob_resume, eJob_go, eJob_poke;"), "{done}");
    assert!(!done.contains("goto"), "{done}");
}

#[test]
fn nested_states_are_flattened_with_inherited_handlers() {
    let out = p(NESTED);
    let job = machine_block(&out, "Job");
    // Only atomic and final states become P states.
    assert!(!job.contains("state running {"), "{job}");
    assert!(!job.contains("state running_hist {"), "{job}");
    let fetching = state_block(job, "running_fetching");
    // Inherited from `running`.
    assert!(has_line(fetching, "on eJob_cancel goto queued;"), "{fetching}");
    assert!(has_line(fetching, "on eJob_fetched goto running_computing;"), "{fetching}");
    let computing = state_block(job, "running_computing");
    // Innermost wins: computing's own `cancel` beats running's.
    assert!(has_line(computing, "on eJob_cancel goto done;"), "{computing}");
    assert!(!computing.contains("goto queued"), "{computing}");
    // Entering the compound `running` enters its default child.
    let queued = state_block(job, "queued");
    assert!(queued.contains("goto running_fetching"), "{queued}");
    check_structure(&out);
}

#[test]
fn history_targets_are_approximated() {
    let out = p(NESTED);
    let queued = state_block(machine_block(&out, "Job"), "queued");
    assert!(queued.contains("on eJob_resume goto running_fetching;"), "{queued}");
    assert!(queued.contains("history approximated"), "{queued}");
}

#[test]
fn guarded_and_nondeterministic_choices_use_dollar() {
    let out = p(NESTED);
    let queued = state_block(machine_block(&out, "Job"), "queued");
    assert!(has_line(queued, "on eJob_go do {"), "{queued}");
    assert!(has_line(queued, "if ($) {"), "{queued}");
    assert!(has_line(queued, "// guard: x > 0"), "{queued}");
    assert!(has_line(queued, "goto done;"), "{queued}");
    assert!(has_line(queued, "} else {"), "{queued}");
    assert!(has_line(queued, "goto running_fetching;"), "{queued}");
    // A lone guarded transition has no else: the trigger is dropped when the
    // guard is false. Newlines in guards become spaces.
    assert!(has_line(queued, "on eJob_poke do {"), "{queued}");
    assert!(has_line(queued, "// guard: a b"), "{queued}");
    check_structure(&out);
}

#[test]
fn multiple_unguarded_candidates_are_a_nondeterministic_choice() {
    let text = r#"
machines:
  M:
    states: [a, b, c]
    transitions:
      - { from: a, to: b, on: go }
      - { from: a, to: c, on: go }
"#;
    let out = p(text);
    let a = state_block(machine_block(&out, "M"), "a");
    assert!(has_line(a, "on eM_go do {"), "{a}");
    assert!(has_line(a, "if ($) {"), "{a}");
    assert!(has_line(a, "goto b;"), "{a}");
    assert!(has_line(a, "} else {"), "{a}");
    assert!(has_line(a, "goto c;"), "{a}");
    check_structure(&out);
}

// --- Controllers ----------------------------------------------------------------

#[test]
fn controllers_forward_events_as_triggers() {
    let out = p(WITH_PAYLOADS);
    let ctl = machine_block(&out, "Fulfillment");
    let wiring = state_block(ctl, "Wiring");
    assert!(has_line(wiring, "defer OrderPaid;"), "{wiring}");
    assert!(has_line(wiring, "on eWire goto Running with (r: tRegistry) {"), "{wiring}");
    let running = state_block(ctl, "Running");
    assert!(has_line(running, "on OrderPaid do (payload: tOrderPaid) {"), "{running}");
    assert!(has_line(running, "// rule 0: fire Shipment.start"), "{running}");
    assert!(has_line(running, "// target: Shipment where orderId == event.orderId"), "{running}");
    assert!(has_line(running, r#"send registry["Shipment"], eShipment_start;"#), "{running}");
    check_structure(&out);
}

#[test]
fn controller_rules_with_conditions_fan_out_and_spawn() {
    let out = p(WITH_PAYLOADS);
    let running = state_block(machine_block(&out, "Fulfillment"), "Running");
    assert!(has_line(running, "// when: express delivery"), "{running}");
    assert!(has_line(running, "if ($) {"), "{running}");
    assert!(running.contains("TODO fan-out"), "{running}");
    assert!(running.contains("TODO spawn"), "{running}");
    assert_eq!(running.matches(r#"send registry["Shipment"], eShipment_start;"#).count(), 3, "{running}");
}

#[test]
fn handlers_without_payload_take_no_parameter() {
    let out = p(SPEC_EXAMPLE);
    let running = state_block(machine_block(&out, "Fulfillment"), "Running");
    assert!(has_line(running, "on OrderPaid do {"), "{running}");
}

// --- Driver ----------------------------------------------------------------------

#[test]
fn test_driver_creates_wires_and_drives() {
    let out = p(SPEC_EXAMPLE);
    let driver = machine_block(&out, "TestDriver");
    assert!(driver.contains("var registry: tRegistry;"), "{driver}");
    for name in ["Order", "Shipment", "Fulfillment"] {
        assert!(has_line(driver, &format!(r#"registry["{name}"] = new {name}();"#)), "{name} in\n{driver}");
    }
    assert!(has_line(driver, "foreach (m in values(registry)) {"), "{driver}");
    assert!(has_line(driver, "send m, eWire, registry;"), "{driver}");
    assert!(has_line(driver, "goto Driving;"), "{driver}");
    let driving = state_block(driver, "Driving");
    assert!(has_line(driving, "while (i < 10) {"), "{driving}");
    assert!(has_line(driving, "// Customer"), "{driving}");
    assert!(has_line(driving, r#"send registry["Order"], eOrder_submit;"#), "{driving}");
    assert!(has_line(driving, r#"send registry["Order"], eOrder_capture_ok;"#), "{driving}");
    assert!(has_line(driving, "// Clock"), "{driving}");
    check_structure(&out);
}

#[test]
fn test_declaration_lists_every_machine() {
    let out = p(SPEC_EXAMPLE);
    assert!(
        has_line(&out, "test tcSystem [main = TestDriver]: { TestDriver, Order, Shipment, Fulfillment };"),
        "{out}"
    );
}

#[test]
fn locals_are_declared_before_statements() {
    let out = p(SPEC_EXAMPLE);
    let driver = machine_block(&out, "TestDriver");
    for block in ["entry {"] {
        for (i, _) in driver.match_indices(block) {
            let body: Vec<&str> = driver[i + block.len()..]
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with("//"))
                .collect();
            let first_statement = body.iter().position(|l| !l.starts_with("var ")).unwrap_or(body.len());
            assert!(
                body[first_statement..].iter().take_while(|l| **l != "}").all(|l| !l.starts_with("var ")),
                "{driver}"
            );
        }
    }
}

#[test]
fn no_external_sources_still_produces_a_driver() {
    let text = "machines:\n  M:\n    states: [a, b]\n    transitions:\n      - { from: a, to: b, on: go }\n";
    let out = p(text);
    let driver = machine_block(&out, "TestDriver");
    assert!(driver.contains("state Driving {"), "{driver}");
    check_structure(&out);
}

#[test]
fn every_example_passes_the_structural_check() {
    for text in [SPEC_EXAMPLE, WITH_PAYLOADS, NESTED] {
        check_structure(&p(text));
    }
}

#[test]
fn output_is_deterministic() {
    assert_eq!(p(WITH_PAYLOADS), p(WITH_PAYLOADS));
}
