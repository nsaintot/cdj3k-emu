//! The FWD, SLIP REV and REV direction switch in the left column. This module
//! draws its housing, its paddle lever and the lever's tilted faces.
//!
//! Every player draws the same switch from the shape constants below; a slate
//! supplies only its position on the panel and the spacing of its three
//! legends.

use cdj3k_emu_panel::Direction;
use egui::{FontFamily, FontId, Pos2, Rect, Vec2};

use crate::app::CdjApp;

use super::{
    draw_bordered_rect_section, DoubleBorderSpec, StrokeSpec, UiScale, COL_BLACK,
    COL_BTN_OUTLINED_WHITE, COL_BTN_OUTLINED_YELLOW, COL_BTN_TEXT, COL_RED, COL_SILVER,
};

/// Where a slate puts the switch: the section rect it sits in, its centre as
/// fractions of that rect, the half-extent of its bordered surround, and the
/// row pitch of the three legends beside it.
#[derive(Copy, Clone, Debug)]
pub(in crate::app) struct DirectionPlacement {
    pub col: Rect,
    pub center_u: f32,
    pub center_v: f32,
    pub half_w: f32,
    pub half_h: f32,
    pub label_row_pitch: f32,
}

impl DirectionPlacement {
    pub fn center_x(&self) -> f32 {
        self.col.left() + self.col.width() * self.center_u
    }

    pub fn center_y(&self) -> f32 {
        self.col.top() + self.col.height() * self.center_v
    }

    /// The bordered surround, in slate reference units.
    pub fn combo_ref(&self) -> Rect {
        let (cx, cy) = (self.center_x(), self.center_y());
        Rect::from_min_max(
            Pos2::new(cx - self.half_w, cy - self.half_h),
            Pos2::new(cx + self.half_w, cy + self.half_h),
        )
    }
}

pub(in crate::app) const DIRECTION_BORDER_ROUNDING: f32 = 24.0;
pub(in crate::app) const DIRECTION_SWITCH_LEFT_FRAC: f32 = 0.55;
pub(in crate::app) const DIRECTION_SWITCH_RIGHT_FRAC: f32 = 0.45;

/// The switch housing, which is the outer shell of the switch slot.
pub(in crate::app) const DIRECTION_HOUSING_W: f32 = 110.0;
pub(in crate::app) const DIRECTION_HOUSING_H: f32 = 200.0;
pub(in crate::app) const DIRECTION_HOUSING_ROUNDING: f32 = 12.0;
/// The gap between the housing's sides and the paddle lever, which fills the
/// housing slot like an American rocker switch.
pub(in crate::app) const DIRECTION_LEVER_W_INSET: f32 = 10.0;
/// The gap between the housing's top or bottom and the lever.
pub(in crate::app) const DIRECTION_LEVER_H_INSET: f32 = 8.0;
pub(in crate::app) const DIRECTION_LEVER_ROUNDING: f32 = 6.0;
pub(in crate::app) const DIRECTION_LEVER_INNER_W_FRAC: f32 = 0.70;
pub(in crate::app) const DIRECTION_LEVER_INNER_H: f32 = 10.0;
pub(in crate::app) const DIRECTION_LEVER_INNER_ROUNDING: f32 = 4.0;
pub(in crate::app) const DIRECTION_MODE_FONT_SIZE: f32 = 28.0;
pub(in crate::app) const DIRECTION_FACE_W_FRAC: f32 = 0.85;
/// The tilted face's height as a fraction of full_face_h.
pub(in crate::app) const DIRECTION_FACE_H_FRAC: f32 = 0.25;
/// The outer base frame's height along the depth axis, as a fraction of full_face_h.
pub(in crate::app) const DIRECTION_BASE_H_FRAC: f32 = 0.20;
/// The width of the outer base's near end as a fraction of lever_w; it is
/// slightly narrower than the far end.
pub(in crate::app) const DIRECTION_BASE_NEAR_W_FRAC: f32 = 0.98;
/// The half-height of the FWD (middle) face as a fraction of full_face_h.
pub(in crate::app) const DIRECTION_FWD_FACE_H_FRAC: f32 = 0.15;
/// The width of the FWD (middle) face as a fraction of lever_w; 1.0 is the full
/// slot width.
pub(in crate::app) const DIRECTION_FWD_FACE_W_FRAC: f32 = 0.95;
/// How far the tilted face protrudes past the housing edge, as a fraction of
/// the housing height. At 0.0 the face is flush with the inset edge; a positive
/// value pushes it outside the housing.
pub(in crate::app) const DIRECTION_TILT_FACE_OVERSHOOT_FRAC: f32 = 0.10;
/// The tilted groove's height as a fraction of the tilted face's height.
pub(in crate::app) const DIRECTION_TILT_GROOVE_H_FRAC: f32 = 0.2;
/// The vertical offset of the groove's center from the face's center, as a
/// fraction of half the face height. A positive value moves it toward the
/// protruding side: the bottom for REV, the top for SLIP REV.
pub(in crate::app) const DIRECTION_TILT_GROOVE_V_OFF_FRAC: f32 = 0.25;

