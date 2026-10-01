//! Text splices: byte ranges of the source replaced in one pass.

use super::error::{Result, Unsupported};

/// Replace `start..end` (bytes) with `text`. An empty range inserts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Splice {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

impl Splice {
    pub fn replace(start: usize, end: usize, text: impl Into<String>) -> Self {
        Self { start, end, text: text.into() }
    }

    pub fn insert(at: usize, text: impl Into<String>) -> Self {
        Self { start: at, end: at, text: text.into() }
    }

    pub fn delete(start: usize, end: usize) -> Self {
        Self { start, end, text: String::new() }
    }
}

/// Apply non-overlapping splices. Insertions at the same offset keep their
/// order; an insertion at the start of a replaced range goes before it.
pub(crate) fn apply(text: &str, mut splices: Vec<Splice>) -> Result<String> {
    splices.sort_by_key(|s| (s.start, s.end));
    let mut out = String::with_capacity(text.len() + splices.iter().map(|s| s.text.len()).sum::<usize>());
    let mut cursor = 0;
    for splice in &splices {
        let valid = splice.start >= cursor
            && splice.start <= splice.end
            && splice.end <= text.len()
            && text.is_char_boundary(splice.start)
            && text.is_char_boundary(splice.end);
        if !valid {
            return Err(Unsupported::Shape.into());
        }
        out.push_str(&text[cursor..splice.start]);
        out.push_str(&splice.text);
        cursor = splice.end;
    }
    out.push_str(&text[cursor..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_in_offset_order() {
        let text = "abcdef";
        let out = apply(
            text,
            vec![Splice::replace(4, 5, "E"), Splice::insert(0, ">"), Splice::delete(1, 3), Splice::insert(4, "|")],
        );
        assert_eq!(out, Ok(">ad|Ef".to_owned()));
    }

    #[test]
    fn rejects_overlaps() {
        assert!(apply("abcdef", vec![Splice::delete(1, 4), Splice::delete(3, 5)]).is_err());
    }
}
