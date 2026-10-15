//! The expression grammar: every production, errors with columns, guards,
//! conditions, assignment values, field types; and canonical display
//! round-trips (hand-picked and property-generated).

use cascade_core::expr::{
    Assignment, CmpOp, Condition, Expr, ExprContext, ExprError, FieldType, Guard, Literal, Operand, parse_condition,
    parse_expr, parse_field_type, parse_guard, parse_operand,
};
use proptest::prelude::*;

fn plain(text: &str) -> Expr {
    match parse_expr(text, ExprContext::Plain) {
        Ok(e) => e,
        Err(err) => panic!("{text}: {err}"),
    }
}

fn filter(text: &str) -> Expr {
    match parse_expr(text, ExprContext::Filter) {
        Ok(e) => e,
        Err(err) => panic!("{text}: {err}"),
    }
}

fn err(text: &str, context: ExprContext) -> (String, usize) {
    match parse_expr(text, context) {
        Ok(e) => panic!("{text} parsed as {e:?}"),
        Err(ExprError { message, column }) => (message, column),
    }
}

fn ev(f: &str) -> Operand {
    Operand::Event(f.into())
}

fn me(f: &str) -> Operand {
    Operand::SelfField(f.into())
}

fn word(t: &str) -> Operand {
    Operand::Lit(Literal { text: t.into(), quoted: false })
}

fn quoted(t: &str) -> Operand {
    Operand::Lit(Literal::quoted(t))
}

fn cmp(left: Operand, op: CmpOp, right: Operand) -> Expr {
    Expr::Cmp { left, op, right }
}

#[test]
fn every_comparison_operator() {
    for op in CmpOp::ALL {
        let text = format!("event.amount {} 0", op.symbol());
        assert_eq!(plain(&text), cmp(ev("amount"), op, word("0")), "{text}");
    }
}

#[test]
fn operands_of_every_kind() {
    assert_eq!(plain("self.tier == gold"), cmp(me("tier"), CmpOp::Eq, word("gold")));
    assert_eq!(plain("event.a != self.b"), cmp(ev("a"), CmpOp::Ne, me("b")));
    assert_eq!(plain("x == 'two words'"), cmp(word("x"), CmpOp::Eq, quoted("two words")));
    assert_eq!(plain("self.ok == true"), cmp(me("ok"), CmpOp::Eq, word("true")));
    assert_eq!(plain("self.ok == false"), cmp(me("ok"), CmpOp::Eq, word("false")));
    assert_eq!(plain("self.n >= -1.5"), cmp(me("n"), CmpOp::Ge, word("-1.5")));
    assert_eq!(plain("self.v == v1.2"), cmp(me("v"), CmpOp::Eq, word("v1.2")));
    assert_eq!(plain("self.k == \"and\""), cmp(me("k"), CmpOp::Eq, quoted("and")));
}

#[test]
fn bare_names_depend_on_context_and_side() {
    // Plain: a literal on both sides.
    assert_eq!(plain("orderId == event.orderId"), cmp(word("orderId"), CmpOp::Eq, ev("orderId")));
    // Filter: the left bare name is the candidate's field; the right stays a literal.
    assert_eq!(filter("orderId == event.orderId"), cmp(me("orderId"), CmpOp::Eq, ev("orderId")));
    assert_eq!(filter("kind == nightly"), cmp(me("kind"), CmpOp::Eq, word("nightly")));
    assert_eq!(
        filter("kind in [a, b]"),
        Expr::In { operand: me("kind"), set: vec![Literal::new("a"), Literal::new("b")], negated: false }
    );
    // Numbers, booleans and quoted text never become fields.
    assert_eq!(filter("5 < event.n"), cmp(word("5"), CmpOp::Lt, ev("n")));
    assert_eq!(filter("true == event.b"), cmp(word("true"), CmpOp::Eq, ev("b")));
    assert_eq!(filter("'x' == event.b"), cmp(quoted("x"), CmpOp::Eq, ev("b")));
    // `event.x == lit` is allowed in filters (#32).
    assert_eq!(filter("event.kind == refund"), cmp(ev("kind"), CmpOp::Eq, word("refund")));
}

#[test]
fn membership_tests() {
    assert_eq!(
        plain("self.tier in [gold, vip, \"a b\", 3]"),
        Expr::In {
            operand: me("tier"),
            set: vec![Literal::new("gold"), Literal::new("vip"), Literal::quoted("a b"), Literal::new("3")],
            negated: false,
        }
    );
    assert_eq!(plain("event.x not in [a]"), Expr::In { operand: ev("x"), set: vec![Literal::new("a")], negated: true });
}