pub(in crate::app) fn draw_direction_switch(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layout: &UiScale,
    place: DirectionPlacement,
) {
    let cy_ref = place.center_y();
    let combo_ref = place.combo_ref();
    let combo_screen = Rect::from_min_max(
        layout.sp(combo_ref.left(), combo_ref.top()),
        layout.sp(combo_ref.right(), combo_ref.bottom()),
    );

    // Each slate draws the static label and the bordered container in its
    // draw_left/statics.rs.

    // --- Housing ---
    let switch_div_x_ref = combo_ref.left() + combo_ref.width() * DIRECTION_SWITCH_LEFT_FRAC;
    let switch_cx_ref = (combo_ref.left() + switch_div_x_ref) * 0.5;
    let switch_center = layout.sp(switch_cx_ref, cy_ref);

    let housing_rect = Rect::from_center_size(
        switch_center,
        Vec2::new(
            layout.sc(DIRECTION_HOUSING_W),
            layout.sc(DIRECTION_HOUSING_H),
        ),
    );
    // A dark fill, a silver outer border and a dark inner trim make the
    // housing look like an inset frame.
    draw_bordered_rect_section(
        p,
        housing_rect,
        Some(layout.sc(DIRECTION_HOUSING_ROUNDING)),
        Some(COL_BLACK),
        DoubleBorderSpec::from_strokes(
            StrokeSpec {
                width: layout.sc(4.0),
                color: COL_SILVER,
            },
            StrokeSpec {
                width: layout.sc(1.5),
                color: COL_BLACK,
            },
        ),
    );

    // Interaction is registered before the lever is drawn, so the pointer
    // position is fresh.
    direction_interact(app, ui, combo_screen, housing_rect);

    let lever = Lever::new(layout, housing_rect);
    match app.direction {
        Direction::Forward => paint_fwd_face(p, layout, &lever),
        dir => paint_tilted_face(p, layout, &lever, dir == Direction::Reverse),
    }

    // --- Right side: the mode labels, colored by the current direction ---
    let right_cx_ref =
        switch_div_x_ref + (combo_ref.right() - switch_div_x_ref) * DIRECTION_SWITCH_RIGHT_FRAC;
    paint_mode_labels(
        app.direction,
        p,
        layout,
        right_cx_ref,
        cy_ref,
        place.label_row_pitch,
    );
}

/// Handles a drag on the switch column. The housing is split into three
/// zones: the top third is SLIP REV, the middle is FWD and the bottom third is
/// REV. SLIP REV springs back to FWD on release. A MISO frame is injected when
/// the direction changes.
fn direction_interact(app: &mut CdjApp, ui: &mut egui::Ui, combo_screen: Rect, housing_rect: Rect) {
    let switch_col_screen = Rect::from_min_max(
        combo_screen.min,
        Pos2::new(
            combo_screen.min.x + combo_screen.width() * DIRECTION_SWITCH_LEFT_FRAC,
            combo_screen.max.y,
        ),
    );
    let switch_resp = ui.interact(
        switch_col_screen,
        egui::Id::new("dir_switch"),
        egui::Sense::drag(),
    );

    let is_dragging = switch_resp.is_pointer_button_down_on();
    let was_dragging_key = egui::Id::new("dir_was_drag");
    let was_dragging: bool = ui.data(|d| d.get_temp(was_dragging_key).unwrap_or(false));
    ui.data_mut(|d| d.insert_temp(was_dragging_key, is_dragging));

    let prev_direction = app.direction;

    if is_dragging {
        if let Some(pos) = ui.ctx().input(|i| i.pointer.interact_pos()) {
            let rel = ((pos.y - housing_rect.top()) / housing_rect.height()).clamp(0.0, 1.0);
            app.direction = if rel < 0.33 {
                Direction::SlipReverse
            } else if rel > 0.67 {
                Direction::Reverse
            } else {
                Direction::Forward
            };
        }
    } else if was_dragging && app.direction == Direction::SlipReverse {
        // Releasing the lever in the SLIP REV zone snaps it back to FWD.
        app.direction = Direction::Forward;
    }

    if app.direction != prev_direction {
        app.inject(app.build_current_frame());
    }
}

/// The paddle lever, which fills the housing slot like an American rocker
/// switch.
struct Lever {
    left: f32,
    right: f32,
    cx: f32,
    w: f32,
    h_inset: f32,
    rounding: f32,
    h_top: f32,
    h_bot: f32,
    full_face_h: f32,
}

