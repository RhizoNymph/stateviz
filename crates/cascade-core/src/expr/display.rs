//! Canonical text: single spaces, minimal parentheses, literals quoted only
//! when they were quoted or must be. Parsing canonical text gives back an
//! equal value (in the context it was printed for).

use std::fmt;

use super::lexer::needs_quotes;
use super::parser::ExprContext;
use super::types::{Assignment, CmpOp, Condition, Expr, Guard, Literal, Operand};
use crate::parse::grammar::is_valid_name;

impl fmt::Display for CmpOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.quoted || needs_quotes(&self.text) { write_quoted(f, &self.text) } else { f.write_str(&self.text) }
    }
}

fn write_quoted(f: &mut fmt::Formatter<'_>, text: &str) -> fmt::Result {
    f.write_str("\"")?;
    for c in text.chars() {
        if matches!(c, '"' | '\\') {
            f.write_str("\\")?;
        }
        write!(f, "{c}")?;
    }
    f.write_str("\"")
}

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Event(field) => write!(f, "event.{field}"),
            Operand::SelfField(field) => write!(f, "self.{field}"),
            Operand::Lit(lit) => lit.fmt(f),
        }
    }
}

impl fmt::Display for Assignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} = {}", self.field, self.value)
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_expr(f, self, ExprContext::Plain)
    }
}

impl Expr {
    /// Canonical text for a filter ([`ExprContext::Filter`]): a `self.x` on
    /// the left of a test is written as the bare name `x`, and a bare-word
    /// literal there is quoted (it would otherwise read as a field).
    pub fn display_filter(&self) -> impl fmt::Display + '_ {
        InContext { expr: self, context: ExprContext::Filter }
    }

    /// Canonical text for `context`.
    pub fn display_in(&self, context: ExprContext) -> impl fmt::Display + '_ {
        InContext { expr: self, context }
    }
}

struct InContext<'a> {
    expr: &'a Expr,
    context: ExprContext,
}

impl fmt::Display for InContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_expr(f, self.expr, self.context)
    }
}

fn write_left(f: &mut fmt::Formatter<'_>, operand: &Operand, context: ExprContext) -> fmt::Result {
    match (context, operand) {
        (ExprContext::Filter, Operand::SelfField(field)) if !needs_quotes(field) => f.write_str(field),
        (ExprContext::Filter, Operand::Lit(lit)) if is_valid_name(&lit.text) && lit.as_bool().is_none() => {
            write_quoted(f, &lit.text)
        }
        _ => write!(f, "{operand}"),
    }
}

fn write_expr(f: &mut fmt::Formatter<'_>, expr: &Expr, context: ExprContext) -> fmt::Result {
    match expr {
        Expr::Cmp { left, op, right } => {
            write_left(f, left, context)?;
            write!(f, " {op} {right}")
        }
        Expr::In { operand, set, negated } => {
            write_left(f, operand, context)?;
            f.write_str(if *negated { " not in [" } else { " in [" })?;
            for (i, lit) in set.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{lit}")?;
            }
            f.write_str("]")
        }
        Expr::Not(inner) => {
            f.write_str("not ")?;
            write_child(f, inner, matches!(**inner, Expr::And(_) | Expr::Or(_)), context)
        }
        Expr::And(items) => write_joined(f, items, " and ", |e| matches!(e, Expr::And(_) | Expr::Or(_)), context),
        Expr::Or(items) => write_joined(f, items, " or ", |e| matches!(e, Expr::Or(_)), context),
    }
}

fn write_joined(
    f: &mut fmt::Formatter<'_>,
    items: &[Expr],
    joiner: &str,
    needs_parens: fn(&Expr) -> bool,
    context: ExprContext,
) -> fmt::Result {
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            f.write_str(joiner)?;
        }
        // A one-item list (not built by the parser) would read back as its
        // item; parenthesizing keeps nested lists apart.
        write_child(f, item, needs_parens(item), context)?;
    }
    Ok(())
}

fn write_child(f: &mut fmt::Formatter<'_>, expr: &Expr, parens: bool, context: ExprContext) -> fmt::Result {
    if parens {
        f.write_str("(")?;
        write_expr(f, expr, context)?;
        f.write_str(")")
    } else {
        write_expr(f, expr, context)
    }
}

impl fmt::Display for Guard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Guard::Expr(expr) => expr.fmt(f),
            Guard::Choice(label) => write!(f, "?{label}"),
            Guard::Else => f.write_str("else"),
        }
    }
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Condition::Expr(expr) => expr.fmt(f),
            Condition::Choice(label) => write!(f, "?{label}"),
        }
    }
}