#[test]
fn boolean_structure_and_precedence() {
    let a = || cmp(me("a"), CmpOp::Eq, word("1"));
    let b = || cmp(me("b"), CmpOp::Eq, word("2"));
    let c = || cmp(me("c"), CmpOp::Eq, word("3"));
    assert_eq!(plain("self.a == 1 and self.b == 2 or self.c == 3"), Expr::Or(vec![Expr::And(vec![a(), b()]), c()]));
    assert_eq!(plain("self.a == 1 or self.b == 2 and self.c == 3"), Expr::Or(vec![a(), Expr::And(vec![b(), c()])]));
    assert_eq!(plain("self.a == 1 and (self.b == 2 or self.c == 3)"), Expr::And(vec![a(), Expr::Or(vec![b(), c()])]));
    assert_eq!(plain("not self.a == 1 and self.b == 2"), Expr::And(vec![a().negate(), b()]));
    assert_eq!(plain("not (self.a == 1 and self.b == 2)"), Expr::And(vec![a(), b()]).negate());
    assert_eq!(plain("not not self.a == 1"), a().negate().negate());
    assert_eq!(plain("self.a == 1 and self.b == 2 and self.c == 3"), Expr::And(vec![a(), b(), c()]));
    // Parentheses keep nesting as written.
    assert_eq!(plain("self.a == 1 and (self.b == 2 and self.c == 3)"), Expr::And(vec![a(), Expr::And(vec![b(), c()])]));
    assert_eq!(plain("((self.a == 1))"), a());
}

#[test]
fn errors_name_the_problem_and_column() {
    use ExprContext::Plain;
    assert_eq!(err("", Plain), ("expected an expression, found end of text".into(), 1));
    assert_eq!(err("   ", Plain), ("expected an expression, found end of text".into(), 4));
    assert_eq!(
        err("self.a", Plain),
        ("expected a comparison (`==`, `!=`, `<`, `<=`, `>`, `>=`) or `in`, found end of text".into(), 7)
    );
    assert_eq!(err("self.a ==", Plain), ("expected an operand, found end of text".into(), 10));
    assert_eq!(err("self.a == and", Plain), ("expected an operand, found `and`".into(), 11));
    assert_eq!(err("self.a == 1 self.b", Plain), ("unexpected `self.b` after the expression".into(), 13));
    assert_eq!(err("(self.a == 1", Plain), ("expected `)`, found end of text".into(), 13));
    assert_eq!(err("self.a == 1 and", Plain), ("expected an operand, found end of text".into(), 16));
    assert_eq!(err("self. == 1", Plain), ("`self.` is not a field reference; expected `self.NAME`".into(), 1));
    assert_eq!(err("event.1x == 1", Plain), ("`event.1x` is not a field reference; expected `event.NAME`".into(), 1));
    assert_eq!(err("self.a in []", Plain), ("expected a literal, found `]`".into(), 12));
    assert_eq!(err("self.a in [event.b]", Plain), ("expected a literal, found `event.b`".into(), 12));
    assert_eq!(err("self.a in [a b]", Plain), ("expected `]`, found `b`".into(), 14));
    assert_eq!(err("self.a in a", Plain), ("expected `[`, found `a`".into(), 11));
    assert_eq!(err("self.a not b", Plain), ("expected `in`, found `b`".into(), 12));
    assert_eq!(
        err("self.a = 1", Plain),
        ("expected a comparison (`==`, `!=`, `<`, `<=`, `>`, `>=`) or `in`, found `=`".into(), 8)
    );
    assert_eq!(err("self.a == 'x", Plain), ("unterminated quoted string".into(), 11));
    assert_eq!(err("amount > 0 @", Plain), ("unexpected character `@`".into(), 12));
    assert_eq!(err("not", Plain), ("expected an operand, found end of text".into(), 4));
}

#[test]
fn error_display_includes_the_column() {
    let e = parse_expr("self.a ==", ExprContext::Plain).err();
    assert_eq!(e.map(|e| e.to_string()), Some("expected an operand, found end of text (column 10)".to_owned()));
}

#[test]
fn guards() {
    assert_eq!(parse_guard("else"), Ok(Guard::Else));
    assert_eq!(parse_guard("  else "), Ok(Guard::Else));
    assert_eq!(parse_guard("?card declined"), Ok(Guard::Choice("card declined".into())));
    assert_eq!(parse_guard("  ?  spaced out  "), Ok(Guard::Choice("spaced out".into())));
    assert_eq!(parse_guard("?a == b and (weird)"), Ok(Guard::Choice("a == b and (weird)".into())));
    assert_eq!(parse_guard("event.amount > 0"), Ok(Guard::Expr(cmp(ev("amount"), CmpOp::Gt, word("0")))));
    assert_eq!(parse_guard("?"), Err(ExprError { message: "a choice needs a label after `?`".into(), column: 2 }));
    assert_eq!(parse_guard("  ?   "), Err(ExprError { message: "a choice needs a label after `?`".into(), column: 4 }));
    // Free text is no longer a guard.
    assert!(parse_guard("amount is positive").is_err());
    assert!(parse_guard("").is_err());
}

