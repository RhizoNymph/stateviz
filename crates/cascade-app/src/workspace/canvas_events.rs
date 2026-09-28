//! The canvas element and its pointer handling.
//!
//! Window positions become scene positions through the viewport; the scene
//! is hit tested with a pick tolerance of a few screen pixels; gestures are
//! driven by the pure `gesture` state machine.

use cascade_core::ElementKey;
use cascade_scene::HitTarget;
use cascade_sim::Trace;
use gpui::{
    Bounds, Context, CursorStyle, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PinchEvent, Pixels,
    ScrollWheelEvent, Window, canvas, div, prelude::*, px,
};

use super::Workspace;
use super::operations::pins_supported;
use crate::build::connect;
use crate::canvas::paint::{ConnectPaint, PaintInput, paint_scene};
use crate::gesture::{ClickAction, Draggable, Gesture, Mods, MoveOutcome, Pick, ReleaseOutcome, classify_click};
use crate::locate;
use crate::mode::AppMode;
use crate::viewport::{self, ScreenPoint, ScreenRect};

/// Edge pick distance in screen pixels.
const HIT_TOLERANCE_PX: f32 = 5.0;
/// Pixels per line for line-based scroll deltas.
const SCROLL_LINE_PX: f32 = 20.0;

fn screen_rect(bounds: Bounds<Pixels>) -> ScreenRect {
    ScreenRect::new(
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    )
}

fn screen_point(p: gpui::Point<Pixels>) -> ScreenPoint {
    ScreenPoint::new(f32::from(p.x), f32::from(p.y))
}

fn mods(m: &gpui::Modifiers) -> Mods {
    Mods { shift: m.shift, alt: m.alt, secondary: m.secondary() }
}

impl Workspace {
    /// The traces the current scene was built with.
    pub(crate) fn current_traces(&self) -> &[Trace] {
        match self.trace_request() {
            Some(request) => self.trace_run.traces_for(&request),
            None => &[],
        }
    }

