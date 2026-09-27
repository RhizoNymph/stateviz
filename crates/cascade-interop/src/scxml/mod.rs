//! SCXML import and export.
//!
//! ## Import
//!
//! | SCXML | Cascade |
//! | --- | --- |
//! | `<scxml name initial>` | one machine named `name` (default `Machine`) |
//! | a sole top-level `<parallel>` | a system: each region `<state>` is a machine named by its `id`; the `<scxml name>` is the system name |
//! | `<state id initial>` / `<initial><transition target/></initial>` | a state; the initial child (a deeper initial is lifted to the child containing it, with a warning; the machine's may be any descendant) |
//! | `<final>` / `<history type="deep">` | `kind: final` / `history` / `deep-history` |
//! | ids | local names: the parent's id prefix (`Order.` in `Order.draft`) is removed |
//! | `<transition event="a b" target cond type>` | one transition per event descriptor (`.*`/`*` suffixes and a `Machine.` prefix removed); targetless → self-transition; `cond` → guard |
//! | `<raise>`/`<send event>` in transitions, `<onentry>`, `<onexit>` (also inside `<if>`/`<foreach>`) | `emits`, attributed to the transitions that run them; routed by `EventRouter` to every machine that handles the event (`#_parent` → the others, `#_<id>` → machine `<id>`) |
//! | events no emit covers | fired by `Environment` |
//! | `<parallel>` anywhere else, eventless transitions, multiple targets | [`InteropError::Unsupported`](crate::InteropError) |
//! | `<datamodel>`, `<script>`, `<invoke>`, `<log>`, `<assign>` | ignored |
//!
//! A document that declares the `cascade:` namespace (Cascade's own export)
//! is read with its annotations instead of synthesized wiring:
//! `cascade:controller` regions become controllers, `<cascade:event>` and
//! `<cascade:source>` become declared events and external sources, and
//! `cascade:color`/`domain`/`fields`/`bounded`/`target`/`system` restore the
//! corresponding attributes.
//!
//! ## Export
//!
//! See [`write`] for the representation. Export then import preserves the
//! model (every element key and attribute). What other SCXML tools lose:
//! guards and rule conditions are free text in `cond`, not expressions;
//! external sources, declared events, colors, domains, fields, target
//! selectors and `bounded` exist only in the `cascade:` namespace. What the
//! text loses: multi-source `from` lists become one `<transition>` per
//! source, implicit initial states become explicit, and source positions
//! are gone.

mod read;
mod write;
mod xml;

use cascade_core::Model;

use crate::error::InteropError;
use crate::import::lower::{LowerInput, lower};
use crate::import::{ImportOptions, Imported, Warnings};

pub(crate) const FORMAT: &str = "SCXML";
pub(crate) const SCXML_NS: &str = "http://www.w3.org/2005/07/scxml";
/// The namespace of Cascade's annotations in exported SCXML.
pub(crate) const CASCADE_NS: &str = "urn:x-cascade:scxml";

pub(crate) fn import(text: &str, options: &ImportOptions) -> Result<Imported, InteropError> {
    let mut warnings = Warnings::default();
    let document = read::read(text, &mut warnings)?;
    let definition = lower(
        LowerInput { format: FORMAT, system: document.system, charts: document.charts, wiring: document.wiring },
        options,
        &mut warnings,
    )?;
    Ok(Imported { definition, warnings: warnings.list })
}

pub(crate) fn export(model: &Model) -> String {
    write::export(model)
}
