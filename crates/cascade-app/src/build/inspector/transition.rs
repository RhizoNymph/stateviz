//! The transition inspector: trigger kind (command, subscription, timer),
//! guard, `sets:` rows, emits with their `with:` rows; and the edited
//! `transitions:` entry each committed field makes.

use cascade_core::definition::{EmitDef, TransitionDef, TriggerSpec};
use cascade_core::expr::{Assignment, ExprContext, parse_duration, parse_expr, parse_guard};
use cascade_core::{Definition, ElementKey, Spanned};

use super::syntax::{TriggerKindTag, parse_assignment};
use super::{Field, FieldError, FieldId, FieldInput, Inspection, field, shown, text};
use crate::build::defs::{self, join_list, split_list};
use crate::build::ops::transition_entry;

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn row(index: usize) -> Option<u16> {
    u16::try_from(index).ok()
}

fn labelled(id: FieldId, label: String, input: FieldInput) -> Field {
    Field { id, label, input }
}

/// The trigger fields for `kind`: the current values when it is the
/// transition's kind, blank when it is a draft.
fn trigger_fields(t: &TransitionDef, kind: TriggerKindTag) -> Vec<Field> {
    let actual = TriggerKindTag::of(&t.trigger) == kind;
    match kind {
        TriggerKindTag::Command => {
            vec![field(FieldId::TransitionTrigger, text(if actual { t.trigger.name() } else { String::new() }))]
        }
        TriggerKindTag::Event => {
            let (event, filter) = match &t.trigger {
                TriggerSpec::Event { event, filter } => (
                    event.value.clone(),
                    filter.as_ref().map(|f| f.value.display_filter().to_string()).unwrap_or_default(),
                ),
                _ => (String::new(), String::new()),
            };
            vec![field(FieldId::TransitionEvent, text(event)), field(FieldId::TransitionWhere, text(filter))]
        }
        TriggerKindTag::Timer => {
            let delay = match &t.trigger {
                TriggerSpec::After(d) => d.value.to_string(),
                _ => String::new(),
            };
            vec![field(FieldId::TransitionAfter, text(delay))]
        }
    }
}

/// The inspection of a transition key.
pub(super) fn inspect(
    definition: &Definition,
    key: &ElementKey,
    draft: Option<TriggerKindTag>,
    base: impl Fn(String) -> Inspection,
) -> Inspection {
    let ElementKey::Transition { machine, from, to, trigger, .. } = key else {
        return base(format!("{key}"));
    };
    let title = format!("Transition {machine}: {from} → {to}");
    let t = match transition_entry(definition, key) {
        Ok((_, _, t)) => t,
        Err(error) => {
            return Inspection {
                notes: vec![format!("Trigger: {trigger}"), format!("Read-only: {error}.")],
                ..base(title)
            };
        }
    };
    let actual = TriggerKindTag::of(&t.trigger);
    let kind = draft.unwrap_or(actual);
    let mut fields = vec![
        field(FieldId::TransitionFrom, text(join_list(t.from.iter().map(|s| s.value.as_str())))),
        field(FieldId::TransitionTo, text(t.to.value.clone())),
        field(
            FieldId::TransitionKind,
            FieldInput::Choice {
                value: kind.name().to_owned(),
                options: TriggerKindTag::ALL.iter().map(|k| (k.name().to_owned(), k.label().to_owned())).collect(),
            },
        ),
    ];
    fields.extend(trigger_fields(t, kind));
    fields.push(field(FieldId::TransitionGuard, text(shown(t.guard.as_ref()))));
    for (index, set) in t.sets.iter().enumerate() {
        if let Some(n) = row(index) {
            fields.push(field(FieldId::TransitionSet(n), text(set.value.to_string())));
        }
    }
    fields.push(field(FieldId::TransitionSetNew, text(String::new())));
    fields.push(field(FieldId::TransitionEmits, text(join_list(t.emits.iter().map(|e| e.event.value.as_str())))));
    for (e, emit) in t.emits.iter().enumerate() {
        let Some(emit_index) = row(e) else { continue };
        let event = &emit.event.value;
        for (r, with) in emit.with.iter().enumerate() {
            if let Some(r) = row(r) {
                fields.push(labelled(
                    FieldId::EmitWith { emit: emit_index, row: r },
                    format!("With {event}"),
                    text(with.value.to_string()),
                ));
            }
        }
        fields.push(labelled(
            FieldId::EmitWithNew { emit: emit_index },
            format!("+ With {event}"),
            text(String::new()),
        ));
    }
    fields.push(field(FieldId::TransitionBounded, FieldInput::Toggle(t.bounded)));
    let mut notes = Vec::new();
    if kind != actual {
        notes.push(kind.prompt().to_owned());
    }
    if !t.sets.is_empty() || t.emits.iter().any(|e| !e.with.is_empty()) {
        notes.push("Clear a Set or With row and press Enter to remove it.".to_owned());
    }
    Inspection { fields, notes, ..base(title) }
}

/// Replace, remove (empty text) or add (`None` row) an assignment row.
fn edit_rows(rows: &mut Vec<Spanned<Assignment>>, row: Option<u16>, text: &str) -> Result<(), FieldError> {
    let parsed = match non_empty(text) {
        Some(t) => Some(parse_assignment(&t).map_err(FieldError::Invalid)?),
        None => None,
    };
    match (row, parsed) {
        (None, Some(a)) => rows.push(Spanned::synthetic(a)),
        (None, None) => {}
        (Some(r), parsed) => {
            let index = usize::from(r);
            if index >= rows.len() {
                return Err(FieldError::Invalid(format!("there is no row {}", index + 1)));
            }
            match parsed {
                Some(a) => rows[index] = Spanned::synthetic(a),
                None => {
                    rows.remove(index);
                }
            }
        }
    }
    Ok(())
}

