//! The small grammars embedded in YAML string values: names, trigger
//! references (`Machine.trigger`) and target selectors.

use crate::definition::{FieldClause, TargetMode, TargetSpec, TriggerRef, ValueExpr};

/// Names of machines, states, triggers, events, controllers, sources and
/// fields: a letter or `_`, then letters, digits, `_` or `-`.
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

/// A state reference: a name, or a dotted path of names (`running.fetching`).
pub fn is_valid_path(path: &str) -> bool {
    path.split('.').all(is_valid_name)
}

/// Parse `Machine.trigger`. Returns `None` unless both halves are valid names.
pub fn parse_trigger_ref(text: &str) -> Option<TriggerRef> {
    let (machine, trigger) = text.trim().split_once('.')?;
    (is_valid_name(machine) && is_valid_name(trigger))
        .then(|| TriggerRef { machine: machine.to_owned(), trigger: trigger.to_owned() })
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Quoted(String),
    EqEq,
    Eq,
    Comma,
}

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == ',' {
            chars.next();
            tokens.push(Token::Comma);
        } else if c == '=' {
            chars.next();
            if chars.peek() == Some(&'=') {
                chars.next();
                tokens.push(Token::EqEq);
            } else {
                tokens.push(Token::Eq);
            }
        } else if c == '"' || c == '\'' {
            chars.next();
            let mut value = String::new();
            let mut closed = false;
            for next in chars.by_ref() {
                if next == c {
                    closed = true;
                    break;
                }
                value.push(next);
            }
            if !closed {
                return Err("unterminated quoted string".to_owned());
            }
            tokens.push(Token::Quoted(value));
        } else if c.is_alphanumeric() || matches!(c, '_' | '-' | '.') {
            let mut word = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_alphanumeric() || matches!(next, '_' | '-' | '.') {
                    word.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(Token::Word(word));
        } else {
            return Err(format!("unexpected character `{c}`"));
        }
    }
    Ok(tokens)
}

fn parse_value(token: Option<Token>) -> Result<ValueExpr, String> {
    match token {
        Some(Token::Quoted(lit)) => Ok(ValueExpr::Literal(lit)),
        Some(Token::Word(word)) => match word.strip_prefix("event.") {
            Some(field) if is_valid_name(field) => Ok(ValueExpr::EventField(field.to_owned())),
            Some(_) => Err(format!("`{word}` is not a valid event field reference")),
            None => Ok(ValueExpr::Literal(word)),
        },
        Some(other) => Err(format!("expected a value, found {}", describe(&other))),
        None => Err("expected a value, found end of selector".to_owned()),
    }
}

fn describe(token: &Token) -> String {
    match token {
        Token::Word(w) => format!("`{w}`"),
        Token::Quoted(q) => format!("{q:?}"),
        Token::EqEq => "`==`".to_owned(),
        Token::Eq => "`=`".to_owned(),
        Token::Comma => "`,`".to_owned(),
    }
}

