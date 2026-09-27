//! Typed accessors over saphyr's marked YAML nodes that report shape errors
//! as diagnostics.

use saphyr::{AnnotatedMappingOwned, MarkedYamlOwned, ScalarOwned, YamlDataOwned};

use crate::error::{Diagnostic, DiagnosticKind, Expected};
use crate::parse::grammar::{is_valid_name, is_valid_path};
use crate::span::{Pos, SourceSpan, Spanned};

pub type Mapping = AnnotatedMappingOwned<MarkedYamlOwned>;

pub fn span_of(node: &MarkedYamlOwned) -> SourceSpan {
    let to_pos = |m: saphyr::Marker| {
        Pos::new(u32::try_from(m.line()).unwrap_or(u32::MAX), u32::try_from(m.col()).unwrap_or(u32::MAX))
    };
    SourceSpan::new(to_pos(node.span.start), to_pos(node.span.end))
}

/// Strip tags; aliases are rejected by the caller via [`Diags::check_alias`].
fn data(node: &MarkedYamlOwned) -> &YamlDataOwned<MarkedYamlOwned> {
    match &node.data {
        YamlDataOwned::Tagged(_, inner) => data(inner),
        other => other,
    }
}

/// The text of a scalar node, or `None` for null and non-scalars.
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
    pub list: Vec<Diagnostic>,
}

impl Diags {
    pub fn push(&mut self, kind: DiagnosticKind, span: SourceSpan) {
        self.list.push(Diagnostic { kind, span });
    }

    pub fn wrong_type(&mut self, node: &MarkedYamlOwned, context: &str, expected: Expected) {
        if matches!(node.data, YamlDataOwned::Alias(_)) {
            self.push(DiagnosticKind::AliasNotSupported, span_of(node));
        } else {
            self.push(DiagnosticKind::WrongType { context: context.to_owned(), expected }, span_of(node));
        }
    }

    pub fn mapping<'a>(&mut self, node: &'a MarkedYamlOwned, context: &str) -> Option<&'a Mapping> {
        let mapping = as_mapping(node);
        if mapping.is_none() {
            self.wrong_type(node, context, Expected::Mapping);
        }
        mapping
    }

    /// A string scalar. Numbers and booleans are accepted and rendered as
    /// text so that guards like `retries < 3` or names like `v2` work.
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
            self.push(DiagnosticKind::InvalidName { context: context.to_owned(), name: text.value }, text.span);
            None
        }
    }

    pub fn bool(&mut self, node: &MarkedYamlOwned, context: &str) -> Option<bool> {
        match data(node) {
            YamlDataOwned::Value(ScalarOwned::Boolean(b)) => Some(*b),
            _ => {
                self.wrong_type(node, context, Expected::Bool);
                None
            }
        }
    }

    /// `x` or `[x, y]`, each a string.
    pub fn string_or_list(&mut self, node: &MarkedYamlOwned, context: &str) -> Vec<Spanned<String>> {
        if let Some(items) = as_sequence(node) {
            items.iter().filter_map(|item| self.string(item, context)).collect()
        } else if let Some(text) = scalar_text(node) {
            vec![Spanned::new(text, span_of(node))]
        } else {
            self.wrong_type(node, context, Expected::StringOrSequence);
            Vec::new()
        }
    }

    /// Like [`Self::string_or_list`] but each item must be a valid name.
    pub fn name_or_list(&mut self, node: &MarkedYamlOwned, context: &str) -> Vec<Spanned<String>> {
        self.string_or_list(node, context)
            .into_iter()
            .filter_map(|item| self.validated(item, context, is_valid_name))
            .collect()
    }

    /// Like [`Self::string_or_list`] but each item must be a state reference.
    pub fn path_or_list(&mut self, node: &MarkedYamlOwned, context: &str) -> Vec<Spanned<String>> {
        self.string_or_list(node, context)
            .into_iter()
            .filter_map(|item| self.validated(item, context, is_valid_path))
            .collect()
    }

    /// Report keys outside `allowed` and return a lookup over the rest.
    pub fn fields<'a>(&mut self, mapping: &'a Mapping, context: &str, allowed: &[&str]) -> Fields<'a> {
        let mut entries = Vec::with_capacity(mapping.len());
        for (key, value) in mapping {
            match scalar_text(key) {
                Some(k) if allowed.contains(&k.as_str()) => entries.push((k, key, value)),
                Some(k) => self.push(DiagnosticKind::UnknownKey { context: context.to_owned(), key: k }, span_of(key)),
                None => self.wrong_type(key, context, Expected::String),
            }
        }
        Fields { entries }
    }

    /// Entries of a mapping whose keys are names (machines, controllers…).
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
    entries: Vec<(String, &'a MarkedYamlOwned, &'a MarkedYamlOwned)>,
}

impl<'a> Fields<'a> {
    pub fn get(&self, key: &str) -> Option<&'a MarkedYamlOwned> {
        self.entries.iter().find(|(k, _, _)| k == key).map(|(_, _, v)| *v)
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
            diags.push(DiagnosticKind::MissingKey { context: context.to_owned(), key: key.to_owned() }, span);
        }
        value
    }
}
