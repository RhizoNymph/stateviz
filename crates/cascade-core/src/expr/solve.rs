//! Static reasoning about expressions: whether two can hold together
//! ([`overlap`]) and whether a set of them always has one holding
//! ([`coverage`]).
//!
//! A sound but incomplete procedure. Expressions are put in disjunctive
//! normal form (bounded by [`MAX_CONJUNCTIONS`]; past it the answer is
//! `MayOverlap` / `Unknown`). A conjunction is decided one field at a time:
//! every test in it reads at most one field (tests comparing two fields are
//! set aside as unknown), so the conjunction is satisfiable exactly when each
//! field has a value satisfying its tests. A field may also be missing (or
//! hold text that does not read as its type), which makes every test on it
//! false and every negated test true.
//!
//! For a field, a finite set of candidate values is tried with the
//! evaluator itself, so a "satisfiable" answer is always exact. Candidates
//! are chosen so that a satisfying value exists only if a candidate
//! satisfies: enum members, both bools, and for numbers and text the
//! literals, their neighbours (`f64::next_up`/`next_down`; text successor
//! `t + "\0"`) and values beyond them. Where that argument does not hold
//! (text ordered against a number-typed literal in an `any` field, ordering
//! of a number against a word) the field is reported unknown.

use std::collections::BTreeMap;

use super::eval::{Scope, Types, eval};
use super::fields::FieldType;
use super::lexer::parse_number;
use super::types::{Expr, Literal, Operand};

/// Past this many conjunctions in a normal form, give up.
pub const MAX_CONJUNCTIONS: usize = 256;

/// At most this many gap examples are reported.
pub const MAX_EXAMPLES: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Overlap {
    /// No scope makes both true.
    Exclusive,
    /// Some scope may make both true (or the procedure could not tell).
    MayOverlap,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Coverage {
    /// In every scope at least one expression holds.
    Complete,
    /// Some scopes make every expression false; readable examples of them,
    /// e.g. `self.tier = basic, event.amount is missing`.
    Gaps(Vec<String>),
    /// The procedure could not tell.
    Unknown,
}

/// Whether `a` and `b` can both hold in one scope.
pub fn overlap(a: &Expr, b: &Expr, types: &Types) -> Overlap {
    let both = Expr::And(vec![a.clone(), b.clone()]);
    let Some(dnf) = dnf(&both, true) else {
        return Overlap::MayOverlap;
    };
    if dnf.iter().all(|c| matches!(decide(c, types), Decision::Unsat)) {
        Overlap::Exclusive
    } else {
        Overlap::MayOverlap
    }
}

/// Whether at least one of `exprs` holds in every scope. No expressions
/// cover nothing.
pub fn coverage(exprs: &[&Expr], types: &Types) -> Coverage {
    let negated = Expr::And(exprs.iter().map(|e| (*e).clone().negate()).collect());
    let Some(dnf) = dnf(&negated, true) else {
        return Coverage::Unknown;
    };
    let mut examples: Vec<String> = Vec::new();
    let mut unknown = false;
    for conjunction in &dnf {
        match decide(conjunction, types) {
            Decision::Unsat => {}
            Decision::Unknown => unknown = true,
            Decision::Sat(witnesses) => {
                for witness in witnesses {
                    if !examples.contains(&witness) && examples.len() < MAX_EXAMPLES {
                        examples.push(witness);
                    }
                }
            }
        }
    }
    if !examples.is_empty() {
        Coverage::Gaps(examples)
    } else if unknown {
        Coverage::Unknown
    } else {
        Coverage::Complete
    }
}

/// A test (`Cmp` or `In`) with its polarity: `positive == false` means the
/// conjunction needs it false.
#[derive(Clone, Debug)]
struct Atom<'a> {
    test: &'a Expr,
    positive: bool,
}

type Conjunction<'a> = Vec<Atom<'a>>;