#[test]
fn conditions() {
    assert_eq!(parse_condition("?not a gift card"), Ok(Condition::Choice("not a gift card".into())));
    assert_eq!(
        parse_condition("event.policy == sanctioned"),
        Ok(Condition::Expr(cmp(ev("policy"), CmpOp::Eq, word("sanctioned"))))
    );
    assert!(parse_condition("else").is_err(), "else is only a guard");
    assert!(parse_condition("not a gift card").is_err());
}

#[test]
fn assignment_values() {
    assert_eq!(parse_operand("event.originAgent"), Ok(ev("originAgent")));
    assert_eq!(parse_operand("self.channelId"), Ok(me("channelId")));
    assert_eq!(parse_operand("open"), Ok(word("open")));
    assert_eq!(parse_operand("42"), Ok(word("42")));
    assert_eq!(parse_operand("'two words'"), Ok(quoted("two words")));
    assert!(parse_operand("a b").is_err());
    assert!(parse_operand("").is_err());
    assert!(parse_operand("and").is_err());
    assert_eq!(Assignment::new("status", word("open")).to_string(), "status = open");
}

#[test]
fn field_types() {
    assert_eq!(parse_field_type("any"), Ok(FieldType::Any));
    assert_eq!(parse_field_type("string"), Ok(FieldType::String));
    assert_eq!(parse_field_type("number"), Ok(FieldType::Number));
    assert_eq!(parse_field_type("bool"), Ok(FieldType::Bool));
    assert_eq!(parse_field_type("enum [basic, gold]"), Ok(FieldType::Enum(vec!["basic".into(), "gold".into()])));
    assert_eq!(parse_field_type("enum['a b',x]"), Ok(FieldType::Enum(vec!["a b".into(), "x".into()])));
    let e = |text: &str| parse_field_type(text).err().map(|e| (e.message, e.column));
    assert_eq!(e("int"), Some(("unknown type `int`; expected any, string, number, bool or enum [..]".into(), 1)));
    assert_eq!(e(""), Some(("expected a type".into(), 1)));
    assert_eq!(e("enum"), Some(("expected `[` after `enum`".into(), 5)));
    assert_eq!(e("enum []"), Some(("expected an enum member".into(), 7)));
    assert_eq!(e("enum [a, a]"), Some(("duplicate enum member `a`".into(), 10)));
    assert_eq!(e("enum [a b]"), Some(("expected `,` or `]`".into(), 9)));
    assert_eq!(e("number x"), Some(("unexpected `x` after the type".into(), 8)));
    for ty in [
        FieldType::Any,
        FieldType::String,
        FieldType::Number,
        FieldType::Bool,
        FieldType::Enum(vec!["a".into(), "two words".into(), "and".into()]),
    ] {
        assert_eq!(parse_field_type(&ty.to_string()), Ok(ty.clone()), "{ty}");
    }
    assert_eq!(FieldType::Enum(vec!["basic".into(), "gold".into()]).to_string(), "enum [basic, gold]");
}

#[test]
fn canonical_text_examples() {
    for (text, canonical) in [
        ("event.amount>0", "event.amount > 0"),
        ("  self.a==1   and(self.b==2  or self.c==3)", "self.a == 1 and (self.b == 2 or self.c == 3)"),
        ("(self.a == 1 and self.b == 2) or self.c == 3", "self.a == 1 and self.b == 2 or self.c == 3"),
        ("not(self.a == 1)", "not self.a == 1"),
        ("not (self.a == 1 or self.b == 2)", "not (self.a == 1 or self.b == 2)"),
        ("self.k == 'gold'", "self.k == \"gold\""),
    ] {
        assert_eq!(plain(text).to_string(), canonical, "{text}");
    }
    assert_eq!(plain("self.k == 'say \"hi\" \\ bye'").to_string(), "self.k == \"say \\\"hi\\\" \\\\ bye\"");
    assert_eq!(filter("orderId == event.orderId").display_filter().to_string(), "orderId == event.orderId");
    assert_eq!(filter("orderId == event.orderId").to_string(), "self.orderId == event.orderId");
    assert_eq!(
        filter("event.kind == refund and amount > 3").display_filter().to_string(),
        "event.kind == refund and amount > 3"
    );
}

#[test]
fn guard_and_condition_display_round_trip() {
    for text in ["else", "?card declined", "event.amount > 0 and self.tier == gold", "?x"] {
        let guard = parse_guard(text).ok();
        assert_eq!(guard.as_ref().map(ToString::to_string).as_deref(), Some(text));
    }
    for text in ["?not a gift card", "event.policy == sanctioned"] {
        let condition = parse_condition(text).ok();
        assert_eq!(condition.as_ref().map(ToString::to_string).as_deref(), Some(text));
    }
}

