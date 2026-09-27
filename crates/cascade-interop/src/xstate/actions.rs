//! XState actions and guards: which events an action emits, and a guard's
//! display text.
//!
//! Actions that emit or send events (as serializable action objects):
//!
//! | Action object | Emits | Delivered to |
//! | --- | --- | --- |
//! | `{ type: "xstate.raise", event }` | `event` | the machine itself |
//! | `{ type: "xstate.sendTo", to: "Id", event }` | `event` | machine `Id` (other machines when `to` is not a string) |
//! | `{ type: "xstate.sendParent", event }` | `event` | the other machines |
//! | `{ type: "xstate.emit", event }` | `event` | the other machines |
//! | `"emit:Name"` or `{ type: "emit:Name" }` | `Name` | every machine, the emitter included |
//!
//! The `xstate.` prefix is optional; `event` may be `"Name"` or
//! `{ type: "Name", … }`, directly or under `params`. The `emit:` prefix is
//! [`ImportOptions::emit_prefix`](crate::ImportOptions). Any other action
//! emits nothing.

use crate::error::InteropError;
use crate::import::Warnings;
use crate::import::chart::{Emit, Route};
use crate::xstate::FORMAT;
use crate::xstate::json::Json;

fn invalid(location: &str, message: impl Into<String>) -> InteropError {
    InteropError::Invalid { format: FORMAT, location: location.to_owned(), message: message.into() }
}

/// The events an `actions`/`entry`/`exit` value emits.
pub(crate) fn emits(
    value: &Json,
    emit_prefix: &str,
    location: &str,
    warnings: &mut Warnings,
) -> Result<Vec<Emit>, InteropError> {
    match value {
        Json::Null => Ok(Vec::new()),
        Json::Array(items) => {
            let mut out = Vec::new();
            for (i, item) in items.iter().enumerate() {
                let item_location = format!("{location}[{i}]");
                match item {
                    Json::Array(_) => return Err(invalid(&item_location, "actions cannot be nested arrays")),
                    other => out.extend(emits(other, emit_prefix, &item_location, warnings)?),
                }
            }
            Ok(out)
        }
        Json::String(name) => Ok(prefixed(name, emit_prefix).into_iter().collect()),
        Json::Object(_) => action_object(value, emit_prefix, location, warnings),
        other => Err(invalid(location, format!("an action must be a string or an object, found {}", other.kind()))),
    }
}

fn prefixed(name: &str, emit_prefix: &str) -> Option<Emit> {
    if emit_prefix.is_empty() {
        return None;
    }
    let event = name.strip_prefix(emit_prefix)?;
    (!event.is_empty()).then(|| Emit { event: event.to_owned(), route: Route::All })
}

fn action_object(
    action: &Json,
    emit_prefix: &str,
    location: &str,
    warnings: &mut Warnings,
) -> Result<Vec<Emit>, InteropError> {
    let Some(kind) = action.get("type").and_then(Json::as_str) else {
        warnings.ignored(location, "an action object without a string `type`");
        return Ok(Vec::new());
    };
    let route = match kind.strip_prefix("xstate.").unwrap_or(kind) {
        "raise" => Route::Own,
        "sendTo" => match action.get("to") {
            Some(Json::String(to)) => Route::Machine(to.clone()),
            _ => Route::Others,
        },
        "sendParent" | "emit" => Route::Others,
        "forwardTo" => {
            warnings.ignored(location, "`forwardTo` (the forwarded event is only known at run time)");
            return Ok(Vec::new());
        }
        _ => return Ok(prefixed(kind, emit_prefix).into_iter().collect()),
    };
    let event = action.get("event").or_else(|| action.get("params").and_then(|p| p.get("event")));
    let name = match event {
        Some(Json::String(name)) => Some(name.as_str()),
        Some(Json::Object(_)) => event.and_then(|e| e.get("type")).and_then(Json::as_str),
        _ => None,
    };
    match name {
        Some(name) if !name.is_empty() => Ok(vec![Emit { event: name.to_owned(), route }]),
        _ => {
            warnings.ignored(location, format!("a `{kind}` action whose event is not a static name"));
            Ok(Vec::new())
        }
    }
}

