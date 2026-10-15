//! Text forms the inspector edits that the core grammar has no single
//! parser for: typed field lists (`orderId: string, tier: enum [a, b]`),
//! assignment rows (`field = event.x`) and the trigger kind selector.
//!
//! Pure. Each parser has a matching formatter, and formatting what was
//! parsed parses back to the same value.

use cascade_core::definition::{FieldDef, TriggerSpec};
use cascade_core::expr::{Assignment, FieldType, parse_field_type, parse_operand};
use cascade_core::parse::grammar::is_valid_name;

/// Split `text` at commas outside brackets and quotes; items are trimmed
/// and empty items dropped.
pub fn split_top_level(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in text.chars() {
        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == q {
                    quote = None;
                }
                current.push(c);
            }
            None => match c {
                '"' | '\'' => {
                    quote = Some(c);
                    current.push(c);
                }
                '[' => {
                    depth += 1;
                    current.push(c);
                }
                ']' => {
                    depth = depth.saturating_sub(1);
                    current.push(c);
                }
                ',' if depth == 0 => items.push(std::mem::take(&mut current)),
                _ => current.push(c),
            },
        }
    }
    items.push(current);
    items.into_iter().map(|i| i.trim().to_owned()).filter(|i| !i.is_empty()).collect()
}

/// Parse a field list: `name` or `name: type` items, comma separated.
///
/// When no item has a type the list is untyped (the YAML list form);
/// otherwise every field is typed and an item without a type is `any`
/// (the mapping form). Names must be valid and distinct.
pub fn parse_field_list(text: &str) -> Result<Vec<FieldDef>, String> {
    let mut parsed: Vec<(String, Option<FieldType>)> = Vec::new();
    for item in split_top_level(text) {
        let (name, ty) = match item.split_once(':') {
            Some((name, ty)) => {
                let ty =
                    parse_field_type(ty.trim()).map_err(|e| format!("type of `{}`: {}", name.trim(), e.message))?;
                (name.trim().to_owned(), Some(ty))
            }
            None => (item.clone(), None),
        };
        if !is_valid_name(&name) {
            return Err(format!("`{name}` is not a valid field name (letters, digits, _ and -)"));
        }
        if parsed.iter().any(|(n, _)| *n == name) {
            return Err(format!("field `{name}` is listed twice"));
        }
        parsed.push((name, ty));
    }
    let typed = parsed.iter().any(|(_, ty)| ty.is_some());
    Ok(parsed
        .into_iter()
        .map(|(name, ty)| match (typed, ty) {
            (false, _) => FieldDef::untyped(name),
            (true, ty) => FieldDef::typed(name, ty.unwrap_or(FieldType::Any)),
        })
        .collect())
}

/// The text [`parse_field_list`] reads back to `fields`.
pub fn format_field_list(fields: &[FieldDef]) -> String {
    fields
        .iter()
        .map(|f| match &f.ty {
            Some(ty) => format!("{}: {}", f.name.value, ty.value),
            None => f.name.value.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether two field lists declare the same names and types in the same
/// form (spans ignored).
pub fn same_fields(a: &[FieldDef], b: &[FieldDef]) -> bool {
    let shape = |fields: &[FieldDef]| {
        fields.iter().map(|f| (f.name.value.clone(), f.ty.as_ref().map(|t| t.value.clone()))).collect::<Vec<_>>()
    };
    shape(a) == shape(b)
}

/// Parse an assignment row `field = value`, where `value` is an operand
/// (`event.x`, `self.x` or a literal).
pub fn parse_assignment(text: &str) -> Result<Assignment, String> {
    let Some((field, value)) = text.split_once('=') else {
        return Err("write `field = value` (value: event.x, self.x or a literal)".to_owned());
    };
    let field = field.trim();
    if !is_valid_name(field) {
        return Err(format!("`{field}` is not a valid field name"));
    }
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("`{field} =` needs a value (event.x, self.x or a literal)"));
    }
    let operand = parse_operand(value).map_err(|e| e.to_string())?;
    Ok(Assignment::new(field, operand))
}

/// The kind of a transition's trigger, as the inspector's selector shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TriggerKindTag {
    /// `on: name`.
    Command,
    /// `on: { event: E, where: … }`.
    Event,
    /// `after: 30s`.
    Timer,
}

impl TriggerKindTag {
    pub const ALL: [TriggerKindTag; 3] = [TriggerKindTag::Command, TriggerKindTag::Event, TriggerKindTag::Timer];

