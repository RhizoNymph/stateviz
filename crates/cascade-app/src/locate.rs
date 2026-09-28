//! From scene items to model elements and back.
//!
//! Hit targets name what was clicked; this module turns them into stable
//! [`ElementKey`]s (so selection is shared across views) and finds where an
//! element is drawn so the viewport can centre it.

use cascade_core::{ElementKey, ElementRef, Finding, Model};
use cascade_layout::{Point, Rect};
use cascade_scene::{HitTarget, Overlay, Scene, SceneNode};
use cascade_sim::{Lifeline, Trace, TraceStepKind};

/// The element a trace step is about.
pub fn step_element(kind: &TraceStepKind) -> ElementRef {
    match kind {
        TraceStepKind::ExternalFire { trigger, .. } | TraceStepKind::Dropped { trigger, .. } => {
            ElementRef::Trigger(*trigger)
        }
        TraceStepKind::Transition { transition, .. } => ElementRef::Transition(*transition),
        TraceStepKind::Emit { event, .. } => ElementRef::Event(*event),
        TraceStepKind::Deliver { handler, .. } => ElementRef::Handler(*handler),
        TraceStepKind::Fire { rule, .. }
        | TraceStepKind::Spawn { rule, .. }
        | TraceStepKind::NoTarget { rule, .. }
        | TraceStepKind::Ambiguous { rule, .. } => ElementRef::Rule(*rule),
    }
}

/// The element a lifeline stands for (an instance stands for its machine).
pub fn lifeline_element(lifeline: &Lifeline) -> ElementRef {
    match lifeline {
        Lifeline::External { source } => ElementRef::External(*source),
        Lifeline::Instance { machine, .. } => ElementRef::Machine(*machine),
        Lifeline::Controller { controller } => ElementRef::Controller(*controller),
    }
}

/// The stable key a hit target selects, if any. `traces` must have been
/// computed from `model`.
pub fn target_key(target: &HitTarget, model: &Model, traces: &[Trace]) -> Option<ElementKey> {
    match target {
        HitTarget::None | HitTarget::MatrixCell { .. } => None,
        HitTarget::ConnectHandle { element } => Some(element.clone()),
        HitTarget::Element(key) => Some(key.clone()),
        HitTarget::MachineStub { machine, .. } => Some(ElementKey::Machine { machine: machine.clone() }),
        HitTarget::TraceStep { ordering, step } => {
            let trace = traces.get(usize::from(*ordering))?;
            let step = trace.steps.get(usize::try_from(*step).ok()?)?;
            Some(model.key_of(step_element(&step.kind)))
        }
        HitTarget::Lifeline { ordering, lifeline } => {
            let trace = traces.get(usize::from(*ordering))?;
            let lifeline = trace.lifelines.get(usize::try_from(*lifeline).ok()?)?;
            Some(model.key_of(lifeline_element(lifeline)))
        }
    }
}

/// The node for `key` that can be dragged to pin it, with its rect.
pub fn pinnable_node<'a>(scene: &'a Scene, key: &ElementKey) -> Option<&'a SceneNode> {
    scene.nodes.iter().find(|n| matches!(&n.target, HitTarget::Element(k) if k == key))
}

fn bounds_of(points: &[Point]) -> Option<Rect> {
    let first = points.first()?;
    let (mut left, mut top, mut right, mut bottom) = (first.x, first.y, first.x, first.y);
    for p in points {
        left = left.min(p.x);
        top = top.min(p.y);
        right = right.max(p.x);
        bottom = bottom.max(p.y);
    }
    Some(Rect::new(left, top, right - left, bottom - top))
}

/// Where `key` is drawn: its node or lane, else its edge, else an overlay
/// rect, else the stub of its hidden machine.
pub fn locate_key(scene: &Scene, key: &ElementKey) -> Option<Rect> {
    let target = HitTarget::Element(key.clone());
    if let Some(rect) = scene.locate(&target) {
        return Some(rect);
    }
    if let Some(rect) = scene.edges.iter().filter(|e| e.target == target).find_map(|e| bounds_of(&e.points)) {
        return Some(rect);
    }
    let overlay = scene.overlays.iter().find_map(|o| match o {
        Overlay::Rect { rect, target: t, .. } if *t == target => Some(*rect),
        _ => None,
    });
    if overlay.is_some() {
        return overlay;
    }
    let machine = key.machine()?;
    scene.nodes.iter().find_map(|n| match &n.target {
        HitTarget::MachineStub { machine: m, .. } if m == machine => Some(n.rect),
        _ => None,
    })
}

