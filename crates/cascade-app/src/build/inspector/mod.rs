//! The inspector's model: the editable fields of the selected element and
//! the edit op each committed field change makes.
//!
//! Pure. [`inspect`] lists the fields with their current values;
//! [`validate`] checks text as it is typed (names, paths, trigger
//! references, target selectors with the core grammar); [`field_op`] turns
//! a committed value into one [`EditOp`](cascade_core::edit::EditOp) (renames use the `Rename*` ops, so
//! references follow). An unchanged value makes no op.

use cascade_core::definition::{StateKindDef, TriggerRef};
use cascade_core::expr::{ExprContext, parse_condition, parse_duration, parse_expr, parse_guard};
use cascade_core::parse::grammar::{is_valid_name, is_valid_path, parse_target, parse_trigger_ref};
use cascade_core::{Definition, ElementKey, PaletteColor, Spanned};

mod commit;
mod feedback;
pub mod syntax;
mod transition;

pub use commit::field_op;
pub use feedback::check;
pub use syntax::TriggerKindTag;

use super::defs::{self, join_list, split_list};
use super::ops::PlanError;

/// One editable property.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldId {
    MachineName,
    MachineColor,
    MachineDomain,
    MachineInitial,
    MachineFields,
    StateName,
    StateKind,
    StateInitial,
    TransitionFrom,
    TransitionTo,
    /// The trigger kind selector (command, subscription, timer).
    TransitionKind,
    /// A command trigger's name.
    TransitionTrigger,
    /// A subscription's event.
    TransitionEvent,
    /// A subscription's `where` filter.
    TransitionWhere,
    /// A timer's delay.
    TransitionAfter,
    TransitionGuard,
    /// Row `n` of `sets:` (`field = value`); an empty commit removes it.
    TransitionSet(u16),
    /// The "add a `sets:` row" field.
    TransitionSetNew,
    /// Row `row` of the `with:` of emit entry `emit`.
    EmitWith {
        emit: u16,
        row: u16,
    },
    /// The "add a `with:` row" field of emit entry `emit`.
    EmitWithNew {
        emit: u16,
    },
    TransitionEmits,
    TransitionBounded,
    EventName,
    EventPayload,
    ControllerName,
    ControllerAddHandler,
    RuleFire,
    RuleTarget,
    RuleWhen,
    RuleBounded,
    SourceName,
    SourceTriggers,
}

/// What a field grammar accepts, for live validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Syntax {
    Name,
    OptionalName,
    Path,
    PathList,
    NameList,
    /// `name[: type], …`.
    FieldList,
    TriggerRef,
    TriggerRefList,
    Target,
    /// `else`, `?label` or an expression; empty for none.
    Guard,
    /// `?label` or an expression; empty for none.
    Condition,
    /// A selector filter (`orderId == event.orderId`); empty for none.
    Filter,
    /// A timer delay (`30s`).
    Duration,
    /// `field = value`; empty removes the row (or adds nothing).
    Assignment,
    Choice,
    Toggle,
}

