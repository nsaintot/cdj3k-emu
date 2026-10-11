//! Shape caches, which keep unchanged geometry from being rebuilt on every
//! frame.
//!
//! # Strategy
//! The panel draws three categories of shapes:
//!
//! | Category | Examples | Frequency |
//! |---|---|---|
//! | **Dynamic** | Jog pitchbend grip rotation | Every jog frame (~60 fps) |
//! | **Semi-static** | JOG ADJUST gear/indicator, tempo knob | Only on user drag |
//! | **Static** | All buttons, labels, borders, outer jog rings | Only on resize |
//!
//! [`ShapeList`] mirrors the painter API: static draw calls push into it
//! instead of into the egui painter. The collected `Vec<Shape>` is stored in a
//! cache and replayed each frame with `painter.extend(...)`, so the geometry is
//! recomputed only when the cache is stale.
//!
//! # Jog wheel split
//! The pitchbend grip sits between the outer body fill and the inner LCD disk,
//! so the static geometry is split into three lists:
//!
//! ```text
//! painter.extend(outer)           - jog outer rings
//! paint_pitchbend_grip(…)         - always recomputed (dynamic rotation)
//! painter.extend(inner_mid)       - platter and inner LCD background disk
//! paint_jog_lcd_stream_texture(…) - jog framebuffer, clipped to the inner LCD silhouette
//! painter.extend(inner_over)      - LCD chrome (stroke, MODE, BEAT SYNC, separators), JOG ADJUST
//! paint_jog_inner_lcd_corner_labels - SLIP, VINYL, SYNC, MASTER in the stream's corner colors
//! ```
//!
//! All three lists share one key, [`JogCacheKey`]: the layout, the
//! pixels-per-point and the JOG ADJUST step. The step is in the key because the
//! gear and indicator in `inner_over` rotate with it.

use std::collections::HashMap;

use egui::{Align2, Color32, FontId, Pos2, Rect, Shape, Stroke};

// ── ShapeList ─────────────────────────────────────────────────────────────────

/// Collects [`egui::Shape`]s for cached rendering. It mirrors the
/// [`egui::Painter`] draw API for static, non-interactive calls.
#[derive(Default)]
pub struct ShapeList(Vec<Shape>);

impl ShapeList {
    pub fn into_shapes(self) -> Vec<Shape> {
        self.0
    }

    // ── Painter-mirroring primitives ─────────────────────────────────────────

    pub fn circle_filled(&mut self, center: Pos2, radius: f32, fill: Color32) {
        self.0.push(Shape::circle_filled(center, radius, fill));
    }

    pub fn circle_stroke(&mut self, center: Pos2, radius: f32, stroke: impl Into<Stroke>) {
        self.0
            .push(Shape::circle_stroke(center, radius, stroke.into()));
    }

    pub fn rect_filled(&mut self, rect: Rect, rounding: f32, fill: Color32) {
        self.0.push(Shape::rect_filled(rect, rounding, fill));
    }

    pub fn rect_stroke(&mut self, rect: Rect, rounding: f32, stroke: impl Into<Stroke>) {
        self.0.push(Shape::rect_stroke(
            rect,
            rounding,
            stroke.into(),
            egui::StrokeKind::Middle,
        ));
    }

    pub fn line_segment(&mut self, pts: [Pos2; 2], stroke: impl Into<Stroke>) {
        self.0.push(Shape::line_segment(pts, stroke.into()));
    }

    pub fn add(&mut self, shape: Shape) {
        self.0.push(shape);
    }

    /// Lays out `text` with the egui font system and collects the resulting
    /// [`Shape::galley`]. It needs an [`egui::Context`] for the fonts.
    pub fn text(
        &mut self,
        ctx: &egui::Context,
        pos: Pos2,
        anchor: Align2,
        text: &str,
        font_id: FontId,
        color: Color32,
    ) {
        let galley = ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font_id, color));
        let top_left = anchor.anchor_size(pos, galley.size()).min;
        self.0.push(Shape::galley(top_left, galley, color));
    }

    /// Fits a single-line caption to a given ink box. The box's height sets
    /// the font size, its width sets the letter spacing, and the ink starts at
    /// the box's top-left corner.
    pub fn text_in_ink_box(
        &mut self,
        ctx: &egui::Context,
        ink: Rect,
        text: &str,
        family: egui::FontFamily,
        color: Color32,
    ) {
        let layout = |size: f32, tracking: f32| {
            let mut job = egui::text::LayoutJob::single_section(
                text.to_owned(),
                egui::TextFormat {
                    font_id: FontId::new(size, family.clone()),
                    color,
                    extra_letter_spacing: tracking,
                    ..Default::default()
                },
            );
            job.wrap.max_width = f32::INFINITY;
            ctx.fonts_mut(|f| f.layout_job(job))
        };
        let probe = layout(100.0, 0.0);
        let size = 100.0 * ink.height() / probe.mesh_bounds.height().max(1.0);
        let natural = layout(size, 0.0);
        let gaps = text.chars().count().saturating_sub(1).max(1) as f32;
        let tracking = (ink.width() - natural.mesh_bounds.width()) / gaps;
        let galley = layout(size, tracking);
        let top_left = ink.min - galley.mesh_bounds.min.to_vec2();
        self.0.push(Shape::galley(top_left, galley, color));
    }

    /// Centers the visible ink of `text` at `center` and snaps it to the
    /// device pixel grid. Use it for single-glyph labels such as icons, arrows
    /// and dots, where centering on the line box drifts visibly with the font
    /// metrics. For longer labels, use [`Self::text`] with
    /// [`Align2::CENTER_CENTER`].
    pub fn text_centered_ink(
        &mut self,
        ctx: &egui::Context,
        center: Pos2,
        text: &str,
        font_id: FontId,
        color: Color32,
    ) {
        let galley = ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font_id, color));
        let ppp = ctx.pixels_per_point();
        let raw = center - galley.mesh_bounds.center().to_vec2();
        let snapped = Pos2::new((raw.x * ppp).round() / ppp, (raw.y * ppp).round() / ppp);
        self.0.push(Shape::galley(snapped, galley, color));
    }
}