impl Lever {
    fn new(layout: &UiScale, housing_rect: Rect) -> Self {
        let left = housing_rect.left() + layout.sc(DIRECTION_LEVER_W_INSET);
        let right = housing_rect.right() - layout.sc(DIRECTION_LEVER_W_INSET);
        let h_inset = layout.sc(DIRECTION_LEVER_H_INSET);
        Self {
            left,
            right,
            cx: (left + right) * 0.5,
            w: right - left,
            h_inset,
            rounding: layout.sc(DIRECTION_LEVER_ROUNDING),
            h_top: housing_rect.top(),
            h_bot: housing_rect.bottom(),
            full_face_h: housing_rect.height() - 2.0 * h_inset,
        }
    }
}

/// The fill color of the paddle face.
const FACE_COLOR: egui::Color32 = egui::Color32::from_rgb(64, 64, 72);

/// Draws the FWD face as a flat rectangle without perspective, centered on the
/// lever.
fn paint_fwd_face(p: &egui::Painter, layout: &UiScale, lever: &Lever) {
    let border_stroke = egui::Stroke::new(layout.sc(2.0), COL_SILVER);
    let face_top = lever.h_top + lever.h_inset;
    let face_bot = lever.h_bot - lever.h_inset;
    let rect_mid = (face_top + face_bot) * 0.5;
    let fwd_half_h = lever.full_face_h * DIRECTION_FWD_FACE_H_FRAC;
    let fwd_half_w = lever.w * 0.5 * DIRECTION_FWD_FACE_W_FRAC;
    let fwd_face_rect = Rect::from_min_max(
        Pos2::new(lever.cx - fwd_half_w, rect_mid - fwd_half_h),
        Pos2::new(lever.cx + fwd_half_w, rect_mid + fwd_half_h),
    );
    p.rect_filled(fwd_face_rect, lever.rounding, FACE_COLOR);
    p.rect_stroke(
        fwd_face_rect,
        lever.rounding,
        border_stroke,
        egui::StrokeKind::Middle,
    );
    // The groove mark on the FWD face.
    let inner_h = layout.sc(DIRECTION_LEVER_INNER_H);
    p.rect_filled(
        Rect::from_center_size(
            fwd_face_rect.center(),
            Vec2::new(lever.w * DIRECTION_LEVER_INNER_W_FRAC, inner_h),
        ),
        layout.sc(DIRECTION_LEVER_INNER_ROUNDING),
        COL_BLACK,
    );
}

/// Draws a tilted lever in perspective: a three-sided base frame, a slim face,
/// and four lines that converge from the wide base to the narrow face.
/// `is_rev` tilts it down for REV; otherwise it tilts up for SLIP REV.
fn paint_tilted_face(p: &egui::Painter, layout: &UiScale, lever: &Lever, is_rev: bool) {
    let tilted = TiltedLever::new(lever, is_rev);
    paint_tilted_frame(p, layout, &tilted, is_rev);
    paint_slim_face(p, layout, lever, tilted.face, is_rev);
}

/// The corner points of a tilted lever. The base frame starts at the housing's
/// middle and extends into the protruding half; the slim face sits at the
/// protruding end.
struct TiltedLever {
    /// The base frame's full-width edge, at the housing's middle.
    far_l: Pos2,
    far_r: Pos2,
    /// The base frame's slightly narrower end, towards the face.
    near_l: Pos2,
    near_r: Pos2,
    face: Rect,
}

impl TiltedLever {
    fn new(lever: &Lever, is_rev: bool) -> Self {
        let (h_top, h_bot, h_inset) = (lever.h_top, lever.h_bot, lever.h_inset);
        let face_hw = lever.w * 0.5 * DIRECTION_FACE_W_FRAC;
        let face_h_abs = lever.full_face_h * DIRECTION_FACE_H_FRAC;
        let base_h_abs = lever.full_face_h * DIRECTION_BASE_H_FRAC;
        let base_near_hw = lever.w * 0.5 * DIRECTION_BASE_NEAR_W_FRAC;

        // The assembly sits in the protruding half of the housing, with the
        // base's far edge at h_mid. For REV it is in the bottom half with the
        // face at h_bot; for SLIP REV it is in the top half with the face at
        // h_top.
        let h_mid = (h_top + h_bot) * 0.5;
        let overshoot = (h_bot - h_top) * DIRECTION_TILT_FACE_OVERSHOOT_FRAC;
        let (base_far_y, base_near_y, face_top, face_bot) = if is_rev {
            let face_bot = h_bot - h_inset + overshoot;
            let face_top = face_bot - face_h_abs;
            (h_mid, h_mid + base_h_abs, face_top, face_bot)
        } else {
            let face_top = h_top + h_inset - overshoot;
            let face_bot = face_top + face_h_abs;
            (h_mid, h_mid - base_h_abs, face_top, face_bot)
        };
        Self {
            far_l: Pos2::new(lever.left, base_far_y),
            far_r: Pos2::new(lever.right, base_far_y),
            near_l: Pos2::new(lever.cx - base_near_hw, base_near_y),
            near_r: Pos2::new(lever.cx + base_near_hw, base_near_y),
            face: Rect::from_min_max(
                Pos2::new(lever.cx - face_hw, face_top),
                Pos2::new(lever.cx + face_hw, face_bot),
            ),
        }
    }
}

