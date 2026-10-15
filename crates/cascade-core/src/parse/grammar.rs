//! The small grammars embedded in YAML string values: names, trigger
//! references (`Machine.trigger`) and target selectors.

use crate::definition::{TargetMode, TargetSpec, TriggerRef};
use crate::expr::lexer::{Tok, tokenize};
use crate::expr::parser::Parser;
use crate::expr::{Assignment, ExprContext, ExprError};

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

/// Parse a target selector.
///
/// ```text
/// selector := ["all"] MACHINE ["where" expr]
///           | "new" MACHINE ["with" FIELD "=" operand ("," FIELD "=" operand)*]
/// ```
///
/// The `where` expression is in [`ExprContext::Filter`]: a bare name on the
/// left of a test is the candidate's field (`orderId == event.orderId`), and
/// `event.x == lit` tests the handled event. Errors carry the column.
pub fn parse_target(text: &str) -> Result<TargetSpec, String> {
    parse_target_inner(text).map_err(|e| e.to_string())
}

fn parse_target_inner(text: &str) -> Result<TargetSpec, ExprError> {
    let tokens = tokenize(text)?;
    let mut p = Parser::new(&tokens, text, ExprContext::Filter);

    let mode = if p.eat_word("all") {
        TargetMode::All
    } else if p.eat_word("new") {
        TargetMode::Spawn
    } else {
        TargetMode::One
    };

    let machine = match p.peek() {
        Some(Tok::Word(w)) if is_valid_name(w) => {
            let w = w.clone();
            p.next();
            w
        }
        _ => return Err(p.expected("a machine name")),
    };

    if mode == TargetMode::Spawn {
        let mut assignments = Vec::new();
        if p.peek().is_some() {
            if !p.eat_word("with") {
                return Err(p.expected("`with`"));
            }
            loop {
                let field = match p.peek() {
                    Some(Tok::Word(w)) if is_valid_name(w) => {
                        let w = w.clone();
                        p.next();
                        w
                    }
                    _ => return Err(p.expected("a field name")),
                };
                p.expect(&Tok::Assign)?;
                let value = p.operand(false)?;
                assignments.push(Assignment { field, value });
                if !p.eat(&Tok::Comma) {
                    break;
                }
            }
        }
        p.finish()?;
        return Ok(TargetSpec::Spawn { machine, assignments });
    }

    let filter = if p.peek().is_some() {
        if !p.eat_word("where") {
            return Err(p.expected("`where`"));
        }
        let filter = p.expr()?;
        p.finish()?;
        Some(filter)
    } else {
        None
    };
    Ok(match mode {
        TargetMode::All => TargetSpec::All { machine, filter },
        TargetMode::One | TargetMode::Spawn => TargetSpec::One { machine, filter },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::expr::{CmpOp, Expr, Literal, Operand};

    fn eq(left: Operand, right: Operand) -> Expr {
        Expr::Cmp { left, op: CmpOp::Eq, right }
    }

    fn me(f: &str) -> Operand {
        Operand::SelfField(f.into())
    }

    fn ev(f: &str) -> Operand {
        Operand::Event(f.into())
    }

    fn lit(t: &str) -> Operand {
        Operand::Lit(Literal::new(t))
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
        assert_eq!(spec, TargetSpec::One { machine: "Shipment".into(), filter: None });
        assert_eq!(spec.mode(), TargetMode::One);
        assert_eq!(spec.machine(), "Shipment");
    }

    #[test]
    fn where_clause_with_event_field() {
        let spec = parse_target("Shipment where orderId == event.orderId").expect("valid");
        assert_eq!(
            spec,
            TargetSpec::One { machine: "Shipment".into(), filter: Some(eq(me("orderId"), ev("orderId"))) }
        );
    }

    #[test]
    fn fan_out_with_multiple_predicates() {
        let spec = parse_target("all Job where batch == event.batch and kind == 'nightly run'").expect("valid");
        assert_eq!(
            spec,
            TargetSpec::All {
                machine: "Job".into(),
                filter: Some(Expr::And(vec![
                    eq(me("batch"), ev("batch")),
                    eq(me("kind"), Operand::Lit(Literal::quoted("nightly run"))),
                ])),
            }
        );
    }

    #[test]
    fn filters_are_full_expressions() {
        let spec =
            parse_target("all Job where event.kind == refund and (prio > 3 or self.vip == true)").expect("valid");
        let Some(filter) = spec.filter() else { panic!("filter") };
        assert_eq!(filter.display_filter().to_string(), "event.kind == refund and (prio > 3 or vip == true)");
        assert!(parse_target("Job where not tier in [a, b]").is_ok());
    }

    #[test]
    fn spawn_with_assignments() {
        let spec = parse_target("new Shipment with orderId = event.orderId, carrier = ups").expect("valid");
        assert_eq!(
            spec,
            TargetSpec::Spawn {
                machine: "Shipment".into(),
                assignments: vec![Assignment::new("orderId", ev("orderId")), Assignment::new("carrier", lit("ups"))],
            }
        );
        assert_eq!(
            parse_target("new Shipment"),
            Ok(TargetSpec::Spawn { machine: "Shipment".into(), assignments: Vec::new() })
        );
    }

    #[test]
    fn display_round_trips() {
        for text in [
            "Shipment",
            "Shipment where orderId == event.orderId",
            "all Job where batch == event.batch and kind == \"nightly run\"",
            "all Job where event.kind == refund or kind != x",
            "new Shipment with orderId = event.orderId, carrier = ups",
            "new Shipment",
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
            "new Shipment with a = 1 b = 2",
            "new Shipment where a == 1",
        ] {
            assert!(parse_target(text).is_err(), "should reject {text:?}");
        }
        assert_eq!(
            parse_target("Shipment where orderId ="),
            Err("expected a comparison (`==`, `!=`, `<`, `<=`, `>`, `>=`) or `in`, found `=` (column 24)".to_owned())
        );
    }
}