/// Disjunctive normal form of `expr` (negated when `!positive`), or `None`
/// past the size bound.
fn dnf(expr: &Expr, positive: bool) -> Option<Vec<Conjunction<'_>>> {
    match (expr, positive) {
        (Expr::Cmp { .. } | Expr::In { .. }, _) => Some(vec![vec![Atom { test: expr, positive }]]),
        (Expr::Not(inner), _) => dnf(inner, !positive),
        (Expr::And(items), true) | (Expr::Or(items), false) => {
            let mut acc: Vec<Conjunction<'_>> = vec![Vec::new()];
            for item in items {
                let part = dnf(item, positive)?;
                if acc.len().saturating_mul(part.len()) > MAX_CONJUNCTIONS {
                    return None;
                }
                acc = acc
                    .iter()
                    .flat_map(|left| {
                        part.iter().map(move |right| left.iter().chain(right.iter()).cloned().collect::<Vec<_>>())
                    })
                    .collect();
            }
            Some(acc)
        }
        (Expr::Or(items), true) | (Expr::And(items), false) => {
            let mut acc = Vec::new();
            for item in items {
                acc.extend(dnf(item, positive)?);
                if acc.len() > MAX_CONJUNCTIONS {
                    return None;
                }
            }
            Some(acc)
        }
    }
}

enum Decision {
    /// Satisfiable, with readable witnesses: one preferring values, and one
    /// with every field that may be missing missing (when different).
    Sat(Vec<String>),
    Unsat,
    Unknown,
}

/// The fields a test reads.
enum Reads<'a> {
    /// Only literals: a constant.
    Nothing,
    One(&'a Operand),
    /// Two different fields: treated as unknown.
    Two,
}

fn field_of(test: &Expr) -> Reads<'_> {
    let fields: Vec<&Operand> = test.operands().into_iter().filter(|o| !o.is_literal()).collect();
    match fields.as_slice() {
        [] => Reads::Nothing,
        [one] => Reads::One(one),
        [a, b] if a == b => Reads::One(a),
        _ => Reads::Two,
    }
}

/// One field's value (or its absence) for evaluating atoms.
struct OneField<'a> {
    field: &'a Operand,
    value: Option<&'a str>,
}

impl Scope for OneField<'_> {
    fn event(&self, field: &str) -> Option<&str> {
        match self.field {
            Operand::Event(f) if f == field => self.value,
            _ => None,
        }
    }

    fn self_field(&self, field: &str) -> Option<&str> {
        match self.field {
            Operand::SelfField(f) if f == field => self.value,
            _ => None,
        }
    }
}

fn decide(conjunction: &Conjunction<'_>, types: &Types) -> Decision {
    let mut by_field: BTreeMap<&Operand, Vec<&Atom<'_>>> = BTreeMap::new();
    let mut uncertain = false;
    for atom in conjunction {
        match field_of(atom.test) {
            Reads::Two => uncertain = true,
            Reads::Nothing => {
                let scope = OneField { field: &Operand::Lit(Literal::new("")), value: None };
                if eval(atom.test, &scope, types) != atom.positive {
                    return Decision::Unsat;
                }
            }
            Reads::One(field) => by_field.entry(field).or_default().push(atom),
        }
    }

    let mut with_values = Vec::new();
    let mut with_missing = Vec::new();
    for (field, atoms) in &by_field {
        let missing = format!("{field} is missing");
        match decide_field(field, atoms, types) {
            FieldDecision::Unsat => return Decision::Unsat,
            FieldDecision::Unknown => uncertain = true,
            FieldDecision::Missing => {
                with_values.push(missing.clone());
                with_missing.push(missing);
            }
            FieldDecision::Value { value, missing_too } => {
                let shown = format!("{field} = {}", Literal::new(value));
                with_missing.push(if missing_too { missing } else { shown.clone() });
                with_values.push(shown);
            }
        }
    }
    if uncertain {
        return Decision::Unknown;
    }
    if with_values.is_empty() {
        return Decision::Sat(vec!["any input".to_owned()]);
    }
    let mut witnesses = vec![with_values.join(", ")];
    let other = with_missing.join(", ");
    if other != witnesses[0] {
        witnesses.push(other);
    }
    Decision::Sat(witnesses)
}