// --- Property: display → parse is the identity --------------------------------

fn name() -> impl Strategy<Value = String> {
    "[a-zA-Z_][a-zA-Z0-9_-]{0,6}"
}

fn literal() -> impl Strategy<Value = Literal> {
    prop_oneof![
        name().prop_map(Literal::new),
        (-1000i32..1000).prop_map(|n| Literal::new(n.to_string())),
        (-100i32..100, 0u8..100).prop_map(|(i, f)| Literal::new(format!("{i}.{f}"))),
        Just(Literal::new("true")),
        Just(Literal::new("false")),
        "[ -~]{0,8}".prop_map(Literal::quoted),
        "[a-z ]{0,6}".prop_map(Literal::new),
    ]
}

fn operand() -> impl Strategy<Value = Operand> {
    prop_oneof![name().prop_map(Operand::Event), name().prop_map(Operand::SelfField), literal().prop_map(Operand::Lit),]
}

fn op() -> impl Strategy<Value = CmpOp> {
    prop::sample::select(CmpOp::ALL.to_vec())
}

fn test_expr() -> impl Strategy<Value = Expr> {
    prop_oneof![
        (operand(), op(), operand()).prop_map(|(left, op, right)| Expr::Cmp { left, op, right }),
        (operand(), prop::collection::vec(literal(), 1..4), any::<bool>())
            .prop_map(|(operand, set, negated)| Expr::In { operand, set, negated }),
    ]
}

fn expr() -> impl Strategy<Value = Expr> {
    test_expr().prop_recursive(4, 24, 4, |inner| {
        prop_oneof![
            inner.clone().prop_map(Expr::negate),
            prop::collection::vec(inner.clone(), 2..4).prop_map(Expr::And),
            prop::collection::vec(inner, 2..4).prop_map(Expr::Or),
        ]
    })
}

/// In plain context a bare-name literal on the left reads back as a literal,
/// so no rewriting is needed.
fn plain_round_trip(e: &Expr) -> Result<Expr, ExprError> {
    parse_expr(&e.to_string(), ExprContext::Plain)
}

/// In filter context, a left bare-word literal is printed quoted.
fn quote_left_words(e: &Expr) -> Expr {
    let fix = |o: &Operand| match o {
        Operand::Lit(lit)
            if !lit.quoted && cascade_core::parse::grammar::is_valid_name(&lit.text) && lit.as_bool().is_none() =>
        {
            Operand::Lit(Literal::quoted(lit.text.clone()))
        }
        other => other.clone(),
    };
    match e {
        Expr::Cmp { left, op, right } => Expr::Cmp { left: fix(left), op: *op, right: right.clone() },
        Expr::In { operand, set, negated } => Expr::In { operand: fix(operand), set: set.clone(), negated: *negated },
        Expr::Not(inner) => quote_left_words(inner).negate(),
        Expr::And(items) => Expr::And(items.iter().map(quote_left_words).collect()),
        Expr::Or(items) => Expr::Or(items.iter().map(quote_left_words).collect()),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn display_then_parse_is_identity(e in expr()) {
        prop_assert_eq!(plain_round_trip(&e), Ok(e.clone()), "{}", e);
    }

    #[test]
    fn filter_display_then_parse_is_identity_up_to_left_quoting(e in expr()) {
        let text = e.display_filter().to_string();
        prop_assert_eq!(parse_expr(&text, ExprContext::Filter), Ok(quote_left_words(&e)), "{}", text);
    }

    #[test]
    fn canonical_text_is_a_fixed_point(e in expr()) {
        let once = e.to_string();
        let twice = parse_expr(&once, ExprContext::Plain).map(|p| p.to_string());
        prop_assert_eq!(twice, Ok(once));
    }

    #[test]
    fn guards_round_trip(label in "[ -~]{1,12}", e in expr(), pick in 0u8..3) {
        let guard = match pick {
            0 => Guard::Else,
            1 => Guard::Choice(label.trim().to_owned()),
            _ => Guard::Expr(e),
        };
        prop_assume!(!matches!(&guard, Guard::Choice(l) if l.is_empty()));
        prop_assert_eq!(parse_guard(&guard.to_string()), Ok(guard.clone()));
    }

    #[test]
    fn parsing_never_panics(text in "[ -~]{0,40}") {
        let _ = parse_expr(&text, ExprContext::Plain);
        let _ = parse_expr(&text, ExprContext::Filter);
        let _ = parse_guard(&text);
        let _ = parse_field_type(&text);
        let _ = cascade_core::expr::parse_duration(&text);
    }
}
