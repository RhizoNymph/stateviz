//! SCXML export and import of the semantics-phase features: guards as
//! `cond`, timers as delayed `<send>`s cancelled on exit, subscriptions as
//! transitions on the event, `sets` as `<assign>`, emit `with` as
//! `<param>`s, typed fields and payloads. Cascade → SCXML → Cascade keeps
//! the model, and foreign documents using the same patterns import too.

mod common;

use cascade_core::model::TriggerKind;
use cascade_core::{ElementRef, Model};
use cascade_interop::{ExportFormat, ImportFormat, Imported, export, import};

use common::{assert_same_design, load, resolve_ok};

const SAGA: &str = r#"
system: Saga
machines:
  Order:
    fields: { orderId: string, amount: number, tier: "enum [basic, gold]" }
    states: [draft, paid, done]
    transitions:
      - { from: draft, to: paid, on: pay, guard: "event.amount > 0 and self.tier == gold", sets: { amount: event.amount, tier: basic }, emits: [OrderPaid, { event: Receipt, with: { total: self.amount, note: "'two words'" } }] }
      - { from: draft, to: done, on: pay, guard: "?card declined" }
      - { from: draft, to: done, on: pay, guard: else }
      - { from: paid, to: done, after: 90s, emits: [Receipt] }
  Checkout:
    fields: [orderId]
    states: [waiting, paid, expired]
    transitions:
      - { from: waiting, to: paid, on: { event: OrderPaid, where: "orderId == event.orderId" }, guard: "event.amount >= 10" }
      - { from: waiting, to: waiting, on: { event: Receipt } }
      - { from: [waiting, paid], to: expired, after: 30m }
      - { from: paid, to: expired, after: 1500ms, guard: "?slow" }
controllers:
  Billing:
    on:
      OrderPaid:
        - fire: Order.pay
          target: Order where orderId == event.orderId
          when: "event.amount >= 100"
events:
  OrderPaid: { payload: { orderId: string, amount: number } }
  Receipt: { payload: [total, note] }
external:
  Customer: [Order.pay]
"#;

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

/// What the common signature leaves out: trigger kinds, subscription
/// filters, `sets` and emit `with`.
fn semantics(model: &Model) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = model
        .all_elements()
        .into_iter()
        .filter_map(|element| {
            let text = match element {
                ElementRef::Trigger(id) => {
                    let kind = match model.trigger(id).kind {
                        TriggerKind::Command => "command".to_owned(),
                        TriggerKind::Event(e) => format!("event {}", model.event(e).name),
                        TriggerKind::Timer(d) => format!("timer {d}"),
                    };
                    kind
                }
                ElementRef::Transition(id) => {
                    let t = model.transition(id);
                    let sets: Vec<String> = t.sets.iter().map(|a| format!("{}={}", a.field, a.value)).collect();
                    let with: Vec<String> = t
                        .emits
                        .iter()
                        .map(|e| {
                            let w: Vec<String> = e.with.iter().map(|a| format!("{}={}", a.field, a.value)).collect();
                            format!("{}{w:?}", model.event(e.event).name)
                        })
                        .collect();
                    format!("filter={:?} sets={sets:?} emits={with:?}", t.filter.as_ref().map(ToString::to_string))
                }
                _ => return None,
            };
            Some((model.key_of(element).to_string(), text))
        })
        .collect();
    out.sort();
    out
}

fn round_trip(model: &Model) -> (String, Model) {
    let text = scxml(model);
    let back = imported(&text);
    assert!(back.warnings.is_empty(), "round trip warnings: {:?}\n{text}", back.warnings);
    let reloaded = resolve_ok(back.definition);
    assert_same_design(model, &reloaded, false);
    assert_eq!(semantics(model), semantics(&reloaded), "\n{text}");
    (text, reloaded)
}

