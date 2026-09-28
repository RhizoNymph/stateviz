//! Committed inspector values as edit ops.

use std::str::FromStr;

use cascade_core::definition::{EventDef, MachineDef, TransitionDef, TriggerRef};
use cascade_core::edit::EditOp;
use cascade_core::parse::grammar::parse_target;
use cascade_core::{Definition, ElementKey, PaletteColor, SourceSpan, Spanned};

use super::{FieldError, FieldId, FieldValue, missing, parse_kind, trigger_ref, validate};
use crate::build::defs::{self, split_list};
use crate::build::ops::{Planned, transition_entry};

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// The op for committing `value` to `field` of `key`; `Ok(None)` when the
/// value is unchanged.
pub fn field_op(
    definition: &Definition,
    key: &ElementKey,
    field: FieldId,
    value: &FieldValue,
) -> Result<Option<Planned>, FieldError> {
    let text_value = match value {
        FieldValue::Text(t) => {
            validate(field, t).map_err(FieldError::Invalid)?;
            t.trim().to_owned()
        }
        FieldValue::Toggle(_) => String::new(),
    };
    let toggle = match value {
        FieldValue::Toggle(b) => *b,
        FieldValue::Text(_) => false,
    };
    let planned = match key {
        ElementKey::Machine { machine } => {
            let m = defs::machine(definition, machine).ok_or_else(|| missing(key))?;
            machine_op(m, field, &text_value)?
        }
        ElementKey::State { machine, path } => {
            let s =
                defs::machine(definition, machine).and_then(|m| defs::state(m, path)).ok_or_else(|| missing(key))?;
            match field {
                FieldId::StateName => (text_value != s.name.value).then(|| {
                    Planned::new(
                        EditOp::RenameState { machine: machine.clone(), path: path.clone(), to: text_value.clone() },
                        format!("Rename state {machine}.{path} to {text_value}"),
                    )
                }),
                FieldId::StateKind => {
                    let kind = parse_kind(&text_value)
                        .ok_or_else(|| FieldError::Invalid(format!("`{text_value}` is not a state kind")))?;
                    (kind != s.kind.value).then(|| {
                        Planned::new(
                            EditOp::SetStateKind { machine: machine.clone(), path: path.clone(), kind },
                            format!("Make {machine}.{path} {}", kind.name()),
                        )
                    })
                }
                FieldId::StateInitial => {
                    let initial = non_empty(&text_value);
                    if let Some(child) = &initial
                        && !s.states.iter().any(|c| c.name.value == *child)
                    {
                        return Err(FieldError::Invalid(format!("{path} has no child `{child}`")));
                    }
                    (initial != s.initial.as_ref().map(|i| i.value.clone())).then(|| {
                        Planned::new(
                            EditOp::SetStateInitial { machine: machine.clone(), path: path.clone(), initial },
                            format!("Set the initial child of {machine}.{path}"),
                        )
                    })
                }
                _ => return Err(wrong_field(field)),
            }
        }
        ElementKey::Transition { .. } => {
            let (machine, index, entry) = transition_entry(definition, key)?;
            let mut t = entry.clone();
            match field {
                FieldId::TransitionFrom => {
                    t.from = split_list(&text_value).iter().map(|p| defs::synthetic(p)).collect()
                }
                FieldId::TransitionTo => t.to = defs::synthetic(&text_value),
                FieldId::TransitionTrigger => t.on = defs::synthetic(&text_value),
                FieldId::TransitionGuard => t.guard = non_empty(&text_value).map(Spanned::synthetic),
                FieldId::TransitionEmits => {
                    t.emits = split_list(&text_value).iter().map(|e| defs::synthetic(e)).collect()
                }
                FieldId::TransitionBounded => t.bounded = toggle,
                _ => return Err(wrong_field(field)),
            }
            (!same_transition(entry, &t)).then(|| {
                Planned::new(
                    EditOp::UpdateTransition { machine: machine.clone(), index, transition: t },
                    format!("Edit transition {machine}: {}", field.label().to_lowercase()),
                )
            })
        }
        ElementKey::Event { event } => match field {
            FieldId::EventName => (text_value != *event).then(|| {
                Planned::new(
                    EditOp::RenameEvent { from: event.clone(), to: text_value.clone() },
                    format!("Rename event {event} to {text_value}"),
                )
            }),
            FieldId::EventPayload => payload_op(definition, event, split_list(&text_value)),
            _ => return Err(wrong_field(field)),
        },
        ElementKey::Controller { controller } => {
            let c = defs::controller(definition, controller).ok_or_else(|| missing(key))?;
            match field {
                FieldId::ControllerName => (text_value != *controller).then(|| {
                    Planned::new(
                        EditOp::RenameController { from: controller.clone(), to: text_value.clone() },
                        format!("Rename controller {controller} to {text_value}"),
                    )
                }),
                FieldId::ControllerAddHandler => {
                    if c.on.iter().any(|h| h.event.value == text_value) {
                        return Err(FieldError::Invalid(format!("{controller} already handles {text_value}")));
                    }
                    Some(Planned::new(
                        EditOp::AddHandler {
                            controller: controller.clone(),
                            handler: defs::handler_def(&text_value),
                            index: None,
                        },
                        format!("{controller} handles {text_value}"),
                    ))
                }
                _ => return Err(wrong_field(field)),
            }
        }
        ElementKey::Rule { controller, event, ordinal } => {
            let r = defs::rule(definition, controller, event, *ordinal).ok_or_else(|| missing(key))?;
            let mut rule = r.clone();
            match field {
                FieldId::RuleFire => {
                    rule.fire = Spanned::synthetic(trigger_ref(&text_value).map_err(FieldError::Invalid)?)
                }
                FieldId::RuleTarget => {
                    rule.target = match non_empty(&text_value) {
                        Some(t) => Some(Spanned::synthetic(parse_target(&t).map_err(FieldError::Invalid)?)),
                        None => None,
                    };
                }
                FieldId::RuleWhen => rule.when = non_empty(&text_value).map(Spanned::synthetic),
                FieldId::RuleBounded => rule.bounded = toggle,
                _ => return Err(wrong_field(field)),
            }
            let changed = rule.fire.value != r.fire.value
                || rule.target.as_ref().map(|t| &t.value) != r.target.as_ref().map(|t| &t.value)
                || rule.when.as_ref().map(|w| &w.value) != r.when.as_ref().map(|w| &w.value)
                || rule.bounded != r.bounded;
            let index = usize::try_from(*ordinal).map_err(|_| missing(key))?;
            changed.then(|| {
                Planned::new(
                    EditOp::UpdateRule { controller: controller.clone(), event: event.clone(), index, rule },
                    format!("Edit rule {controller}/{event}#{ordinal}: {}", field.label().to_lowercase()),
                )
            })
        }
        ElementKey::External { source } => {
            let e = defs::external(definition, source).ok_or_else(|| missing(key))?;
            match field {
                FieldId::SourceName => (text_value != *source).then(|| {
                    Planned::new(
                        EditOp::RenameExternal { from: source.clone(), to: text_value.clone() },
                        format!("Rename source {source} to {text_value}"),
                    )
                }),
                FieldId::SourceTriggers => {
                    let triggers = split_list(&text_value)
                        .iter()
                        .map(|t| trigger_ref(t))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(FieldError::Invalid)?;
                    let current: Vec<TriggerRef> = e.triggers.iter().map(|t| t.value.clone()).collect();
                    (triggers != current).then(|| {
                        Planned::new(
                            EditOp::SetExternalTriggers { external: source.clone(), triggers },
                            format!("Set the triggers of {source}"),
                        )
                    })
                }
                _ => return Err(wrong_field(field)),
            }
        }
        ElementKey::Trigger { .. } | ElementKey::Handler { .. } => return Err(wrong_field(field)),
    };
    Ok(planned)
}

