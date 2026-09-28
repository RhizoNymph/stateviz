//! Controller, handler and rule ops.

use cascade_core::definition::{ControllerDef, HandlerDef, RuleDef};

use super::{Child, add_child, add_section, check_new_name, definition, insert_at, nav, remove_section, step};
use crate::patch::collection::{
    Value, remove_child, replace_scalar, replace_value, set_key, set_scalar_key, set_text_key,
};
use crate::patch::doc::{Doc, Kind, Node};
use crate::patch::error::{Result, Unsupported, name_taken, not_found, out_of_range};
use crate::patch::render::{self, Block, rule_block, rule_flow};
use crate::patch::splice::{self, Splice};
use crate::yaml::selector;

pub(super) fn add(text: &str, controller: &ControllerDef, index: Option<usize>) -> Result<String> {
    step(text, |doc, style| {
        let block = render::controller(controller, style);
        let name = &controller.name.value;
        let Some(section) = nav::section(doc, "controllers")?.filter(|s| !s.value.is_null()) else {
            check_new_name("controller", name, false)?;
            insert_at(index, 0, "controllers")?;
            return match nav::section(doc, "controllers")? {
                Some(section) => add_child(doc, style, section, 0, &Child { block, inline: None, item: false }, true),
                None => add_section(doc, style, "controllers", block),
            };
        };
        let taken = section.value.as_map().is_some_and(|m| m.get(name).is_some());
        check_new_name("controller", name, taken)?;
        let len = section.value.as_map().map_or(0, |m| m.entries.len());
        let at = insert_at(index, len, "controllers")?;
        add_child(doc, style, section, at, &Child { block, inline: None, item: false }, true)
    })
}

pub(super) fn remove(text: &str, name: &str) -> Result<String> {
    step(text, |doc, _| {
        let section = nav::section(doc, "controllers")?.ok_or_else(|| not_found("controller", name))?;
        let (index, _) = nav::named(&section.value, name, "controller")?;
        match section.value.as_map().map_or(0, |m| m.entries.len()) {
            1 => remove_section(doc, "controllers"),
            _ => Ok(vec![remove_child(doc, &section.value, index)?]),
        }
    })
}

pub(super) fn rename(text: &str, from: &str, to: &str) -> Result<String> {
    step(text, |doc, _| {
        let (_, entry) = nav::controller(doc, from)?;
        if from == to {
            return Ok(Vec::new());
        }
        let taken = nav::controller(doc, to).is_ok();
        check_new_name("controller", to, taken)?;
        Ok(vec![replace_scalar(doc, &entry.key, to)?])
    })
}

pub(super) fn add_handler(text: &str, controller: &str, handler: &HandlerDef, index: Option<usize>) -> Result<String> {
    step(text, |doc, style| {
        let on = nav::handlers(doc, controller)?;
        let event = &handler.event.value;
        if on.value.as_map().is_some_and(|m| m.get(event).is_some()) {
            return Err(name_taken("handler", format!("{controller}.{event}")));
        }
        let len = on.value.as_map().map_or(0, |m| m.entries.len());
        let at = insert_at(index, len, "handlers")?;
        let child = Child { block: render::handler(handler, style), inline: None, item: false };
        add_child(doc, style, on, at, &child, false)
    })
}

pub(super) fn remove_handler(text: &str, controller: &str, event: &str) -> Result<String> {
    step(text, |doc, _| {
        let on = nav::handlers(doc, controller)?;
        let (index, _) = nav::named(&on.value, event, "handler")?;
        match on.value.as_map().map_or(0, |m| m.entries.len()) {
            1 => replace_value(doc, on, Value::Inline("{}".to_owned())),
            _ => Ok(vec![remove_child(doc, &on.value, index)?]),
        }
    })
}

/// Whether the rules of a list are block mappings rather than flow ones.
fn block_rules(seq: &[&Node]) -> bool {
    let block = seq.iter().filter(|n| n.as_map().is_some_and(|m| !m.flow)).count();
    block * 2 >= seq.len()
}

pub(super) fn add_rule(
    text: &str,
    controller: &str,
    event: &str,
    rule: &RuleDef,
    index: Option<usize>,
) -> Result<String> {
    let mut text = text.to_owned();
    for _ in 0..2 {
        let doc = crate::patch::doc::Doc::parse(&text)?;
        let style = render::Style::detect(&doc);
        let (_, handler) = nav::handler(&doc, controller, event)?;
        let value = &handler.value;
        if value.as_map().is_some() {
            // One rule written as a mapping: make it a list of one first.
            let splices = single_rule_to_list(&doc, style, handler)?;
            text = splice::apply(&text, splices)?;
            continue;
        }
        let rules = nav::rules(value);
        let at = insert_at(index, rules.len(), "rules")?;
        let brace_pad =
            rules.first().is_none_or(|r| !r.is_flow() || doc.text().get(r.start + 1..r.start + 2) == Some(" "));
        let flow = rule_flow(rule, brace_pad, style.quote_text);
        let block = match rules.iter().find(|r| r.as_map().is_some_and(|m| !m.flow)) {
            Some(first) if block_rules(&rules) => {
                let head = rule_head(&doc, first)?;
                rule_block(rule, doc.col(first.start).saturating_sub(doc.col(head)).max(2), style.quote_text)
            }
            None if rules.is_empty() => rule_block(rule, 2, style.quote_text),
            _ => {
                let mut b = Block::new();
                b.line(0, format!("- {flow}"));
                b
            }
        };
        let splices = add_child(&doc, style, handler, at, &Child { block, inline: Some(flow), item: true }, false)?;
        return splice::apply(&text, splices);
    }
    Err(Unsupported::SingleRuleMapping.into())
}

