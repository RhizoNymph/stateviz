//! Scenario YAML → [`Scenario`].
//!
//! Parsing is total over well-formed YAML: every shape problem becomes a
//! diagnostic and parsing continues with the next sibling, so one run
//! reports every problem in the file.

use saphyr::{LoadableYamlNode, MarkedYamlOwned};

use cascade_core::definition::TriggerRef;
use cascade_core::error::Expected;
use cascade_core::parse::grammar::parse_trigger_ref;
use cascade_core::span::{SourceSpan, Spanned};

use super::node::{Diags, as_mapping, is_null, pos, scalar_text, span_of};
use super::{InstanceDecl, Scenario, Step, StepTiming, ValueMap};
use crate::error::{ScenarioError, ScenarioErrorKind};

/// Parse a scenario file. Names are not checked against a model here; see
/// [`validate`](super::validate).
pub fn parse_scenario(text: &str) -> Result<Scenario, ScenarioError> {
    let documents = match MarkedYamlOwned::load_from_str(text) {
        Ok(docs) => docs,
        Err(err) => {
            let at = pos(*err.marker());
            return Err(ScenarioError::single(
                ScenarioErrorKind::YamlSyntax { message: err.info().to_owned() },
                SourceSpan::new(at, at),
            ));
        }
    };

    let root = match documents.as_slice() {
        [] => return Err(ScenarioError::single(ScenarioErrorKind::EmptyDocument, SourceSpan::unknown())),
        [root] if is_null(root) => {
            return Err(ScenarioError::single(ScenarioErrorKind::EmptyDocument, span_of(root)));
        }
        [root] => root,
        [_, second, ..] => {
            return Err(ScenarioError::single(ScenarioErrorKind::MultipleDocuments, span_of(second)));
        }
    };

    let mut diags = Diags::default();
    let scenario = parse_root(root, &mut diags);
    ScenarioError::from_list(diags.list)?;
    Ok(scenario)
}

/// Walk the document. When a diagnostic is recorded the returned scenario is
/// incomplete and is discarded by the caller.
fn parse_root(root: &MarkedYamlOwned, diags: &mut Diags) -> Scenario {
    let mut scenario = Scenario { name: String::new(), instances: Vec::new(), steps: Vec::new() };
    let Some(mapping) = diags.mapping(root, "scenario file") else {
        return scenario;
    };
    let fields = diags.fields(mapping, "scenario file", &["scenario", "instances", "steps"]);

    if let Some(name) =
        fields.require("scenario", diags, "scenario file", span_of(root)).and_then(|n| diags.string(n, "scenario"))
    {
        scenario.name = name.value;
    }

    if let Some(node) = fields.get("instances").filter(|n| !is_null(n))
        && let Some(instances) = diags.mapping(node, "instances")
    {
        for (name, body) in diags.named_entries(instances, "instances") {
            if let Some(decl) = parse_instance(name, body, diags) {
                scenario.instances.push(decl);
            }
        }
    }

    if let Some(node) = fields.require("steps", diags, "scenario file", span_of(root))
        && !is_null(node)
        && let Some(items) = diags.sequence(node, "steps")
    {
        for (i, item) in items.iter().enumerate() {
            if let Some(step) = parse_step(i + 1, item, diags) {
                scenario.steps.push(step);
            }
        }
    }

    scenario
}

/// `name: { machine: M, fields: {…}, state: s }`, or `name: M` for an
/// instance with no fields in the machine's initial state.
fn parse_instance(name: Spanned<String>, body: &MarkedYamlOwned, diags: &mut Diags) -> Option<InstanceDecl> {
    let context = format!("instance `{}`", name.value);
    let span = SourceSpan::new(name.span.start, span_of(body).end);
    if as_mapping(body).is_none() && scalar_text(body).is_some() {
        let machine = diags.name(body, &format!("{context} machine"))?;
        return Some(InstanceDecl { name, machine, fields: ValueMap::new(), state: None, span });
    }

    let Some(mapping) = as_mapping(body) else {
        diags.wrong_type(body, &context, Expected::Mapping);
        return None;
    };
    let fields = diags.fields(mapping, &context, &["machine", "fields", "state"]);
    let machine = fields
        .require("machine", diags, &context, name.span)
        .and_then(|n| diags.name(n, &format!("{context} machine")));
    let values = fields.get("fields").map(|n| value_map(n, &format!("{context} fields"), diags)).unwrap_or_default();
    let state = fields.get("state").and_then(|n| diags.path(n, &format!("{context} state")));
    Some(InstanceDecl { name, machine: machine?, fields: values, state, span })
}

fn parse_step(number: usize, item: &MarkedYamlOwned, diags: &mut Diags) -> Option<Step> {
    let context = format!("step {number}");
    let span = span_of(item);
    let mapping = diags.mapping(item, &context)?;
    let fields = diags.fields(mapping, &context, &["source", "fire", "target", "payload", "timing"]);

    let source =
        fields.require("source", diags, &context, span).and_then(|n| diags.name(n, &format!("{context} source")));
    let fire = fields
        .require("fire", diags, &context, span)
        .and_then(|n| diags.string(n, &format!("{context} fire")))
        .and_then(|text| trigger_ref(text, diags));
    let target = fields.get("target").and_then(|n| diags.name(n, &format!("{context} target")));
    let payload = fields.get("payload").map(|n| value_map(n, &format!("{context} payload"), diags)).unwrap_or_default();
    let timing = match fields.get("timing").and_then(|n| diags.string(n, &format!("{context} timing"))) {
        None => StepTiming::default(),
        Some(text) => match text.value.as_str() {
            "immediate" => StepTiming::Immediate,
            "after-quiescence" | "after_quiescence" => StepTiming::AfterQuiescence,
            _ => {
                diags.push(ScenarioErrorKind::UnknownTiming { text: text.value }, text.span);
                StepTiming::default()
            }
        },
    };

    Some(Step { source: source?, fire: fire?, target, payload, timing, span })
}

fn trigger_ref(text: Spanned<String>, diags: &mut Diags) -> Option<Spanned<TriggerRef>> {
    match parse_trigger_ref(&text.value) {
        Some(parsed) => Some(Spanned::new(parsed, text.span)),
        None => {
            diags.push(ScenarioErrorKind::InvalidTriggerRef { text: text.value }, text.span);
            None
        }
    }
}

/// A mapping of names to scalar values (`fields:` and `payload:`). Null
/// means empty.
fn value_map(node: &MarkedYamlOwned, context: &str, diags: &mut Diags) -> ValueMap {
    let mut map = ValueMap::new();
    if is_null(node) {
        return map;
    }
    let Some(mapping) = diags.mapping(node, context) else {
        return map;
    };
    for (key, value) in diags.named_entries(mapping, context) {
        let value_context = format!("{context} `{}`", key.value);
        if let Some(value) = diags.string(value, &value_context) {
            map.insert(key, value);
        }
    }
    map
}
