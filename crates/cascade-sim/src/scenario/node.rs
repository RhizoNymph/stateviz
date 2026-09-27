//! Typed accessors over saphyr's marked YAML nodes that record shape errors
//! as scenario diagnostics instead of failing.

use saphyr::{AnnotatedMappingOwned, MarkedYamlOwned, ScalarOwned, YamlDataOwned};

use cascade_core::error::Expected;
use cascade_core::parse::grammar::{is_valid_name, is_valid_path};
use cascade_core::span::{Pos, SourceSpan, Spanned};

use crate::error::{ScenarioDiagnostic, ScenarioErrorKind};

pub type Mapping = AnnotatedMappingOwned<MarkedYamlOwned>;

/// saphyr's lines are 1-based but its columns are 0-based (despite its
/// docs); shift the column so spans match `cascade_core`'s 1-based `Pos`.
pub fn pos(marker: saphyr::Marker) -> Pos {
    Pos::new(
        u32::try_from(marker.line()).unwrap_or(u32::MAX),
        u32::try_from(marker.col()).map_or(u32::MAX, |c| c.saturating_add(1)),
    )
}

pub fn span_of(node: &MarkedYamlOwned) -> SourceSpan {
    SourceSpan::new(pos(node.span.start), pos(node.span.end))
}

/// Strip tags. (saphyr expands aliases while loading, so anchors and
/// aliases simply work.)
fn data(node: &MarkedYamlOwned) -> &YamlDataOwned<MarkedYamlOwned> {
    match &node.data {
        YamlDataOwned::Tagged(_, inner) => data(inner),
        other => other,
    }
}

/// The text of a scalar node, or `None` for null and non-scalars. Numbers and
/// booleans are rendered as text, so `orderId: 1` and `orderId: "1"` agree.
pub fn scalar_text(node: &MarkedYamlOwned) -> Option<String> {
    match data(node) {
        YamlDataOwned::Value(ScalarOwned::String(s)) => Some(s.clone()),
        YamlDataOwned::Value(ScalarOwned::Integer(i)) => Some(i.to_string()),
        YamlDataOwned::Value(ScalarOwned::FloatingPoint(f)) => Some(f.to_string()),
        YamlDataOwned::Value(ScalarOwned::Boolean(b)) => Some(b.to_string()),
        YamlDataOwned::Representation(s, _, _) => Some(s.clone()),
        YamlDataOwned::Value(ScalarOwned::Null)
        | YamlDataOwned::Sequence(_)
        | YamlDataOwned::Mapping(_)
        | YamlDataOwned::Tagged(_, _)
        | YamlDataOwned::Alias(_)
        | YamlDataOwned::BadValue => None,
    }
}

pub fn is_null(node: &MarkedYamlOwned) -> bool {
    matches!(data(node), YamlDataOwned::Value(ScalarOwned::Null))
}

pub fn as_mapping(node: &MarkedYamlOwned) -> Option<&Mapping> {
    match data(node) {
        YamlDataOwned::Mapping(m) => Some(m),
        _ => None,
    }
}

pub fn as_sequence(node: &MarkedYamlOwned) -> Option<&Vec<MarkedYamlOwned>> {
    match data(node) {
        YamlDataOwned::Sequence(s) => Some(s),
        _ => None,
    }
}

/// Diagnostic sink shared by the whole parse.
#[derive(Default)]
pub struct Diags {
    pub list: Vec<ScenarioDiagnostic>,
}

impl Diags {
    pub fn push(&mut self, kind: ScenarioErrorKind, span: SourceSpan) {
        self.list.push(ScenarioDiagnostic { kind, span });
    }

    pub fn wrong_type(&mut self, node: &MarkedYamlOwned, context: &str, expected: Expected) {
        self.push(ScenarioErrorKind::WrongType { context: context.to_owned(), expected }, span_of(node));
    }

    pub fn mapping<'a>(&mut self, node: &'a MarkedYamlOwned, context: &str) -> Option<&'a Mapping> {
        let mapping = as_mapping(node);
        if mapping.is_none() {
            self.wrong_type(node, context, Expected::Mapping);
        }
        mapping
    }

    pub fn sequence<'a>(&mut self, node: &'a MarkedYamlOwned, context: &str) -> Option<&'a Vec<MarkedYamlOwned>> {
        let sequence = as_sequence(node);
        if sequence.is_none() {
            self.wrong_type(node, context, Expected::Sequence);
        }
        sequence
    }

    /// A scalar rendered as text.
    pub fn string(&mut self, node: &MarkedYamlOwned, context: &str) -> Option<Spanned<String>> {
        let text = scalar_text(node);
        if text.is_none() {
            self.wrong_type(node, context, Expected::String);
        }
        text.map(|t| Spanned::new(t, span_of(node)))
    }

    /// A string that must also be a valid name.
    pub fn name(&mut self, node: &MarkedYamlOwned, context: &str) -> Option<Spanned<String>> {
        let text = self.string(node, context)?;
        self.validated(text, context, is_valid_name)
    }

    /// A string that must be a state reference (a name or dotted path).
    pub fn path(&mut self, node: &MarkedYamlOwned, context: &str) -> Option<Spanned<String>> {
        let text = self.string(node, context)?;
        self.validated(text, context, is_valid_path)
    }

    fn validated(&mut self, text: Spanned<String>, context: &str, valid: fn(&str) -> bool) -> Option<Spanned<String>> {
        if valid(&text.value) {
            Some(text)
        } else {
            self.push(ScenarioErrorKind::InvalidName { context: context.to_owned(), name: text.value }, text.span);
            None
        }
    }

    /// Report keys outside `allowed` and return a lookup over the rest.
    pub fn fields<'a>(&mut self, mapping: &'a Mapping, context: &str, allowed: &[&str]) -> Fields<'a> {
        let mut entries = Vec::with_capacity(mapping.len());
        for (key, value) in mapping {
            match scalar_text(key) {
                Some(k) if allowed.contains(&k.as_str()) => entries.push((k, value)),
                Some(k) => {
                    self.push(ScenarioErrorKind::UnknownKey { context: context.to_owned(), key: k }, span_of(key));
                }
                None => self.wrong_type(key, context, Expected::String),
            }
        }
        Fields { entries }
    }

    /// Entries of a mapping whose keys must be names, with their key nodes'
    /// spans.
    pub fn named_entries<'a>(
        &mut self,
        mapping: &'a Mapping,
        context: &str,
    ) -> Vec<(Spanned<String>, &'a MarkedYamlOwned)> {
        mapping.iter().filter_map(|(key, value)| self.name(key, context).map(|name| (name, value))).collect()
    }
}

/// The recognised keys of one mapping.
pub struct Fields<'a> {
    entries: Vec<(String, &'a MarkedYamlOwned)>,
}

impl<'a> Fields<'a> {
    pub fn get(&self, key: &str) -> Option<&'a MarkedYamlOwned> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| *v)
    }

    /// Like [`Self::get`] but reports a missing key at `span`.
    pub fn require(
        &self,
        key: &str,
        diags: &mut Diags,
        context: &str,
        span: SourceSpan,
    ) -> Option<&'a MarkedYamlOwned> {
        let value = self.get(key);
        if value.is_none() {
            diags.push(ScenarioErrorKind::MissingKey { context: context.to_owned(), key: key.to_owned() }, span);
        }
        value
    }
}
