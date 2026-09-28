//! Scalars: new ones plain when safe, changed ones in their old style.

use crate::patch::doc::{Scalar, ScalarStyle};
use crate::patch::error::{Result, Unsupported};
use crate::yaml::quote::{Ctx, double_quoted, scalar};

/// `value` plain when that reads back the same, else double-quoted.
pub(crate) fn plain_or_quoted(value: &str, in_flow: bool) -> String {
    scalar(value, if in_flow { Ctx::Flow } else { Ctx::Block }).into_owned()
}

/// Free text (guards, conditions): double-quoted when the file quotes its
/// free text, else plain when that reads back the same.
pub(crate) fn free_text(value: &str, in_flow: bool, quote: bool) -> String {
    if quote { double_quoted(value) } else { plain_or_quoted(value, in_flow) }
}

/// `[a, b]` with each item quoted only when needed.
pub(crate) fn flow_list<S: AsRef<str>>(items: &[S]) -> String {
    let items: Vec<String> = items.iter().map(|s| plain_or_quoted(s.as_ref(), true)).collect();
    format!("[{}]", items.join(", "))
}

/// The text for `value` replacing `old`, in `old`'s quoting style: plain
/// stays plain when it can, quoted stays quoted with the same quotes.
pub(crate) fn restyle(old: &Scalar, value: &str) -> Result<String> {
    let ctx = if old.in_flow { Ctx::Flow } else { Ctx::Block };
    match old.style {
        ScalarStyle::Plain => Ok(scalar(value, ctx).into_owned()),
        ScalarStyle::DoubleQuoted => Ok(double_quoted(value)),
        ScalarStyle::SingleQuoted => {
            if value.chars().any(|c| c.is_control()) {
                Ok(double_quoted(value))
            } else {
                Ok(format!("'{}'", value.replace('\'', "''")))
            }
        }
        ScalarStyle::Complex => Err(Unsupported::ComplexScalar.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn old(style: ScalarStyle, in_flow: bool) -> Scalar {
        Scalar { value: "x".into(), style, in_flow }
    }

    #[test]
    fn keeps_the_quoting_style() {
        assert_eq!(restyle(&old(ScalarStyle::Plain, true), "Charge").as_deref(), Ok("Charge"));
        assert_eq!(restyle(&old(ScalarStyle::Plain, true), "a, b").as_deref(), Ok("\"a, b\""));
        assert_eq!(restyle(&old(ScalarStyle::Plain, false), "a, b").as_deref(), Ok("a, b"));
        assert_eq!(restyle(&old(ScalarStyle::DoubleQuoted, false), "Charge").as_deref(), Ok("\"Charge\""));
        assert_eq!(restyle(&old(ScalarStyle::SingleQuoted, false), "it's").as_deref(), Ok("'it''s'"));
        assert!(restyle(&old(ScalarStyle::Complex, false), "x").is_err());
    }

    #[test]
    fn lists_quote_items_only_when_needed() {
        assert_eq!(flow_list(&["a", "b c", "d,e"]), "[a, b c, \"d,e\"]");
        assert_eq!(flow_list::<&str>(&[]), "[]");
    }
}
