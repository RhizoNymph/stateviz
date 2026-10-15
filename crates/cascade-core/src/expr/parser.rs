//! Recursive-descent parser for expressions, guards, conditions and
//! assignment values.
//!
//! ```text
//! expr     := or
//! or       := and ("or" and)*
//! and      := unary ("and" unary)*
//! unary    := "not" unary | "(" expr ")" | test
//! test     := operand cmp operand
//!           | operand ["not"] "in" "[" literal ("," literal)* "]"
//! cmp      := "==" | "!=" | "<" | "<=" | ">" | ">="
//! operand  := "event." NAME | "self." NAME | NAME | literal
//! literal  := NUMBER | QUOTED | WORD | "true" | "false"
//! ```

use super::error::ExprError;
use super::lexer::{Tok, Token, tokenize};
use super::types::{Condition, Expr, Guard, Literal, Operand};
use crate::parse::grammar::is_valid_name;

/// What a bare `NAME` on the left of a test means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExprContext {
    /// Guards and conditions: a bare name is a literal word everywhere.
    Plain,
    /// Selector and subscription filters: a bare name on the left of a test
    /// is `self.NAME` (the candidate instance's field).
    Filter,
}

/// A cursor over tokens, shared with the target selector grammar.
pub(crate) struct Parser<'t> {
    tokens: &'t [Token],
    pos: usize,
    /// Column reported for "end of text".
    end_col: usize,
    context: ExprContext,
}

impl<'t> Parser<'t> {
    pub fn new(tokens: &'t [Token], text: &str, context: ExprContext) -> Self {
        Self { tokens, pos: 0, end_col: text.chars().count() + 1, context }
    }