    pub(super) fn render_canvas_area(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scene = self.scene.clone();
        let theme = self.theme.clone();
        let mono = self.mono.clone();
        let viewport = self.view.viewport;
        let hover = self.canvas.hover.clone();
        let drag = self.canvas.gesture.drag_preview().map(|(k, p)| (k.clone(), p));
        let connect = self.connect_paint();
        let weak = cx.weak_entity();
        let surface = canvas(
            move |bounds, _window, cx| {
                let rect = screen_rect(bounds);
                if let Err(error) = weak.update(cx, |ws, _| ws.canvas.bounds = Some(rect)) {
                    tracing::debug!(%error, "canvas outlived the workspace");
                }
                rect
            },
            move |_bounds, rect, window, cx| {
                let vp = viewport.unwrap_or_else(|| viewport::fit(scene.bounds, rect));
                let input = PaintInput {
                    scene: &scene,
                    viewport: vp,
                    canvas: rect,
                    theme: &theme,
                    mono,
                    hover: hover.as_ref(),
                    drag: drag.as_ref().map(|(k, p)| (k, *p)),
                    connect: connect.as_ref(),
                };
                paint_scene(&input, window, cx);
            },
        )
        .size_full();

        let cursor = match &self.canvas.gesture {
            Gesture::Panning { .. } | Gesture::Dragging { .. } => CursorStyle::ClosedHand,
            Gesture::Connecting { .. } => CursorStyle::Crosshair,
            _ if self.canvas.hover.is_some() => CursorStyle::PointingHand,
            _ => CursorStyle::Arrow,
        };
        div()
            .relative()
            .flex_1()
            .h_full()
            .min_w_0()
            .overflow_hidden()
            .child(
                div()
                    .id("canvas")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .cursor(cursor)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::canvas_mouse_down))
                    .on_mouse_move(cx.listener(Self::canvas_mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::canvas_mouse_up))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::canvas_mouse_up))
                    .on_scroll_wheel(cx.listener(Self::canvas_scroll))
                    .on_pinch(cx.listener(Self::canvas_pinch))
                    .child(surface),
            )
            .children(self.render_notes(cx))
            .children(self.render_trace_controls(cx))
            .child(self.render_search(window, cx))
    }

    /// What is under `at`: the pick for a click and, when the view supports
    /// pins, the node that a drag would move.
    fn pick_at(&self, at: ScreenPoint) -> (Pick, Option<Draggable>, Option<HitTarget>) {
        let (Some(vp), Some(canvas)) = (self.effective_viewport(), self.canvas.bounds) else {
            return (Pick::Nothing, None, None);
        };
        if !canvas.contains(at) {
            return (Pick::Nothing, None, None);
        }
        let p = viewport::to_scene(vp, canvas, at);
        let Some(target) = self.scene.hit_test(p, HIT_TOLERANCE_PX / vp.zoom) else {
            return (Pick::Nothing, None, None);
        };
        let pick = match target {
            HitTarget::MatrixCell { row, column, .. } => Pick::Pair { row: row.clone(), column: column.clone() },
            other => self
                .doc
                .loaded()
                .and_then(|l| locate::target_key(other, &l.model, self.current_traces()))
                .map_or(Pick::Nothing, Pick::Element),
        };
        let draggable = match target {
            HitTarget::Element(key) if pins_supported(self.view.view) => locate::pinnable_node(&self.scene, key)
                .map(|node| Draggable { key: key.clone(), origin: node.rect.origin }),
            _ => None,
        };
        (pick, draggable, Some(target.clone()))
    }

    fn canvas_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.search.open = false;
        let at = screen_point(event.position);
        let (pick, draggable, target) = self.pick_at(at);
        let mods = mods(&event.modifiers);
        self.canvas.gesture = match target {
            Some(HitTarget::ConnectHandle { element })
                if self.mode == AppMode::Build && connect::is_source(&element) && !mods.any() =>
            {
                Gesture::press_handle(at, element, mods, event.click_count)
            }
            _ => Gesture::press(at, pick, draggable, mods, event.click_count),
        };
        cx.notify();
    }

    /// The element a connect drag would drop on at `at`.
    fn drop_at(&self, at: ScreenPoint) -> Option<ElementKey> {
        let (vp, canvas) = (self.effective_viewport()?, self.canvas.bounds?);
        if !canvas.contains(at) {
            return None;
        }
        let target = self.scene.hit_test(viewport::to_scene(vp, canvas, at), HIT_TOLERANCE_PX / vp.zoom)?;
        locate::drop_key(target).cloned()
    }

    /// The rubber band and drop highlights of a connect drag, in screen
    /// coordinates.
    fn connect_paint(&self) -> Option<ConnectPaint> {
        let (from, start, current) = self.canvas.gesture.connect_preview()?;
        let (vp, canvas) = (self.effective_viewport()?, self.canvas.bounds?);
        let targets: Vec<ScreenRect> = locate::drop_targets(&self.scene, |k| connect::can_connect(from, k))
            .into_iter()
            .map(|(_, rect)| viewport::rect_to_screen(vp, canvas, rect))
            .collect();
        let hot = self
            .drop_at(current)
            .filter(|k| connect::can_connect(from, k))
            .and_then(|k| locate::locate_key(&self.scene, &k))
            .map(|rect| viewport::rect_to_screen(vp, canvas, rect));
        Some(ConnectPaint { from: start, to: current, targets, hot })
    }

    /// A connect drag ended: connect to what is under the pointer, or
    /// cancel on empty canvas.
    fn finish_connect(&mut self, from: ElementKey, at: ScreenPoint, cx: &mut Context<Self>) {
        match self.drop_at(at) {
            Some(to) if connect::can_connect(&from, &to) => self.connect_keys(&from, &to, cx),
            Some(to) => {
                self.set_status(format!("Cannot connect {from} to {to}"), false);
            }
            None => self.set_status("Connection cancelled", false),
        }
    }

    fn canvas_mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let at = screen_point(event.position);
        if self.canvas.gesture.is_active() {
            if !event.dragging() {
                // The button was released somewhere we did not see.
                self.canvas.gesture = Gesture::Idle;
                cx.notify();
                return;
            }
            let zoom = self.effective_viewport().map_or(1.0, |v| v.zoom);
            match self.canvas.gesture.moved(at, zoom) {
                MoveOutcome::Pan(dx, dy) => {
                    if let Some(vp) = self.effective_viewport() {
                        self.view.viewport = Some(viewport::pan_by(vp, dx, dy));
                        cx.notify();
                    }
                }
                MoveOutcome::DragPreview | MoveOutcome::ConnectPreview => cx.notify(),
                MoveOutcome::Nothing => {}
            }
            return;
        }
        let (_, _, hover) = self.pick_at(at);
        if hover != self.canvas.hover {
            self.canvas.hover = hover;
            cx.notify();
        }
    }

    fn canvas_mouse_up(&mut self, _event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        match self.canvas.gesture.release() {
            ReleaseOutcome::Nothing => {}
            ReleaseOutcome::Click { pick, mods, clicks } => self.click(classify_click(pick, mods, clicks), cx),
            ReleaseOutcome::Drop { key, top_left } => self.pin(key, top_left, cx),
            ReleaseOutcome::Connect { from, at } => self.finish_connect(from, at, cx),
        }
        cx.notify();
    }

    fn click(&mut self, action: ClickAction, cx: &mut Context<Self>) {
        match action {
            ClickAction::Select(key) => self.select_key(key, cx),
            ClickAction::AddToSelection(key) => self.add_key(key, cx),
            ClickAction::OpenSource(key) => {
                self.select_key(key.clone(), cx);
                self.open_source(&key, cx);
            }
            ClickAction::Unpin(key) => self.unpin(&key, cx),
            ClickAction::OpenPair { row, column } => self.open_pair(row, column, cx),
            ClickAction::Nothing => {}
        }
    }

    fn canvas_scroll(&mut self, event: &ScrollWheelEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let (Some(vp), Some(canvas)) = (self.effective_viewport(), self.canvas.bounds) else {
            return;
        };
        let delta = event.delta.pixel_delta(px(SCROLL_LINE_PX));
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        self.view.viewport = Some(if event.modifiers.control || event.modifiers.platform {
            viewport::zoom_about(vp, canvas, screen_point(event.position), viewport::wheel_zoom_factor(dy))
        } else if event.modifiers.shift && dx == 0.0 {
            viewport::pan_by(vp, dy, 0.0)
        } else {
            viewport::pan_by(vp, dx, dy)
        });
        cx.notify();
    }

    fn canvas_pinch(&mut self, event: &PinchEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let (Some(vp), Some(canvas)) = (self.effective_viewport(), self.canvas.bounds) else {
            return;
        };
        let factor = viewport::pinch_zoom_factor(event.delta);
        self.view.viewport = Some(viewport::zoom_about(vp, canvas, screen_point(event.position), factor));
        cx.notify();
    }
}
