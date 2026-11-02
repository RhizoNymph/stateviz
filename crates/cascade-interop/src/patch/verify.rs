//! Comparing definitions while ignoring where things were written.

use cascade_core::definition::Definition;
use cascade_core::edit::without_spans;

/// Whether two definitions are equal apart from their source spans.
pub(crate) fn same_definition(a: &Definition, b: &Definition) -> bool {
    without_spans(a) == without_spans(b)
}

#[cfg(test)]
mod tests {
    use cascade_core::parse_definition;

    use super::*;

    #[test]
    fn formatting_does_not_matter_but_content_does() {
        let a = parse_definition("machines:\n  A:\n    states: [x, y]\n").expect("parses");
        let b = parse_definition("# c\nmachines: { A: { states: [ x,\n  y ] } }\n").expect("parses");
        let c = parse_definition("machines:\n  A:\n    states: [x, z]\n").expect("parses");
        assert!(same_definition(&a, &b));
        assert!(!same_definition(&a, &c));
    }
}
