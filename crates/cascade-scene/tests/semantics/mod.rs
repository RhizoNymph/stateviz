//! Fixtures for the subscription, timer, choice and `sets` drawing tests:
//! a small checkout saga and a hand-made trace of it.

#![allow(dead_code)]

use std::collections::BTreeMap;

use cascade_core::expr::Duration;
use cascade_core::{ElementRef, Model};
use cascade_scene::{EdgeKind, HitTarget, Label, Overlay, Scene, SceneEdge, SceneNode};
use cascade_sim::{Lifeline, LifelineIx, StepIx, Trace, TraceStep, TraceStepKind};

pub const SAGA: &str = include_str!("../fixtures/saga.yaml");

/// Transition keys of the saga.
pub const CHECKOUT: &str = "transition:Order:cart->placed@checkout";
pub const CONFIRMED: &str = "transition:Order:placed->confirmed@PaymentCaptured";
pub const ORDER_FAILED: &str = "transition:Order:placed->cancelled@PaymentFailed";
pub const EXPIRED: &str = "transition:Order:placed->cancelled@after-30m";
pub const CHARGING: &str = "transition:Payment:idle->charging@OrderPlaced";
pub const CAPTURED: &str = "transition:Payment:charging->captured@settle";
pub const DECLINED: &str = "transition:Payment:charging->failed@settle";
pub const RETRY: &str = "transition:Payment:charging->charging@after-10s";
pub const SEND: &str = "transition:Receipt:none->sent@send";

fn key(s: &str) -> cascade_core::ElementKey {
    s.parse().unwrap_or_else(|_| panic!("bad key {s}"))
}

fn resolve(model: &Model, s: &str) -> ElementRef {
    model.resolve_key(&key(s)).unwrap_or_else(|| panic!("no element {s}"))
}

fn step(cause: Option<u32>, kind: TraceStepKind) -> TraceStep {
    TraceStep { cause: cause.map(StepIx), kind }
}

/// Lifelines of [`saga_trace`].
pub const SHOPPER: LifelineIx = LifelineIx(0);
pub const BANK: LifelineIx = LifelineIx(1);
pub const O1: LifelineIx = LifelineIx(2);
pub const P1: LifelineIx = LifelineIx(3);
pub const R1: LifelineIx = LifelineIx(4);
pub const MAILER: LifelineIx = LifelineIx(5);

/// Step indices of [`saga_trace`] worth naming.
pub const NOTIFY_PLACED: u32 = 3;
pub const SET_ORDER_ID: u32 = 5;
pub const WAIT: u32 = 6;
pub const TIMER: u32 = 7;
pub const CHOICE: u32 = 10;
pub const NOTIFY_CAPTURED: u32 = 13;