fn wrong_field(field: FieldId) -> FieldError {
    FieldError::Invalid(format!("{} does not apply to this element", field.label()))
}

fn machine_op(m: &MachineDef, field: FieldId, value: &str) -> Result<Option<Planned>, FieldError> {
    let name = m.name.value.clone();
    let planned = match field {
        FieldId::MachineName => (value != name).then(|| {
            Planned::new(
                EditOp::RenameMachine { from: name.clone(), to: value.to_owned() },
                format!("Rename machine {name} to {value}"),
            )
        }),
        FieldId::MachineColor => {
            let color = match non_empty(value) {
                Some(c) => Some(PaletteColor::from_str(&c).map_err(|e| FieldError::Invalid(e.to_string()))?),
                None => None,
            };
            (color != m.color.as_ref().map(|c| c.value)).then(|| {
                Planned::new(EditOp::SetMachineColor { machine: name.clone(), color }, format!("Color {name}"))
            })
        }
        FieldId::MachineDomain => {
            let domain = non_empty(value);
            (domain != m.domain.as_ref().map(|d| d.value.clone())).then(|| {
                Planned::new(
                    EditOp::SetMachineDomain { machine: name.clone(), domain },
                    format!("Set the domain of {name}"),
                )
            })
        }
        FieldId::MachineInitial => {
            let initial = non_empty(value);
            if let Some(state) = &initial
                && !m.states.iter().any(|s| s.name.value == *state)
            {
                return Err(FieldError::Invalid(format!("{name} has no top-level state `{state}`")));
            }
            (initial != m.initial.as_ref().map(|i| i.value.clone())).then(|| {
                Planned::new(
                    EditOp::SetMachineInitial { machine: name.clone(), initial },
                    format!("Set the initial state of {name}"),
                )
            })
        }
        FieldId::MachineFields => {
            let fields = split_list(value);
            let current: Vec<String> = m.fields.iter().map(|f| f.value.clone()).collect();
            (fields != current).then(|| {
                Planned::new(
                    EditOp::SetMachineFields { machine: name.clone(), fields },
                    format!("Set the fields of {name}"),
                )
            })
        }
        _ => return Err(wrong_field(field)),
    };
    Ok(planned)
}