/// Draws the three-sided base frame (the far edge and the two sides) and the
/// four perspective lines from its corners to the slim face's corners.
fn paint_tilted_frame(p: &egui::Painter, layout: &UiScale, tilted: &TiltedLever, is_rev: bool) {
    let frame_stroke = egui::Stroke::new(layout.sc(1.5), egui::Color32::from_rgb(100, 100, 110));
    let edge_stroke = egui::Stroke::new(layout.sc(1.5), egui::Color32::from_rgb(55, 55, 62));
    let TiltedLever {
        far_l,
        far_r,
        near_l,
        near_r,
        face,
    } = *tilted;

    p.line_segment([far_l, far_r], frame_stroke);
    p.line_segment([far_l, near_l], frame_stroke);
    p.line_segment([far_r, near_r], frame_stroke);

    // For REV the base is above the face, so the far corners join the face's
    // top edge and the near corners its bottom edge. SLIP REV is the mirror.
    let (top, bottom) = (
        [face.left_top(), face.right_top()],
        [face.left_bottom(), face.right_bottom()],
    );
    let (far_dst, near_dst) = if is_rev { (top, bottom) } else { (bottom, top) };
    p.line_segment([far_l, far_dst[0]], edge_stroke);
    p.line_segment([far_r, far_dst[1]], edge_stroke);
    p.line_segment([near_l, near_dst[0]], edge_stroke);
    p.line_segment([near_r, near_dst[1]], edge_stroke);
}

/// Draws the tilted lever's slim face with a thinner border than FWD, and its
/// groove mark offset towards the protruding side: the bottom for REV, the
/// top for SLIP REV.
fn paint_slim_face(p: &egui::Painter, layout: &UiScale, lever: &Lever, face: Rect, is_rev: bool) {
    let tilt_face_stroke = egui::Stroke::new(layout.sc(1.0), COL_SILVER);
    p.rect_filled(face, lever.rounding * 0.5, FACE_COLOR);
    p.rect_stroke(
        face,
        lever.rounding * 0.5,
        tilt_face_stroke,
        egui::StrokeKind::Middle,
    );

    let groove_w = face.width() * DIRECTION_LEVER_INNER_W_FRAC;
    let groove_h = face.height() * DIRECTION_TILT_GROOVE_H_FRAC;
    let v_off = face.height() * 0.5 * DIRECTION_TILT_GROOVE_V_OFF_FRAC;
    let groove_cy = if is_rev {
        face.center().y + v_off
    } else {
        face.center().y - v_off
    };
    p.rect_filled(
        Rect::from_center_size(
            Pos2::new(face.center().x, groove_cy),
            Vec2::new(groove_w, groove_h),
        ),
        layout.sc(DIRECTION_LEVER_INNER_ROUNDING),
        COL_BLACK,
    );
}

/// Draws the REV, SLIP REV and FWD legends beside the switch, with the current
/// one lit.
fn paint_mode_labels(
    direction: Direction,
    p: &egui::Painter,
    layout: &UiScale,
    right_cx_ref: f32,
    cy_ref: f32,
    row_pitch: f32,
) {
    let label_entries: [(f32, &str, Direction); 3] = [
        (cy_ref + row_pitch, "REV", Direction::Reverse),
        (cy_ref - row_pitch, "SLIP REV", Direction::SlipReverse),
        (cy_ref, "FWD", Direction::Forward),
    ];
    for (y_ref, text, dir_val) in label_entries {
        let color = if direction == dir_val {
            match dir_val {
                Direction::Reverse => COL_RED,
                Direction::SlipReverse => COL_BTN_OUTLINED_YELLOW,
                Direction::Forward => COL_BTN_OUTLINED_WHITE,
            }
        } else {
            COL_BTN_TEXT
        };
        let font_id = if dir_val != Direction::Forward {
            FontId::new(
                layout.sc(DIRECTION_MODE_FONT_SIZE),
                FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_BOLD.into()),
            )
        } else {
            FontId::proportional(layout.sc(DIRECTION_MODE_FONT_SIZE))
        };
        p.text(
            layout.sp(right_cx_ref, y_ref),
            egui::Align2::CENTER_CENTER,
            text,
            font_id,
            color,
        );
    }
}