impl FieldId {
    pub const fn label(self) -> &'static str {
        match self {
            FieldId::MachineName | FieldId::StateName | FieldId::EventName | FieldId::ControllerName => "Name",
            FieldId::SourceName => "Name",
            FieldId::MachineColor => "Color",
            FieldId::MachineDomain => "Domain",
            FieldId::MachineInitial => "Initial",
            FieldId::MachineFields => "Fields",
            FieldId::StateKind => "Kind",
            FieldId::StateInitial => "Initial child",
            FieldId::TransitionFrom => "From",
            FieldId::TransitionTo => "To",
            FieldId::TransitionKind => "Trigger kind",
            FieldId::TransitionTrigger => "Trigger",
            FieldId::TransitionEvent => "Event",
            FieldId::TransitionWhere => "Where",
            FieldId::TransitionAfter => "After",
            FieldId::TransitionGuard => "Guard",
            FieldId::TransitionSet(_) => "Set",
            FieldId::TransitionSetNew => "+ Set",
            FieldId::EmitWith { .. } => "With",
            FieldId::EmitWithNew { .. } => "+ With",
            FieldId::TransitionEmits => "Emits",
            FieldId::TransitionBounded | FieldId::RuleBounded => "Bounded",
            FieldId::EventPayload => "Payload",
            FieldId::ControllerAddHandler => "Handle event",
            FieldId::RuleFire => "Fire",
            FieldId::RuleTarget => "Target",
            FieldId::RuleWhen => "When",
            FieldId::SourceTriggers => "Triggers",
        }
    }

    const fn syntax(self) -> Syntax {
        match self {
            FieldId::MachineName
            | FieldId::StateName
            | FieldId::EventName
            | FieldId::ControllerName
            | FieldId::SourceName
            | FieldId::TransitionTrigger
            | FieldId::TransitionEvent
            | FieldId::ControllerAddHandler => Syntax::Name,
            FieldId::MachineDomain => Syntax::OptionalName,
            FieldId::TransitionTo => Syntax::Path,
            FieldId::TransitionFrom => Syntax::PathList,
            FieldId::TransitionEmits => Syntax::NameList,
            FieldId::MachineFields | FieldId::EventPayload => Syntax::FieldList,
            FieldId::TransitionWhere => Syntax::Filter,
            FieldId::TransitionAfter => Syntax::Duration,
            FieldId::TransitionSet(_)
            | FieldId::TransitionSetNew
            | FieldId::EmitWith { .. }
            | FieldId::EmitWithNew { .. } => Syntax::Assignment,
            FieldId::RuleFire => Syntax::TriggerRef,
            FieldId::SourceTriggers => Syntax::TriggerRefList,
            FieldId::RuleTarget => Syntax::Target,
            FieldId::TransitionGuard => Syntax::Guard,
            FieldId::RuleWhen => Syntax::Condition,
            FieldId::MachineColor
            | FieldId::MachineInitial
            | FieldId::StateKind
            | FieldId::StateInitial
            | FieldId::TransitionKind => Syntax::Choice,
            FieldId::TransitionBounded | FieldId::RuleBounded => Syntax::Toggle,
        }
    }

    /// Placeholder text for a text field.
    pub const fn hint(self) -> &'static str {
        match self.syntax() {
            Syntax::Name => "name",
            Syntax::OptionalName => "(none)",
            Syntax::Path => "state path",
            Syntax::PathList => "a, b.c",
            Syntax::NameList => "a, b",
            Syntax::FieldList => "id, amount: number, tier: enum [a, b]",
            Syntax::Filter => "orderId == event.orderId",
            Syntax::Duration => "30s, 5m, 2h",
            Syntax::Assignment => "field = event.x",
            Syntax::TriggerRef => "Machine.trigger",
            Syntax::TriggerRefList => "Machine.trigger, …",
            Syntax::Target => "the one instance",
            Syntax::Guard | Syntax::Condition => "(none)",
            Syntax::Choice | Syntax::Toggle => "",
        }
    }
}

/// How a field is edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldInput {
    Text(String),
    /// `(value, label)` options; the empty value is the default.
    Choice {
        value: String,
        options: Vec<(String, String)>,
    },
    Toggle(bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub id: FieldId,
    /// Usually `id.label()`; rows say what they belong to (`With Paid`).
    pub label: String,
    pub input: FieldInput,
}

/// A row in a list section: something to select, delete, or both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    pub label: String,
    pub key: ElementKey,
}

/// Buttons the inspector offers besides its fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectorAction {
    AddChildState,
    Delete,
}

/// Everything the inspector shows for one element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspection {
    pub key: ElementKey,
    /// E.g. "State Order.placed".
    pub title: String,
    pub fields: Vec<Field>,
    /// A titled list (a controller's handlers, a handler's rules).
    pub list: Option<(String, Vec<ListItem>)>,
    /// Read-only facts and explanations.
    pub notes: Vec<String>,
    pub actions: Vec<InspectorAction>,
}

/// A committed value that makes no op.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FieldError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Plan(#[from] PlanError),
}

/// A committed field value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldValue {
    Text(String),
    Toggle(bool),
}

fn text(value: impl Into<String>) -> FieldInput {
    FieldInput::Text(value.into())
}

fn field(id: FieldId, input: FieldInput) -> Field {
    Field { id, label: id.label().to_owned(), input }
}

fn opt(value: Option<&Spanned<String>>) -> String {
    value.map(|v| v.value.clone()).unwrap_or_default()
}

/// Canonical text of an optional guard or condition.
fn shown<T: std::fmt::Display>(value: Option<&Spanned<T>>) -> String {
    value.map(|v| v.value.to_string()).unwrap_or_default()
}

fn kind_options() -> Vec<(String, String)> {
    [StateKindDef::Normal, StateKindDef::Final, StateKindDef::History, StateKindDef::DeepHistory]
        .iter()
        .map(|k| (k.name().to_owned(), k.name().to_owned()))
        .collect()
}

fn parse_kind(value: &str) -> Option<StateKindDef> {
    [StateKindDef::Normal, StateKindDef::Final, StateKindDef::History, StateKindDef::DeepHistory]
        .into_iter()
        .find(|k| k.name() == value)
}

fn missing(key: &ElementKey) -> PlanError {
    PlanError::NotFound(key.clone())
}

