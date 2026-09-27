//! SCXML import and export: the representation, lossless round trips, the
//! generic mapping, and errors.

mod common;

use cascade_core::model::StateKind;
use cascade_core::{ElementKey, ElementRef, Model};
use cascade_interop::{ExportFormat, ImportFormat, Imported, InteropError, export, import};

use common::{analyze_ok, assert_same_design, assert_yaml_round_trip, example_definitions, load, read, resolve_ok};

fn scxml(model: &Model) -> String {
    match export(ExportFormat::Scxml, model) {
        Ok(text) => text,
        Err(err) => panic!("{err}"),
    }
}

fn imported(text: &str) -> Imported {
    match import(ImportFormat::Scxml, text) {
        Ok(imported) => imported,
        Err(err) => panic!("import failed: {err}\n---\n{text}"),
    }
}

fn model(text: &str) -> Model {
    resolve_ok(imported(text).definition)
}

fn import_err(text: &str) -> InteropError {
    match import(ImportFormat::Scxml, text) {
        Ok(imported) => panic!("expected an error, got:\n{}", cascade_interop::to_yaml(&imported.definition)),
        Err(err) => err,
    }
}

fn has(model: &Model, key: &str) -> bool {
    match key.parse::<ElementKey>() {
        Ok(key) => model.resolve_key(&key).is_some(),
        Err(err) => panic!("{err}"),
    }
}

fn transition<'m>(model: &'m Model, key: &str) -> &'m cascade_core::model::Transition {
    match key.parse::<ElementKey>().ok().and_then(|k| model.resolve_key(&k)) {
        Some(ElementRef::Transition(t)) => model.transition(t),
        _ => panic!("no transition {key}"),
    }
}

fn emits(model: &Model, key: &str) -> Vec<String> {
    transition(model, key).emits.iter().map(|&e| model.event(e).name.clone()).collect()
}

/// Export, import, and check the model survived.
fn assert_scxml_round_trip(model: &Model) -> Model {
    let text = scxml(model);
    let back = match import(ImportFormat::Scxml, &text) {
        Ok(imported) => imported,
        Err(err) => panic!("re-import failed: {err}\n---\n{text}"),
    };
    assert!(back.warnings.is_empty(), "round trip warnings: {:?}", back.warnings);
    let reloaded = resolve_ok(back.definition);
    assert_same_design(model, &reloaded, false);
    reloaded
}

const KITCHEN_SINK: &str = r#"
system: Kitchen Sink
machines:
  Order:
    color: blue
    domain: sales
    initial: pending.authorizing
    fields: [orderId, customer]
    states:
      - draft
      - pending:
          initial: authorizing
          states:
            - waiting
            - authorizing
            - hist: { kind: history }
            - deep: { kind: deep-history }
      - paid: { kind: final }
      - cancelled
    transitions:
      - { from: draft, to: pending, on: submit, guard: "amount > 0 && x < \"y\"", emits: [OrderSubmitted] }
      - { from: [draft, pending.waiting], to: cancelled, on: cancel }
      - { from: pending, to: paid, on: capture_ok, emits: [OrderPaid, Audit], bounded: true }
      - { from: pending, to: paid, on: capture_ok, guard: "line one\nline two" }
      - { from: pending.authorizing, to: pending.hist, on: interrupt }
      - { from: cancelled, to: cancelled, on: poke }
  Shipment:
    color: sky-blue
    fields: [orderId, carrier]
    states: [idle, picking, shipped]
    transitions:
      - { from: idle, to: picking, on: start }
      - { from: picking, to: shipped, on: handoff, emits: [Shipped] }
events:
  OrderSubmitted: {}
  OrderPaid: { payload: [orderId, amount] }
  Audit: {}
  Shipped: { payload: [orderId] }
  Unused: {}