/// A guard's display text: `"name"`, `{ type: "name" }` (with parameters as
/// `name({"min":1})`), and the `and`/`or`/`not` combinators.
pub(crate) fn guard_text(value: &Json, location: &str) -> Result<String, InteropError> {
    match value {
        Json::String(text) => Ok(text.clone()),
        Json::Object(_) => {
            let Some(kind) = value.get("type").and_then(Json::as_str) else {
                return Err(invalid(location, "a guard object needs a string `type`"));
            };
            let parts = |joiner: &str| -> Result<String, InteropError> {
                let Some(Json::Array(guards)) = value.get("guards") else {
                    return Err(invalid(location, format!("a `{kind}` guard needs a `guards` array")));
                };
                let texts = guards.iter().map(|g| guard_text(g, location)).collect::<Result<Vec<_>, _>>()?;
                Ok(texts.iter().map(|t| format!("({t})")).collect::<Vec<_>>().join(joiner))
            };
            match kind.strip_prefix("xstate.").unwrap_or(kind) {
                "and" => parts(" && "),
                "or" => parts(" || "),
                "not" => Ok(format!("!{}", parts("")?)),
                _ => match value.get("params") {
                    None | Some(Json::Null) => Ok(kind.to_owned()),
                    Some(params) => Ok(format!("{kind}({})", params.to_compact())),
                },
            }
        }
        other => Err(invalid(location, format!("a guard must be a string or an object, found {}", other.kind()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(text: &str) -> Json {
        match serde_json::from_str(text) {
            Ok(j) => j,
            Err(err) => panic!("{err}"),
        }
    }

    fn events(text: &str) -> Vec<(String, Route)> {
        let mut warnings = Warnings::default();
        match emits(&json(text), "emit:", "test", &mut warnings) {
            Ok(list) => list.into_iter().map(|e| (e.event, e.route)).collect(),
            Err(err) => panic!("{err}"),
        }
    }

    #[test]
    fn built_in_actions_emit_their_events() {
        assert_eq!(
            events(r#"{ "type": "xstate.raise", "event": { "type": "RETRY" } }"#),
            [("RETRY".into(), Route::Own)]
        );
        assert_eq!(
            events(r#"{ "type": "xstate.sendTo", "to": "Payment", "event": "CHARGE" }"#),
            [("CHARGE".into(), Route::Machine("Payment".into()))]
        );
        assert_eq!(
            events(r#"{ "type": "sendParent", "event": { "type": "DONE" } }"#),
            [("DONE".into(), Route::Others)]
        );
        assert_eq!(
            events(r#"{ "type": "xstate.emit", "params": { "event": { "type": "Loaded" } } }"#),
            [("Loaded".into(), Route::Others)]
        );
    }

    #[test]
    fn prefixed_action_names_emit() {
        assert_eq!(
            events(r#"["log", "emit:OrderPaid", { "type": "emit:Audit" }, null]"#),
            [("OrderPaid".into(), Route::All), ("Audit".into(), Route::All)]
        );
        assert!(events(r#""emit:""#).is_empty());
    }

    #[test]
    fn dynamic_events_are_ignored_with_a_warning() {
        let mut warnings = Warnings::default();
        let got = emits(&json(r#"{ "type": "xstate.raise" }"#), "emit:", "here", &mut warnings);
        assert_eq!(got.map(|v| v.len()), Ok(0));
        assert_eq!(warnings.list.len(), 1);
    }

    #[test]
    fn guard_texts() {
        let text = |t: &str| guard_text(&json(t), "g");
        assert_eq!(text(r#""isValid""#), Ok("isValid".into()));
        assert_eq!(text(r#"{ "type": "isValid" }"#), Ok("isValid".into()));
        assert_eq!(text(r#"{ "type": "above", "params": { "min": 1 } }"#), Ok(r#"above({"min":1})"#.into()));
        assert_eq!(
            text(r#"{ "type": "xstate.and", "guards": ["a", { "type": "xstate.not", "guards": ["b"] }] }"#),
            Ok("(a) && (!(b))".into())
        );
        assert!(text("3").is_err());
        assert!(text(r#"{ "params": 1 }"#).is_err());
    }
}
