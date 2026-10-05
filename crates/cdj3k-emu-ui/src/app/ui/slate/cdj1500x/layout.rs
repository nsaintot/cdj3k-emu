//! Geometry of the CDJ-1500X reference canvas (`super::REF_W` x `super::REF_H`),
//! traced off the plan drawing in drawing pixels and converted by [`px`].
//!
//! The drawing is a true plan view: the jog's outer ring fits a circle to half
//! a pixel and spans 886 x 885 px, so its pixels map to the canvas by one
//! scale on both axes.

use egui::{Pos2, Rect, Vec2};

/// The drawing's size, in its own pixels.
pub const DRAWING_W: f32 = 1430.0;
pub const DRAWING_H: f32 = 2125.0;

/// Reference units per drawing pixel.
const PX: f32 = super::REF_W / DRAWING_W;

/// A drawing-pixel length or coordinate, in reference units.
pub const fn px(v: f32) -> f32 {
    v * PX
}

// ── Cabinet ───────────────────────────────────────────────────────────────────
/// Corner radius of the cabinet's outline.
pub const CABINET_R: f32 = px(16.0);

/// The rear lip: a band across the back, between the outline's rear edge and
/// the line where the housing's top begins. Its sides stand in from the
/// outline by [`LIP_INSET`]; the housing's front corners round off below the
/// line with [`CABINET_R`].
pub const LIP_BOT: f32 = px(34.0);
pub const LIP_INSET: f32 = px(17.0);

// ── Screen housing ────────────────────────────────────────────────────────────
/// The slab the LCD sits in: a rounded rectangle, with a bevel line
/// [`SLAB_BEVEL`] inside its edge.
pub const SLAB: Rect = Rect::from_min_max(
    Pos2::new(px(17.0), px(48.0)),
    Pos2::new(px(1412.0), px(970.0)),
);
pub const SLAB_R: f32 = px(8.0);
pub const SLAB_BEVEL: f32 = px(7.0);
/// The bevel line's pen, in reference units.
pub const SLAB_BEVEL_STROKE: f32 = 2.0;

/// The housing's front edge, running the cabinet's whole width.
pub const HOUSING_FRONT: f32 = px(968.0);

/// The display aperture, as the drawing outlines it.
pub const GLASS: Rect = Rect::from_min_max(
    Pos2::new(px(98.0), px(131.0)),
    Pos2::new(px(1331.0), px(891.0)),
);

/// The panel is 1280 x 800.
const LCD_ASPECT: f32 = 1280.0 / 800.0;

/// The LCD's bezel: the aperture's width, at the panel's own aspect, on the
/// aperture's centre (the drawing's aperture is 1.6 % shallower than 16:10).
///
/// `View > Extend Screen` grows it to the slab's whole width inside the bevel
/// line, which stays visible; at 16:10 that is the tighter bound, the slab
/// being deeper than the panel then is.
pub fn lcd_bezel(extended: bool) -> Rect {
    let w = if extended {
        SLAB.shrink(SLAB_BEVEL + SLAB_BEVEL_STROKE).width()
    } else {
        GLASS.width()
    };
    Rect::from_center_size(GLASS.center(), Vec2::new(w, w / LCD_ASPECT))
}
