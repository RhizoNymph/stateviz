//! XState v5 import.
//!
//! Input is JSON: one machine config (the `createMachine` argument), an
//! array of configs, or `{ "machines": { "Name": config } }`. A machine is
//! named by its config's `id`, else its key in `machines`, else `Machine`.
//!
//! | XState | Cascade |
//! | --- | --- |
//! | `states` (nested) | states (nested), in document order |
//! | `initial` | `initial` (machine: path; compound state: child) |
//! | `type: "final"` | `kind: final` |
//! | `type: "history"`, `history: "deep"` | `kind: history` / `deep-history` |
//! | `on: { EVENT: target \| {target, guard, actions} \| [...] }` | one transition per alternative, trigger `EVENT` |
//! | targets `"sibling"`, `".child"`, `"#id.path"` | full state paths |
//! | targetless transition | self-transition |
//! | root-level `on` | one transition from every top-level state |
//! | `guard: "name"` / `{ type, params }` / `and`/`or`/`not` | free-text guard |
//! | `after: { 1000: target }` | trigger `after_1000ms`, fired by `Clock` |
//! | `invoke: { id, onDone, onError }` | triggers `<id>_done` / `<id>_error`, fired by a source named after the actor |
//! | `onDone` of a compound state | trigger `<state>_done`, fired by `Environment` (approximation) |
//! | `raise`, `sendTo`, `sendParent`, `emit`, `emit:Name` actions (transition, entry, exit) | `emits` on the transitions that run them, routed by `EventRouter` |
//! | events no emit covers | fired by `Environment` |
//! | `always`, `type: "parallel"`, multiple targets | [`InteropError::Unsupported`] |
//! | `context`, `meta`, `tags`, `description`, other actions | ignored |

mod actions;
mod json;
mod parse;

use crate::error::InteropError;
use crate::import::lower::{LowerInput, lower};
use crate::import::wiring::Wiring;
use crate::import::{ImportOptions, Imported, Warnings};

use json::Json;

pub(crate) const FORMAT: &str = "XState";

fn invalid(location: &str, message: impl Into<String>) -> InteropError {
    InteropError::Invalid { format: FORMAT, location: location.to_owned(), message: message.into() }
}

pub(crate) fn import(text: &str, options: &ImportOptions) -> Result<Imported, InteropError> {
    let json: Json = serde_json::from_str(text).map_err(|err| InteropError::Syntax {
        format: FORMAT,
        line: u32::try_from(err.line()).unwrap_or(u32::MAX),
        col: u32::try_from(err.column()).unwrap_or(u32::MAX),
        message: err.to_string(),
    })?;
    let mut warnings = Warnings::default();
    let prefix = options.emit_prefix.as_str();
    let charts = match &json {
        Json::Array(configs) => configs
            .iter()
            .enumerate()
            .map(|(i, config)| parse::machine(None, config, format!("[{i}]"), prefix, &mut warnings))
            .collect::<Result<Vec<_>, _>>()?,
        Json::Object(_) if json.get("machines").is_some() && json.get("states").is_none() => {
            let Some(machines) = json.get("machines").and_then(Json::as_object) else {
                return Err(invalid("machines", "`machines` must map machine names to configs"));
            };
            machines
                .iter()
                .map(|(key, config)| {
                    parse::machine(Some(key), config, format!("machines.{key}"), prefix, &mut warnings)
                })
                .collect::<Result<Vec<_>, _>>()?
        }
        Json::Object(_) => {
            let location = json.get("id").and_then(Json::as_str).unwrap_or("machine").to_owned();
            vec![parse::machine(None, &json, location, prefix, &mut warnings)?]
        }
        other => {
            return Err(invalid(
                "$",
                format!(
                    "expected a machine config, an array of configs or {{ \"machines\": … }}, found {}",
                    other.kind()
                ),
            ));
        }
    };
    if charts.is_empty() {
        return Err(invalid("$", "the input contains no machines"));
    }
    let definition =
        lower(LowerInput { format: FORMAT, system: None, charts, wiring: Wiring::Synthesize }, options, &mut warnings)?;
    Ok(Imported { definition, warnings: warnings.list })
}
