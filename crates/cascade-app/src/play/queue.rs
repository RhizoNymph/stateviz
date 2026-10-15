//! The play panel's queue rows: what clicking an item delivers, and the
//! per-consumer buttons of an event that several consumers still have to
//! receive.
//!
//! Pure.

use cascade_sim::{PendingItem, PendingKind};

use super::choices::StepRequest;

/// Deliver an event to one of its remaining consumers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsumerItem {
    /// "→ Fulfillment".
    pub label: String,
    pub request: StepRequest,
}

/// One queue row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueRow {
    /// "head", "2", "3", …
    pub position: String,
    pub label: String,
    /// Clicking the row: deliver to every remaining consumer.
    pub request: StepRequest,
    /// "waiting for: Fulfillment, o1", when more than one consumer is left.
    pub waiting: Option<String>,
    /// One button per remaining consumer, when there are several (an event
    /// delivered to one consumer stays queued for the others).
    pub consumers: Vec<ConsumerItem>,
}

/// The queue's rows. The head is addressed as `choice: None`, like Space.
pub fn queue_rows(pending: &[PendingItem]) -> Vec<QueueRow> {
    pending
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let choice = if index == 0 { None } else { u32::try_from(index).ok() };
            let several = matches!(item.kind, PendingKind::Event { .. }) && item.consumers_left.len() > 1;
            let consumers = if several {
                item.consumers_left
                    .iter()
                    .map(|consumer| ConsumerItem {
                        label: format!("→ {consumer}"),
                        request: StepRequest { choice, consumer: Some(consumer.clone()) },
                    })
                    .collect()
            } else {
                Vec::new()
            };
            QueueRow {
                position: if index == 0 { "head".to_owned() } else { (index + 1).to_string() },
                label: item.label.clone(),
                request: StepRequest { choice, consumer: None },
                waiting: several.then(|| format!("waiting for: {}", item.consumers_left.join(", "))),
                consumers,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use cascade_sim::PendingId;

    use super::*;

    fn model() -> cascade_core::Model {
        cascade_core::load_str(
            "\
machines:
  Order:
    states: [a, b]
    transitions:
      - { from: a, to: b, on: go, emits: [Done] }
controllers:
  C1: { on: { Done: [ { fire: Order.go } ] } }
",
        )
        .expect("loads")
    }

    fn items() -> Vec<PendingItem> {
        let m = model();
        let event = m.event_by_name("Done").expect("event");
        let rule = m.rules().next().map(|(id, _)| id).expect("rule");
        let template = |kind: PendingKind, label: &str, consumers: &[&str], id: u32| PendingItem {
            id: PendingId(id),
            kind,
            cause: cascade_sim::StepIx(0),
            label: label.into(),
            consumers_left: consumers.iter().map(|c| (*c).to_owned()).collect(),
        };
        vec![
            template(PendingKind::Event { event }, "Done from o1", &["C1", "s1"], 0),
            template(PendingKind::Event { event }, "Done from o2", &["C1"], 1),
            template(PendingKind::Fire { rule, target: cascade_sim::LifelineIx(0) }, "C1 → o1: go", &["o1"], 2),
        ]
    }

    #[test]
    fn rows_address_the_head_like_space() {
        let rows = queue_rows(&items());
        assert_eq!(rows.iter().map(|r| r.position.as_str()).collect::<Vec<_>>(), ["head", "2", "3"]);
        assert_eq!(rows[0].request, StepRequest { choice: None, consumer: None });
        assert_eq!(rows[2].request, StepRequest { choice: Some(2), consumer: None });
        assert_eq!(rows[1].label, "Done from o2");
    }

    #[test]
    fn events_with_several_consumers_get_one_button_each() {
        let rows = queue_rows(&items());
        assert_eq!(
            rows[0].consumers,
            [
                ConsumerItem {
                    label: "→ C1".into(),
                    request: StepRequest { choice: None, consumer: Some("C1".into()) }
                },
                ConsumerItem {
                    label: "→ s1".into(),
                    request: StepRequest { choice: None, consumer: Some("s1".into()) }
                },
            ]
        );
        assert_eq!(rows[0].waiting.as_deref(), Some("waiting for: C1, s1"));
        assert!(rows[1].consumers.is_empty() && rows[1].waiting.is_none(), "one consumer: the row is enough");
        assert!(rows[2].consumers.is_empty(), "fires have one consumer");
    }

    #[test]
    fn later_items_keep_their_position() {
        let mut pending = items();
        pending.swap(0, 1);
        let rows = queue_rows(&pending);
        assert_eq!(rows[1].consumers[1].request, StepRequest { choice: Some(1), consumer: Some("s1".into()) });
        assert!(queue_rows(&[]).is_empty());
    }
}