controllers:
  Fulfillment:
    on:
      OrderPaid:
        - fire: Shipment.start
          target: Shipment where orderId == event.orderId
          when: not a <gift> card
        - fire: Shipment.start
          target: all Shipment where orderId == "A, B"
          bounded: true
        - fire: Shipment.restart
          target: new Shipment with orderId = event.orderId, carrier = 'the "fast" one'
      Audit: []
  Shipment:
    on: {}
external:
  Customer: [Order.submit, Order.cancel]
  Gateway: [Order.capture_ok]
  Nobody: []
"#;

// --- Export representation -------------------------------------------------------

#[test]
fn golden_export_of_the_spec_example() {
    let model = load(&read("examples/order-fulfillment/cascade.yaml"));
    assert_eq!(scxml(&model), read("examples/scxml/order-fulfillment.scxml"));
}

#[test]
fn export_uses_a_parallel_region_per_machine_and_controller() {
    let model = load(KITCHEN_SINK);
    let text = scxml(&model);
    for needle in [
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:cascade="urn:x-cascade:scxml" version="1.0" name="Kitchen Sink">"#,
        r#"<parallel id="system">"#,
        r#"<state id="Order" initial="Order.pending.authorizing" cascade:color="blue" cascade:domain="sales" cascade:fields="orderId customer">"#,
        r#"<state id="Order.pending" initial="Order.pending.authorizing">"#,
        r#"<history id="Order.pending.deep" type="deep"/>"#,
        r#"<final id="Order.paid"/>"#,
        r#"<transition event="Order.capture_ok" target="Order.paid" cascade:bounded="true">"#,
        r#"<send event="OrderPaid"/>"#,
        r#"cond="amount &gt; 0 &amp;&amp; x &lt; &quot;y&quot;""#,
        r#"cond="line one&#10;line two""#,
        r#"<cascade:event name="OrderPaid" payload="orderId amount"/>"#,
        r#"<cascade:source name="Nobody" fires=""/>"#,
        r#"<cascade:source name="Customer" fires="Order.submit Order.cancel"/>"#,
        r#"<state id="Fulfillment" cascade:controller="Fulfillment">"#,
        r#"<if cond="not a &lt;gift&gt; card">"#,
        r#"<send event="Shipment.start" cascade:target="all Shipment where orderId == &quot;A, B&quot;" cascade:bounded="true"/>"#,
        r#"<transition event="Audit"/>"#,
        // A controller named like a machine gets a distinct region id.
        r#"<state id="Shipment_2" cascade:controller="Shipment"/>"#,
    ] {
        assert!(text.contains(needle), "missing {needle}\n---\n{text}");
    }
    // Multi-source transitions are written once per source.
    assert_eq!(text.matches(r#"event="Order.cancel""#).count(), 2);
    // The document is well-formed XML.
    assert!(roxmltree_parses(&text));
}

fn roxmltree_parses(text: &str) -> bool {
    // Parsing through the importer checks well-formedness and structure.
    import(ImportFormat::Scxml, text).is_ok()
}

#[test]
fn a_single_machine_without_controllers_is_written_flat() {
    let model = load(
        "system: Solo\nmachines:\n  Job:\n    color: green\n    states: [queued, running, done]\n    transitions:\n      - { from: queued, to: running, on: go }\n",
    );
    let text = scxml(&model);
    assert!(!text.contains("<parallel id"), "{text}");
    assert!(text.contains(r#"name="Job" initial="Job.queued" cascade:system="Solo" cascade:color="green">"#), "{text}");
    assert_scxml_round_trip(&model);
}

// --- Round trips --------------------------------------------------------------------

#[test]
fn every_example_round_trips_through_scxml() {
    for (path, text) in example_definitions() {
        let model = load(&text);
        let back = assert_scxml_round_trip(&model);
        assert_eq!(back.machine_count(), model.machine_count(), "{}", path.display());
    }
}

#[test]
fn kitchen_sink_round_trips_through_scxml() {
    let back = assert_scxml_round_trip(&load(KITCHEN_SINK));
    // The imported definition is itself valid YAML material.
    assert_yaml_round_trip(&back);
}

#[test]
fn xstate_imports_round_trip_through_scxml() {
    for name in ["traffic-light.json", "fetch.json", "checkout.json"] {
        let imported = import(ImportFormat::XState, &read(&format!("examples/xstate/{name}"))).expect("xstate import");
        let model = resolve_ok(imported.definition);
        assert_scxml_round_trip(&model);
    }
}

#[test]
fn scxml_fixtures_import_resolve_and_round_trip() {
    for name in ["microwave.scxml", "vending.scxml", "order-fulfillment.scxml"] {
        let model = model(&read(&format!("examples/scxml/{name}")));
        analyze_ok(&model);
        assert_yaml_round_trip(&model);
        assert_scxml_round_trip(&model);
    }
}

#[test]
fn the_golden_file_imports_as_the_spec_example() {
    let expected = load(&read("examples/order-fulfillment/cascade.yaml"));
    let imported = imported(&read("examples/scxml/order-fulfillment.scxml"));
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
    assert_same_design(&expected, &resolve_ok(imported.definition), false);
}

// --- Generic SCXML ---------------------------------------------------------------------

#[test]
fn microwave_fixture_maps_generic_scxml() {
    let imported = imported(&read("examples/scxml/microwave.scxml"));
    let model = resolve_ok(imported.definition.clone());
    assert!(has(&model, "machine:microwave"));
    let m = model.machine_by_name("microwave").expect("machine");
    let on = model.state_by_path(m, "on").expect("on");
    let StateKind::Compound { initial, .. } = model.state(on).kind else { panic!("on is compound") };
    assert_eq!(model.state(initial).path, "on.idle", "<initial> element");
    let history = model.state_by_path(m, "on.paused_at").expect("history");
    assert_eq!(model.state(history).kind, StateKind::History { deep: false });
    // Dotted event names become valid trigger names.
    assert!(has(&model, "transition:microwave:off->on@turn_on"));
    assert_eq!(
        transition(&model, "transition:microwave:on.idle->on.cooking@door_close").guard.as_deref(),
        Some("timer < cook_time")
    );
    // onentry/onexit sends and raises are attributed in execution order.
    assert_eq!(emits(&model, "transition:microwave:off->on@turn_on"), ["display_on"]);
    assert_eq!(emits(&model, "transition:microwave:on->off@turn_off"), ["display_off"]);
    assert_eq!(emits(&model, "transition:microwave:on.cooking->on.finished@done"), ["light_off", "beep"]);
    // A targetless internal transition is a self-transition that runs only
    // its own content (the raise inside <if>).
    assert_eq!(emits(&model, "transition:microwave:on.cooking->on.cooking@time"), ["done"]);
    // One transition per event descriptor.
    assert!(has(&model, "transition:microwave:light->light@light_on"));
    assert!(has(&model, "transition:microwave:light->light@light_off"));
    // Raised events route back into the machine.
    assert!(has(&model, "rule:EventRouter/done#0"));
    let texts: Vec<String> = imported.warnings.iter().map(ToString::to_string).collect();
    assert!(texts.iter().any(|t| t.contains("default transition of a history state")), "{texts:?}");
}

#[test]
fn vending_fixture_is_a_system_of_regions() {
    let model = model(&read("examples/scxml/vending.scxml"));
    assert_eq!(model.definition().system.as_ref().map(|s| s.value.as_str()), Some("Vending"));
    assert!(has(&model, "machine:Coins"));
    assert!(has(&model, "state:Coins:credited"), "the region prefix is removed from ids");
    // The machine's own prefix is removed from event names.
    assert!(has(&model, "transition:Dispenser:vending->ready@item_dropped"));
    // `#_Dispenser` targets the Dispenser region.
    assert!(has(&model, "rule:EventRouter/dispense#0"));
    let env = model.external(model.external_by_name("Environment").expect("environment"));
    let names: Vec<String> = env.triggers.iter().map(|&t| model.trigger(t).name.clone()).collect();
    assert!(!names.contains(&"dispense".to_owned()), "{names:?}");
    assert!(names.contains(&"serviced".to_owned()), "{names:?}");
}

#[test]
fn documents_without_the_scxml_namespace_are_accepted() {
    let model =
        model(r#"<scxml name="bare"><state id="a"><transition event="go" target="b"/></state><state id="b"/></scxml>"#);
    assert!(has(&model, "transition:bare:a->b@go"));
}

#[test]
fn a_deep_initial_state_is_lifted_with_a_warning() {
    let text = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" name="m" initial="c">
      <state id="p" initial="c"><state id="q"><state id="c"/></state><state id="r"/></state>
    </scxml>"#;
    let imported = imported(text);
    let model = resolve_ok(imported.definition.clone());
    let m = model.machine_by_name("m").expect("m");
    // The machine may start anywhere …
    assert_eq!(model.state(model.machine(m).initial).path, "p.q.c");
    // … but a compound state's initial must be a child.
    let p = model.state_by_path(m, "p").expect("p");
    let StateKind::Compound { initial, .. } = model.state(p).kind else { panic!("compound") };
    assert_eq!(model.state(initial).path, "p.q");
    assert!(imported.warnings.iter().any(|w| w.to_string().contains("not a direct child")));
}

#[test]
fn states_without_ids_get_generated_names() {
    let imported =
        imported(r#"<scxml name="m"><state><transition event="go" target="b"/></state><state id="b"/></scxml>"#);
    let model = resolve_ok(imported.definition.clone());
    assert!(has(&model, "transition:m:state1->b@go"));
    assert!(imported.warnings.iter().any(|w| w.to_string().contains("without an `id`")));
}

// --- Errors ------------------------------------------------------------------------------

#[test]
fn unsupported_constructs() {
    let cases = [
        (r#"<scxml name="m"><state id="a"><parallel id="p"><state id="x"/></parallel></state></scxml>"#, "parallel"),
        (r#"<scxml name="m"><parallel id="p"><parallel id="q"/></parallel></scxml>"#, "parallel"),
        (r#"<scxml name="m"><state id="a"><transition target="b"/></state><state id="b"/></scxml>"#, "eventless"),
        (
            r#"<scxml name="m"><state id="a"><transition event="e" target="a b"/></state><state id="b"/></scxml>"#,
            "multiple",
        ),
        (
            r#"<scxml><parallel id="s"><state id="A"><state id="a"><transition event="e" target="b"/></state></state><state id="B"><state id="b"/></state></parallel></scxml>"#,
            "another machine",
        ),
    ];
    for (text, what_contains) in cases {
        match import_err(text) {
            InteropError::Unsupported { what, .. } => assert!(what.contains(what_contains), "{what}"),
            other => panic!("{text}: {other:?}"),
        }
    }
}

#[test]
fn syntax_and_structure_errors() {
    match import_err("<scxml name=\"m\">\n  <state id=\"a\">\n</scxml>") {
        InteropError::Syntax { line, .. } => assert!(line >= 2, "line {line}"),
        other => panic!("{other:?}"),
    }
    for text in [
        r#"<machine/>"#,
        r#"<scxml name="m"><state id="a"><transition event="e" target="nowhere"/></state></scxml>"#,
        r#"<scxml name="m"><state id="a"/><state id="a"/></scxml>"#,
        r#"<scxml name="m" initial="zzz"><state id="a"/></scxml>"#,
        r#"<scxml name="m"><final id="f"><state id="x"/></final></scxml>"#,
    ] {
        assert!(matches!(import_err(text), InteropError::Invalid { .. }), "{text}");
    }
    match import_err(r#"<scxml name="m"/>"#) {
        InteropError::Unsupported { what, .. } => assert!(what.contains("without states")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn annotated_documents_with_bad_references_are_invalid() {
    let text = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:cascade="urn:x-cascade:scxml" name="m">
      <cascade:source name="S" fires="Ghost.go"/>
      <state id="a"/>
    </scxml>"#;
    assert!(matches!(import_err(text), InteropError::Invalid { .. }));
}