    pub const fn name(self) -> &'static str {
        match self {
            TriggerKindTag::Command => "command",
            TriggerKindTag::Event => "event",
            TriggerKindTag::Timer => "timer",
        }
    }

    /// The selector's button label.
    pub const fn label(self) -> &'static str {
        match self {
            TriggerKindTag::Command => "command",
            TriggerKindTag::Event => "subscribe to event",
            TriggerKindTag::Timer => "timer",
        }
    }

    pub fn parse(text: &str) -> Option<TriggerKindTag> {
        Self::ALL.into_iter().find(|k| k.name() == text.trim())
    }

    pub fn of(spec: &TriggerSpec) -> TriggerKindTag {
        match spec {
            TriggerSpec::Command(_) => TriggerKindTag::Command,
            TriggerSpec::Event { .. } => TriggerKindTag::Event,
            TriggerSpec::After(_) => TriggerKindTag::Timer,
        }
    }

    /// What to enter to switch a transition to this kind.
    pub const fn prompt(self) -> &'static str {
        match self {
            TriggerKindTag::Command => "Enter a command name to make this trigger a command.",
            TriggerKindTag::Event => "Enter the event to subscribe to (then an optional `where`).",
            TriggerKindTag::Timer => "Enter a delay (e.g. 30s, 5m, 2h) to make this a timer.",
        }
    }
}

#[cfg(test)]
mod tests {
    use cascade_core::expr::{Literal, Operand};

    use super::*;

    #[test]
    fn split_respects_brackets_and_quotes() {
        assert_eq!(split_top_level("a, b: enum [x, y], c"), ["a", "b: enum [x, y]", "c"]);
        assert_eq!(split_top_level("t: enum [\"a, b\", c]"), ["t: enum [\"a, b\", c]"]);
        assert_eq!(split_top_level("k = 'x, y', z = 1"), ["k = 'x, y'", "z = 1"]);
        assert_eq!(split_top_level(" , a,, "), ["a"]);
        assert!(split_top_level("").is_empty());
    }

    #[test]
    fn untyped_lists_stay_untyped() {
        let fields = parse_field_list("orderId, customerId").expect("parses");
        assert_eq!(fields, vec![FieldDef::untyped("orderId"), FieldDef::untyped("customerId")]);
        assert_eq!(format_field_list(&fields), "orderId, customerId");
        assert!(parse_field_list("").expect("empty is fine").is_empty());
    }

    #[test]
    fn one_type_makes_the_whole_list_typed() {
        let fields = parse_field_list("orderId: string, amount: number, gift: bool, tier: enum [basic, gold], note")
            .expect("parses");
        assert_eq!(
            fields,
            vec![
                FieldDef::typed("orderId", FieldType::String),
                FieldDef::typed("amount", FieldType::Number),
                FieldDef::typed("gift", FieldType::Bool),
                FieldDef::typed("tier", FieldType::Enum(vec!["basic".into(), "gold".into()])),
                FieldDef::typed("note", FieldType::Any),
            ]
        );
        let text = format_field_list(&fields);
        assert_eq!(text, "orderId: string, amount: number, gift: bool, tier: enum [basic, gold], note: any");
        assert!(same_fields(&parse_field_list(&text).expect("round trip"), &fields));
    }

    #[test]
    fn bad_field_lists_say_why() {
        assert!(parse_field_list("a b").unwrap_err().contains("not a valid field name"));
        assert!(parse_field_list("a, a").unwrap_err().contains("twice"));
        assert!(parse_field_list("a: integer").unwrap_err().starts_with("type of `a`"));
        assert!(parse_field_list("a: enum [x, x]").unwrap_err().contains("duplicate"));
        assert!(parse_field_list("a: enum []").is_err());
    }

    #[test]
    fn same_fields_compares_form_and_types() {
        let untyped = vec![FieldDef::untyped("a")];
        let any = vec![FieldDef::typed("a", FieldType::Any)];
        assert!(same_fields(&untyped, &untyped));
        assert!(!same_fields(&untyped, &any), "list form and mapping form differ");
        assert!(!same_fields(&any, &[FieldDef::typed("a", FieldType::Number)]));
    }

    #[test]
    fn assignments() {
        assert_eq!(
            parse_assignment("fromAgent = event.originAgent"),
            Ok(Assignment::new("fromAgent", Operand::Event("originAgent".into())))
        );
        assert_eq!(
            parse_assignment(" status=open "),
            Ok(Assignment::new("status", Operand::Lit(Literal::new("open"))))
        );
        assert_eq!(
            parse_assignment("note = \"a = b\""),
            Ok(Assignment::new("note", Operand::Lit(Literal::quoted("a = b"))))
        );
        assert!(parse_assignment("status").unwrap_err().contains("field = value"));
        assert!(parse_assignment("bad name = 1").unwrap_err().contains("not a valid field name"));
        assert!(parse_assignment("x =").unwrap_err().contains("needs a value"));
        assert!(parse_assignment("x = event.").is_err());
        assert!(parse_assignment("x = a b").is_err());
    }

    #[test]
    fn kind_tags() {
        for kind in TriggerKindTag::ALL {
            assert_eq!(TriggerKindTag::parse(kind.name()), Some(kind));
        }
        assert_eq!(TriggerKindTag::parse("nope"), None);
        assert_eq!(TriggerKindTag::of(&TriggerSpec::command("go")), TriggerKindTag::Command);
    }
}
