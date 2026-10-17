//! Declared field types: how a field's string value is read when an
//! expression uses it.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::error::ExprError;
use super::lexer::{Tok, parse_number, tokenize};
use super::types::Literal;

/// The type of an instance field or payload field.
///
/// Runtime values stay strings; the type says how they compare (see
/// `docs/features/expressions.md`). `Any` is the type of every undeclared
/// field.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FieldType {
    #[default]
    Any,
    String,
    Number,
    Bool,
    /// One of the listed members (in declaration order, no duplicates).
    Enum(Vec<String>),
}

impl FieldType {
    /// Whether `value` is valid text for this type.
    pub fn accepts(&self, value: &str) -> bool {
        match self {
            FieldType::Any | FieldType::String => true,
            FieldType::Number => parse_number(value).is_some(),
            FieldType::Bool => matches!(value, "true" | "false"),
            FieldType::Enum(members) => members.iter().any(|m| m == value),
        }
    }

    /// A short description of what the type accepts, for diagnostics.
    pub fn expected(&self) -> String {
        match self {
            FieldType::Any | FieldType::String => "any text".to_owned(),
            FieldType::Number => "a decimal number".to_owned(),
            FieldType::Bool => "true or false".to_owned(),
            FieldType::Enum(members) => format!("one of {}", members.join(", ")),
        }
    }
}

impl fmt::Display for FieldType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldType::Any => f.write_str("any"),
            FieldType::String => f.write_str("string"),
            FieldType::Number => f.write_str("number"),
            FieldType::Bool => f.write_str("bool"),
            FieldType::Enum(members) => {
                f.write_str("enum [")?;
                for (i, member) in members.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    Literal::new(member.clone()).fmt(f)?;
                }
                f.write_str("]")
            }
        }
    }
}

/// A declared field: name and type (`Any` when untyped).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FieldDecl {
    pub name: String,
    pub ty: FieldType,
}

impl FieldDecl {
    pub fn new(name: impl Into<String>, ty: FieldType) -> Self {
        Self { name: name.into(), ty }
    }

    pub fn untyped(name: impl Into<String>) -> Self {
        Self::new(name, FieldType::Any)
    }
}

/// Parse a field type: `any`, `string`, `number`, `bool` or
/// `enum [a, b, …]` (members are words or quoted strings, at least one, no
/// duplicates).
pub fn parse_field_type(text: &str) -> Result<FieldType, ExprError> {
    let tokens = tokenize(text)?;
    let end = text.chars().count() + 1;
    let mut iter = tokens.iter();
    let ty = match iter.next() {
        Some(t) if t.tok.is_word("any") => FieldType::Any,
        Some(t) if t.tok.is_word("string") => FieldType::String,
        Some(t) if t.tok.is_word("number") => FieldType::Number,
        Some(t) if t.tok.is_word("bool") => FieldType::Bool,
        Some(t) if t.tok.is_word("enum") => {
            let open = iter.next();
            if !matches!(open.map(|t| &t.tok), Some(Tok::LBracket)) {
                return Err(ExprError::new("expected `[` after `enum`", open.map_or(end, |t| t.col)));
            }
            let mut members: Vec<String> = Vec::new();
            loop {
                let member = iter.next();
                let text = match member.map(|t| &t.tok) {
                    Some(Tok::Word(w) | Tok::Quoted(w)) => w.clone(),
                    _ => return Err(ExprError::new("expected an enum member", member.map_or(end, |t| t.col))),
                };
                if members.contains(&text) {
                    return Err(ExprError::new(
                        format!("duplicate enum member `{text}`"),
                        member.map_or(end, |t| t.col),
                    ));
                }
                members.push(text);
                let sep = iter.next();
                match sep.map(|t| &t.tok) {
                    Some(Tok::Comma) => {}
                    Some(Tok::RBracket) => break,
                    _ => return Err(ExprError::new("expected `,` or `]`", sep.map_or(end, |t| t.col))),
                }
            }
            FieldType::Enum(members)
        }
        Some(t) => {
            return Err(ExprError::new(
                format!("unknown type {}; expected any, string, number, bool or enum [..]", t.tok.describe()),
                t.col,
            ));
        }
        None => return Err(ExprError::new("expected a type", end)),
    };
    match iter.next() {
        None => Ok(ty),
        Some(t) => Err(ExprError::new(format!("unexpected {} after the type", t.tok.describe()), t.col)),
    }
}
