# Expressions

The small typed language of transition guards, rule conditions, selector and
subscription filters, and assignments (`sets:`, emit `with:`, spawn
`with`), plus declared field types and durations. Part of the semantics
phase; the cross-workstream contract is
[semantics-contracts.md](semantics-contracts.md).

## Scope

- Syntax: the grammar, parse errors with columns, canonical text
  (`Display`) that parses back to an equal value.
- Guards (`Guard`: expression, `?choice`, `else`) and conditions
  (`Condition`: expression, `?choice`).
- Field types (`any`, `string`, `number`, `bool`, `enum [..]`), parsing and
  display; declarations (`FieldDecl`).
- Evaluation against string-valued fields (`eval`, `value`, `compare`).
- Static reasoning: `overlap` (can two expressions both hold?) and
  `coverage` (does one of several always hold?).
- Durations (`30s`, `2m`, …) for timers and the virtual clock.
- Type checking of expressions in a definition (in the resolver, see
  `crates/cascade-core/src/resolve/typing.rs`).

## Non-scope

- Arithmetic, string functions, wall-clock time.
- Deciding choices, applying `sets`/`with`, subscriptions and timers at run
  time (the simulator, wave 2).
- Using `overlap`/`coverage` in the checks (static analysis, wave 2).
- Writing expressions to YAML (cascade-interop prints the canonical text).

## Grammar

```text
expr     := or
or       := and ("or" and)*
and      := unary ("and" unary)*
unary    := "not" unary | "(" expr ")" | test
test     := operand cmp operand
          | operand ["not"] "in" "[" literal ("," literal)* "]"
cmp      := "==" | "!=" | "<" | "<=" | ">" | ">="
operand  := "event." NAME | "self." NAME | NAME | literal
literal  := NUMBER | QUOTED | WORD | "true" | "false"
guard    := "else" | "?" LABEL | expr
condition:= "?" LABEL | expr
type     := "any" | "string" | "number" | "bool" | "enum" "[" member ("," member)* "]"
duration := INTEGER ("ms" | "s" | "m" | "h" | "d")
```

- Tokens: words are letters, digits, `_`, `-` and `.` (so `event.x`, `-1.5`
  and `v1.2` are one token); quoted strings use `"…"` (with `\"` and `\\`
  escapes) or `'…'` (no escapes); `=` alone is the selector assignment.
- NUMBER is a decimal (`42`, `-1.5`; no exponent, no leading `+` or `.`).
- A bare NAME on the left of a test is `self.NAME` in
  `ExprContext::Filter` (selector `where` and subscription `where`) and a
  literal word in `ExprContext::Plain` (guards, conditions). On the right it
  is always a literal.
- `and`, `or`, `not`, `in` are reserved; quote them to use them as text.
  `true`/`false` are bool literals.
- A choice label is the trimmed text after `?`, non-empty, any characters.

Canonical text: single spaces, minimal parentheses (an `and` inside an
`and`, or an `or` inside an `and`/`or`, keeps its parentheses so nesting
survives), literals quoted when they were written quoted or must be. In
filter context (`Expr::display_filter`) a left `self.x` prints as `x`, and a
left bare-word literal prints quoted.

## Semantics

Values are strings; types say how they are read:

| Type | Accepts | `==`/`!=`/`in` | Ordering |
| --- | --- | --- | --- |
| `any` | anything | text | numbers when both sides are numbers, else text |
| `string` | anything | text | text |
| `number` | decimals | numeric (`5 == 5.0`) | numeric, total order |
| `bool` | `true`/`false` | by value | false |
| `enum [..]` | members | by value | false |

- A test's type is the declared non-`any` type of either field operand (left
  first), else `any`.
- A test with a missing operand is false (`!=` and `not in` included); a
  value that does not read as its type makes the test false.
  `not (event.x == a)` is true when `x` is missing.

## Data and control flow

```text
YAML string ──parse::yaml──▶ parse_guard / parse_condition / parse_expr(Filter)
            / parse_operand / parse_field_type / parse_duration ──▶ Definition
Definition ──resolve (typing.rs)──▶ Model (Transition.guard/filter/sets/emits,
                                     Rule.condition, Target.filter/assignments)
Model::types_for(machine, event) ──▶ Types
eval(expr, &MapScope{event, self_fields}, &types) ──▶ bool   (cascade-sim)
overlap / coverage(exprs, &types) ──▶ Overlap / Coverage      (analysis, wave 2)
```