enum FieldDecision {
    /// A value satisfies the tests; `missing_too` when absence does as well.
    Value {
        value: String,
        missing_too: bool,
    },
    Missing,
    Unsat,
    Unknown,
}

fn decide_field(field: &Operand, atoms: &[&Atom<'_>], types: &Types) -> FieldDecision {
    let holds = |value: Option<&str>| {
        let scope = OneField { field, value };
        atoms.iter().all(|a| eval(a.test, &scope, types) == a.positive)
    };
    let ty = types.operand_type(field).cloned().unwrap_or_default();
    let candidates = candidates(atoms, &ty);
    let missing = holds(None);
    if let Some(value) = candidates.values.into_iter().find(|v| holds(Some(v))) {
        return FieldDecision::Value { value, missing_too: missing };
    }
    if missing {
        return FieldDecision::Missing;
    }
    if candidates.complete { FieldDecision::Unsat } else { FieldDecision::Unknown }
}

struct Candidates {
    values: Vec<String>,
    /// Whether some candidate satisfies the atoms whenever any value does.
    complete: bool,
}

/// Every literal text in the atoms, in order of appearance, deduplicated.
fn literal_texts(atoms: &[&Atom<'_>]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |text: &str| {
        if !out.iter().any(|t| t == text) {
            out.push(text.to_owned());
        }
    };
    for atom in atoms {
        match atom.test {
            Expr::Cmp { left, right, .. } => {
                for operand in [left, right] {
                    if let Operand::Lit(lit) = operand {
                        push(&lit.text);
                    }
                }
            }
            Expr::In { set, .. } => set.iter().for_each(|lit| push(&lit.text)),
            Expr::Not(_) | Expr::And(_) | Expr::Or(_) => {}
        }
    }
    out
}

fn has_ordering(atoms: &[&Atom<'_>], against: impl Fn(&str) -> bool) -> bool {
    atoms.iter().any(|atom| match atom.test {
        Expr::Cmp { left, op, right } if op.is_ordering() => [left, right].into_iter().any(|o| match o {
            Operand::Lit(lit) => against(&lit.text),
            Operand::Event(_) | Operand::SelfField(_) => false,
        }),
        _ => false,
    })
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

/// Numbers around each numeric literal: itself (as written and as printed),
/// ±1 and its immediate neighbours, so every non-empty interval between or
/// beyond the literals has a candidate.
fn numeric_candidates(texts: &[String], values: &mut Vec<String>) {
    let mut numbers: Vec<f64> = texts.iter().filter_map(|t| parse_number(t)).collect();
    if numbers.is_empty() {
        numbers.push(0.0);
    }
    for text in texts.iter().filter(|t| parse_number(t).is_some()) {
        push_unique(values, text.clone());
    }
    for &n in &numbers {
        for candidate in [n, n - 1.0, n + 1.0, n.next_down(), n.next_up()] {
            if candidate.is_finite() {
                push_unique(values, format!("{candidate}"));
            }
        }
    }
}

/// Text candidates: the empty string (the least text), each literal and
/// its successor `t + "\0"` (the least text above it), and a plain word.
fn text_candidates(texts: &[String], values: &mut Vec<String>, keep: impl Fn(&str) -> bool) {
    for text in texts {
        if keep(text) {
            push_unique(values, text.clone());
        }
    }
    for fresh in ["other", "other2"] {
        push_unique(values, fresh.to_owned());
    }
    push_unique(values, String::new());
    for text in texts {
        push_unique(values, format!("{text}\0"));
    }
}

fn candidates(atoms: &[&Atom<'_>], ty: &FieldType) -> Candidates {
    let texts = literal_texts(atoms);
    let mut values = Vec::new();
    let complete = match ty {
        FieldType::Bool => {
            values = vec!["true".to_owned(), "false".to_owned()];
            true
        }
        FieldType::Enum(members) => {
            values.clone_from(members);
            true
        }
        FieldType::Number => {
            numeric_candidates(&texts, &mut values);
            true
        }
        FieldType::String => {
            text_candidates(&texts, &mut values, |_| true);
            true
        }
        FieldType::Any => {
            // Numeric values: equality is textual (so also try another
            // spelling of each number), ordering numeric against numbers but
            // textual against words, which the candidates do not cover.
            numeric_candidates(&texts, &mut values);
            for text in texts.iter().filter(|t| parse_number(t).is_some()) {
                let other = if text.contains('.') { format!("{text}0") } else { format!("{text}.0") };
                push_unique(&mut values, other);
            }
            let numeric_complete = !has_ordering(atoms, |t| parse_number(t).is_none());
            // Other values: every test is textual.
            text_candidates(&texts, &mut values, |t| parse_number(t).is_none());
            numeric_complete
        }
    };
    Candidates { values, complete }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::fields::FieldDecl;
    use crate::expr::parser::{ExprContext, parse_expr};

    fn e(text: &str) -> Expr {
        match parse_expr(text, ExprContext::Plain) {
            Ok(expr) => expr,
            Err(err) => panic!("{text}: {err}"),
        }
    }

    fn typed() -> Types {
        Types::new(
            &[
                FieldDecl::new("tier", FieldType::Enum(vec!["basic".into(), "gold".into(), "vip".into()])),
                FieldDecl::new("gift", FieldType::Bool),
                FieldDecl::new("n", FieldType::Number),
                FieldDecl::new("name", FieldType::String),
            ],
            &[FieldDecl::new("amount", FieldType::Number)],
        )
    }

    fn ov(a: &str, b: &str, types: &Types) -> Overlap {
        overlap(&e(a), &e(b), types)
    }

    #[test]
    fn numbers_as_intervals() {
        let t = typed();
        assert_eq!(ov("event.amount > 0", "event.amount <= 0", &t), Overlap::Exclusive);
        assert_eq!(ov("event.amount > 0", "event.amount < 10", &t), Overlap::MayOverlap);
        assert_eq!(ov("event.amount >= 10", "event.amount < 10", &t), Overlap::Exclusive);
        assert_eq!(ov("event.amount > 1 and event.amount < 2", "event.amount == 1.5", &t), Overlap::MayOverlap);
        assert_eq!(ov("event.amount > 1 and event.amount < 2", "event.amount == 2", &t), Overlap::Exclusive);
        assert_eq!(ov("event.amount == 5", "event.amount == 5.0", &t), Overlap::MayOverlap, "numeric equality");
        assert_eq!(ov("event.amount == 5", "event.amount != 5", &t), Overlap::Exclusive);
    }

    #[test]
    fn missing_values_break_negations() {
        let t = typed();
        // Both negations hold when `n` is missing.
        assert_eq!(ov("not self.n > 5", "not self.n <= 5", &t), Overlap::MayOverlap);
        // But a positive test on each side needs a value.
        assert_eq!(ov("self.n > 5", "self.n <= 5", &t), Overlap::Exclusive);
    }

    #[test]
    fn enums_and_bools_are_finite() {
        let t = typed();
        assert_eq!(ov("self.tier == gold", "self.tier == basic", &t), Overlap::Exclusive);
        assert_eq!(ov("self.tier in [gold, vip]", "self.tier == basic", &t), Overlap::Exclusive);
        assert_eq!(ov("self.tier not in [gold, vip]", "self.tier != basic", &t), Overlap::Exclusive);
        assert_eq!(ov("self.tier != gold", "self.tier != basic", &t), Overlap::MayOverlap);
        assert_eq!(ov("self.gift == true", "self.gift == false", &t), Overlap::Exclusive);
        assert_eq!(ov("self.gift == true", "self.gift != false", &t), Overlap::MayOverlap);
    }

    #[test]
    fn strings_and_untyped_fields() {
        let t = typed();
        assert_eq!(ov("self.name == a", "self.name == b", &t), Overlap::Exclusive);
        assert_eq!(ov("self.name != a", "self.name != b", &t), Overlap::MayOverlap);
        let u = Types::untyped();
        assert_eq!(ov("event.x == a", "event.x == b", &u), Overlap::Exclusive);
        assert_eq!(ov("event.x > 0", "event.x <= 0", &u), Overlap::Exclusive);
        assert_eq!(ov("event.x == 5", "event.x != 5 and event.x >= 5 and event.x <= 5", &u), Overlap::Exclusive);
        // `5.0` is not the text `5` but is the number 5.
        assert_eq!(ov("event.x != 5", "event.x >= 5 and event.x <= 5", &u), Overlap::MayOverlap);
        // Text ordering of words: "b" sits between "a" and "c".
        assert_eq!(ov("event.x > a", "event.x < c", &u), Overlap::MayOverlap);
        assert_eq!(ov("event.x > c", "event.x < a", &u), Overlap::MayOverlap, "unknown is conservative");
    }

    #[test]
    fn separate_fields_are_independent() {
        let t = typed();
        assert_eq!(ov("self.tier == gold and event.amount > 0", "self.tier == basic", &t), Overlap::Exclusive);
        assert_eq!(ov("self.tier == gold", "event.amount > 0", &t), Overlap::MayOverlap);
        // `event.x` and `self.x` are different fields.
        let u = Types::untyped();
        assert_eq!(ov("event.x == a", "self.x == b", &u), Overlap::MayOverlap);
    }

    #[test]
    fn two_field_tests_are_unknown() {
        let u = Types::untyped();
        assert_eq!(ov("self.a == event.b", "self.a != event.b", &u), Overlap::MayOverlap);
        assert_eq!(ov("self.a == event.b and self.a == x", "self.a == y", &u), Overlap::Exclusive);
    }

    #[test]
    fn disjunctions_and_negations() {
        let t = typed();
        assert_eq!(ov("self.tier == gold or self.tier == vip", "self.tier == basic", &t), Overlap::Exclusive);
        assert_eq!(ov("self.tier == gold or event.amount > 3", "self.tier == basic", &t), Overlap::MayOverlap);
        assert_eq!(ov("not (self.tier == gold or self.tier == vip)", "self.tier == vip", &t), Overlap::Exclusive);
        assert_eq!(ov("1 == 1", "2 == 3", &t), Overlap::Exclusive, "constant false");
    }

    #[test]
    fn dnf_is_bounded() {
        let big: Vec<String> = (0..10).map(|i| format!("(event.a{i} == x or event.b{i} == y)")).collect();
        let big = big.join(" and ");
        assert!(dnf(&e(&big), true).is_none());
        assert_eq!(ov(&big, "event.a0 == q", &Types::untyped()), Overlap::MayOverlap);
        // Negating an `or` of `and`s multiplies out.
        let wide: Vec<String> = (0..10).map(|i| format!("(event.a{i} == x and event.b{i} == y)")).collect();
        assert_eq!(coverage(&[&e(&wide.join(" or "))], &Types::untyped()), Coverage::Unknown);
    }

    #[test]
    fn coverage_of_enums_numbers_and_bools() {
        let t = typed();
        let check = |guards: &[&str]| {
            let exprs: Vec<Expr> = guards.iter().map(|g| e(g)).collect();
            let refs: Vec<&Expr> = exprs.iter().collect();
            coverage(&refs, &t)
        };
        assert_eq!(
            check(&["self.tier == gold", "self.tier == basic"]),
            Coverage::Gaps(vec!["self.tier = vip".into(), "self.tier is missing".into()])
        );
        // Only missing remains.
        assert_eq!(
            check(&["self.tier in [gold, vip]", "self.tier == basic"]),
            Coverage::Gaps(vec!["self.tier is missing".into()])
        );
        assert_eq!(check(&["self.tier in [gold, vip]", "not self.tier in [gold, vip]"]), Coverage::Complete);
        assert_eq!(check(&["event.amount > 0", "not event.amount > 0"]), Coverage::Complete);
        assert_eq!(
            check(&["event.amount > 0", "event.amount < 0"]),
            Coverage::Gaps(vec!["event.amount = 0".into(), "event.amount is missing".into()])
        );
        assert_eq!(check(&["self.gift == true", "not self.gift == true"]), Coverage::Complete);
        assert_eq!(check(&[]), Coverage::Gaps(vec!["any input".into()]));
        assert_eq!(check(&["self.a == event.b", "self.a != event.b"]), Coverage::Unknown);
    }
}
