//! Mermaid export: `stateDiagram-v2` structure and `flowchart LR` causal
//! graph.

use cascade_core::{Model, load_str};
use cascade_interop::{ExportFormat, export};

const SPEC_EXAMPLE: &str = include_str!("../../../examples/order-fulfillment/cascade.yaml");

const NESTED: &str = r#"
system: "Shop: v2"
machines:
  Job:
    color: purple
    initial: running.fetching
    states:
      - queued
      - running:
          initial: fetching
          states:
            - fetching
            - computing
            - hist: { kind: deep-history }
            - shallow: { kind: history }
      - done: { kind: final }
    transitions:
      - { from: queued, to: running, on: start }
      - { from: running.fetching, to: running.computing, on: fetched, guard: 'amount > 0; x: "y" # z %% c' }
      - { from: running.computing, to: done, on: finish, emits: [JobDone] }
      - { from: running, to: queued, on: cancel }
      - { from: queued, to: running.hist, on: resume }
      - { from: queued, to: queued, on: poke }
  Order-Flow:
    states: [queued, done]
    transitions:
      - { from: queued, to: done, on: go }
"#;

fn load(text: &str) -> Model {
    match load_str(text) {
        Ok(model) => model,
        Err(err) => panic!("expected the definition to load:\n{err}"),
    }
}

fn structure(text: &str) -> String {
    export(ExportFormat::Mermaid, &load(text)).expect("mermaid export")
}

fn causal(text: &str) -> String {
    export(ExportFormat::MermaidCausal, &load(text)).expect("mermaid causal export")
}

fn lines(text: &str) -> Vec<&str> {
    text.lines().map(str::trim).collect()
}

fn has_line(text: &str, line: &str) -> bool {
    lines(text).contains(&line)
}

/// The composite blocks (by id) enclosing each line of a state diagram.
fn scopes(text: &str) -> Vec<(Vec<String>, String)> {
    let mut stack: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line == "}" {
            stack.pop();
            continue;
        }
        out.push((stack.clone(), line.to_owned()));
        if let Some(head) = line.strip_suffix(" {") {
            let id = head.rsplit(" as ").next().unwrap_or(head).to_owned();
            stack.push(id);
        }
    }
    out
}

fn scope_of(text: &str, line: &str) -> Vec<String> {
    scopes(text)
        .into_iter()
        .find(|(_, l)| l == line)
        .map(|(scope, _)| scope)
        .unwrap_or_else(|| panic!("line {line:?} not found in:\n{text}"))
}

// --- Structure --------------------------------------------------------------