    pub fn peek(&self) -> Option<&'t Tok> {
        self.tokens.get(self.pos).map(|t| &t.tok)
    }

    pub fn next(&mut self) -> Option<&'t Token> {
        let token = self.tokens.get(self.pos);
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    /// The column of the next token, or of the end of text.
    pub fn col(&self) -> usize {
        self.tokens.get(self.pos).map_or(self.end_col, |t| t.col)
    }

    /// "expected `what`, found …" at the next token.
    pub fn expected(&self, what: &str) -> ExprError {
        let found = self.peek().map_or_else(|| "end of text".to_owned(), Tok::describe);
        ExprError::new(format!("expected {what}, found {found}"), self.col())
    }

    pub fn eat_word(&mut self, word: &str) -> bool {
        if self.peek().is_some_and(|t| t.is_word(word)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    pub fn eat(&mut self, tok: &Tok) -> bool {
        if self.peek() == Some(tok) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    pub fn expect(&mut self, tok: &Tok) -> Result<(), ExprError> {
        if self.eat(tok) { Ok(()) } else { Err(self.expected(&tok.describe())) }
    }

    /// Fail unless every token was consumed.
    pub fn finish(&self) -> Result<(), ExprError> {
        match self.tokens.get(self.pos) {
            None => Ok(()),
            Some(t) => Err(ExprError::new(format!("unexpected {} after the expression", t.tok.describe()), t.col)),
        }
    }

    pub fn expr(&mut self) -> Result<Expr, ExprError> {
        let first = self.and()?;
        let mut items = vec![first];
        while self.eat_word("or") {
            items.push(self.and()?);
        }
        Ok(if items.len() == 1 { items.remove(0) } else { Expr::Or(items) })
    }

    fn and(&mut self) -> Result<Expr, ExprError> {
        let first = self.unary()?;
        let mut items = vec![first];
        while self.eat_word("and") {
            items.push(self.unary()?);
        }
        Ok(if items.len() == 1 { items.remove(0) } else { Expr::And(items) })
    }

    fn unary(&mut self) -> Result<Expr, ExprError> {
        if self.eat_word("not") {
            return Ok(self.unary()?.negate());
        }
        if self.eat(&Tok::LParen) {
            let inner = self.expr()?;
            self.expect(&Tok::RParen)?;
            return Ok(inner);
        }
        self.test()
    }

    fn test(&mut self) -> Result<Expr, ExprError> {
        let left = self.operand(true)?;
        match self.peek() {
            Some(Tok::Cmp(op)) => {
                let op = *op;
                self.pos += 1;
                let right = self.operand(false)?;
                Ok(Expr::Cmp { left, op, right })
            }
            Some(t) if t.is_word("in") || t.is_word("not") => {
                let negated = self.eat_word("not");
                if !self.eat_word("in") {
                    return Err(self.expected("`in`"));
                }
                let set = self.literal_list()?;
                Ok(Expr::In { operand: left, set, negated })
            }
            _ => Err(self.expected("a comparison (`==`, `!=`, `<`, `<=`, `>`, `>=`) or `in`")),
        }
    }

    fn literal_list(&mut self) -> Result<Vec<Literal>, ExprError> {
        self.expect(&Tok::LBracket)?;
        let mut set = vec![self.literal()?];
        while self.eat(&Tok::Comma) {
            set.push(self.literal()?);
        }
        self.expect(&Tok::RBracket)?;
        Ok(set)
    }

    /// A literal (for `in` lists).
    pub fn literal(&mut self) -> Result<Literal, ExprError> {
        let col = self.col();
        match self.peek() {
            Some(Tok::Quoted(q)) => {
                self.pos += 1;
                Ok(Literal::quoted(q.clone()))
            }
            Some(Tok::Word(w)) => match classify(w, col)? {
                Word::Literal(lit) => {
                    self.pos += 1;
                    Ok(lit)
                }
                Word::Event(_) | Word::SelfField(_) | Word::Keyword => Err(self.expected("a literal")),
            },
            _ => Err(self.expected("a literal")),
        }
    }

    /// An operand. `left` says whether it is the left side of a test, where
    /// a bare name means `self.NAME` in [`ExprContext::Filter`].
    pub fn operand(&mut self, left: bool) -> Result<Operand, ExprError> {
        let col = self.col();
        match self.peek() {
            Some(Tok::Quoted(q)) => {
                self.pos += 1;
                Ok(Operand::Lit(Literal::quoted(q.clone())))
            }
            Some(Tok::Word(w)) => {
                let operand = match classify(w, col)? {
                    Word::Event(f) => Operand::Event(f),
                    Word::SelfField(f) => Operand::SelfField(f),
                    Word::Literal(lit)
                        if left
                            && self.context == ExprContext::Filter
                            && !lit.quoted
                            && is_valid_name(&lit.text)
                            && lit.as_bool().is_none() =>
                    {
                        Operand::SelfField(lit.text)
                    }
                    Word::Literal(lit) => Operand::Lit(lit),
                    Word::Keyword => return Err(self.expected("an operand")),
                };
                self.pos += 1;
                Ok(operand)
            }
            _ => Err(self.expected("an operand")),
        }
    }
}

enum Word {
    Event(String),
    SelfField(String),
    Literal(Literal),
    /// `and`, `or`, `not`, `in`.
    Keyword,
}

fn classify(word: &str, col: usize) -> Result<Word, ExprError> {
    for (prefix, make) in [("event.", Word::Event as fn(String) -> Word), ("self.", Word::SelfField)] {
        if let Some(field) = word.strip_prefix(prefix) {
            return if is_valid_name(field) {
                Ok(make(field.to_owned()))
            } else {
                Err(ExprError::new(format!("`{word}` is not a field reference; expected `{prefix}NAME`"), col))
            };
        }
    }
    Ok(match word {
        "and" | "or" | "not" | "in" => Word::Keyword,
        _ => Word::Literal(Literal { text: word.to_owned(), quoted: false }),
    })
}

/// Parse an expression. `context` says what a bare name on the left of a
/// test means.
pub fn parse_expr(text: &str, context: ExprContext) -> Result<Expr, ExprError> {
    let tokens = tokenize(text)?;
    let mut parser = Parser::new(&tokens, text, context);
    if parser.peek().is_none() {
        return Err(ExprError::new("expected an expression, found end of text", parser.col()));
    }
    let expr = parser.expr()?;
    parser.finish()?;
    Ok(expr)
}

/// A choice label after `?`: trimmed, non-empty. `None` when `text` is not
/// a choice.
fn choice(text: &str) -> Option<Result<String, ExprError>> {
    let leading = text.chars().take_while(|c| c.is_whitespace()).count();
    let rest = text.trim_start().strip_prefix('?')?;
    let label = rest.trim();
    Some(if label.is_empty() {
        Err(ExprError::new("a choice needs a label after `?`", leading + 2))
    } else {
        Ok(label.to_owned())
    })
}

/// Parse a transition guard: `else`, `?label` or an expression (bare names
/// are literals; guards write `self.x`).
pub fn parse_guard(text: &str) -> Result<Guard, ExprError> {
    if text.trim() == "else" {
        return Ok(Guard::Else);
    }
    if let Some(label) = choice(text) {
        return label.map(Guard::Choice);
    }
    parse_expr(text, ExprContext::Plain).map(Guard::Expr)
}

/// Parse a rule condition: `?label` or an expression.
pub fn parse_condition(text: &str) -> Result<Condition, ExprError> {
    if let Some(label) = choice(text) {
        return label.map(Condition::Choice);
    }
    parse_expr(text, ExprContext::Plain).map(Condition::Expr)
}

/// Parse the value of an assignment (`sets:`, `with:`): `event.x`, `self.x`
/// or a literal (a bare name is a literal).
pub fn parse_operand(text: &str) -> Result<Operand, ExprError> {
    let tokens = tokenize(text)?;
    let mut parser = Parser::new(&tokens, text, ExprContext::Plain);
    let operand = parser.operand(false)?;
    parser.finish()?;
    Ok(operand)
}