/// Parse a target selector.
///
/// ```text
/// selector := ["all"] MACHINE ["where" FIELD "==" value ("and" FIELD "==" value)*]
///           | "new" MACHINE ["with" FIELD "=" value ("," FIELD "=" value)*]
/// value    := "event." FIELD | WORD | QUOTED
/// ```
pub fn parse_target(text: &str) -> Result<TargetSpec, String> {
    let mut tokens = tokenize(text)?.into_iter().peekable();

    let mode = match tokens.peek() {
        Some(Token::Word(w)) if w == "all" => TargetMode::All,
        Some(Token::Word(w)) if w == "new" => TargetMode::Spawn,
        _ => TargetMode::One,
    };
    if mode != TargetMode::One {
        tokens.next();
    }

    let machine = match tokens.next() {
        Some(Token::Word(w)) if is_valid_name(&w) => w,
        Some(other) => return Err(format!("expected a machine name, found {}", describe(&other))),
        None => return Err("expected a machine name".to_owned()),
    };

    let mut clauses = Vec::new();
    let (lead, op, joiner) = match mode {
        TargetMode::Spawn => ("with", Token::Eq, None),
        TargetMode::One | TargetMode::All => ("where", Token::EqEq, Some("and")),
    };
    match tokens.next() {
        None => {
            return Ok(TargetSpec { mode, machine, clauses });
        }
        Some(Token::Word(w)) if w == lead => {}
        Some(other) => return Err(format!("expected `{lead}`, found {}", describe(&other))),
    }

    loop {
        let field = match tokens.next() {
            Some(Token::Word(w)) if is_valid_name(&w) => w,
            Some(other) => return Err(format!("expected a field name, found {}", describe(&other))),
            None => return Err("expected a field name".to_owned()),
        };
        match tokens.next() {
            Some(ref t) if *t == op => {}
            Some(other) => {
                return Err(format!("expected {}, found {}", describe(&op), describe(&other)));
            }
            None => return Err(format!("expected {} after `{field}`", describe(&op))),
        }
        let value = parse_value(tokens.next())?;
        clauses.push(FieldClause { field, value });

        match tokens.next() {
            None => break,
            Some(Token::Comma) if joiner.is_none() => {}
            Some(Token::Word(w)) if Some(w.as_str()) == joiner => {}
            Some(other) => {
                let expected = joiner.map_or("`,`".to_owned(), |j| format!("`{j}`"));
                return Err(format!("expected {expected} or end of selector, found {}", describe(&other)));
            }
        }
    }

    Ok(TargetSpec { mode, machine, clauses })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clause(field: &str, value: ValueExpr) -> FieldClause {
        FieldClause { field: field.to_owned(), value }
    }

    #[test]
    fn names() {
        assert!(is_valid_name("Order"));
        assert!(is_valid_name("capture_ok"));
        assert!(is_valid_name("deep-history"));
        assert!(is_valid_name("_x1"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("1abc"));
        assert!(!is_valid_name("a.b"));
        assert!(!is_valid_name("a b"));
        assert!(is_valid_path("running.fetching"));
        assert!(is_valid_path("idle"));
        assert!(!is_valid_path("running."));
        assert!(!is_valid_path(".x"));
    }

    #[test]
    fn trigger_refs() {
        assert_eq!(
            parse_trigger_ref("Shipment.start"),
            Some(TriggerRef { machine: "Shipment".into(), trigger: "start".into() })
        );
        assert_eq!(parse_trigger_ref("Shipment"), None);
        assert_eq!(parse_trigger_ref("Shipment.start.now"), None);
        assert_eq!(parse_trigger_ref(".start"), None);
    }

    #[test]
    fn bare_machine_selects_the_one_instance() {
        let spec = parse_target("Shipment").expect("valid");
        assert_eq!(spec.mode, TargetMode::One);
        assert_eq!(spec.machine, "Shipment");
        assert!(spec.clauses.is_empty());
    }

    #[test]
    fn where_clause_with_event_field() {
        let spec = parse_target("Shipment where orderId == event.orderId").expect("valid");
        assert_eq!(spec.mode, TargetMode::One);
        assert_eq!(spec.clauses, vec![clause("orderId", ValueExpr::EventField("orderId".into()))]);
    }

    #[test]
    fn fan_out_with_multiple_predicates() {
        let spec = parse_target("all Job where batch == event.batch and kind == 'nightly run'").expect("valid");
        assert_eq!(spec.mode, TargetMode::All);
        assert_eq!(spec.machine, "Job");
        assert_eq!(
            spec.clauses,
            vec![
                clause("batch", ValueExpr::EventField("batch".into())),
                clause("kind", ValueExpr::Literal("nightly run".into())),
            ]
        );
    }

    #[test]
    fn spawn_with_assignments() {
        let spec = parse_target("new Shipment with orderId = event.orderId, carrier = ups").expect("valid");
        assert_eq!(spec.mode, TargetMode::Spawn);
        assert_eq!(
            spec.clauses,
            vec![
                clause("orderId", ValueExpr::EventField("orderId".into())),
                clause("carrier", ValueExpr::Literal("ups".into())),
            ]
        );
    }

    #[test]
    fn display_round_trips() {
        for text in [
            "Shipment",
            "Shipment where orderId == event.orderId",
            "all Job where batch == event.batch and kind == \"nightly run\"",
            "new Shipment with orderId = event.orderId, carrier = ups",
        ] {
            let spec = parse_target(text).expect("valid");
            assert_eq!(spec.to_string(), text);
            assert_eq!(parse_target(&spec.to_string()), Ok(spec));
        }
    }

    #[test]
    fn rejects_malformed_selectors() {
        for text in [
            "",
            "where x == 1",
            "Shipment orderId == 1",
            "Shipment where orderId = 1",
            "Shipment where orderId ==",
            "new Shipment with a == 1",
            "Shipment where a == 1, b == 2",
            "Shipment where a == 'open",
            "Shipment where a == event.",
            "Shipment where a == 1 b",
            "Shipment where a == 1 @",
        ] {
            assert!(parse_target(text).is_err(), "should reject {text:?}");
        }
    }
}
