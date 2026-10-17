//! Tokens of the expression language (also used by target selectors), with
//! 1-based character columns for error messages.

use super::error::ExprError;
use super::types::CmpOp;

/// Reserved words; quote them to use them as literals.
pub const KEYWORDS: [&str; 6] = ["and", "or", "not", "in", "true", "false"];

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Tok {
    /// Letters, digits, `_`, `-` and `.`: names, `event.x`, numbers, words.
    Word(String),
    /// A quoted string, quotes removed and escapes applied.
    Quoted(String),
    Cmp(CmpOp),
    /// A single `=` (selector assignments).
    Assign,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
}

impl Tok {
    pub fn describe(&self) -> String {
        match self {
            Tok::Word(w) => format!("`{w}`"),
            Tok::Quoted(q) => format!("{q:?}"),
            Tok::Cmp(op) => format!("`{}`", op.symbol()),
            Tok::Assign => "`=`".to_owned(),
            Tok::LParen => "`(`".to_owned(),
            Tok::RParen => "`)`".to_owned(),
            Tok::LBracket => "`[`".to_owned(),
            Tok::RBracket => "`]`".to_owned(),
            Tok::Comma => "`,`".to_owned(),
        }
    }

    pub fn is_word(&self, word: &str) -> bool {
        matches!(self, Tok::Word(w) if w == word)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Token {
    pub tok: Tok,
    /// 1-based character column of the token's first character.
    pub col: usize,
}

pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.')
}

pub(crate) fn tokenize(text: &str) -> Result<Vec<Token>, ExprError> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().enumerate().peekable();
    while let Some(&(i, c)) = chars.peek() {
        let col = i + 1;
        let single = |tok| Token { tok, col };
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '(' | ')' | '[' | ']' | ',' => {
                chars.next();
                tokens.push(single(match c {
                    '(' => Tok::LParen,
                    ')' => Tok::RParen,
                    '[' => Tok::LBracket,
                    ']' => Tok::RBracket,
                    _ => Tok::Comma,
                }));
            }
            '=' | '!' | '<' | '>' => {
                chars.next();
                let eq = chars.next_if(|&(_, n)| n == '=').is_some();
                let tok = match (c, eq) {
                    ('=', true) => Tok::Cmp(CmpOp::Eq),
                    ('=', false) => Tok::Assign,
                    ('!', true) => Tok::Cmp(CmpOp::Ne),
                    ('!', false) => {
                        return Err(ExprError::new("`!` must be followed by `=` (use `not` to negate)", col));
                    }
                    ('<', true) => Tok::Cmp(CmpOp::Le),
                    ('<', false) => Tok::Cmp(CmpOp::Lt),
                    ('>', true) => Tok::Cmp(CmpOp::Ge),
                    _ => Tok::Cmp(CmpOp::Gt),
                };
                tokens.push(single(tok));
            }
            '"' | '\'' => {
                chars.next();
                let mut value = String::new();
                let mut closed = false;
                while let Some((j, next)) = chars.next() {
                    if next == c {
                        closed = true;
                        break;
                    }
                    if next == '\\' && c == '"' {
                        match chars.next() {
                            Some((_, escaped @ ('"' | '\\'))) => value.push(escaped),
                            Some((k, other)) => {
                                return Err(ExprError::new(format!("unknown escape `\\{other}`"), k + 1));
                            }
                            None => return Err(ExprError::new("unterminated escape", j + 1)),
                        }
                    } else {
                        value.push(next);
                    }
                }
                if !closed {
                    return Err(ExprError::new("unterminated quoted string", col));
                }
                tokens.push(single(Tok::Quoted(value)));
            }
            c if is_word_char(c) => {
                let mut word = String::new();
                while let Some((_, next)) = chars.next_if(|&(_, n)| is_word_char(n)) {
                    word.push(next);
                }
                tokens.push(single(Tok::Word(word)));
            }
            other => return Err(ExprError::new(format!("unexpected character `{other}`"), col)),
        }
    }
    Ok(tokens)
}