fn emit_mut(t: &mut TransitionDef, emit: u16) -> Result<&mut EmitDef, FieldError> {
    t.emits.get_mut(usize::from(emit)).ok_or_else(|| FieldError::Invalid(format!("there is no emit {}", emit + 1)))
}

/// Apply a committed text field to a copy of the entry. `toggle` is the
/// value of a toggle field.
pub(super) fn edit(
    entry: &TransitionDef,
    field: FieldId,
    text_value: &str,
    toggle: bool,
) -> Result<TransitionDef, FieldError> {
    let mut t = entry.clone();
    match field {
        FieldId::TransitionFrom => t.from = split_list(text_value).iter().map(|p| defs::synthetic(p)).collect(),
        FieldId::TransitionTo => t.to = defs::synthetic(text_value),
        FieldId::TransitionKind => {
            return Err(FieldError::Invalid("pick a kind, then enter its name, event or delay".to_owned()));
        }
        // A command name makes the trigger a command (from any kind).
        FieldId::TransitionTrigger => {
            if TriggerKindTag::of(&t.trigger) != TriggerKindTag::Command || text_value != t.trigger.name() {
                t.trigger = TriggerSpec::Command(defs::synthetic(text_value));
            }
        }
        // An event makes it a subscription; a subscription keeps its filter.
        FieldId::TransitionEvent => {
            let filter = match &t.trigger {
                TriggerSpec::Event { filter, .. } => filter.clone(),
                _ => None,
            };
            t.trigger = TriggerSpec::Event { event: defs::synthetic(text_value), filter };
        }
        FieldId::TransitionWhere => match &mut t.trigger {
            TriggerSpec::Event { filter, .. } => {
                *filter = match non_empty(text_value) {
                    Some(f) => Some(Spanned::synthetic(
                        parse_expr(&f, ExprContext::Filter).map_err(|e| FieldError::Invalid(e.to_string()))?,
                    )),
                    None => None,
                };
            }
            _ => {
                return Err(FieldError::Invalid(
                    "`where` filters subscriptions: enter the event to subscribe to first".to_owned(),
                ));
            }
        },
        FieldId::TransitionAfter => {
            let delay = parse_duration(text_value).map_err(|e| FieldError::Invalid(e.to_string()))?;
            if delay.is_zero() {
                return Err(FieldError::Invalid("a timer needs a delay above zero".to_owned()));
            }
            t.trigger = TriggerSpec::After(Spanned::synthetic(delay));
        }
        FieldId::TransitionGuard => {
            t.guard = match non_empty(text_value) {
                Some(g) => Some(Spanned::synthetic(parse_guard(&g).map_err(|e| FieldError::Invalid(e.to_string()))?)),
                None => None,
            };
        }
        FieldId::TransitionSet(r) => edit_rows(&mut t.sets, Some(r), text_value)?,
        FieldId::TransitionSetNew => edit_rows(&mut t.sets, None, text_value)?,
        FieldId::TransitionEmits => {
            // Keep the `with:` of events that stay.
            t.emits = split_list(text_value)
                .iter()
                .map(|e| {
                    entry
                        .emits
                        .iter()
                        .find(|old| old.event.value == *e)
                        .cloned()
                        .unwrap_or_else(|| EmitDef { event: defs::synthetic(e), with: Vec::new() })
                })
                .collect();
        }
        FieldId::EmitWith { emit, row } => edit_rows(&mut emit_mut(&mut t, emit)?.with, Some(row), text_value)?,
        FieldId::EmitWithNew { emit } => edit_rows(&mut emit_mut(&mut t, emit)?.with, None, text_value)?,
        FieldId::TransitionBounded => t.bounded = toggle,
        _ => return Err(super::commit::wrong_field(field)),
    }
    Ok(t)
}

/// Whether two entries are the same edit-wise (spans ignored).
pub(super) fn same_transition(a: &TransitionDef, b: &TransitionDef) -> bool {
    let values = |items: &[Spanned<String>]| items.iter().map(|s| s.value.clone()).collect::<Vec<_>>();
    let emits = |t: &TransitionDef| {
        t.emits
            .iter()
            .map(|e| (e.event.value.clone(), e.with.iter().map(|w| w.value.clone()).collect::<Vec<_>>()))
            .collect::<Vec<_>>()
    };
    let trigger = |t: &TransitionDef| match &t.trigger {
        TriggerSpec::Command(n) => (0, n.value.clone(), None),
        TriggerSpec::Event { event, filter } => (1, event.value.clone(), filter.as_ref().map(|f| f.value.clone())),
        TriggerSpec::After(d) => (2, d.value.to_string(), None),
    };
    values(&a.from) == values(&b.from)
        && a.to.value == b.to.value
        && trigger(a) == trigger(b)
        && a.guard.as_ref().map(|g| &g.value) == b.guard.as_ref().map(|g| &g.value)
        && a.sets.iter().map(|s| &s.value).eq(b.sets.iter().map(|s| &s.value))
        && emits(a) == emits(b)
        && a.bounded == b.bounded
}
