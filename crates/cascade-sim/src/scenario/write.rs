//! [`Scenario`] → YAML in the scenario file format.
//!
//! The output parses back with [`parse_scenario`](super::parse_scenario) to
//! the same scenario (spans aside), and writing that again gives the same
//! text. Every entry is one flow-style line, like hand-written scenarios:
//!
//! ```yaml
//! scenario: manual race
//! instances:
//!   o1: { machine: Order, fields: { orderId: "1" } }
//! steps:
//!   - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
//!   - step
//!   - { step: 1 }
//! end: pause
//! ```
//!
//! Field and payload values are always double-quoted, so `"1.50"` stays
//! text rather than becoming the number `1.5`. Names are written plain
//! unless YAML would read them as something else (`true`, `null`, …).

use super::{Directive, InstanceDecl, Scenario, ScenarioEnd, ScenarioEntry, Step, StepTiming, ValueMap};

/// Write a scenario as YAML in the scenario file format.
pub fn scenario_to_yaml(scenario: &Scenario) -> String {
    let mut out = format!("scenario: {}\n", free_text(&scenario.name));
    if !scenario.instances.is_empty() {
        out.push_str("instances:\n");
        for decl in &scenario.instances {
            out.push_str(&format!("  {}: {{ {} }}\n", name(&decl.name.value), instance_body(decl)));
        }
    }
    let entries: Vec<String> = scenario.entries().map(entry).collect();
    if entries.is_empty() {
        out.push_str("steps: []\n");
    } else {
        out.push_str("steps:\n");
        for entry in entries {
            out.push_str(&format!("  - {entry}\n"));
        }
    }
    if scenario.end != ScenarioEnd::default() {
        out.push_str(&format!("end: {}\n", scenario.end.name()));
    }
    out
}

fn entry(entry: ScenarioEntry<'_>) -> String {
    match entry {
        ScenarioEntry::Fire(step) => fire(step),
        ScenarioEntry::Directive(Directive::Deliver { choice: None, .. }) => "step".to_owned(),
        ScenarioEntry::Directive(Directive::Deliver { choice: Some(n), .. }) => format!("{{ step: {n} }}"),
        ScenarioEntry::Directive(Directive::Run { .. }) => "run".to_owned(),
        ScenarioEntry::Directive(Directive::Create(decl)) => {
            format!("{{ create: {}, {} }}", name(&decl.name.value), instance_body(decl))
        }
        ScenarioEntry::Directive(Directive::Remove { name: removed, .. }) => {
            format!("{{ remove: {} }}", name(&removed.value))
        }
    }
}

fn fire(step: &Step) -> String {
    let mut parts =
        vec![format!("source: {}", name(&step.source.value)), format!("fire: {}", name(&step.fire.value.to_string()))];
    if let Some(target) = &step.target {
        parts.push(format!("target: {}", name(&target.value)));
    }
    if !step.payload.is_empty() {
        parts.push(format!("payload: {}", values(&step.payload)));
    }
    if step.timing != StepTiming::default() {
        parts.push(format!("timing: {}", step.timing.name()));
    }
    format!("{{ {} }}", parts.join(", "))
}

/// `machine: M, fields: {…}, state: s` (without braces).
fn instance_body(decl: &InstanceDecl) -> String {
    let mut parts = vec![format!("machine: {}", name(&decl.machine.value))];
    if !decl.fields.is_empty() {
        parts.push(format!("fields: {}", values(&decl.fields)));
    }
    if let Some(state) = &decl.state {
        parts.push(format!("state: {}", name(&state.value)));
    }
    parts.join(", ")
}

fn values(map: &ValueMap) -> String {
    let entries: Vec<String> =
        map.iter().map(|(key, entry)| format!("{}: {}", name(key), quoted(&entry.value.value))).collect();
    format!("{{ {} }}", entries.join(", "))
}

/// Words YAML reads as something other than a string when plain.
fn is_reserved(text: &str) -> bool {
    matches!(
        text.to_ascii_lowercase().as_str(),
        "null" | "true" | "false" | "yes" | "no" | "on" | "off" | "y" | "n" | "~" | ".nan" | ".inf"
    )
}

/// A name, dotted path or trigger reference: plain when it is made of name
/// characters and YAML would not read it as a bool or null.
fn name(text: &str) -> String {
    let mut chars = text.chars();
    let plain = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !is_reserved(text);
    if plain { text.to_owned() } else { quoted(text) }
}

/// Free text such as the scenario name: plain when it is words separated by
/// single spaces, quoted otherwise.
fn free_text(text: &str) -> String {
    let plain = !text.is_empty()
        && text.chars().next().is_some_and(|c| c.is_alphabetic())
        && !text.ends_with(' ')
        && !text.contains("  ")
        && text.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.'))
        && !is_reserved(text);
    if plain { text.to_owned() } else { quoted(text) }
}

/// A double-quoted YAML scalar.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_plain_unless_yaml_would_misread_them() {
        assert_eq!(name("o1"), "o1");
        assert_eq!(name("Order.capture_ok"), "Order.capture_ok");
        assert_eq!(name("playing.fast"), "playing.fast");
        assert_eq!(name("true"), "\"true\"");
        assert_eq!(name("Null"), "\"Null\"");
        assert_eq!(name("_x-1"), "_x-1");
    }

    #[test]
    fn free_text_is_quoted_when_needed() {
        assert_eq!(free_text("happy path"), "happy path");
        assert_eq!(free_text("a: b"), "\"a: b\"");
        assert_eq!(free_text("# not a comment"), "\"# not a comment\"");
        assert_eq!(free_text(""), "\"\"");
        assert_eq!(free_text("no"), "\"no\"");
        assert_eq!(free_text("1st run"), "\"1st run\"");
    }

    #[test]
    fn quoting_escapes() {
        assert_eq!(quoted("say \"hi\"\n\\"), "\"say \\\"hi\\\"\\n\\\\\"");
        assert_eq!(quoted("\u{7}"), "\"\\u0007\"");
    }
}
