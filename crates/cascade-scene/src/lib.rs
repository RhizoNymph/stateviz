//! Cascade scenes: turns a model plus view state into a backend-neutral
//! display list ([`Scene`]) for the four views, and exports scenes as SVG and
//! PNG.
//!
//! ```text
//! Model + CausalGraph + Findings + ViewState + Theme ──SceneBuilder──▶ Scene
//!                                                                        │
//!                              GPUI app (paint + hit test) ◀─────────────┤
//!                              to_svg / to_png ◀─────────────────────────┘
//! ```
//!
//! Every visual decision is made here, so the GPUI app, SVG export and any
//! future web frontend draw the same picture.

pub mod color;
pub mod emphasis;
pub mod export;
pub mod pins;
pub mod scene;
pub mod text;
pub mod view_state;
pub mod views;

pub use color::{MachineStyle, Rgba, Theme, ThemeMode, machine_colors, machine_styles};
pub use export::{ExportError, to_png, to_svg};
pub use pins::{LayoutSidecar, SidecarError, load_sidecar, save_sidecar, sidecar_path};
pub use scene::{
    Arrow, Badge, Border, Dash, EdgeKind, Emphasis, FontWeight, HitTarget, Label, Lane, Layer, Overlay, Scene,
    SceneEdge, SceneNode, Shape, Stroke,
};
pub use text::{MonoMeasure, TextMeasure};
pub use view_state::{ConeFocus, DiffRefs, OutsideFocus, ViewKind, ViewLinkError, ViewState, Viewport};
pub use views::{SceneBuilder, SceneError, SceneInput};