/// A decimal number: `42`, `-1.5`, `0.25`. No exponents, no leading `+`,
/// no bare `.5`.
pub fn parse_number(text: &str) -> Option<f64> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let (int, frac) = match digits.split_once('.') {
        Some((int, frac)) => (int, Some(frac)),
        None => (digits, None),
    };
    let all_digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    if !all_digits(int) || frac.is_some_and(|f| !all_digits(f)) {
        return None;
    }
    text.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Whether `text` must be quoted to read back as the same literal: empty,
/// characters outside a word, an operator keyword (`and`, `or`, `not`,
/// `in`; `true`/`false` are literals already), or a field reference.
pub fn needs_quotes(text: &str) -> bool {
    text.is_empty()
        || !text.chars().all(is_word_char)
        || matches!(text, "and" | "or" | "not" | "in")
        || text.starts_with("event.")
        || text.starts_with("self.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(text: &str) -> Vec<Tok> {
        match tokenize(text) {
            Ok(tokens) => tokens.into_iter().map(|t| t.tok).collect(),
            Err(err) => panic!("{text}: {err}"),
        }
    }

    #[test]
    fn every_token_kind() {
        assert_eq!(
            toks("a==b != c<d<=e>f>=g = ( ) [ ] , 'x y' \"q\\\"z\\\\\""),
            vec![
                Tok::Word("a".into()),
                Tok::Cmp(CmpOp::Eq),
                Tok::Word("b".into()),
                Tok::Cmp(CmpOp::Ne),
                Tok::Word("c".into()),
                Tok::Cmp(CmpOp::Lt),
                Tok::Word("d".into()),
                Tok::Cmp(CmpOp::Le),
                Tok::Word("e".into()),
                Tok::Cmp(CmpOp::Gt),
                Tok::Word("f".into()),
                Tok::Cmp(CmpOp::Ge),
                Tok::Word("g".into()),
                Tok::Assign,
                Tok::LParen,
                Tok::RParen,
                Tok::LBracket,
                Tok::RBracket,
                Tok::Comma,
                Tok::Quoted("x y".into()),
                Tok::Quoted("q\"z\\".into()),
            ]
        );
    }

    #[test]
    fn words_hold_dots_and_dashes() {
        assert_eq!(
            toks("event.orderId -1.5 a-b"),
            vec![Tok::Word("event.orderId".into()), Tok::Word("-1.5".into()), Tok::Word("a-b".into())]
        );
    }

    #[test]
    fn columns_are_one_based_characters() {
        let tokens = tokenize("é == 'x'").unwrap_or_default();
        let cols: Vec<usize> = tokens.iter().map(|t| t.col).collect();
        assert_eq!(cols, [1, 3, 6]);
    }

    #[test]
    fn errors_carry_columns() {
        let err = |text: &str| tokenize(text).err().map(|e| (e.message, e.column));
        assert_eq!(err("a ! b"), Some(("`!` must be followed by `=` (use `not` to negate)".into(), 3)));
        assert_eq!(err("a == 'open"), Some(("unterminated quoted string".into(), 6)));
        assert_eq!(err("a == @"), Some(("unexpected character `@`".into(), 6)));
        assert_eq!(err("a == \"x\\n\""), Some(("unknown escape `\\n`".into(), 9)));
    }

    #[test]
    fn numbers() {
        assert_eq!(parse_number("42"), Some(42.0));
        assert_eq!(parse_number("-1.5"), Some(-1.5));
        assert_eq!(parse_number("0.25"), Some(0.25));
        for bad in ["", "-", "1.", ".5", "1e5", "+1", "1.2.3", "abc", "1_000", "NaN", "inf"] {
            assert_eq!(parse_number(bad), None, "{bad}");
        }
    }

    #[test]
    fn quoting() {
        for bare in ["gold", "42", "-1.5", "a-b", "v1.2", "_x"] {
            assert!(!needs_quotes(bare), "{bare}");
        }
        assert!(!needs_quotes("true"), "a bool literal");
        for quoted in ["", "two words", "and", "in", "event.x", "self.y", "a=b", "x'"] {
            assert!(needs_quotes(quoted), "{quoted}");
        }
    }
}