/// There is no "set payload" op: redeclare the event in place (or declare
/// it for the first time).
fn payload_op(definition: &Definition, event: &str, payload: Vec<String>) -> Option<Planned> {
    let declared = definition.events.iter().position(|e| e.name.value == event);
    let new_def = EventDef {
        name: defs::synthetic(event),
        payload: payload.iter().map(|p| defs::synthetic(p)).collect(),
        span: SourceSpan::unknown(),
    };
    let label = format!("Set the payload of {event}");
    match declared {
        Some(index) => {
            let current: Vec<String> = definition.events[index].payload.iter().map(|p| p.value.clone()).collect();
            (current != payload).then(|| {
                Planned::new(
                    EditOp::Batch(vec![
                        EditOp::RemoveEventDeclaration { event: event.to_owned() },
                        EditOp::DeclareEvent { event: new_def, index: Some(index) },
                    ]),
                    label,
                )
            })
        }
        None => Some(Planned::new(EditOp::DeclareEvent { event: new_def, index: None }, label)),
    }
}

fn same_transition(a: &TransitionDef, b: &TransitionDef) -> bool {
    let values = |items: &[Spanned<String>]| items.iter().map(|s| s.value.clone()).collect::<Vec<_>>();
    values(&a.from) == values(&b.from)
        && a.to.value == b.to.value
        && a.on.value == b.on.value
        && a.guard.as_ref().map(|g| &g.value) == b.guard.as_ref().map(|g| &g.value)
        && values(&a.emits) == values(&b.emits)
        && a.bounded == b.bounded
}