/// Elements closely tied to `element`, to centre on when it is not drawn
/// itself (states are hidden in the causal view, rules are edges there).
pub fn related(model: &Model, element: ElementRef) -> Vec<ElementRef> {
    match element {
        ElementRef::State(s) => {
            let machine = model.state(s).machine;
            model
                .machine(machine)
                .transitions
                .iter()
                .copied()
                .filter(|&t| {
                    let tr = model.transition(t);
                    model.is_ancestor_or_self(s, tr.from) || model.is_ancestor_or_self(s, tr.to)
                })
                .map(ElementRef::Transition)
                .chain(std::iter::once(ElementRef::Machine(machine)))
                .collect()
        }
        ElementRef::Rule(r) => {
            let rule = model.rule(r);
            vec![ElementRef::Handler(rule.handler), ElementRef::Controller(rule.controller)]
        }
        ElementRef::Handler(h) => vec![ElementRef::Controller(model.handler(h).controller)],
        ElementRef::Trigger(t) => {
            let trigger = model.trigger(t);
            trigger
                .accepted_by
                .iter()
                .copied()
                .map(ElementRef::Transition)
                .chain(std::iter::once(ElementRef::Machine(trigger.machine)))
                .collect()
        }
        ElementRef::Controller(c) => model.controller(c).handlers.iter().copied().map(ElementRef::Handler).collect(),
        ElementRef::Machine(_) | ElementRef::Transition(_) | ElementRef::Event(_) | ElementRef::External(_) => {
            Vec::new()
        }
    }
}

/// Where to centre for `key`: the element itself, else the first related
/// element that is drawn.
pub fn locate_with_fallback(scene: &Scene, model: &Model, key: &ElementKey) -> Option<Rect> {
    locate_key(scene, key).or_else(|| {
        let element = model.resolve_key(key)?;
        related(model, element).into_iter().find_map(|r| locate_key(scene, &model.key_of(r)))
    })
}

/// Where to centre for a finding: its primary element, then its other
/// subjects, then anything related to them.
pub fn locate_finding(scene: &Scene, model: &Model, finding: &Finding) -> Option<Rect> {
    let primary = finding.detail.primary();
    std::iter::once(primary)
        .chain(finding.detail.subjects())
        .find_map(|e| locate_key(scene, &model.key_of(e)))
        .or_else(|| locate_with_fallback(scene, model, &model.key_of(primary)))
}

/// The `index`-th race candidate, counting only race candidates in finding
/// order (the meaning of `ViewState::race`).
pub fn race_finding(findings: &[Finding], index: u32) -> Option<&Finding> {
    findings.iter().filter(|f| f.check() == cascade_core::Check::RaceCandidate).nth(usize::try_from(index).ok()?)
}

