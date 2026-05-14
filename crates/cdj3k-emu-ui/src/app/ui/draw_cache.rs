//! Static shape cache - avoids recomputing unchanged geometry every frame.
//!
//! # Strategy
//! The CDJ-UI has three rendering categories:
//!
//! | Category | Examples | Frequency |
//! |---|---|---|
//! | **Dynamic** | Jog pitchbend grip rotation | Every jog frame (~60 fps) |
//! | **Semi-static** | JOG ADJUST gear/indicator, tempo knob | Only on user drag |
//! | **Static** | All buttons, labels, borders, outer jog rings | Only on resize |
//!
//! [`ShapeList`] is a painter-mirroring collector: static draw calls push
//! into it instead of directly into the egui painter.  The collected
//! `Vec<Shape>` is stored in [`JogStaticCache`] and replayed each frame via
//! `painter.extend(...)` - no geometry recomputation until the cache is stale.
//!
//! # Jog wheel split
//! The pitchbend grip sits between the outer body fill and the inner LCD disk,
//! so the static geometry is split into two lists:
//!
//! ```text
//! painter.extend(outer)           - jog outer rings
//! paint_pitchbend_grip(…)         - always recomputed (dynamic rotation)
//! painter.extend(inner_mid)       - platter + inner LCD background disk
//! paint_jog_lcd_stream_texture(…) - jog FB mesh-clipped to inner LCD silhouette (c0 = scale only)
//! painter.extend(inner_over)      - LCD chrome (stroke, MODE, BEAT SYNC, separators) + JOG ADJUST
//! paint_jog_inner_lcd_corner_labels - SLIP / VINYL / SYNC / MASTER (stream corner colors)
//! ```
//!
//! The `inner_over` list is keyed on `jog_adjust` because the gear/indicator rotate
//! with it; every other section is keyed on the `UiScale` triple alone.

use std::collections::HashMap;

use egui::{Align2, Color32, FontId, Pos2, Rect, Shape, Stroke};

// ── ShapeList ─────────────────────────────────────────────────────────────────

/// Collects [`egui::Shape`]s for deferred/cached rendering, mirroring the
/// [`egui::Painter`] draw API for purely *static* (non-interactive) calls.
pub struct ShapeList(Vec<Shape>);

impl Default for ShapeList {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl ShapeList {
    /// Consume the list and return the inner `Vec<Shape>`.
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
        self.0
            .push(Shape::rect_stroke(rect, rounding, stroke.into()));
    }

    pub fn line_segment(&mut self, pts: [Pos2; 2], stroke: impl Into<Stroke>) {
        self.0.push(Shape::line_segment(pts, stroke.into()));
    }

    pub fn add(&mut self, shape: Shape) {
        self.0.push(shape);
    }

    /// Lay out `text` through the egui font system and collect the resulting
    /// [`Shape::galley`].  Requires a [`egui::Context`] for font access.
    pub fn text(
        &mut self,
        ctx: &egui::Context,
        pos: Pos2,
        anchor: Align2,
        text: &str,
        font_id: FontId,
        color: Color32,
    ) {
        let galley = ctx.fonts(|f| f.layout_no_wrap(text.to_owned(), font_id, color));
        let top_left = anchor.anchor_size(pos, galley.size()).min;
        self.0.push(Shape::galley(top_left, galley, color));
    }

    /// Center `text` on its visible ink bounds (not the line box) at `center`,
    /// then snap to the device pixel grid. Use for single-glyph labels (icons,
    /// arrows, dots) where line-box centering drifts visibly with font metrics
    /// and rasterizer rounding. For multi-char labels, prefer [`Self::text`]
    /// with [`Align2::CENTER_CENTER`].
    pub fn text_centered_ink(
        &mut self,
        ctx: &egui::Context,
        center: Pos2,
        text: &str,
        font_id: FontId,
        color: Color32,
    ) {
        let galley = ctx.fonts(|f| f.layout_no_wrap(text.to_owned(), font_id, color));
        let ppp = ctx.pixels_per_point();
        let raw = center - galley.mesh_bounds.center().to_vec2();
        let snapped = Pos2::new((raw.x * ppp).round() / ppp, (raw.y * ppp).round() / ppp);
        self.0.push(Shape::galley(snapped, galley, color));
    }
}

// ── Jog wheel static cache ────────────────────────────────────────────────────

