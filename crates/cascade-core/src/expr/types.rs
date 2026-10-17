//! The expression language's syntax tree: operands, literals, comparisons,
//! boolean structure, guards, rule conditions and assignments.

use serde::{Deserialize, Serialize};

/// A literal as written: `gold`, `42`, `true`, `"two words"`.
///
/// `quoted` records that it was written in quotes, which makes it a string
/// for type checking (a quoted `"42"` is not a number) and keeps it quoted in
/// canonical text. An unquoted literal's text is always a bare word, number
/// or `true`/`false`: build literals with [`Literal::new`] (quoted only when
/// the text needs it) or [`Literal::quoted`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Literal {
    pub text: String,
    pub quoted: bool,
}

impl Literal {
    /// A literal written bare when its text allows it, quoted otherwise.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let quoted = super::lexer::needs_quotes(&text);
        Self { text, quoted }
    }

    /// A literal written in quotes (always a string).
    pub fn quoted(text: impl Into<String>) -> Self {
        Self { text: text.into(), quoted: true }
    }

    /// The literal's text as a number, when it is an unquoted decimal.
    pub fn as_number(&self) -> Option<f64> {
        if self.quoted { None } else { super::lexer::parse_number(&self.text) }
    }

    /// The literal as a bool, when it is an unquoted `true` or `false`.
    pub fn as_bool(&self) -> Option<bool> {
        match (self.quoted, self.text.as_str()) {
            (false, "true") => Some(true),
            (false, "false") => Some(false),
            _ => None,
        }
    }
}

/// One side of a comparison.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Operand {
    /// `event.x`: a field of the payload in scope.
    Event(String),
    /// `self.x`: a field of the instance in scope.
    SelfField(String),
    /// A literal value.
    Lit(Literal),
}

impl Operand {
    /// The field this operand reads, if it is not a literal.
    pub fn field(&self) -> Option<&str> {
        match self {
            Operand::Event(f) | Operand::SelfField(f) => Some(f),
            Operand::Lit(_) => None,
        }
    }

    pub fn is_literal(&self) -> bool {
        matches!(self, Operand::Lit(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub const ALL: [CmpOp; 6] = [CmpOp::Eq, CmpOp::Ne, CmpOp::Lt, CmpOp::Le, CmpOp::Gt, CmpOp::Ge];

    pub const fn symbol(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }

    /// Whether the operator orders its operands (`<`, `<=`, `>`, `>=`).
    pub const fn is_ordering(self) -> bool {
        matches!(self, CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge)
    }

    /// The operator with its operands swapped: `a < b` ⇔ `b > a`.
    pub const fn flipped(self) -> CmpOp {
        match self {
            CmpOp::Eq => CmpOp::Eq,
            CmpOp::Ne => CmpOp::Ne,
            CmpOp::Lt => CmpOp::Gt,
            CmpOp::Le => CmpOp::Ge,
            CmpOp::Gt => CmpOp::Lt,
            CmpOp::Ge => CmpOp::Le,
        }
    }
}

/// A boolean expression over operands.
///
/// `And` and `Or` hold at least two items when built by the parser or the
/// [`Expr::and`] / [`Expr::or`] constructors. Nesting is kept as written
/// (`a and (b and c)` is an `And` holding an `And`), so display and parse
/// are inverse.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Expr {
    Cmp { left: Operand, op: CmpOp, right: Operand },
    In { operand: Operand, set: Vec<Literal>, negated: bool },
    Not(Box<Expr>),
    And(Vec<Expr>),
    Or(Vec<Expr>),
}

impl Expr {
    pub fn cmp(left: Operand, op: CmpOp, right: Operand) -> Self {
        Expr::Cmp { left, op, right }
    }

    /// `items` joined by `and`; a single item is returned as is. `None` for
    /// no items.
    pub fn and(mut items: Vec<Expr>) -> Option<Self> {
        match items.len() {
            0 => None,
            1 => items.pop(),
            _ => Some(Expr::And(items)),
        }
    }

    /// `items` joined by `or`; a single item is returned as is. `None` for
    /// no items.
    pub fn or(mut items: Vec<Expr>) -> Option<Self> {
        match items.len() {
            0 => None,
            1 => items.pop(),
            _ => Some(Expr::Or(items)),
        }
    }

    pub fn negate(self) -> Self {
        Expr::Not(Box::new(self))
    }

    /// Every operand in the expression, in text order (an `in` test yields
    /// only its tested operand).
    pub fn operands(&self) -> Vec<&Operand> {
        let mut out = Vec::new();
        self.collect_operands(&mut out);
        out
    }

    fn collect_operands<'a>(&'a self, out: &mut Vec<&'a Operand>) {
        match self {
            Expr::Cmp { left, right, .. } => {
                out.push(left);
                out.push(right);
            }
            Expr::In { operand, .. } => out.push(operand),
            Expr::Not(inner) => inner.collect_operands(out),
            Expr::And(items) | Expr::Or(items) => items.iter().for_each(|e| e.collect_operands(out)),
        }
    }

    /// Mutable access to every operand, for renames.
    pub fn operands_mut(&mut self) -> Vec<&mut Operand> {
        let mut out = Vec::new();
        self.collect_operands_mut(&mut out);
        out
    }

    fn collect_operands_mut<'a>(&'a mut self, out: &mut Vec<&'a mut Operand>) {
        match self {
            Expr::Cmp { left, right, .. } => {
                out.push(left);
                out.push(right);
            }
            Expr::In { operand, .. } => out.push(operand),
            Expr::Not(inner) => inner.collect_operands_mut(out),
            Expr::And(items) | Expr::Or(items) => items.iter_mut().for_each(|e| e.collect_operands_mut(out)),
        }
    }

    /// Every `self.x` field read, in text order, with repeats.
    pub fn self_fields(&self) -> Vec<&str> {
        self.operands()
            .into_iter()
            .filter_map(|o| match o {
                Operand::SelfField(f) => Some(f.as_str()),
                Operand::Event(_) | Operand::Lit(_) => None,
            })
            .collect()
    }

    /// Every `event.x` field read, in text order, with repeats.
    pub fn event_fields(&self) -> Vec<&str> {
        self.operands()
            .into_iter()
            .filter_map(|o| match o {
                Operand::Event(f) => Some(f.as_str()),
                Operand::SelfField(_) | Operand::Lit(_) => None,
            })
            .collect()
    }
}

/// A transition guard.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Guard {
    /// Decided by the data: holds when the expression is true.
    Expr(Expr),
    /// A named decision the data does not carry (`?card declined`).
    Choice(String),
    /// The fallback among a trigger's candidates.
    Else,
}

impl Guard {
    pub fn expr(&self) -> Option<&Expr> {
        match self {
            Guard::Expr(e) => Some(e),
            Guard::Choice(_) | Guard::Else => None,
        }
    }
}

/// A rule's `when:` condition.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Condition {
    Expr(Expr),
    Choice(String),
}

impl Condition {
    pub fn expr(&self) -> Option<&Expr> {
        match self {
            Condition::Expr(e) => Some(e),
            Condition::Choice(_) => None,
        }
    }
}

/// `field = value`: a spawn assignment, a transition's `sets:` entry or an
/// emit's `with:` entry.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Assignment {
    pub field: String,
    pub value: Operand,
}

impl Assignment {
    pub fn new(field: impl Into<String>, value: Operand) -> Self {
        Self { field: field.into(), value }
    }
}