#[test]
fn the_saga_round_trips() {
    let model = load(SAGA);
    let (text, _) = round_trip(&model);

    // Guards are their canonical text; a choice gets a comment.
    assert!(text.contains(r#"cond="event.amount &gt; 0 and self.tier == gold""#), "{text}");
    assert!(text.contains(r#"cond="?card declined""#), "{text}");
    assert!(text.contains("<!-- ?card declined: a choice"), "{text}");

    // Timers: armed on entry, cancelled on exit, taken on their event.
    assert!(
        text.contains(r#"<send event="Checkout.after-30m" delay="1800s" id="Checkout.waiting.after-30m"/>"#),
        "{text}"
    );
    assert!(text.contains(r#"<cancel sendid="Checkout.waiting.after-30m"/>"#), "{text}");
    assert!(
        text.contains(r#"<send event="Checkout.after-1500ms" delay="1500ms" id="Checkout.paid.after-1500ms"/>"#),
        "{text}"
    );
    assert!(text.contains(r#"<transition event="Checkout.after-30m" target="Checkout.expired"/>"#), "{text}");

    // Subscriptions: a transition on the event, the filter in `cond`.
    assert!(
        text.contains(
            r#"<transition event="OrderPaid" target="Checkout.paid" cond="self.orderId == event.orderId and event.amount &gt;= 10" cascade:where="orderId == event.orderId" cascade:guard="event.amount &gt;= 10"/>"#
        ),
        "{text}"
    );
    assert!(text.contains(r#"<transition event="Receipt" target="Checkout.waiting"/>"#), "{text}");

    // sets → <assign>, with → <param>.
    assert!(text.contains(r#"<assign location="amount" expr="event.amount"/>"#), "{text}");
    assert!(text.contains(r#"<param name="note" expr="&quot;two words&quot;"/>"#), "{text}");

    // Typed fields and payloads.
    assert!(text.contains(r#"<cascade:field name="tier" type="enum [basic, gold]"/>"#), "{text}");
    assert!(text.contains(r#"cascade:fields="orderId""#), "{text}");
}

#[test]
fn a_single_machine_with_timers_round_trips_flat() {
    let model = load(
        r#"
machines:
  Light:
    fields: { cycles: number }
    states: [red, green, yellow]
    transitions:
      - { from: red, to: green, after: 30s, sets: { cycles: "1" } }
      - { from: green, to: yellow, after: 25s }
      - { from: yellow, to: red, after: 5s }
      - { from: [red, green, yellow], to: red, on: reset }
external:
  Operator: [Light.reset]
"#,
    );
    let (text, _) = round_trip(&model);
    assert!(text.contains(r#"<scxml xmlns="http://www.w3.org/2005/07/scxml""#), "{text}");
    assert!(text.contains(r#"<send event="Light.after-30s" delay="30s" id="Light.red.after-30s"/>"#), "{text}");
}

#[test]
fn every_example_still_round_trips_with_the_new_keys() {
    for (path, text) in common::example_definitions() {
        let model = load(&text);
        let back = imported(&scxml(&model));
        assert!(back.warnings.is_empty(), "{}: {:?}", path.display(), back.warnings);
        let reloaded = resolve_ok(back.definition);
        assert_eq!(semantics(&model), semantics(&reloaded), "{}", path.display());
    }
}

// --- Foreign documents -------------------------------------------------------------------

#[test]
fn a_delayed_send_cancelled_on_exit_is_a_timer() {
    let text = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" name="Door" initial="open">
  <state id="open">
    <onentry><send event="autoclose" delay="2.5s" id="t1"/></onentry>
    <onexit><cancel sendid="t1"/></onexit>
    <transition event="autoclose" target="closed"/>
    <transition event="close" target="closed" cond="_event.data.force"/>
  </state>
  <state id="closed">
    <onentry><send event="ping" delay="1s"/></onentry>
    <transition event="open" target="open"/>
  </state>
</scxml>"#;
    let imported = imported(text);
    let model = resolve_ok(imported.definition.clone());
    let kinds = semantics(&model);
    assert!(kinds.contains(&("trigger:Door.after-2500ms".to_owned(), "timer 2500ms".to_owned())), "{kinds:?}");
    // A delayed send nothing in its state takes is still an emitted event.
    let yaml = cascade_interop::to_yaml(&imported.definition);
    assert!(yaml.contains("after: 2500ms"), "{yaml}");
    assert!(yaml.contains(r#"guard: "?_event.data.force""#), "{yaml}");
    assert!(yaml.contains("emits: [ping]"), "{yaml}");
    // The timer is not an environment trigger.
    assert!(!yaml.contains("Door.after"), "{yaml}");
}

#[test]
fn foreign_assigns_and_params_are_kept_only_in_cascade_syntax() {
    let text = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" name="M" initial="a">
  <state id="a">
    <transition event="go" target="b">
      <assign location="count" expr="event.count"/>
      <assign location="other" expr="_event.data.x + 1"/>
      <send event="Went"><param name="n" expr="self.count"/><param name="m" expr="Math.max(1, 2)"/></send>
    </transition>
  </state>
  <state id="b"/>
</scxml>"#;
    let imported = imported(text);
    let yaml = cascade_interop::to_yaml(&imported.definition);
    assert!(yaml.contains("sets: { count: event.count }"), "{yaml}");
    assert!(yaml.contains("{ event: Went, with: { n: self.count } }"), "{yaml}");
    let ignored = imported.warnings.iter().filter(|w| w.to_string().contains("ignored")).count();
    assert_eq!(ignored, 2, "{:?}", imported.warnings);
    resolve_ok(imported.definition);
}
