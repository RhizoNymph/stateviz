//! Sanitizing imported names into valid, unique Cascade names.
//!
//! Cascade names match `[A-Za-z_][A-Za-z0-9_-]*` (letters and digits may be
//! Unicode). Other formats allow anything (`"Loading Data"`,
//! `"user.submit"`, `"xstate.done.actor.fetch"`), so every imported name
//! goes through a [`NameTable`] for its scope: invalid characters become
//! `_`, a leading digit gets a `_` prefix, and collisions get a numeric
//! suffix. Every change is reported as a [`WarningKind::Renamed`].

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use cascade_core::parse::grammar::is_valid_name;

use crate::import::{NameKind, WarningKind, Warnings};

/// `raw` as a valid Cascade name: runs of invalid characters become one `_`
/// (and are trimmed at the ends); a name that would start with a digit or
/// `-` gets a `_` prefix; an empty name becomes `_`.
pub(crate) fn sanitize(raw: &str) -> String {
    if is_valid_name(raw) {
        return raw.to_owned();
    }
    let valid = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    let parts: Vec<&str> = raw.split(|c: char| !valid(c)).filter(|p| !p.is_empty()).collect();
    let mut name = parts.join("_");
    match name.chars().next() {
        None => name.push('_'),
        Some(first) if !(first.is_alphabetic() || first == '_') => name.insert(0, '_'),
        Some(_) => {}
    }
    name
}

/// Unique names within one scope (a machine's triggers, the siblings of a
/// state, all machines, …), claimed in input order.
#[derive(Debug)]
pub(crate) struct NameTable {
    kind: NameKind,
    used: HashSet<String>,
}

impl NameTable {
    pub fn new(kind: NameKind) -> Self {
        Self { kind, used: HashSet::new() }
    }

    /// A fresh name for `raw`, warning at `location` when it differs.
    pub fn claim(&mut self, raw: &str, location: &str, warnings: &mut Warnings) -> String {
        let base = sanitize(raw);
        let mut name = base.clone();
        let mut n = 2;
        while !self.used.insert(name.clone()) {
            name = format!("{base}_{n}");
            n += 1;
        }
        if name != raw {
            warnings
                .push(location, WarningKind::Renamed { what: self.kind, original: raw.to_owned(), name: name.clone() });
        }
        name
    }
}

/// A [`NameTable`] that remembers the name given to each key, so the same
/// input name (or trigger kind) maps to the same Cascade name everywhere.
#[derive(Debug)]
pub(crate) struct KeyedNames<K> {
    table: NameTable,
    names: HashMap<K, String>,
}

impl<K: Eq + Hash + Clone> KeyedNames<K> {
    pub fn new(kind: NameKind) -> Self {
        Self { table: NameTable::new(kind), names: HashMap::new() }
    }

    /// The name for `key`, claiming one derived from `raw` on first use.
    pub fn name(&mut self, key: &K, raw: &str, location: &str, warnings: &mut Warnings) -> String {
        if let Some(name) = self.names.get(key) {
            return name.clone();
        }
        let name = self.table.claim(raw, location, warnings);
        self.names.insert(key.clone(), name.clone());
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_invalid_names() {
        assert_eq!(sanitize("idle"), "idle");
        assert_eq!(sanitize("go-now"), "go-now");
        assert_eq!(sanitize("Loading Data"), "Loading_Data");
        assert_eq!(sanitize("user.submit"), "user_submit");
        assert_eq!(sanitize("xstate.done.actor.fetch"), "xstate_done_actor_fetch");
        assert_eq!(sanitize("(machine)"), "machine");
        assert_eq!(sanitize("1000"), "_1000");
        assert_eq!(sanitize("-x"), "_-x");
        assert_eq!(sanitize("a..b"), "a_b");
        assert_eq!(sanitize(""), "_");
        assert_eq!(sanitize("..."), "_");
        assert_eq!(sanitize("café crème"), "café_crème");
        for raw in ["Loading Data", "1000", "", "...", "-x", "#id.a"] {
            assert!(is_valid_name(&sanitize(raw)), "{raw:?}");
        }
    }

    #[test]
    fn claims_unique_names_and_reports_changes() {
        let mut warnings = Warnings::default();
        let mut table = NameTable::new(NameKind::State);
        assert_eq!(table.claim("idle", "here", &mut warnings), "idle");
        assert!(warnings.list.is_empty());
        assert_eq!(table.claim("a.b", "here", &mut warnings), "a_b");
        assert_eq!(table.claim("a_b", "there", &mut warnings), "a_b_2");
        assert_eq!(table.claim("a b", "there", &mut warnings), "a_b_3");
        assert_eq!(warnings.list.len(), 3);
        assert_eq!(
            warnings.list[1].kind,
            WarningKind::Renamed { what: NameKind::State, original: "a_b".into(), name: "a_b_2".into() }
        );
    }

    #[test]
    fn keyed_names_are_stable() {
        let mut warnings = Warnings::default();
        let mut names: KeyedNames<&str> = KeyedNames::new(NameKind::Event);
        let first = names.name(&"k", "user.submit", "a", &mut warnings);
        let again = names.name(&"k", "whatever", "b", &mut warnings);
        assert_eq!(first, again);
        assert_eq!(first, "user_submit");
        assert_eq!(warnings.list.len(), 1);
    }
}