// ── Jog wheel static cache ────────────────────────────────────────────────────

/// The cache key for the jog wheel section.
///
/// The layout triple (`ox`, `oy`, `scale`) changes when the window is resized
/// or moved. `jog_adjust_step` is the rotation of the JOG ADJUST gear and
/// indicator, stored as one of 13 steps, 0 to 12, to avoid float noise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JogCacheKey {
    pub layout: (i32, i32, i32),
    pub jog_adjust_step: u8,
    pub ppp: i32,
}

impl JogCacheKey {
    pub fn new(ox: f32, oy: f32, scale: f32, jog_adjust: f32, ppp: f32) -> Self {
        let q = |v: f32| (v * 1000.0).round() as i32;
        let step = (jog_adjust.clamp(0.0, 1.0) * 12.0).round() as u8;
        Self {
            layout: (q(ox), q(oy), q(scale)),
            jog_adjust_step: step,
            ppp: (ppp * 100.0).round() as i32,
        }
    }
}

/// Cached jog wheel geometry, split around the pitchbend grip, which is
/// redrawn on every frame.
///
/// Draw order every frame:
/// 1. `painter.extend(outer)`: the silver and body outer disks and the ring border.
/// 2. `paint_pitchbend_grip(angle)`, which is always recomputed.
/// 3. `painter.extend(inner_mid)`: the touch platter, its border and the inner LCD background disk.
/// 4. The jog stream texture, a mesh clipped to the inner LCD silhouette.
/// 5. `painter.extend(inner_over)`: the inner LCD outline, MODE, BEAT SYNC and JOG ADJUST.
/// 6. `paint_jog_inner_lcd_corner_labels`: SLIP, VINYL, SYNC and MASTER.
pub struct JogStaticCache {
    pub key: JogCacheKey,
    /// Shapes drawn before the pitchbend grip.
    pub outer: Vec<Shape>,
    /// Shapes drawn after the grip and below the stream texture: the platter,
    /// its border and the inner LCD's filled disk.
    pub inner_mid: Vec<Shape>,
    /// Shapes drawn after the jog LCD texture: the LCD chrome and the JOG
    /// ADJUST assembly.
    pub inner_over: Vec<Shape>,
}

// ── Generic ShapeCache<K> ─────────────────────────────────────────────────────

/// A shape cache that rebuilds its `Vec<Shape>` only when `key` changes.
///
/// Use it for elements that depend on a known set of values, such as LED bytes,
/// a slider value or a set of corner-label colors, and need no per-widget ID.
pub struct ShapeCache<K: PartialEq + Clone> {
    key: Option<K>,
    shapes: Vec<Shape>,
}

impl<K: PartialEq + Clone> Default for ShapeCache<K> {
    fn default() -> Self {
        Self {
            key: None,
            shapes: Vec::new(),
        }
    }
}

impl<K: PartialEq + Clone> ShapeCache<K> {
    /// Returns the cached shapes, rebuilt first if `key` changed.
    pub fn get_or_build(&mut self, key: K, build: impl FnOnce(&mut ShapeList)) -> &[Shape] {
        if self.key.as_ref() != Some(&key) {
            let mut list = ShapeList::default();
            build(&mut list);
            self.shapes = list.into_shapes();
            self.key = Some(key);
        }
        &self.shapes
    }
}

// ── BtnShapeCache ─────────────────────────────────────────────────────────────

/// The per-widget cache key: the quantized screen rect, the packed visual state
/// and the pressed flag.
///
/// The rect is multiplied by 100 and rounded, which removes float noise but
/// still catches sub-pixel layout changes from a window resize. `state` packs up
/// to 4 bytes of MOSI LED or color data, such as the bytes of
/// `Color32::to_array()` or a raw LED byte.
#[derive(PartialEq, Eq, Clone)]
pub struct BtnCacheKey {
    /// [min_x, min_y, max_x, max_y] in screen pixels, multiplied by 100 and rounded.
    rect: [i32; 4],
    /// Packed visual state: up to 4 MOSI bytes or a `Color32` RGBA.
    pub state: u32,
    /// Whether the button is visually pressed (pointer down or latched).
    pub pressed: bool,
    /// pixels_per_point multiplied by 100 and rounded, so that a DPI change
    /// invalidates the key.
    ppp: i32,
}