/// The inspector for `key` (no draft; the app calls [`inspect_with`]).
#[cfg(test)]
pub fn inspect(definition: &Definition, key: &ElementKey) -> Result<Inspection, PlanError> {
    inspect_with(definition, key, None)
}

/// The inspector for `key`, showing a transition's trigger as `draft`
/// (the kind picked in the selector but not committed yet) when given.
pub fn inspect_with(
    definition: &Definition,
    key: &ElementKey,
    draft: Option<TriggerKindTag>,
) -> Result<Inspection, PlanError> {
    let base = |title: String| Inspection {
        key: key.clone(),
        title,
        fields: Vec::new(),
        list: None,
        notes: Vec::new(),
        actions: vec![InspectorAction::Delete],
    };
    let inspection = match key {
        ElementKey::Machine { machine } => {
            let m = defs::machine(definition, machine).ok_or_else(|| missing(key))?;
            let mut color_options = vec![(String::new(), "auto".to_owned())];
            color_options.extend(PaletteColor::ALL.iter().map(|c| (c.name().to_owned(), c.name().to_owned())));
            let mut initial_options = vec![(String::new(), "first state".to_owned())];
            initial_options.extend(m.states.iter().map(|s| (s.name.value.clone(), s.name.value.clone())));
            Inspection {
                fields: vec![
                    field(FieldId::MachineName, text(machine.clone())),
                    field(
                        FieldId::MachineColor,
                        FieldInput::Choice {
                            value: m.color.as_ref().map(|c| c.value.name().to_owned()).unwrap_or_default(),
                            options: color_options,
                        },
                    ),
                    field(
                        FieldId::MachineInitial,
                        FieldInput::Choice { value: opt(m.initial.as_ref()), options: initial_options },
                    ),
                    field(FieldId::MachineFields, text(syntax::format_field_list(&m.fields))),
                    field(FieldId::MachineDomain, text(opt(m.domain.as_ref()))),
                ],
                ..base(format!("Machine {machine}"))
            }
        }
        ElementKey::State { machine, path } => {
            let s =
                defs::machine(definition, machine).and_then(|m| defs::state(m, path)).ok_or_else(|| missing(key))?;
            let mut fields = vec![
                field(FieldId::StateName, text(s.name.value.clone())),
                field(
                    FieldId::StateKind,
                    FieldInput::Choice { value: s.kind.value.name().to_owned(), options: kind_options() },
                ),
            ];
            if !s.states.is_empty() {
                let mut options = vec![(String::new(), "first child".to_owned())];
                options.extend(s.states.iter().map(|c| (c.name.value.clone(), c.name.value.clone())));
                fields
                    .push(field(FieldId::StateInitial, FieldInput::Choice { value: opt(s.initial.as_ref()), options }));
            }
            Inspection {
                fields,
                actions: vec![InspectorAction::AddChildState, InspectorAction::Delete],
                ..base(format!("State {machine}.{path}"))
            }
        }
        ElementKey::Transition { .. } => transition::inspect(definition, key, draft, base),
        ElementKey::Trigger { machine, trigger } => Inspection {
            notes: vec![
                format!("Trigger {trigger} of {machine}."),
                "Rename it on its transitions; connect a controller or source to a transition to fire it.".to_owned(),
            ],
            actions: Vec::new(),
            ..base(format!("Trigger {machine}.{trigger}"))
        },
        ElementKey::Event { event } => {
            let declared = definition.events.iter().find(|e| e.name.value == *event);
            let mut notes = Vec::new();
            if declared.is_none() {
                notes.push("Not declared under events:; setting a payload declares it.".to_owned());
            }
            Inspection {
                fields: vec![
                    field(FieldId::EventName, text(event.clone())),
                    field(
                        FieldId::EventPayload,
                        text(declared.map(|e| syntax::format_field_list(&e.payload)).unwrap_or_default()),
                    ),
                ],
                notes,
                ..base(format!("Event {event}"))
            }
        }
        ElementKey::Controller { controller } => {
            let c = defs::controller(definition, controller).ok_or_else(|| missing(key))?;
            let items =
                c.on.iter()
                    .map(|h| ListItem {
                        label: format!(
                            "on {} ({} rule{})",
                            h.event.value,
                            h.rules.len(),
                            if h.rules.len() == 1 { "" } else { "s" }
                        ),
                        key: ElementKey::Handler { controller: controller.clone(), event: h.event.value.clone() },
                    })
                    .collect();
            Inspection {
                fields: vec![
                    field(FieldId::ControllerName, text(controller.clone())),
                    field(FieldId::ControllerAddHandler, text(String::new())),
                ],
                list: Some(("Handlers".to_owned(), items)),
                ..base(format!("Controller {controller}"))
            }
        }
        ElementKey::Handler { controller, event } => {
            let h = defs::handler(definition, controller, event).ok_or_else(|| missing(key))?;
            let items = h
                .rules
                .iter()
                .enumerate()
                .map(|(i, r)| ListItem {
                    label: format!("fire {}", r.fire.value),
                    key: ElementKey::Rule {
                        controller: controller.clone(),
                        event: event.clone(),
                        ordinal: u32::try_from(i).unwrap_or(u32::MAX),
                    },
                })
                .collect();
            Inspection {
                list: Some(("Rules".to_owned(), items)),
                notes: vec!["Connect this controller to a transition to add a rule.".to_owned()],
                ..base(format!("Handler {controller} on {event}"))
            }
        }
        ElementKey::Rule { controller, event, ordinal } => {
            let r = defs::rule(definition, controller, event, *ordinal).ok_or_else(|| missing(key))?;
            Inspection {
                fields: vec![
                    field(FieldId::RuleFire, text(r.fire.value.to_string())),
                    field(
                        FieldId::RuleTarget,
                        text(r.target.as_ref().map(|t| t.value.to_string()).unwrap_or_default()),
                    ),
                    field(FieldId::RuleWhen, text(shown(r.when.as_ref()))),
                    field(FieldId::RuleBounded, FieldInput::Toggle(r.bounded)),
                ],
                ..base(format!("Rule {controller} on {event} #{ordinal}"))
            }
        }
        ElementKey::External { source } => {
            let e = defs::external(definition, source).ok_or_else(|| missing(key))?;
            Inspection {
                fields: vec![
                    field(FieldId::SourceName, text(source.clone())),
                    field(
                        FieldId::SourceTriggers,
                        text(join_list(
                            e.triggers
                                .iter()
                                .map(|t| t.value.to_string())
                                .collect::<Vec<_>>()
                                .iter()
                                .map(String::as_str),
                        )),
                    ),
                ],
                ..base(format!("Source {source}"))
            }
        }
    };
    Ok(inspection)
}