/// The `-` of a rule in a block list.
fn rule_head(doc: &Doc, rule: &Node) -> Result<usize> {
    let before = doc.text().get(..rule.start).ok_or(Unsupported::Shape)?;
    let trimmed = before.trim_end_matches([' ', '\t']);
    match trimmed.ends_with('-') {
        true => Ok(trimmed.len() - 1),
        false => Err(Unsupported::Shape.into()),
    }
}

/// `Event: { fire: … }` → a block list holding that one rule.
fn single_rule_to_list(doc: &Doc, style: render::Style, handler: &crate::patch::doc::Entry) -> Result<Vec<Splice>> {
    let value = &handler.value;
    if !value.is_flow() {
        return Err(Unsupported::SingleRuleMapping.into());
    }
    let source = doc.text().get(value.start..value.end).ok_or(Unsupported::Shape)?;
    if source.contains('\n') || source.contains('#') {
        return Err(Unsupported::SingleRuleMapping.into());
    }
    let mut lines = Block::new();
    lines.line(style.seq_offset, format!("- {source}"));
    replace_value(doc, handler, Value::Lines(lines))
}

fn rule_node<'d>(doc: &'d Doc, controller: &str, event: &str, index: usize) -> Result<&'d Node> {
    let (_, handler) = nav::handler(doc, controller, event)?;
    let rules = nav::rules(&handler.value);
    let len = rules.len();
    rules.get(index).copied().ok_or_else(|| out_of_range("rules", index, len))
}

/// Replace a rule field by field.
pub(super) fn update_rule(text: &str, controller: &str, event: &str, index: usize, new: &RuleDef) -> Result<String> {
    let def = definition(text)?;
    let old = def
        .controllers
        .iter()
        .find(|c| c.name.value == controller)
        .and_then(|c| c.on.iter().find(|h| h.event.value == event))
        .ok_or_else(|| not_found("handler", format!("{controller}.{event}")))?
        .rules
        .get(index)
        .ok_or_else(|| out_of_range("rules", index, 0))?
        .clone();
    let mut text = text.to_owned();
    let edit = |text: &mut String, f: &dyn Fn(&Doc, &Node) -> Result<Vec<Splice>>| -> Result<()> {
        *text = step(text, |doc, _| f(doc, rule_node(doc, controller, event, index)?))?;
        Ok(())
    };
    if old.fire.value != new.fire.value {
        let fire = new.fire.value.to_string();
        edit(&mut text, &|doc, n| Ok(vec![replace_scalar(doc, &nav::require(n, "fire")?.value, &fire)?]))?;
    }
    if old.target.as_ref().map(|t| &t.value) != new.target.as_ref().map(|t| &t.value) {
        let target = new.target.as_ref().map(|t| selector(&t.value));
        edit(&mut text, &|doc, n| set_scalar_key(doc, n, "target", target.as_deref(), nav::RULE_ORDER))?;
    }
    if old.when.as_ref().map(|w| &w.value) != new.when.as_ref().map(|w| &w.value) {
        let when = new.when.as_ref().map(|w| w.value.as_str());
        edit(&mut text, &|doc, n| set_text_key(doc, n, "when", when, nav::RULE_ORDER))?;
    }
    if old.bounded != new.bounded {
        let bounded = new.bounded.then(|| Value::Inline("true".to_owned()));
        edit(&mut text, &|doc, n| set_key(doc, n, "bounded", bounded.clone(), nav::RULE_ORDER))?;
    }
    Ok(text)
}

/// Remove a rule; a handler left without rules keeps an empty list.
pub(super) fn remove_rule(text: &str, controller: &str, event: &str, index: usize) -> Result<String> {
    step(text, |doc, _| {
        let (_, handler) = nav::handler(doc, controller, event)?;
        let rules = nav::rules(&handler.value);
        if index >= rules.len() {
            return Err(out_of_range("rules", index, rules.len()));
        }
        match (&handler.value.kind, rules.len()) {
            (Kind::Seq(_), n) if n > 1 => Ok(vec![remove_child(doc, &handler.value, index)?]),
            _ => replace_value(doc, handler, Value::Inline("[]".to_owned())),
        }
    })
}