`solve` normalizes to DNF (bounded by `MAX_CONJUNCTIONS` = 256; past it
`MayOverlap`/`Unknown`). Each conjunction is decided one field at a time
(tests comparing two fields are set aside as unknown). For a field it tries
candidate values with `eval` itself: enum members, both bools, numeric
literals with ±1 and their `next_up`/`next_down` neighbours, text literals,
`""` and each literal's successor `t + "\0"`, and "missing". The candidates
are complete (if any value satisfies the tests, one of them does) except for
an `any` field ordered against a word, which reports unknown. `coverage`
negates the expressions and reports up to `MAX_EXAMPLES` readable gaps
(`self.tier = vip`, `event.amount is missing`).

Resolver typing (`resolve/typing.rs`): `self.x` must be declared when the
machine declares fields, and is `SelfNotAllowed` in rule conditions and spawn
assignments; `event.x` must be declared when the event in scope is known and
declares a payload (a command trigger's guard has an open event: `any`).
Tests need compatible types (`TypeMismatch`), enum literals must be members
(`NotAMember`), ordering needs numbers or `any`. Assignments must fit the
target field's type. A test that reads no field (two literals compared, or
`in` on a literal) is `ConstantTest { test, what, suggestion }`: since a bare
word is a literal outside a filter's left-hand side, `x > 0` in a guard would
silently be constant, so the message suggests `self.x` or `event.x` (only
`event.x` where `self` is not available).

## Files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-core/src/expr/mod.rs` | Module root, re-exports | — |
| `crates/cascade-core/src/expr/types.rs` | Syntax tree | `Expr`, `Operand`, `Literal`, `CmpOp`, `Guard`, `Condition`, `Assignment` |
| `crates/cascade-core/src/expr/lexer.rs` | Tokens with columns, numbers, quoting | `KEYWORDS`, `parse_number`, `needs_quotes` |
| `crates/cascade-core/src/expr/parser.rs` | Recursive descent (shared with selectors) | `ExprContext`, `parse_expr`, `parse_guard`, `parse_condition`, `parse_operand` |
| `crates/cascade-core/src/expr/display.rs` | Canonical text | `Display` impls, `Expr::display_filter`, `Expr::display_in` |
| `crates/cascade-core/src/expr/fields.rs` | Field types | `FieldType`, `FieldDecl`, `parse_field_type` |
| `crates/cascade-core/src/expr/eval.rs` | Evaluation | `Scope`, `MapScope`, `Types`, `eval`, `value`, `compare`, `literal_fits` |
| `crates/cascade-core/src/expr/solve.rs` | Exclusivity and coverage | `overlap`, `Overlap`, `coverage`, `Coverage`, `MAX_CONJUNCTIONS`, `MAX_EXAMPLES` |
| `crates/cascade-core/src/expr/duration.rs` | Durations | `Duration`, `parse_duration` |
| `crates/cascade-core/src/expr/error.rs` | Parse errors | `ExprError { message, column }` |
| `crates/cascade-core/src/resolve/typing.rs` | Type checking in the resolver | (crate-private) |
| `crates/cascade-core/tests/expr_parse.rs` | Every production, errors with columns, display round trips (property tests) | — |
| `crates/cascade-core/tests/expr_eval.rs` | Missing operands, every type, `in`, `not` | — |
| `crates/cascade-core/tests/semantics_resolve.rs` | Definition format and resolver diagnostics | — |

## Invariants and constraints

- `parse_expr(e.to_string(), Plain) == Ok(e)` for every `Expr` whose `and`/
  `or` lists hold at least two items and whose unquoted literals are bare
  words (true for everything the parser produces; property-tested).
  `display_filter` round-trips in filter context up to quoting of left
  bare-word literals.
- Canonical text is a fixed point: printing a parsed canonical text gives
  the same text.
- No free text: every guard is an expression, `?label` or `else`; every
  condition an expression or `?label`.
- `eval` never panics and never reads a field it was not given; tests on
  missing values are false.
- `overlap` returns `Exclusive` only when no scope satisfies both;
  `coverage` returns `Complete` only when every scope satisfies one (sound,
  incomplete).
- `Duration` displays the largest exact unit and parses back to itself; a
  timer delay must be non-zero (resolver).
