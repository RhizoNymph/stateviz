//! The pinned examples: the spec's order-fulfillment design and the M1 shop
//! design, whose findings must be exactly the planted ones.

use super::*;
use crate::analysis::Check::{
    CascadeCycle, InvalidFire, Nondeterminism, OrphanController, RaceCandidate, StateDependentFire, UnhandledEvent,
    UnreachableState,
};
use crate::analysis::Severity::{Error, Info, Warning};

pub(crate) const SPEC_EXAMPLE: &str = include_str!("../../../../../examples/order-fulfillment/cascade.yaml");
pub(crate) const SHOP: &str = include_str!("../../../../../examples/shop/cascade.yaml");

fn assert_exact(text: &str, expected: &[(Check, &str, Severity)]) {
    let (model, findings) = run(text);
    let actual = summary(&model, &findings);
    let expected: Vec<(Check, String, Severity)> = expected.iter().map(|&(c, k, s)| (c, k.to_owned(), s)).collect();
    assert_eq!(actual, expected, "messages:\n{:#?}", findings.iter().map(|f| &f.message).collect::<Vec<_>>());
}

#[test]
fn order_fulfillment_findings_are_pinned() {
    assert_exact(
        SPEC_EXAMPLE,
        &[
            (UnhandledEvent, "event:OrderCancelled", Warning),
            (UnhandledEvent, "event:Shipped", Warning),
            (UnreachableState, "state:Shipment:shipped", Warning),
            (StateDependentFire, "rule:Fulfillment/OrderPaid#0", Info),
        ],
    );
}

#[test]
fn shop_findings_are_exactly_the_planted_ones() {
    assert_exact(
        SHOP,
        &[
            (InvalidFire, "rule:Fulfillment/OrderCancelled#0", Error),
            (InvalidFire, "external:Customer", Error),
            (Nondeterminism, "state:Inventory:requested", Error),
            (CascadeCycle, "transition:Order:placed->cancelled@cancel", Warning),
            (UnhandledEvent, "event:ShipmentLost", Warning),
            (OrphanController, "handler:Returns/ReturnRequested", Warning),
            (UnreachableState, "state:Order:returned", Warning),
            (RaceCandidate, "rule:Billing/PaymentAuthorized#0", Info),
            (StateDependentFire, "rule:Billing/PaymentAuthorized#0", Info),
            (StateDependentFire, "rule:FraudCheck/PaymentAuthorized#0", Info),
            (StateDependentFire, "rule:Orders/PaymentCaptured#0", Info),
            (StateDependentFire, "rule:Orders/StockReserved#0", Info),
            (StateDependentFire, "rule:Orders/ShipmentDelivered#0", Info),
            (StateDependentFire, "rule:Orders/PaymentRefunded#0", Info),
            (StateDependentFire, "rule:Refunds/OrderCancelled#0", Info),
            (StateDependentFire, "rule:Shipping/ShipmentDispatched#0", Info),
            (StateDependentFire, "rule:Tracking/ShipmentDispatched#0", Info),
            (StateDependentFire, "rule:Tracking/TrackingPolled#0", Info),
            (StateDependentFire, "rule:Returns/ReturnRequested#0", Info),
        ],
    );
}

#[test]
fn shop_exercises_every_check() {
    let (_, findings) = run(SHOP);
    for check in Check::ALL {
        assert!(findings.iter().any(|f| f.check() == check), "no {check} finding in the shop example");
    }
    assert!(findings.iter().any(|f| matches!(f.detail, FindingDetail::DeadExternalTrigger { .. })));
}

#[test]
fn shop_planted_details() {
    let (model, findings) = run(SHOP);
    let subjects_of = |check: Check| keys(&model, &single(&findings, check).detail.subjects());

    assert_eq!(
        subjects_of(Nondeterminism),
        [
            "state:Inventory:requested",
            "transition:Inventory:requested->reserved@reserve",
            "transition:Inventory:requested->backordered@reserve"
        ]
    );
    assert_eq!(
        subjects_of(CascadeCycle),
        [
            "transition:Order:placed->cancelled@cancel",
            "rule:Refunds/OrderCancelled#0",
            "transition:Payment:captured->refunded@refund",
            "rule:Orders/PaymentRefunded#0"
        ]
    );
    let race = single(&findings, RaceCandidate);
    match race.detail {
        FindingDetail::RaceCandidate { origin, machine, .. } => {
            assert_eq!(key(&model, ElementRef::Event(origin)), "event:PaymentAuthorized");
            assert_eq!(key(&model, ElementRef::Machine(machine)), "machine:Payment");
        }
        ref other => panic!("{other:?}"),
    }
    assert_eq!(subjects_of(RaceCandidate), ["rule:Billing/PaymentAuthorized#0", "rule:FraudCheck/PaymentAuthorized#0"]);

    let refund_note = of(&findings, StateDependentFire)
        .into_iter()
        .find(|f| key(&model, f.detail.primary()) == "rule:Refunds/OrderCancelled#0")
        .unwrap_or_else(|| panic!("no refund note"));
    assert_eq!(
        keys(&model, &refund_note.detail.subjects())[1..],
        [
            "state:Payment:created",
            "state:Payment:authorizing",
            "state:Payment:authorized",
            "state:Payment:voided",
            "state:Payment:failed",
            "state:Payment:refunded"
        ]
    );
}