/// Cache validity key for the jog wheel section.
///
/// The layout triple (`ox`, `oy`, `scale`) covers window resize/reposition.
/// `jog_adjust_step` covers the JOG ADJUST gear/indicator rotation, which is
/// encoded discretely (13 detent positions → steps 0–12) to avoid float noise.
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

/// Cached jog wheel geometry, split around the always-dynamic pitchbend grip.
///
/// Draw order every frame:
/// 1. `painter.extend(outer)` - silver/body outer disks, cosmetic ring border
/// 2. `paint_pitchbend_grip(angle)` - always recomputed
/// 3. `painter.extend(inner_mid)` - touch platter, border, inner LCD background disk
/// 4. jog stream texture (dynamic) - textured mesh clipped to inner LCD silhouette
/// 5. `painter.extend(inner_over)` - inner LCD outline, MODE / BEAT SYNC, JOG ADJUST
/// 6. `paint_jog_inner_lcd_corner_labels` - SLIP / VINYL / SYNC / MASTER
pub struct JogStaticCache {
    pub key: JogCacheKey,
    /// Shapes drawn *before* the pitchbend grip layer.
    pub outer: Vec<Shape>,
    /// After grip: platter + border + inner LCD filled disk (below stream texture).
    pub inner_mid: Vec<Shape>,
    /// After jog LCD texture: decorative LCD chrome + JOG ADJUST assembly.
    pub inner_over: Vec<Shape>,
}

// ── Generic ShapeCache<K> ─────────────────────────────────────────────────────

/// Generic shape cache: stores a `Vec<Shape>` that is rebuilt only when `key` changes.
///
/// Use this for elements that change on a known set of parameters (e.g. LED bytes,
/// a slider value, a corner-label color set) but don't need per-widget ID tracking.
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
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the cached shapes, rebuilding if `key` changed.
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

/// Per-widget cache key: quantized screen rect + packed visual state + pressed flag.
///
/// The rect is quantized to ×100 integer to avoid float noise while remaining sensitive
/// to sub-pixel layout changes caused by window resize. `state` packs up to 4 bytes of
/// relevant MOSI LED / color data (use `Color32::to_array()` bytes or raw LED byte).
#[derive(PartialEq, Eq, Clone)]
pub struct BtnCacheKey {
    /// Quantized [min_x, min_y, max_x, max_y] in screen pixels × 100.
    rect: [i32; 4],
    /// Packed visual state: up to 4 MOSI bytes or a `Color32` RGBA.
    pub state: u32,
    /// Whether the button is visually pressed (pointer down or latched).
    pub pressed: bool,
    /// Quantized pixels_per_point × 100 - invalidates on screen DPI change.
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

/// Cache for interactive buttons. Each button is identified by its egui [`Id`]
/// (unique per call site), so all buttons in the entire UI share one cache map.
///
/// Shapes are rebuilt only when the key changes - i.e., when layout, LED color,
/// or pressed state changes for that specific button.
pub struct BtnShapeCache {
    map: HashMap<egui::Id, (BtnCacheKey, Vec<Shape>)>,
}

impl Default for BtnShapeCache {
    fn default() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
}

impl BtnShapeCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return shapes for `id`, rebuilding if `key` changed since last call (or on first call).
    pub fn get_or_build(
        &mut self,
        id: egui::Id,
        key: BtnCacheKey,
        build: impl FnOnce(&mut ShapeList),
    ) -> &[Shape] {
        if self.map.get(&id).map_or(true, |(k, _)| k != &key) {
            let mut list = ShapeList::default();
            build(&mut list);
            self.map.insert(id, (key, list.into_shapes()));
        }
        &self.map[&id].1
    }
}

// ── StaticShapeCache ──────────────────────────────────────────────────────────

/// Shape cache for purely static regions (chassis, labels, separators).
/// Keyed by the quantized `(ox, oy, scale, ppp)` triple - rebuilt on resize or DPI change.
pub struct StaticShapeCache {
    key: Option<(i32, i32, i32, i32)>,
    shapes: Vec<Shape>,
}

impl Default for StaticShapeCache {
    fn default() -> Self {
        Self {
            key: None,
            shapes: Vec::new(),
        }
    }
}

impl StaticShapeCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the cached shapes, rebuilding if the layout or DPI changed.
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
}
