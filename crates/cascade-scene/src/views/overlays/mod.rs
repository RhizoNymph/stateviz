//! Build and play decorations over a laid-out scene: connect handles (edit
//! mode), instance markers, the last step (`active`) and the queue
//! (`pending`).
//!
//! Like emphasis, everything here runs after layout and only adds overlays
//! or changes outline weight and dash, so a play overlay can never
//! relayout (the layout cache key never sees it) and never changes a hue.
//!
//! ```text
//! realized scene + metas ──handles::add──▶ ConnectHandle circles   (structure, edit mode)
//!                        ──PlayDecor::apply──▶ active: outline weight + glow ring (Under)
//!                                              pending: dotted outline + queue-position chip
//!                                              markers: instance chips on the top edge
//! ```

mod chip;
mod handles;
mod markers;
mod queue;

use cascade_core::Model;

pub(crate) use handles::add_handles;
pub(crate) use markers::Placement;

use crate::play::PlayOverlay;
use crate::scene::Scene;
use crate::views::draft::{EdgeInfo, Meta};
use crate::views::style::Painter;

/// Everything the play decorations read.
pub(crate) struct PlayDecor<'a> {
    pub model: &'a Model,
    pub painter: &'a Painter<'a>,
}

impl PlayDecor<'_> {
    /// Draw `play` over `scene`. `nodes` and `edges` are aligned with
    /// `scene.nodes` and `scene.edges`. Keys that name nothing in the model,
    /// or nothing drawn, are ignored.
    pub fn apply(&self, scene: &mut Scene, nodes: &[Meta], edges: &[EdgeInfo], play: &PlayOverlay, place: Placement) {
        queue::active(self, scene, nodes, edges, &play.active);
        queue::pending(self, scene, nodes, edges, &play.pending);
        markers::draw(self, scene, &play.markers, place);
    }
}
