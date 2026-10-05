//! The pointer while a slate is tilted.
//!
//! The slate's controls hit-test the plan, so while the deck is drawn tilted
//! every pointer position over the panel is mapped back onto it before egui
//! sees it; a position off the panel is sent nowhere. The raw position is
//! kept for the controls drawn on the deck's other faces and for the tilt
//! toggle.

use egui::{Event, PointerButton, Pos2, RawInput, Rect};

use super::ui::tilt::Tilt;
use super::CdjApp;

/// Where a remapped position that lands on no control goes.
const NOWHERE: Pos2 = Pos2::new(-1.0e6, -1.0e6);

/// The pointer as the window saw it, before any remapping.
#[derive(Default, Clone, Copy)]
pub(super) struct RawPointer {
    pub pos: Option<Pos2>,
    pub down: bool,
    /// The primary button went down this frame.
    pub pressed: bool,
}

/// What the input hook needs to map the pointer: the tilt drawn last frame
/// and the panel it was drawn in, the plan's canvas size, and the tops of
/// the parts standing above the panel - on screen, with their heights -
/// frontmost first.
pub(super) struct TiltInput {
    pub tilt: Tilt,
    pub panel: Rect,
    pub canvas: (f32, f32),
    pub tops: Vec<(Vec<Pos2>, f32)>,
}

/// Whether `p` is inside the convex polygon `poly`, either winding.
fn inside(poly: &[Pos2], p: Pos2) -> bool {
    let n = poly.len();
    let side = |i: usize| {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        (b - a).x * (p - a).y - (b - a).y * (p - a).x
    };
    n >= 3 && ((0..n).all(|i| side(i) >= 0.0) || (0..n).all(|i| side(i) <= 0.0))
}

impl CdjApp {
    /// Record the raw pointer and, while tilted, map every position over the
    /// panel back onto its plan. Runs before each frame.
    pub(super) fn tilt_input_hook(&mut self, raw: &mut RawInput) {
        self.raw_pointer.pressed = false;
        let map = self.tilt_input.take();
        let remap = |p: &mut Pos2| {
            let Some(ti) = &map else { return };
            if !ti.panel.contains(*p) {
                return;
            }
            let (w, h) = ti.canvas;
            let deck = match ti.tops.iter().find(|(top, _)| inside(top, *p)) {
                Some((_, z)) => ti.tilt.unproject(*p, *z),
                None => ti.tilt.unproject_surface(*p),
            };
            *p = match deck {
                Some(d) if (0.0..=w).contains(&d.x) && (0.0..=h).contains(&d.y) => {
                    ti.tilt.plan_screen(d)
                }
                _ => NOWHERE,
            };
        };
        for ev in &mut raw.events {
            match ev {
                Event::PointerMoved(p) => {
                    self.raw_pointer.pos = Some(*p);
                    remap(p);
                }
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    ..
                } => {
                    self.raw_pointer.pos = Some(*pos);
                    self.raw_pointer.pressed |= *pressed && !self.raw_pointer.down;
                    self.raw_pointer.down = *pressed;
                    remap(pos);
                }
                Event::PointerButton { pos, .. } | Event::Touch { pos, .. } => remap(pos),
                Event::PointerGone | Event::WindowFocused(false) => {
                    self.raw_pointer = RawPointer::default();
                }
                _ => {}
            }
        }
        self.tilt_input = map;
    }
}