impl BtnCacheKey {
    pub fn new(rect: Rect, state: u32, pressed: bool, ppp: f32) -> Self {
        let q = |v: f32| (v * 100.0).round() as i32;
        Self {
            rect: [q(rect.min.x), q(rect.min.y), q(rect.max.x), q(rect.max.y)],
            state,
            pressed,
            ppp: (ppp * 100.0).round() as i32,
        }
    }
}

/// The cache for interactive buttons. Each button is identified by its egui
/// [`Id`](egui::Id), which is unique per call site, so every button in the UI shares one
/// cache map.
///
/// A button's shapes are rebuilt only when its key changes: its layout, its LED
/// color or its pressed state.
#[derive(Default)]
pub struct BtnShapeCache {
    map: HashMap<egui::Id, (BtnCacheKey, Vec<Shape>)>,
}

impl BtnShapeCache {
    /// Returns the shapes for `id`, built on the first call and rebuilt when
    /// `key` changes.
    pub fn get_or_build(
        &mut self,
        id: egui::Id,
        key: BtnCacheKey,
        build: impl FnOnce(&mut ShapeList),
    ) -> &[Shape] {
        if self.map.get(&id).is_none_or(|(k, _)| k != &key) {
            let mut list = ShapeList::default();
            build(&mut list);
            self.map.insert(id, (key, list.into_shapes()));
        }
        &self.map[&id].1
    }
}

// ── StaticShapeCache ──────────────────────────────────────────────────────────

/// A shape cache for static regions such as the chassis, labels and
/// separators. It is keyed on the quantized `(ox, oy, scale, ppp)` tuple and is
/// rebuilt on a resize or a DPI change.
#[derive(Default)]
pub struct StaticShapeCache {
    key: Option<(i32, i32, i32, i32)>,
    shapes: Vec<Shape>,
}

impl StaticShapeCache {
    /// Returns the cached shapes, rebuilt first if the layout or DPI changed.
    pub fn get_or_build(
        &mut self,
        ox: f32,
        oy: f32,
        scale: f32,
        ppp: f32,
        build: impl FnOnce(&mut ShapeList),
    ) -> &[Shape] {
        let q = |v: f32| (v * 1000.0).round() as i32;
        let k = (q(ox), q(oy), q(scale), (ppp * 100.0).round() as i32);
        if self.key != Some(k) {
            let mut list = ShapeList::default();
            build(&mut list);
            self.shapes = list.into_shapes();
            self.key = Some(k);
        }
        &self.shapes
    }

    /// Paints the cached shapes with `p`, rebuilt first if the layout or DPI
    /// changed, and returns the number of shapes painted.
    pub(in crate::app) fn paint(
        &mut self,
        p: &egui::Painter,
        layout: &super::UiScale,
        ppp: f32,
        build: impl FnOnce(&mut ShapeList),
    ) -> u64 {
        let (ox, oy, scale) = layout.cache_key();
        let shapes = self.get_or_build(ox, oy, scale, ppp, build);
        p.extend(shapes.iter().cloned());
        shapes.len() as u64
    }
}

// ── ShapeCaches ───────────────────────────────────────────────────────────────

/// The cache key for two knobs: the layout, the knobs' angles and their
/// pressed states.
pub(in crate::app) type KnobCacheKey = (i32, i32, i32, [u32; 2], [bool; 2]);

/// Every shape cache the panel draws through. A model change drops them all.
#[derive(Default)]
pub(in crate::app) struct ShapeCaches {
    pub(in crate::app) btn: BtnShapeCache,
    pub(in crate::app) chassis_bg: StaticShapeCache,
    pub(in crate::app) chassis_lcd_overlay: StaticShapeCache,
    pub(in crate::app) top_statics: StaticShapeCache,
    pub(in crate::app) left_statics: StaticShapeCache,
    pub(in crate::app) right_statics: StaticShapeCache,
    pub(in crate::app) jog_statics: StaticShapeCache,
    pub(in crate::app) jog_ring_lights: ShapeCache<(i32, i32, i32, u8, bool)>,
    pub(in crate::app) jog_corner_labels: ShapeCache<(i32, i32, i32, i32, [u8; 16])>,
    pub(in crate::app) grip: ShapeCache<(i32, i32, i32, i32)>,
    /// A jog knurl, keyed on layout and the platter's angle.
    pub(in crate::app) knurl: ShapeCache<(i32, i32, i32, u32)>,
    /// Two rotary knobs, keyed on layout, their angles and pressed states.
    pub(in crate::app) knob: ShapeCache<KnobCacheKey>,
}
