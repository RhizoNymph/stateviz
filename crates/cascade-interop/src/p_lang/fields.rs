//! Typed fields in P: declared field types become P types, and guards,
//! conditions and assignments over typed fields become P expressions.
//!
//! | Cascade | P |
//! | --- | --- |
//! | `any` (or an untyped list) | `any` |
//! | `string` | `string` |
//! | `number` | `float` (Cascade numbers are decimals; literals are written `42.0`) |
//! | `bool` | `bool` |
//! | `enum [a, b]` | `enum t<Owner>_<field> { <Owner>_<field>_a, … }` |
//!
//! An expression is translated only when every test compares a typed
//! field with a literal of its type or with a field of the same type; any
//! other test (an `any` field, a trigger payload the P event does not
//! carry, an ordering on an enum) keeps the whole guard a nondeterministic
//! choice (`$`).

use cascade_core::expr::{CmpOp, Expr, FieldDecl, FieldType, Literal, Operand};

use super::names::Idents;

/// How a field's values are written in P.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PKind {
    Any,
    Str,
    Number,
    Bool,
    /// The enum type's name and, per Cascade member, its P identifier.
    Enum {
        name: String,
        members: Vec<(String, String)>,
    },
}

impl PKind {
    pub(super) fn p_type(&self) -> &str {
        match self {
            PKind::Any => "any",
            PKind::Str => "string",
            PKind::Number => "float",
            PKind::Bool => "bool",
            PKind::Enum { name, .. } => name,
        }
    }
}

/// A Cascade field with its P identifier and kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PField {
    pub cascade: String,
    pub ident: String,
    pub kind: PKind,
}

/// The P kinds of a field list; enum types get global names
/// `t<owner>_<field>` and members `<owner>_<field>_<member>`.
pub(super) fn kinds(fields: &[FieldDecl], idents: &[String], owner: &str, global: &mut Idents) -> Vec<PField> {
    fields
        .iter()
        .zip(idents)
        .map(|(f, ident)| {
            let kind = match &f.ty {
                FieldType::Any => PKind::Any,
                FieldType::String => PKind::Str,
                FieldType::Number => PKind::Number,
                FieldType::Bool => PKind::Bool,
                FieldType::Enum(members) => {
                    let name = global.allocate(&format!("t{owner}_{ident}"));
                    let members =
                        members.iter().map(|m| (m.clone(), global.allocate(&format!("{owner}_{ident}_{m}")))).collect();
                    PKind::Enum { name, members }
                }
            };
            PField { cascade: f.name.clone(), ident: ident.clone(), kind }
        })
        .collect()
}

/// A literal as a P value of `kind`, when it fits.
pub(super) fn literal(lit: &Literal, kind: &PKind) -> Option<String> {
    match kind {
        PKind::Any => None,
        PKind::Str => Some(format!("\"{}\"", lit.text.replace('\\', "\\\\").replace('"', "\\\""))),
        PKind::Number if !lit.quoted => {
            let value: f64 = lit.text.parse().ok()?;
            value.is_finite().then(|| if lit.text.contains('.') { lit.text.clone() } else { format!("{}.0", lit.text) })
        }
        PKind::Bool if !lit.quoted && matches!(lit.text.as_str(), "true" | "false") => Some(lit.text.clone()),
        PKind::Enum { members, .. } => members.iter().find(|(m, _)| *m == lit.text).map(|(_, ident)| ident.clone()),
        PKind::Number | PKind::Bool => None,
    }
}

/// How the operands of an expression read in P: `None` when the operand
/// has no P value here.
pub(super) type Scope<'a> = &'a dyn Fn(&Operand) -> Option<(String, &'a PKind)>;

/// `expr` as a P boolean expression, when every test translates.
pub(super) fn condition(expr: &Expr, scope: Scope<'_>) -> Option<String> {
    match expr {
        Expr::Cmp { left, op, right } => compare(left, *op, right, scope),
        Expr::In { operand, set, negated } => {
            let tests: Vec<String> = set
                .iter()
                .map(|lit| compare(operand, CmpOp::Eq, &Operand::Lit(lit.clone()), scope))
                .collect::<Option<_>>()?;
            let any = format!("({})", tests.join(" || "));
            Some(if *negated { format!("!{any}") } else { any })
        }
        Expr::Not(inner) => Some(format!("!({})", condition(inner, scope)?)),
        Expr::And(items) => join(items, " && ", scope),
        Expr::Or(items) => join(items, " || ", scope),
    }
}

fn join(items: &[Expr], op: &str, scope: Scope<'_>) -> Option<String> {
    let parts: Vec<String> =
        items.iter().map(|e| condition(e, scope).map(|c| format!("({c})"))).collect::<Option<_>>()?;
    Some(parts.join(op))
}

fn compare(left: &Operand, op: CmpOp, right: &Operand, scope: Scope<'_>) -> Option<String> {
    let side = |operand: &Operand, other: &PKind| -> Option<String> {
        match operand {
            Operand::Lit(lit) => literal(lit, other),
            field => scope(field).filter(|(_, k)| *k == other).map(|(text, _)| text),
        }
    };
    let kind = [left, right].into_iter().find_map(|o| match o {
        Operand::Lit(_) => None,
        field => scope(field).map(|(_, k)| k.clone()),
    })?;
    let ordering = matches!(op, CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge);
    if matches!(kind, PKind::Any) || (ordering && !matches!(kind, PKind::Number | PKind::Str)) {
        return None;
    }
    let op = match op {
        CmpOp::Eq => "==",
        CmpOp::Ne => "!=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
    };
    Some(format!("{} {op} {}", side(left, &kind)?, side(right, &kind)?))
}

#[cfg(test)]
mod tests {
    use cascade_core::expr::{ExprContext, parse_expr};

    use super::*;

    fn tier() -> PKind {
        PKind::Enum { name: "tOrder_tier".into(), members: vec![("gold".into(), "Order_tier_gold".into())] }
    }

    fn translate(text: &str) -> Option<String> {
        let expr = parse_expr(text, ExprContext::Plain).expect("parses");
        let (number, tier) = (PKind::Number, tier());
        let any = PKind::Any;
        let scope = |o: &Operand| -> Option<(String, &PKind)> {
            match o {
                Operand::SelfField(f) if f == "amount" => Some(("amount".into(), &number)),
                Operand::SelfField(f) if f == "tier" => Some(("tier".into(), &tier)),
                Operand::SelfField(f) if f == "note" => Some(("note".into(), &any)),
                _ => None,
            }
        };
        condition(&expr, &scope)
    }

    #[test]
    fn typed_tests_translate() {
        assert_eq!(translate("self.amount > 0").as_deref(), Some("amount > 0.0"));
        assert_eq!(
            translate("self.amount >= 1.5 and self.tier == gold").as_deref(),
            Some("(amount >= 1.5) && (tier == Order_tier_gold)")
        );
        assert_eq!(translate("not self.tier in [gold]").as_deref(), Some("!((tier == Order_tier_gold))"));
    }

    #[test]
    fn untranslatable_tests_keep_the_choice() {
        assert_eq!(translate("event.amount > 0"), None);
        assert_eq!(translate("self.note == x"), None);
        assert_eq!(translate("self.tier < gold"), None);
        assert_eq!(translate("self.tier == silver"), None);
        assert_eq!(translate("self.amount > 0 or event.x == 1"), None);
    }
}
