//! XState v5 import: every mapping, the synthesized wiring, errors, and the
//! fixtures end to end.

mod common;

use cascade_core::model::StateKind;
use cascade_core::{ElementKey, Model};
use cascade_interop::{
    ImportFormat, ImportOptions, Imported, InteropError, NameKind, WarningKind, import, import_with,
};

use common::{analyze_ok, assert_yaml_round_trip, read, resolve_ok};

fn imported(json: &str) -> Imported {
    match import(ImportFormat::XState, json) {
        Ok(imported) => imported,
        Err(err) => panic!("import failed: {err}\n---\n{json}"),
    }
}

fn model(json: &str) -> Model {
    resolve_ok(imported(json).definition)
}

fn import_err(json: &str) -> InteropError {
    match import(ImportFormat::XState, json) {
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

fn transition_keys(model: &Model) -> Vec<String> {
    model.transition_ids().map(|t| model.key_of(cascade_core::ElementRef::Transition(t)).to_string()).collect()
}

fn source_triggers(model: &Model, source: &str) -> Vec<String> {
    let Some(id) = model.external_by_name(source) else {
        return Vec::new();
    };
    model
        .external(id)
        .triggers
        .iter()
        .map(|&t| format!("{}.{}", model.machine(model.trigger(t).machine).name, model.trigger(t).name))
        .collect()
}

fn emits_of(model: &Model, key: &str) -> Vec<String> {
    let Some(cascade_core::ElementRef::Transition(t)) = key.parse().ok().and_then(|k| model.resolve_key(&k)) else {
        panic!("no transition {key}");
    };
    model.transition(t).emits.iter().map(|&e| model.event(e).name.clone()).collect()
}

fn guard_of(model: &Model, key: &str) -> Option<String> {
    let Some(cascade_core::ElementRef::Transition(t)) = key.parse().ok().and_then(|k| model.resolve_key(&k)) else {
        panic!("no transition {key}");
    };
    model.transition(t).guard.clone()
}

// --- Fixtures ------------------------------------------------------------------

fn fixture(name: &str) -> (Imported, Model) {
    let imported = imported(&read(&format!("examples/xstate/{name}")));
    let model = resolve_ok(imported.definition.clone());
    analyze_ok(&model);
    assert_yaml_round_trip(&model);
    (imported, model)
}

#[test]
fn traffic_light_fixture() {
    let (imported, model) = fixture("traffic-light.json");
    assert!(has(&model, "machine:trafficLight"));
    assert!(has(&model, "state:trafficLight:red.walk"));
    let red = model.state_by_path(model.machine_by_name("trafficLight").expect("machine"), "red").expect("red");
    assert!(matches!(model.state(red).kind, StateKind::Compound { .. }));
    assert!(has(&model, "transition:trafficLight:green->yellow@after_30000ms"));
    assert!(has(&model, "transition:trafficLight:red->green@red_done"));
    // The root-level FAULT handler applies to every top-level state.
    for from in ["green", "yellow", "red", "flashing"] {
        assert!(has(&model, &format!("transition:trafficLight:{from}->flashing@FAULT")), "{from}");
    }
    // Entry actions of the entered states are attributed to the transition.
    assert_eq!(emits_of(&model, "transition:trafficLight:yellow->red@after_5000ms"), ["LightChanged", "WalkSignalOn"]);
    assert_eq!(
        guard_of(&model, "transition:trafficLight:green->yellow@PEDESTRIAN_WAITING").as_deref(),
        Some(r#"minimumGreenElapsed({"seconds":10})"#)
    );
    assert!(source_triggers(&model, "Clock").contains(&"trafficLight.after_5000ms".to_owned()));
    assert!(source_triggers(&model, "Environment").contains(&"trafficLight.FAULT".to_owned()));
    // The start-up emit and the completion approximation are reported.
    let messages: Vec<String> = imported.warnings.iter().map(ToString::to_string).collect();
    assert!(messages.iter().any(|m| m.contains("emitted when the machine starts")), "{messages:?}");
    assert!(messages.iter().any(|m| m.contains("completion")), "{messages:?}");
}

#[test]
fn fetch_fixture() {
    let (_, model) = fixture("fetch.json");
    assert!(has(&model, "state:fetch:success.hist"));
    let fetch = model.machine_by_name("fetch").expect("fetch");
    let hist = model.state_by_path(fetch, "success.hist").expect("hist");
    assert_eq!(model.state(hist).kind, StateKind::History { deep: true });
    assert!(has(&model, "transition:fetch:loading->success@fetchData_done"));
    assert!(has(&model, "transition:fetch:loading->retrying@fetchData_error"));
    assert!(has(&model, "transition:fetch:loading->failure@fetchData_error"));
    assert!(has(&model, "transition:fetch:paused->success.hist@RESUME"));
    // `#fetch.loading` resolves through the machine id.
    assert!(has(&model, "transition:fetch:success->loading@REFRESH"));
    assert_eq!(source_triggers(&model, "fetchData"), ["fetch.fetchData_done", "fetch.fetchData_error"]);
    // `raise` routes REFRESH back to the machine, so the environment does
    // not expose it.
    assert!(!source_triggers(&model, "Environment").contains(&"fetch.REFRESH".to_owned()));
    assert!(has(&model, "rule:EventRouter/REFRESH#0"));
    assert_eq!(emits_of(&model, "transition:fetch:loading->failure@after_10000ms"), ["FetchTimedOut"]);
}

#[test]
fn checkout_fixture_routes_between_machines() {
    let (_, model) = fixture("checkout.json");
    for machine in ["checkout", "payment", "notifier"] {
        assert!(has(&model, &format!("machine:{machine}")), "{machine}");
    }
    assert!(has(&model, "rule:EventRouter/CHARGE#0"));
    let router = model.controller_by_name("EventRouter").expect("router");
    let fired: Vec<String> = model
        .controller(router)
        .handlers
        .iter()
        .flat_map(|&h| model.handler(h).rules.clone())
        .map(|r| {
            let t = model.trigger(model.rule(r).trigger);
            format!("{}.{}", model.machine(t.machine).name, t.name)
        })
        .collect();
    assert_eq!(fired, ["payment.CHARGE", "notifier.SEND_RECEIPT", "checkout.PAYMENT_OK", "checkout.PAYMENT_FAILED"]);
    // The final state's entry action is attributed to the transition into it.
    assert_eq!(emits_of(&model, "transition:checkout:paying->confirmed@PAYMENT_OK"), ["SEND_RECEIPT"]);
    // Targetless transitions become self-transitions.
    assert!(has(&model, "transition:checkout:cart->cart@ADD_ITEM"));
    assert_eq!(source_triggers(&model, "Environment"), ["checkout.ADD_ITEM", "checkout.CHECKOUT"]);
    assert_eq!(source_triggers(&model, "chargeCard"), ["payment.chargeCard_done", "payment.chargeCard_error"]);
}

// --- Input shapes ------------------------------------------------------------------

const TOGGLE: &str = r#"{ "id": "toggle", "initial": "off", "states": { "off": { "on": { "TOGGLE": "on" } }, "on": { "on": { "TOGGLE": "off" } } } }"#;

#[test]
fn single_config_is_named_by_its_id() {
    let model = model(TOGGLE);
    assert!(has(&model, "machine:toggle"));
    assert!(has(&model, "transition:toggle:off->on@TOGGLE"));
    assert_eq!(source_triggers(&model, "Environment"), ["toggle.TOGGLE"]);
}

#[test]
fn arrays_and_machine_maps() {
    let array = format!("[{TOGGLE}, {{ \"states\": {{ \"a\": {{}} }} }}, {{ \"states\": {{ \"b\": {{}} }} }}]");
    let model = model(&array);
    assert!(has(&model, "machine:toggle"));
    assert!(has(&model, "machine:Machine"));
    assert!(has(&model, "machine:Machine_2"));

    let map = r#"{ "machines": { "Door": { "states": { "open": {}, "closed": {} } }, "Lamp": { "id": "lamp", "states": { "lit": {} } } } }"#;
    let model = self::model(map);
    assert!(has(&model, "machine:Door"));
    assert!(has(&model, "machine:lamp"), "the id wins over the key");
}

#[test]
fn state_order_initial_and_kinds() {
    let json = r#"{
      "id": "m",
      "initial": "zeta",
      "states": {
        "zeta": { "on": { "GO": "alpha" } },
        "alpha": {
          "initial": "two",
          "states": { "one": {}, "two": {}, "h": { "type": "history" }, "dh": { "type": "history", "history": "deep" } }
        },
        "end": { "type": "final" }
      }
    }"#;
    let model = model(json);
    let m = model.machine_by_name("m").expect("m");
    let paths: Vec<&str> = model.machine(m).states.iter().map(|&s| model.state(s).path.as_str()).collect();
    assert_eq!(paths, ["zeta", "alpha", "alpha.one", "alpha.two", "alpha.h", "alpha.dh", "end"]);
    assert_eq!(model.state(model.machine(m).initial).path, "zeta");
    let alpha = model.state_by_path(m, "alpha").expect("alpha");
    let StateKind::Compound { initial, .. } = model.state(alpha).kind else { panic!("alpha is compound") };
    assert_eq!(model.state(initial).path, "alpha.two");
    let kind = |p: &str| model.state(model.state_by_path(m, p).expect(p)).kind.clone();
    assert_eq!(kind("alpha.h"), StateKind::History { deep: false });
    assert_eq!(kind("alpha.dh"), StateKind::History { deep: true });
    assert_eq!(kind("end"), StateKind::Final);
}

#[test]
fn a_leading_history_child_does_not_become_the_default_initial() {
    let json = r#"{ "id": "m", "states": { "p": { "states": { "h": { "type": "history" }, "a": {}, "b": {} } } } }"#;
    let model = model(json);
    let m = model.machine_by_name("m").expect("m");
    let p = model.state_by_path(m, "p").expect("p");
    let StateKind::Compound { initial, .. } = model.state(p).kind else { panic!("compound") };
    assert_eq!(model.state(initial).path, "p.a");
}

// --- Targets ------------------------------------------------------------------------

#[test]
fn target_forms_resolve_to_paths() {
    let json = r##"{
      "id": "m",
      "initial": "a",
      "states": {
        "a": {
          "initial": "a1",
          "states": {
            "a1": { "on": { "SIB": "a2", "DEEP": "#m.b.b1", "BYID": "#special" } },
            "a2": {}
          },
          "on": { "CHILD": ".a2", "OUT": "b.b2" }
        },
        "b": { "states": { "b1": { "id": "special" }, "b2": {} } }
      },
      "on": { "HOME": "a", "HOME2": ".b" }
    }"##;
    let model = model(json);
    for key in [
        "transition:m:a.a1->a.a2@SIB",
        "transition:m:a.a1->b.b1@DEEP",
        "transition:m:a.a1->b.b1@BYID",
        "transition:m:a->a.a2@CHILD",
        "transition:m:a->b.b2@OUT",
        "transition:m:a->a@HOME",
        "transition:m:b->a@HOME",
        "transition:m:b->b@HOME2",
    ] {
        assert!(has(&model, key), "{key}: {:?}", transition_keys(&model));
    }
}

#[test]
fn unknown_targets_are_invalid_with_a_location() {
    let err = import_err(r#"{ "id": "m", "states": { "a": { "on": { "GO": "nowhere" } } } }"#);
    match err {
        InteropError::Invalid { location, message, .. } => {
            assert_eq!(location, "m.states.a.on.GO");
            assert!(message.contains("nowhere"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        import_err(r##"{ "id": "m", "states": { "a": { "on": { "GO": "#missing" } } } }"##),
        InteropError::Invalid { .. }
    ));
}

// --- Transitions ----------------------------------------------------------------------

#[test]
fn guarded_alternatives_and_guard_forms() {
    let json = r#"{
      "id": "m",
      "states": {
        "a": {
          "on": {
            "GO": [
              { "target": "b", "guard": "isAdmin" },
              { "target": "c", "guard": { "type": "hasRole", "params": { "role": "ops" } } },
              { "target": "d", "cond": "legacyGuard" },
              { "target": "e" }
            ]
          }
        },
        "b": {}, "c": {}, "d": {}, "e": {}
      }
    }"#;
    let model = model(json);
    assert_eq!(guard_of(&model, "transition:m:a->b@GO").as_deref(), Some("isAdmin"));
    assert_eq!(guard_of(&model, "transition:m:a->c@GO").as_deref(), Some(r#"hasRole({"role":"ops"})"#));
    assert_eq!(guard_of(&model, "transition:m:a->d@GO").as_deref(), Some("legacyGuard"));
    assert_eq!(guard_of(&model, "transition:m:a->e@GO"), None);
}

#[test]
fn delays_become_clock_triggers() {
    let json = r#"{ "id": "m", "states": { "a": { "after": { "1000": "b", "LONG_DELAY": "c" } }, "b": {}, "c": {} } }"#;
    let model = model(json);
    assert!(has(&model, "transition:m:a->b@after_1000ms"));
    assert!(has(&model, "transition:m:a->c@after_LONG_DELAY"));
    assert_eq!(source_triggers(&model, "Clock"), ["m.after_1000ms", "m.after_LONG_DELAY"]);
    assert!(source_triggers(&model, "Environment").is_empty());
    assert!(model.external_by_name("Environment").is_none(), "no empty sources");
}

#[test]
fn invocations_become_actor_sources() {
    let json = r#"{
      "id": "m",
      "states": {
        "loading": {
          "invoke": [
            { "src": "loadUser", "onDone": "ready", "onError": "failed" },
            { "id": "poller", "src": { "type": "pollLogic" }, "onDone": "ready" }
          ]
        },
        "ready": {}, "failed": {}
      }
    }"#;
    let model = model(json);
    assert!(has(&model, "transition:m:loading->ready@loadUser_done"));
    assert!(has(&model, "transition:m:loading->failed@loadUser_error"));
    assert!(has(&model, "transition:m:loading->ready@poller_done"));
    assert_eq!(source_triggers(&model, "loadUser"), ["m.loadUser_done", "m.loadUser_error"]);
    assert_eq!(source_triggers(&model, "poller"), ["m.poller_done"]);
}

#[test]
fn transitions_out_of_final_states_are_dropped_with_a_warning() {
    let json =
        r#"{ "id": "m", "states": { "a": { "on": { "GO": "z" } }, "z": { "type": "final", "on": { "BACK": "a" } } } }"#;
    let imported = imported(json);
    let model = resolve_ok(imported.definition.clone());
    assert!(!has(&model, "trigger:m.BACK"));
    assert!(imported.warnings.iter().any(|w| w.to_string().contains("final state")));
}

// --- Emits and routing ---------------------------------------------------------------------

#[test]
fn entry_and_exit_actions_are_attributed_in_execution_order() {
    let json = r#"{
      "id": "m",
      "initial": "a",
      "states": {
        "a": { "exit": "emit:LeftA", "on": { "GO": { "target": "b", "actions": "emit:Going" } } },
        "b": { "entry": ["emit:EnteredB"], "initial": "b1", "states": { "b1": { "entry": "emit:EnteredB1" } } }
      }
    }"#;
    let model = model(json);
    assert_eq!(emits_of(&model, "transition:m:a->b@GO"), ["LeftA", "Going", "EnteredB", "EnteredB1"]);
}

#[test]
fn raise_routes_to_the_same_machine_and_send_to_another() {
    let json = r#"[
      { "id": "a", "states": { "idle": { "on": {
          "START": { "target": "busy", "actions": [
            { "type": "xstate.raise", "event": { "type": "TICK" } },
            { "type": "xstate.sendTo", "to": "b", "event": { "type": "PING" } },
            { "type": "xstate.sendTo", "to": "ghost", "event": { "type": "LOST" } }
          ] } } },
        "busy": { "on": { "TICK": "idle" } } } },
      { "id": "b", "states": { "wait": { "on": { "PING": "wait", "TICK": "wait" } } } }
    ]"#;
    let imported = imported(json);
    let model = resolve_ok(imported.definition.clone());
    let rules = |event: &str| -> Vec<String> {
        let router = model.controller_by_name("EventRouter").expect("router");
        let e = model.event_by_name(event).expect("event");
        let Some(h) = model.handler_for(router, e) else { return Vec::new() };
        model
            .handler(h)
            .rules
            .iter()
            .map(|&r| {
                let t = model.trigger(model.rule(r).trigger);
                format!("{}.{}", model.machine(t.machine).name, t.name)
            })
            .collect()
    };
    // `raise` reaches only the raising machine, even though `b` also
    // handles TICK.
    assert_eq!(rules("TICK"), ["a.TICK"]);
    assert_eq!(rules("PING"), ["b.PING"]);
    assert!(model.event_by_name("LOST").is_some(), "the event is still emitted");
    assert!(rules("LOST").is_empty());
    assert!(imported.warnings.iter().any(|w| w.to_string().contains("ghost")));
    // b.TICK is not covered by an emit that reaches b.
    assert!(source_triggers(&model, "Environment").contains(&"b.TICK".to_owned()));
    assert!(!source_triggers(&model, "Environment").contains(&"a.TICK".to_owned()));
}

#[test]
fn custom_options_rename_the_conventions() {
    let json = r#"{ "id": "m", "states": {
        "a": { "on": { "GO": { "target": "b", "actions": ["notify:Done"] } }, "after": { "5": "b" } },
        "b": { "on": { "Done": "a" } } } }"#;
    let options = ImportOptions {
        emit_prefix: "notify:".into(),
        environment_source: "User".into(),
        clock_source: "Timer".into(),
        router_controller: "Bus".into(),
    };
    let model = resolve_ok(import_with(ImportFormat::XState, json, &options).expect("imports").definition);
    assert_eq!(emits_of(&model, "transition:m:a->b@GO"), ["Done"]);
    assert!(has(&model, "rule:Bus/Done#0"));
    assert_eq!(source_triggers(&model, "User"), ["m.GO"]);
    assert_eq!(source_triggers(&model, "Timer"), ["m.after_5ms"]);
}

// --- Names -------------------------------------------------------------------------------

#[test]
fn invalid_names_are_sanitized_with_warnings() {
    let json = r#"{ "id": "my machine", "states": {
        "Loading Data": { "on": { "user.submit": "done", "user_submit": "done" } },
        "done": {} } }"#;
    let imported = imported(json);
    let model = resolve_ok(imported.definition.clone());
    assert!(has(&model, "machine:my_machine"));
    assert!(has(&model, "transition:my_machine:Loading_Data->done@user_submit"));
    assert!(has(&model, "transition:my_machine:Loading_Data->done@user_submit_2"));
    let renamed: Vec<(NameKind, String, String)> = imported
        .warnings
        .iter()
        .filter_map(|w| match &w.kind {
            WarningKind::Renamed { what, original, name } => Some((*what, original.clone(), name.clone())),
            _ => None,
        })
        .collect();
    assert!(renamed.contains(&(NameKind::Machine, "my machine".into(), "my_machine".into())));
    assert!(renamed.contains(&(NameKind::State, "Loading Data".into(), "Loading_Data".into())));
    assert!(renamed.contains(&(NameKind::Trigger, "user_submit".into(), "user_submit_2".into())));
}

