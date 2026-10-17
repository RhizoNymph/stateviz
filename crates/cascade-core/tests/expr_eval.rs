//! Evaluating expressions: missing operands, declared types (numbers,
//! bools, enums, strings, any), membership and negation.

use std::collections::BTreeMap;

use cascade_core::expr::{ExprContext, FieldDecl, FieldType, MapScope, Operand, Types, eval, parse_expr, value};

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

struct Fixture {
    event: BTreeMap<String, String>,
    me: BTreeMap<String, String>,
    types: Types,
}

impl Fixture {
    fn new(event: &[(&str, &str)], me: &[(&str, &str)]) -> Self {
        let types = Types::new(
            &[
                FieldDecl::new("n", FieldType::Number),
                FieldDecl::new("tier", FieldType::Enum(vec!["basic".into(), "gold".into()])),
                FieldDecl::new("gift", FieldType::Bool),
                FieldDecl::new("name", FieldType::String),
            ],
            &[FieldDecl::new("amount", FieldType::Number), FieldDecl::new("ok", FieldType::Bool)],
        );
        Self { event: map(event), me: map(me), types }
    }

    fn holds(&self, text: &str) -> bool {
        let expr = match parse_expr(text, ExprContext::Plain) {
            Ok(e) => e,
            Err(err) => panic!("{text}: {err}"),
        };
        eval(&expr, &MapScope { event: &self.event, self_fields: &self.me }, &self.types)
    }

    fn holds_untyped(&self, text: &str) -> bool {
        let expr = match parse_expr(text, ExprContext::Plain) {
            Ok(e) => e,
            Err(err) => panic!("{text}: {err}"),
        };
        eval(&expr, &MapScope { event: &self.event, self_fields: &self.me }, &Types::untyped())
    }
}

#[test]
fn missing_operands_make_tests_false() {
    let f = Fixture::new(&[], &[]);
    for text in [
        "event.x == a",
        "event.x != a",
        "event.x < 3",
        "event.x in [a]",
        "event.x not in [a]",
        "self.y == event.x",
        "self.n >= 0",
    ] {
        assert!(!f.holds(text), "{text}");
        assert!(f.holds(&format!("not ({text})")), "not ({text})");
    }
}

#[test]
fn numbers_compare_numerically() {
    let f = Fixture::new(&[("amount", "5.0")], &[("n", "10")]);
    assert!(f.holds("event.amount == 5"));
    assert!(f.holds("event.amount > 4.5"));
    assert!(f.holds("event.amount < self.n"));
    assert!(f.holds("self.n > 9"));
    assert!(!f.holds("self.n > 10"));
    assert!(f.holds("self.n >= 10 and self.n <= 10"));
    assert!(f.holds("event.amount != 6"));
    assert!(f.holds("event.amount in [1, 5, 7]"));
    // Text compare would say "10" < "9".
    assert!(!f.holds("self.n < 9"));
}

#[test]
fn values_that_do_not_read_as_their_type_make_tests_false() {
    let f = Fixture::new(&[("amount", "lots")], &[("tier", "platinum"), ("gift", "yes")]);
    assert!(!f.holds("event.amount > 0"));
    assert!(!f.holds("event.amount != 0"));
    assert!(!f.holds("self.tier != gold"));
    assert!(!f.holds("self.tier not in [gold]"));
    assert!(!f.holds("self.gift == false"));
    assert!(!f.holds("self.gift != true"));
}

#[test]
fn bools_and_enums_support_equality_only() {
    let f = Fixture::new(&[("ok", "true")], &[("tier", "gold"), ("gift", "false")]);
    assert!(f.holds("event.ok == true"));
    assert!(f.holds("self.gift == false"));
    assert!(f.holds("self.gift != event.ok"));
    assert!(f.holds("self.tier == gold"));
    assert!(f.holds("self.tier != basic"));
    assert!(f.holds("self.tier in [basic, gold]"));
    assert!(f.holds("self.tier not in [basic]"));
    assert!(!f.holds("self.tier > basic"));
    assert!(!f.holds("self.gift < true"));
}

#[test]
fn strings_compare_as_text() {
    let f = Fixture::new(&[], &[("name", "10")]);
    assert!(f.holds("self.name == 10"));
    assert!(!f.holds("self.name == 10.0"));
    assert!(f.holds("self.name < 9"), "text order");
}

#[test]
fn any_compares_text_but_orders_numbers() {
    let f = Fixture::new(&[("x", "10"), ("w", "pear")], &[]);
    assert!(f.holds_untyped("event.x == 10"));
    assert!(!f.holds_untyped("event.x == 10.0"), "equality is textual");
    assert!(f.holds_untyped("event.x > 9"), "ordering is numeric when both are numbers");
    assert!(f.holds_untyped("event.w > apple"), "otherwise textual");
    assert!(f.holds_untyped("event.w < 'zebra'"));
}

#[test]
fn boolean_structure() {
    let f = Fixture::new(&[("amount", "3")], &[("tier", "gold")]);
    assert!(f.holds("event.amount > 0 and self.tier == gold"));
    assert!(!f.holds("event.amount > 5 and self.tier == gold"));
    assert!(f.holds("event.amount > 5 or self.tier == gold"));
    assert!(!f.holds("not (event.amount > 0)"));
    assert!(f.holds("not not event.amount > 0"));
    assert!(f.holds("event.missing == x or (event.amount == 3 and not self.tier == basic)"));
}

#[test]
fn literal_comparisons_and_quoted_literals() {
    let f = Fixture::new(&[("s", "a b")], &[]);
    assert!(f.holds("1 < 2"));
    assert!(!f.holds("a == b"));
    assert!(f.holds("event.s == 'a b'"));
    assert!(f.holds("event.s in [\"a b\", c]"));
}

#[test]
fn value_reads_operands() {
    let event = map(&[("x", "1")]);
    let me = map(&[("y", "2")]);
    let scope = MapScope { event: &event, self_fields: &me };
    assert_eq!(value(&Operand::Event("x".into()), &scope), Some("1"));
    assert_eq!(value(&Operand::SelfField("y".into()), &scope), Some("2"));
    assert_eq!(value(&Operand::Event("y".into()), &scope), None);
    assert_eq!(value(&Operand::Lit(cascade_core::expr::Literal::new("z")), &scope), Some("z"));
}

#[test]
fn types_default_to_any() {
    let types = Types::new(&[FieldDecl::new("n", FieldType::Number)], &[]);
    assert_eq!(types.self_type("n"), &FieldType::Number);
    assert_eq!(types.self_type("other"), &FieldType::Any);
    assert_eq!(types.event_type("n"), &FieldType::Any);
    assert!(FieldType::Number.accepts("-3.25"));
    assert!(!FieldType::Number.accepts("3e2"));
    assert!(FieldType::Bool.accepts("false"));
    assert!(!FieldType::Bool.accepts("False"));
}