/// The race index of `finding` among `findings`, if it is a race candidate.
pub fn race_index(findings: &[Finding], finding: &Finding) -> Option<u32> {
    let position =
        findings.iter().filter(|f| f.check() == cascade_core::Check::RaceCandidate).position(|f| f == finding)?;
    u32::try_from(position).ok()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use cascade_core::{FindingDetail, Severity, load_str};
    use cascade_scene::{Arrow, Border, EdgeKind, Emphasis, Rgba, SceneEdge, Shape, Stroke, ViewKind};
    use cascade_sim::{LifelineIx, TraceStep};

    use super::*;

    const SPEC: &str = include_str!("../../../examples/order-fulfillment/cascade.yaml");

    fn node(target: HitTarget, rect: Rect) -> SceneNode {
        SceneNode {
            target,
            shape: Shape::Rect,
            rect,
            fill: None,
            stroke: Stroke::solid(Rgba::hex(0), 1.0),
            border: Border::Single,
            labels: Vec::new(),
            badge: None,
            opacity: 1.0,
            emphasis: Emphasis::Normal,
            diff: None,
        }
    }

    fn edge(target: HitTarget, points: Vec<Point>) -> SceneEdge {
        SceneEdge {
            target,
            kind: EdgeKind::Fire,
            points,
            stroke: Stroke::solid(Rgba::hex(0), 1.0),
            arrow: Arrow::End,
            label: None,
            opacity: 1.0,
            emphasis: Emphasis::Normal,
            back_edge: false,
            diff: None,
        }
    }

    fn key(s: &str) -> ElementKey {
        s.parse().expect("valid key")
    }

    #[test]
    fn hit_targets_map_to_keys() {
        let model = load_str(SPEC).expect("loads");
        let k = key("event:OrderPaid");
        assert_eq!(target_key(&HitTarget::Element(k.clone()), &model, &[]), Some(k));
        assert_eq!(
            target_key(&HitTarget::MachineStub { machine: "Order".into(), links: 3 }, &model, &[]),
            Some(key("machine:Order"))
        );
        let cell = HitTarget::MatrixCell { row: "Order".into(), column: "Shipment".into(), count: 1 };
        assert_eq!(target_key(&cell, &model, &[]), None);
        assert_eq!(target_key(&HitTarget::None, &model, &[]), None);
        assert_eq!(target_key(&HitTarget::TraceStep { ordering: 0, step: 0 }, &model, &[]), None);
    }

    #[test]
    fn trace_steps_and_lifelines_map_to_their_elements() {
        let model = load_str(SPEC).expect("loads");
        let order = model.machine_by_name("Order").expect("Order");
        let paid = model.event_by_name("OrderPaid").expect("event");
        let trace = Trace {
            scenario: "s".into(),
            ordering: None,
            lifelines: vec![Lifeline::Instance { machine: order, name: "o1".into() }],
            steps: vec![TraceStep { cause: None, kind: TraceStepKind::Emit { instance: LifelineIx(0), event: paid } }],
            final_states: BTreeMap::new(),
        };
        let traces = [trace];
        assert_eq!(
            target_key(&HitTarget::TraceStep { ordering: 0, step: 0 }, &model, &traces),
            Some(key("event:OrderPaid"))
        );
        assert_eq!(
            target_key(&HitTarget::Lifeline { ordering: 0, lifeline: 0 }, &model, &traces),
            Some(key("machine:Order"))
        );
        assert_eq!(target_key(&HitTarget::TraceStep { ordering: 1, step: 0 }, &model, &traces), None);
        assert_eq!(target_key(&HitTarget::TraceStep { ordering: 0, step: 7 }, &model, &traces), None);
    }

    #[test]
    fn locate_prefers_nodes_then_edges_then_stubs() {
        let mut scene = Scene::empty(ViewKind::Causal, Rgba::hex(0xFFFFFF));
        let paid = key("event:OrderPaid");
        let rule = key("rule:Fulfillment/OrderPaid#0");
        scene.nodes.push(node(HitTarget::Element(paid.clone()), Rect::new(1.0, 2.0, 3.0, 4.0)));
        scene.edges.push(edge(HitTarget::Element(rule.clone()), vec![Point::new(0.0, 10.0), Point::new(20.0, 30.0)]));
        scene.nodes.push(node(
            HitTarget::MachineStub { machine: "Order".into(), links: 2 },
            Rect::new(50.0, 50.0, 10.0, 10.0),
        ));
        assert_eq!(locate_key(&scene, &paid), Some(Rect::new(1.0, 2.0, 3.0, 4.0)));
        assert_eq!(locate_key(&scene, &rule), Some(Rect::new(0.0, 10.0, 20.0, 20.0)));
        assert_eq!(
            locate_key(&scene, &key("transition:Order:draft->pending@submit")),
            Some(Rect::new(50.0, 50.0, 10.0, 10.0))
        );
        assert_eq!(locate_key(&scene, &key("event:Shipped")), None);
    }

    #[test]
    fn hidden_states_fall_back_to_their_transitions() {
        let model = load_str(SPEC).expect("loads");
        let mut scene = Scene::empty(ViewKind::Causal, Rgba::hex(0xFFFFFF));
        let t = key("transition:Order:pending->paid@capture_ok");
        scene.nodes.push(node(HitTarget::Element(t), Rect::new(10.0, 10.0, 5.0, 5.0)));
        let paid_state = key("state:Order:paid");
        assert_eq!(locate_key(&scene, &paid_state), None);
        assert_eq!(locate_with_fallback(&scene, &model, &paid_state), Some(Rect::new(10.0, 10.0, 5.0, 5.0)));
    }

    #[test]
    fn findings_locate_through_subjects() {
        let model = load_str(SPEC).expect("loads");
        let event = model.event_by_name("OrderCancelled").expect("event");
        let finding = Finding {
            severity: Severity::Warning,
            detail: FindingDetail::UnhandledEvent { event },
            message: "unhandled".into(),
        };
        let mut scene = Scene::empty(ViewKind::Causal, Rgba::hex(0xFFFFFF));
        assert_eq!(locate_finding(&scene, &model, &finding), None);
        scene.nodes.push(node(HitTarget::Element(key("event:OrderCancelled")), Rect::new(0.0, 0.0, 1.0, 1.0)));
        assert_eq!(locate_finding(&scene, &model, &finding), Some(Rect::new(0.0, 0.0, 1.0, 1.0)));
    }

    #[test]
    fn race_indices_count_only_races() {
        let model = load_str(SPEC).expect("loads");
        let event = model.event_by_name("OrderPaid").expect("event");
        let machine = model.machine_by_name("Shipment").expect("machine");
        let rule = model.rule_ids().next().expect("a rule");
        let unhandled = Finding {
            severity: Severity::Warning,
            detail: FindingDetail::UnhandledEvent { event },
            message: String::new(),
        };
        let race = Finding {
            severity: Severity::Info,
            detail: FindingDetail::RaceCandidate { origin: event, machine, first: rule, second: rule },
            message: "race".into(),
        };
        let findings = vec![unhandled.clone(), race.clone()];
        assert_eq!(race_index(&findings, &race), Some(0));
        assert_eq!(race_index(&findings, &unhandled), None);
        assert_eq!(race_finding(&findings, 0), Some(&race));
        assert_eq!(race_finding(&findings, 1), None);
    }
}