/// The saga, run by hand: checkout, Payment picks the order up by
/// subscription and records its id, a retry timer fires after a wait, the
/// bank settles with a defaulted choice, and Order confirms by
/// subscription.
pub fn saga_trace(model: &Model) -> Trace {
    let t = |k: &str| match resolve(model, k) {
        ElementRef::Transition(t) => t,
        other => panic!("{k} is {other:?}"),
    };
    let s = |k: &str| match resolve(model, k) {
        ElementRef::State(s) => s,
        other => panic!("{k} is {other:?}"),
    };
    let trig = |k: &str| match resolve(model, k) {
        ElementRef::Trigger(t) => t,
        other => panic!("{k} is {other:?}"),
    };
    let ev = |k: &str| match resolve(model, k) {
        ElementRef::Event(e) => e,
        other => panic!("{k} is {other:?}"),
    };
    let machine = |n: &str| model.machine_by_name(n).unwrap_or_else(|| panic!("no machine {n}"));
    let transition = |instance, k: &str, from: &str, to: &str| TraceStepKind::Transition {
        instance,
        transition: t(k),
        from: s(from),
        to: s(to),
    };
    Trace {
        scenario: "checkout saga".into(),
        ordering: None,
        lifelines: vec![
            Lifeline::External { source: model.external_by_name("Shopper").expect("shopper") },
            Lifeline::External { source: model.external_by_name("Bank").expect("bank") },
            Lifeline::Instance { machine: machine("Order"), name: "o1".into() },
            Lifeline::Instance { machine: machine("Payment"), name: "p1".into() },
            Lifeline::Instance { machine: machine("Receipt"), name: "r1".into() },
            Lifeline::Controller { controller: model.controller_by_name("Mailer").expect("mailer") },
        ],
        steps: vec![
            // 0
            step(
                None,
                TraceStepKind::ExternalFire { source: SHOPPER, target: O1, trigger: trig("trigger:Order.checkout") },
            ),
            // 1
            step(Some(0), transition(O1, CHECKOUT, "state:Order:cart", "state:Order:placed")),
            // 2
            step(Some(1), TraceStepKind::Emit { instance: O1, event: ev("event:OrderPlaced") }),
            // 3
            step(Some(2), TraceStepKind::Notify { instance: P1, event: ev("event:OrderPlaced") }),
            // 4
            step(Some(3), transition(P1, CHARGING, "state:Payment:idle", "state:Payment:charging")),
            // 5
            step(
                Some(4),
                TraceStepKind::SetFields {
                    instance: P1,
                    fields: BTreeMap::from([("orderId".to_owned(), "o-1".to_owned())]),
                },
            ),
            // 6
            step(None, TraceStepKind::Wait { by: Duration::from_secs(10), now: Duration::from_secs(10) }),
            // 7
            step(Some(6), TraceStepKind::TimerExpired { instance: P1, trigger: trig("trigger:Payment.after-10s") }),
            // 8
            step(Some(7), transition(P1, RETRY, "state:Payment:charging", "state:Payment:charging")),
            // 9
            step(
                None,
                TraceStepKind::ExternalFire { source: BANK, target: P1, trigger: trig("trigger:Payment.settle") },
            ),
            // 10
            step(
                Some(9),
                TraceStepKind::Choice {
                    at: P1,
                    options: vec!["card accepted".into(), "else".into()],
                    chosen: "card accepted".into(),
                    defaulted: true,
                },
            ),
            // 11
            step(Some(10), transition(P1, CAPTURED, "state:Payment:charging", "state:Payment:captured")),
            // 12
            step(Some(11), TraceStepKind::Emit { instance: P1, event: ev("event:PaymentCaptured") }),
            // 13
            step(Some(12), TraceStepKind::Notify { instance: O1, event: ev("event:PaymentCaptured") }),
            // 14
            step(Some(13), transition(O1, CONFIRMED, "state:Order:placed", "state:Order:confirmed")),
        ],
        final_states: BTreeMap::from([
            (O1, s("state:Order:confirmed")),
            (P1, s("state:Payment:captured")),
            (R1, s("state:Receipt:none")),
        ]),
    }
}

pub fn target(key: &str) -> HitTarget {
    HitTarget::Element(self::key(key))
}

pub fn node<'a>(scene: &'a Scene, key: &str) -> &'a SceneNode {
    let t = target(key);
    scene.nodes.iter().find(|n| n.target == t).unwrap_or_else(|| panic!("no node {key}"))
}

pub fn label_texts(node: &SceneNode) -> Vec<&str> {
    node.labels.iter().map(|l| l.text.as_str()).collect()
}

pub fn edges_of(scene: &Scene, kind: EdgeKind) -> Vec<&SceneEdge> {
    scene.edges.iter().filter(|e| e.kind == kind).collect()
}

/// Edge labels (text) with their edges, for finding an edge by its label.
pub fn labelled<'a>(scene: &'a Scene, text: &str) -> Vec<&'a SceneEdge> {
    scene.edges.iter().filter(|e| e.label.as_ref().is_some_and(|l| l.text == text)).collect()
}

pub fn label_of(edge: &SceneEdge) -> &Label {
    edge.label.as_ref().unwrap_or_else(|| panic!("unlabelled edge {edge:?}"))
}

/// Overlay texts.
pub fn overlay_texts(scene: &Scene) -> Vec<&Label> {
    scene
        .overlays
        .iter()
        .filter_map(|o| match o {
            Overlay::Text { label, .. } => Some(label),
            _ => None,
        })
        .collect()
}
