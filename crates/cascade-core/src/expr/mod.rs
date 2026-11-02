//! The expression language of guards, rule conditions, selector and
//! subscription filters, and assignments; declared field types; durations.
//!
//! ```text
//! text ──parse_expr / parse_guard / parse_condition──▶ Expr / Guard / Condition
//!       (Display prints canonical text that parses back to an equal value)
//! Expr + Scope (string values) + Types (declarations) ──eval──▶ bool
//! Expr × Expr ──overlap──▶ Exclusive | MayOverlap
//! [Expr] ──coverage──▶ Complete | Gaps(examples) | Unknown
//! ```
//!
//! See `docs/features/expressions.md` for the grammar and semantics.

mod display;
mod duration;
mod error;
mod eval;
mod fields;
pub(crate) mod lexer;
pub(crate) mod parser;
mod solve;
mod types;

pub use duration::{Duration, parse_duration};
pub use error::ExprError;
pub use eval::{MapScope, Scope, Types, compare, eval, literal_fits, value};
pub use fields::{FieldDecl, FieldType, parse_field_type};
pub use lexer::{KEYWORDS, needs_quotes, parse_number};
pub use parser::{ExprContext, parse_condition, parse_expr, parse_guard, parse_operand};
pub use solve::{Coverage, MAX_CONJUNCTIONS, MAX_EXAMPLES, Overlap, coverage, overlap};
pub use types::{Assignment, CmpOp, Condition, Expr, Guard, Literal, Operand};