/// Check `value` as typed into `field`: `Err` holds the inline message.
pub fn validate(field: FieldId, value: &str) -> Result<(), String> {
    let value = value.trim();
    let name = |v: &str| {
        if is_valid_name(v) { Ok(()) } else { Err(format!("`{v}` is not a valid name (letters, digits, _ and -)")) }
    };
    match field.syntax() {
        Syntax::Name => name(value),
        Syntax::OptionalName => {
            if value.is_empty() {
                Ok(())
            } else {
                name(value)
            }
        }
        Syntax::Path => {
            if is_valid_path(value) {
                Ok(())
            } else {
                Err(format!("`{value}` is not a valid state path"))
            }
        }
        Syntax::PathList => {
            let items = split_list(value);
            if items.is_empty() {
                return Err("name at least one state".to_owned());
            }
            match items.iter().find(|p| !is_valid_path(p)) {
                Some(bad) => Err(format!("`{bad}` is not a valid state path")),
                None => Ok(()),
            }
        }
        Syntax::NameList => split_list(value).iter().try_for_each(|n| name(n)),
        Syntax::FieldList => syntax::parse_field_list(value).map(|_| ()),
        Syntax::Filter if !value.is_empty() => {
            parse_expr(value, ExprContext::Filter).map(|_| ()).map_err(|e| e.to_string())
        }
        Syntax::Duration => match parse_duration(value) {
            Ok(d) if d.is_zero() => Err("a timer needs a delay above zero".to_owned()),
            Ok(_) => Ok(()),
            Err(e) => Err(e.to_string()),
        },
        Syntax::Assignment if !value.is_empty() => syntax::parse_assignment(value).map(|_| ()),
        Syntax::TriggerRef => trigger_ref(value).map(|_| ()),
        Syntax::TriggerRefList => split_list(value).iter().try_for_each(|t| trigger_ref(t).map(|_| ())),
        Syntax::Target => {
            if value.is_empty() {
                Ok(())
            } else {
                parse_target(value).map(|_| ())
            }
        }
        Syntax::Guard if !value.is_empty() => parse_guard(value).map(|_| ()).map_err(|e| e.to_string()),
        Syntax::Condition if !value.is_empty() => parse_condition(value).map(|_| ()).map_err(|e| e.to_string()),
        Syntax::Guard | Syntax::Condition | Syntax::Filter | Syntax::Assignment | Syntax::Choice | Syntax::Toggle => {
            Ok(())
        }
    }
}

fn trigger_ref(value: &str) -> Result<TriggerRef, String> {
    parse_trigger_ref(value).ok_or_else(|| format!("`{value}` is not Machine.trigger"))
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_semantics;