#[test]
fn wildcards_and_unknown_keys_are_ignored_with_warnings() {
    let json = r#"{ "id": "m", "bogus": 1, "states": { "a": { "on": { "*": "a", "GO": "a" } } } }"#;
    let imported = imported(json);
    let texts: Vec<String> = imported.warnings.iter().map(ToString::to_string).collect();
    assert!(texts.iter().any(|t| t.contains("wildcard")), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("unknown key `bogus`")), "{texts:?}");
    let _ = resolve_ok(imported.definition);
}

// --- Errors ----------------------------------------------------------------------------------

#[test]
fn unsupported_constructs() {
    let cases = [
        (r#"{ "id": "m", "states": { "a": { "always": "b" }, "b": {} } }"#, "always"),
        (r#"{ "id": "m", "type": "parallel", "states": { "a": {}, "b": {} } }"#, "parallel"),
        (r#"{ "id": "m", "states": { "p": { "type": "parallel", "states": { "a": {} } } } }"#, "parallel"),
        (
            r#"{ "id": "m", "states": { "a": { "on": { "GO": { "target": ["b", "c"] } } }, "b": {}, "c": {} } }"#,
            "multiple",
        ),
        (r#"{ "id": "m", "initial": "h", "states": { "h": { "type": "history" }, "a": {} } }"#, "history"),
        (r#"{ "id": "m" }"#, "without states"),
    ];
    for (json, what_contains) in cases {
        match import_err(json) {
            InteropError::Unsupported { what, .. } => assert!(what.contains(what_contains), "{what}"),
            other => panic!("{json}: {other:?}"),
        }
    }
}

#[test]
fn syntax_and_shape_errors() {
    match import_err("{ \"id\": \"m\",\n  \"states\": [ }") {
        InteropError::Syntax { line, .. } => assert_eq!(line, 2),
        other => panic!("{other:?}"),
    }
    for json in [
        "42",
        r#"{ "machines": [] }"#,
        r#"{ "id": "m", "states": { "a": { "on": { "GO": 5 } } } }"#,
        r#"{ "id": "m", "states": { "a": { "type": "weird" } } }"#,
        r#"{ "id": "m", "states": { "a": { "id": "x" }, "b": { "id": "x" } } }"#,
        r#"{ "id": "m", "states": { "f": { "type": "final", "states": { "x": {} } } } }"#,
        r#"[]"#,
    ] {
        assert!(matches!(import_err(json), InteropError::Invalid { .. }), "{json}");
    }
}