#[test]
fn structure_header_and_machine_composites() {
    let out = structure(SPEC_EXAMPLE);
    assert_eq!(out.lines().next(), Some("stateDiagram-v2"));
    assert!(has_line(&out, r#"state "Order" as Order {"#), "{out}");
    assert!(has_line(&out, r#"state "Shipment" as Shipment {"#), "{out}");
    assert!(has_line(&out, r#"state "draft" as Order_draft"#), "{out}");
    assert!(has_line(&out, "[*] --> Order_draft"), "{out}");
    assert!(has_line(&out, "Order_draft --> Order_pending : submit"), "{out}");
    assert!(has_line(&out, "Shipment_picking --> Shipment_shipped : handoff"), "{out}");
    assert_eq!(scope_of(&out, "Order_draft --> Order_pending : submit"), ["Order"]);
    assert_eq!(scope_of(&out, "[*] --> Shipment_idle"), ["Shipment"]);
    assert!(out.ends_with("}\n"), "{out}");
    // No front matter without a system name.
    assert!(!out.starts_with("---"));
}

#[test]
fn structure_front_matter_title() {
    let out = structure(NESTED);
    let head: Vec<&str> = out.lines().take(4).collect();
    assert_eq!(head, ["---", r#"title: "Shop: v2""#, "---", "stateDiagram-v2"]);
}

#[test]
fn structure_nests_compound_states() {
    let out = structure(NESTED);
    assert!(has_line(&out, r#"state "running" as Job_running {"#), "{out}");
    assert_eq!(scope_of(&out, r#"state "fetching" as Job_running_fetching"#), ["Job", "Job_running"]);
    assert_eq!(scope_of(&out, r#"state "queued" as Job_queued"#), ["Job"]);
    // Compound initial inside the compound; machine initial lifted to the
    // top-level ancestor of `running.fetching`.
    assert_eq!(scope_of(&out, "[*] --> Job_running_fetching"), ["Job", "Job_running"]);
    assert_eq!(scope_of(&out, "[*] --> Job_running"), ["Job"]);
}

#[test]
fn structure_marks_finals_and_history() {
    let out = structure(NESTED);
    assert_eq!(scope_of(&out, "Job_done --> [*]"), ["Job"]);
    assert!(has_line(&out, r#"state "H*" as Job_running_hist"#), "{out}");
    assert!(has_line(&out, r#"state "H" as Job_running_shallow"#), "{out}");
}

#[test]
fn structure_ids_are_sanitized_and_unique() {
    let out = structure(NESTED);
    assert!(has_line(&out, r#"state "Order-Flow" as Order_Flow {"#), "{out}");
    assert!(has_line(&out, r#"state "queued" as Order_Flow_queued"#), "{out}");
    assert!(has_line(&out, "Order_Flow_queued --> Order_Flow_done : go"), "{out}");

    let collide = r#"
machines:
  A:
    states:
      - b_c
      - b: { states: [c] }
    transitions:
      - { from: b_c, to: b.c, on: go }
"#;
    let out = structure(collide);
    assert!(has_line(&out, r#"state "b_c" as A_b_c"#), "{out}");
    assert!(has_line(&out, r#"state "c" as A_b_c_2"#), "{out}");
    assert!(has_line(&out, "A_b_c --> A_b : go (b_c → b.c)"), "{out}");

    // Every declared id is unique and uses only safe characters.
    let mut ids = Vec::new();
    for line in lines(&out) {
        if let Some(rest) = line.strip_prefix("state ") {
            let id = rest.trim_end_matches(" {").rsplit(" as ").next().unwrap_or_default().to_owned();
            assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{id}");
            assert!(!ids.contains(&id), "duplicate id {id}");
            ids.push(id);
        }
    }
    assert_eq!(ids.len(), 4);
}

#[test]
fn structure_ids_avoid_keywords_and_non_ascii() {
    let text = r#"
machines:
  note:
    states: [end, "état"]
    transitions:
      - { from: end, to: "état", on: go }
"#;
    let out = structure(text);
    assert!(has_line(&out, r#"state "note" as note_ {"#), "{out}");
    assert!(has_line(&out, r#"state "état" as note_tat"#) || has_line(&out, r#"state "état" as note__tat"#), "{out}");
    for line in lines(&out) {
        if let Some(rest) = line.strip_prefix("state ") {
            let id = rest.trim_end_matches(" {").rsplit(" as ").next().unwrap_or_default();
            assert!(id.is_ascii(), "{id}");
            assert_ne!(id.to_ascii_lowercase(), "note");
        }
    }
}

#[test]
fn structure_escapes_guard_labels() {
    let out = structure(NESTED);
    let expected = "Job_running_fetching --> Job_running_computing : fetched [amount #62; 0#59; x#58; #quot;y#quot; #35; z #37;#37; c]";
    assert!(has_line(&out, expected), "{out}");
    assert_eq!(scope_of(&out, expected), ["Job", "Job_running"]);
}

#[test]
fn structure_lifts_cross_composite_transitions() {
    let out = structure(NESTED);
    // running.computing → done crosses out of `running`: drawn in Job from
    // the `running` composite, labelled with the real endpoints.
    let finish = "Job_running --> Job_done : finish (running.computing → done)";
    assert!(has_line(&out, finish), "{out}");
    assert_eq!(scope_of(&out, finish), ["Job"]);
    let resume = "Job_queued --> Job_running : resume (queued → running.hist)";
    assert!(has_line(&out, resume), "{out}");
    // Transitions between direct children are not annotated.
    assert!(has_line(&out, "Job_running --> Job_queued : cancel"), "{out}");
    assert!(has_line(&out, "Job_queued --> Job_running : start"), "{out}");
    assert!(has_line(&out, "Job_queued --> Job_queued : poke"), "{out}");
}

#[test]
fn structure_transition_from_parent_to_child() {
    let text = r#"
machines:
  M:
    states:
      - p: { states: [a, b] }
    transitions:
      - { from: p, to: p.b, on: jump }
"#;
    let out = structure(text);
    let line = "M_p --> M_p : jump (p → p.b)";
    assert!(has_line(&out, line), "{out}");
    assert_eq!(scope_of(&out, line), ["M"]);
}

#[test]
fn structure_braces_balance() {
    for text in [SPEC_EXAMPLE, NESTED] {
        let out = structure(text);
        let open = out.matches(" {\n").count();
        let close = lines(&out).iter().filter(|l| **l == "}").count();
        assert_eq!(open, close, "{out}");
    }
}

#[test]
fn structure_is_deterministic() {
    assert_eq!(structure(NESTED), structure(NESTED));
}

// --- Causal -------------------------------------------------------------------

#[test]
fn causal_nodes_for_the_spec_example() {
    let out = causal(SPEC_EXAMPLE);
    assert_eq!(out.lines().next(), Some("flowchart LR"));
    for node in [
        r#"s0[/"Customer"/]"#,
        r#"s1[/"PaymentGateway"/]"#,
        r#"s2[/"Clock"/]"#,
        r#"t0(["Order: draft → pending"])"#,
        r#"t1(["Order: pending → paid"])"#,
        r#"t2(["Order: pending → cancelled"])"#,
        r#"t3(["Shipment: idle → picking"])"#,
        r#"t4(["Shipment: picking → shipped"])"#,
        r#"e0>"OrderPaid"]"#,
        r#"e1>"OrderCancelled"]"#,
        r#"e2>"Shipped"]"#,
        r#"c0{{"Fulfillment"}}"#,
    ] {
        assert!(has_line(&out, node), "missing {node} in\n{out}");
    }
    let pills = lines(&out).iter().filter(|l| l.contains("([\"")).count();
    assert_eq!(pills, 5);
    assert!(!out.contains("(no transition)"));
}

#[test]
fn causal_edges_for_the_spec_example() {
    let out = causal(SPEC_EXAMPLE);
    for edge in [
        r#"s0 -->|"submit"| t0"#,
        r#"s1 -->|"capture_ok"| t1"#,
        r#"s2 -->|"timeout"| t2"#,
        "t1 -.-> e0",
        "t2 -.-> e1",
        "t4 -.-> e2",
        "e0 --> c0",
        r#"c0 -.->|"start"| t3"#,
    ] {
        assert!(has_line(&out, edge), "missing {edge} in\n{out}");
    }
    assert_eq!(edge_lines(&out).len(), 8);
}

fn edge_lines(out: &str) -> Vec<String> {
    lines(out)
        .into_iter()
        .filter(|l| l.contains(" --> ") || l.contains(" -.-> ") || l.contains(" -.->|") || l.contains(" -->|"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn causal_styles_machines_events_and_controllers() {
    let out = causal(SPEC_EXAMPLE);
    // Order is blue, Shipment green (declared colors).
    assert!(out.contains("fill:#0072B2"), "{out}");
    assert!(out.contains("fill:#009E73"), "{out}");
    let class_lines: Vec<&str> = lines(&out).into_iter().filter(|l| l.starts_with("class ")).collect();
    assert!(class_lines.iter().any(|l| l.starts_with("class t0,t1,t2 ")), "{out}");
    assert!(class_lines.iter().any(|l| l.starts_with("class t3,t4 ")), "{out}");
    assert!(class_lines.iter().any(|l| l.starts_with("class e0,e1,e2 ")), "{out}");
    assert!(class_lines.iter().any(|l| l.starts_with("class c0 ")), "{out}");
    // Style lines never end with `;` (Mermaid would read `#hex;` as an entity).
    for line in lines(&out) {
        if line.starts_with("classDef") || line.starts_with("linkStyle") {
            assert!(!line.ends_with(';'), "{line}");
        }
    }
}

#[test]
fn causal_link_styles_color_fires_by_target_machine() {
    let out = causal(SPEC_EXAMPLE);
    let edges = edge_lines(&out);
    let fire_index = edges.iter().position(|e| e.starts_with("c0 -.->")).expect("fire edge");
    let link_styles: Vec<&str> = lines(&out).into_iter().filter(|l| l.starts_with("linkStyle ")).collect();
    assert!(!link_styles.is_empty(), "{out}");
    let mut styled = Vec::new();
    for line in &link_styles {
        let indices = line.trim_start_matches("linkStyle ").split(' ').next().unwrap_or_default();
        for index in indices.split(',') {
            let index: usize = index.parse().expect("numeric link index");
            assert!(index < edges.len(), "linkStyle index {index} out of range in\n{out}");
            styled.push((index, *line));
        }
    }
    let fire_style = styled.iter().find(|(i, _)| *i == fire_index).map(|(_, l)| *l).expect("fire edge styled");
    assert!(fire_style.contains("stroke:#009E73"), "{fire_style}");
}

#[test]
fn causal_invalid_fire_gets_a_placeholder_and_when_labels() {
    let text = r#"
machines:
  Order:
    states: [a, b]
    transitions:
      - { from: a, to: b, on: pay, emits: [Paid] }
  Shipment:
    states: [idle, picking]
    transitions:
      - { from: idle, to: picking, on: start }
controllers:
  Fulfillment:
    on:
      Paid:
        - fire: Shipment.start
          when: "paid in full"
        - fire: Shipment.restart
external:
  User: [Order.pay, Order.refund]
"#;
    let out = causal(text);
    assert!(has_line(&out, r#"c0 -.->|"start [paid in full]"| t1"#), "{out}");
    assert!(has_line(&out, r#"u0["Shipment.restart (no transition)"]"#), "{out}");
    assert!(has_line(&out, r#"c0 -.->|"restart"| u0"#), "{out}");
    assert!(has_line(&out, r#"u1["Order.refund (no transition)"]"#), "{out}");
    assert!(has_line(&out, r#"s0 -->|"refund"| u1"#), "{out}");
    // Undeclared colors: Order takes the first palette hue, Shipment the next.
    assert!(out.contains("fill:#0072B2"), "{out}");
    assert!(out.contains("fill:#E69F00"), "{out}");
}

#[test]
fn causal_escapes_labels() {
    let text = r#"
machines:
  M:
    states: [a, b]
    transitions:
      - { from: a, to: b, on: go, emits: [E] }
controllers:
  C:
    on:
      E:
        - fire: M.go
          when: 'x > "1" # y'
"#;
    let out = causal(text);
    assert!(has_line(&out, r#"c0 -.->|"go [x #62; #quot;1#quot; #35; y]"| t0"#), "{out}");
}

#[test]
fn causal_one_subscribe_edge_per_handler_and_fan_to_every_accepting_transition() {
    let text = r#"
machines:
  M:
    states: [a, b, c]
    transitions:
      - { from: a, to: b, on: go, emits: [E] }
      - { from: b, to: c, on: go, emits: [E] }
controllers:
  C:
    on:
      E: [{ fire: M.go }, { fire: M.go }]
  D:
    on:
      E: { fire: M.go }
"#;
    let out = causal(text);
    assert!(has_line(&out, "e0 --> c0"), "{out}");
    assert!(has_line(&out, "e0 --> c1"), "{out}");
    let subscribes = edge_lines(&out).iter().filter(|l| l.starts_with("e0 --> ")).count();
    assert_eq!(subscribes, 2);
    let fires_c0 = edge_lines(&out).iter().filter(|l| l.starts_with("c0 -.->")).count();
    assert_eq!(fires_c0, 4, "two rules × two accepting transitions:\n{out}");
    assert!(has_line(&out, r#"c1 -.->|"go"| t0"#), "{out}");
    assert!(has_line(&out, r#"c1 -.->|"go"| t1"#), "{out}");
}

#[test]
fn causal_is_deterministic() {
    assert_eq!(causal(SPEC_EXAMPLE), causal(SPEC_EXAMPLE));
    assert_eq!(causal(NESTED), causal(NESTED));
}

#[test]
fn causal_nested_pill_labels_use_paths() {
    let out = causal(NESTED);
    assert!(has_line(&out, r#"t1(["Job: running.fetching → running.computing"])"#), "{out}");
    assert!(has_line(&out, r#"t2(["Job: running.computing → done"])"#), "{out}");
    // Machine without transitions to a controller still gets its pills styled.
    assert!(out.contains("fill:#CC79A7"), "{out}");
}
