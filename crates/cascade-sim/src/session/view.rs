//! What a host shows about the session's current state: instances, the
//! queue and the triggers the player could fire.

use cascade_core::Model;

use super::{AvailableFire, InstanceState, PendingId, PendingItem, PendingKind, PlaySession};
use crate::engine::core::QueueItem;
use crate::engine::drive::trigger_ref;
use crate::engine::instance::InstanceIx;
use crate::trace::{LifelineIx, StepIx};

impl PlaySession {
    fn lifeline_of(&self, ix: InstanceIx) -> LifelineIx {
        self.lifelines.get(ix.0).copied().unwrap_or(LifelineIx(u32::MAX))
    }

    /// Live instances (removed ones left out) in lifeline order, with their
    /// current states.
    pub fn instances(&self) -> Vec<InstanceState> {
        let mut out: Vec<InstanceState> = self
            .core
            .instances()
            .iter()
            .enumerate()
            .filter(|(_, instance)| !instance.removed)
            .map(|(i, instance)| InstanceState {
                lifeline: self.lifeline_of(InstanceIx(i)),
                name: instance.name.clone(),
                machine: instance.machine,
                state: instance.leaf(),
                fields: instance.fields.clone(),
            })
            .collect();
        out.sort_by_key(|i| i.lifeline);
        out
    }

    /// The queue, head first. `Step { choice: Some(i) }` delivers item `i`.
    pub fn pending(&self) -> Vec<PendingItem> {
        self.core
            .queue()
            .iter()
            .map(|queued| PendingItem {
                id: queued.id,
                kind: match &queued.item {
                    QueueItem::Event { event, .. } => PendingKind::Event { event: *event },
                    QueueItem::Fire { rule, target, .. } => {
                        PendingKind::Fire { rule: *rule, target: self.lifeline_of(*target) }
                    }
                },
                cause: StepIx(u32::try_from(queued.item.cause()).unwrap_or(u32::MAX)),
                label: queued.label.clone(),
            })
            .collect()
    }

    /// Whether the queue item `id` is still queued.
    pub fn is_pending(&self, id: PendingId) -> bool {
        self.core.queue().iter().any(|q| q.id == id)
    }

    /// Every external trigger × live instance of the trigger's machine: for
    /// each source in model order, each trigger it lists, each instance in
    /// lifeline order, with whether the instance's current state accepts
    /// the trigger (`Model::enabled_transitions`).
    pub fn available_fires(&self, model: &Model) -> Vec<AvailableFire> {
        let instances = self.instances();
        let mut out = Vec::new();
        for (_, source) in model.externals() {
            for &trigger in &source.triggers {
                let machine = model.trigger(trigger).machine;
                for instance in instances.iter().filter(|i| i.machine == machine) {
                    out.push(AvailableFire {
                        source: source.name.clone(),
                        trigger: trigger_ref(model, trigger),
                        target: instance.name.clone(),
                        accepted: !model.enabled_transitions(instance.state, trigger).is_empty(),
                    });
                }
            }
        }
        out
    }
}
