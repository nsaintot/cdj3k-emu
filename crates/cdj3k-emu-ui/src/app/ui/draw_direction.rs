//! FWD / SLIP_REV / REV direction flip-switch widget for the left column.
//! Owns its housing, paddle lever, and the tilted-state perspective faces.
//!
//! One switch, drawn for every player: the shape constants below are the
//! control itself, and a slate supplies only where on its panel it sits and
//! how far apart its three legends print.

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

/// Switch housing - outer shell of the physical switch slot.
pub(in crate::app) const DIRECTION_HOUSING_W: f32 = 110.0;
pub(in crate::app) const DIRECTION_HOUSING_H: f32 = 200.0;
pub(in crate::app) const DIRECTION_HOUSING_ROUNDING: f32 = 12.0;
/// Paddle lever - fills the housing slot like an american rocker switch.
pub(in crate::app) const DIRECTION_LEVER_W_INSET: f32 = 10.0; // gap from housing sides to lever edges
pub(in crate::app) const DIRECTION_LEVER_H_INSET: f32 = 8.0; // gap from housing top/bottom to lever when in position
pub(in crate::app) const DIRECTION_LEVER_ROUNDING: f32 = 6.0;
pub(in crate::app) const DIRECTION_LEVER_TILT_SCALE: f32 = 0.50; // face H fraction when tilted (low = more foreshortening)
pub(in crate::app) const DIRECTION_LEVER_INNER_W_FRAC: f32 = 0.70;
pub(in crate::app) const DIRECTION_LEVER_INNER_H: f32 = 10.0;
pub(in crate::app) const DIRECTION_LEVER_INNER_ROUNDING: f32 = 4.0;
pub(in crate::app) const DIRECTION_MODE_FONT_SIZE: f32 = 28.0;
pub(in crate::app) const DIRECTION_FACE_W_FRAC: f32 = 0.85;
/// Tilted-state face height as fraction of full_face_h (slim).
pub(in crate::app) const DIRECTION_FACE_H_FRAC: f32 = 0.25;
/// Outer base frame height (depth axis) as fraction of full_face_h.
pub(in crate::app) const DIRECTION_BASE_H_FRAC: f32 = 0.20;
/// Outer base near-end width as fraction of lever_w (slightly narrower than far end).
pub(in crate::app) const DIRECTION_BASE_NEAR_W_FRAC: f32 = 0.98;
/// FWD (middle) face half-height as fraction of full_face_h.
pub(in crate::app) const DIRECTION_FWD_FACE_H_FRAC: f32 = 0.15;
/// FWD (middle) face width as fraction of lever_w (1.0 = full slot width).
pub(in crate::app) const DIRECTION_FWD_FACE_W_FRAC: f32 = 0.95;
/// How far the tilted face can protrude past the housing edge, as fraction of housing height.
/// 0.0 = flush with inset edge; positive values push it outside the housing bounds.
pub(in crate::app) const DIRECTION_TILT_FACE_OVERSHOOT_FRAC: f32 = 0.10;
/// Tilted groove height as fraction of the tilted face height.
pub(in crate::app) const DIRECTION_TILT_GROOVE_H_FRAC: f32 = 0.2;
/// Vertical offset of the groove center from the face center, as fraction of half-face-height.
/// Positive = toward the protruding/near side (bottom for dir=1, top for dir=2).
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

    // Static label + combo bordered container live in statics.rs.

    // --- Housing + rail channels ---
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
    // Housing: dark fill + silver outer border + dark inner trim (inset frame look).
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

    // --- Interaction (registered before thumb drawing so pointer pos is fresh) ---

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

    // Simple 3-zone split: top third → SLIP REV, bottom third → REV, middle → FWD.
    use cdj3k_emu_panel::Direction;
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
        // Released while in SLIP REV zone → snap back to FWD.
        app.direction = Direction::Forward;
    }

    // Inject MISO frame on direction change.
    if app.direction != prev_direction {
        app.inject(app.build_current_frame().finalize());
    }

    // --- Paddle/rocker lever - fills the housing slot like an american light switch ---
    let lever_left = housing_rect.left() + layout.sc(DIRECTION_LEVER_W_INSET);
    let lever_right = housing_rect.right() - layout.sc(DIRECTION_LEVER_W_INSET);
    let lever_cx = (lever_left + lever_right) * 0.5;
    let lever_w = lever_right - lever_left;
    let h_inset = layout.sc(DIRECTION_LEVER_H_INSET);
    let lever_rounding = layout.sc(DIRECTION_LEVER_ROUNDING);

    let h_top = housing_rect.top();
    let h_bot = housing_rect.bottom();
    let full_face_h = housing_rect.height() - 2.0 * h_inset;
    let tilt_face_h = full_face_h * DIRECTION_LEVER_TILT_SCALE;

    let (face_top, face_bot) = match app.direction {
        Direction::Reverse => (h_bot - h_inset - tilt_face_h, h_bot - h_inset),
        Direction::SlipReverse => (h_top + h_inset, h_top + h_inset + tilt_face_h),
        Direction::Forward => (h_top + h_inset, h_bot - h_inset),
    };

    // Paddle face - 3D perspective design for tilted states, flat rect for FWD.
    let face_color = egui::Color32::from_rgb(64, 64, 72);
    let border_stroke = egui::Stroke::new(layout.sc(2.0), COL_SILVER);
    let frame_stroke = egui::Stroke::new(layout.sc(1.5), egui::Color32::from_rgb(100, 100, 110));
    let edge_stroke = egui::Stroke::new(layout.sc(1.5), egui::Color32::from_rgb(55, 55, 62));

    match app.direction {
        Direction::Forward => {
            // FWD - flat rectangle, no perspective, but only half the height.
            let rect_mid = (face_top + face_bot) * 0.5;
            let fwd_half_h = full_face_h * DIRECTION_FWD_FACE_H_FRAC;
            let fwd_half_w = lever_w * 0.5 * DIRECTION_FWD_FACE_W_FRAC;
            let fwd_face_rect = Rect::from_min_max(
                Pos2::new(lever_cx - fwd_half_w, rect_mid - fwd_half_h),
                Pos2::new(lever_cx + fwd_half_w, rect_mid + fwd_half_h),
            );
            p.rect_filled(fwd_face_rect, lever_rounding, face_color);
            p.rect_stroke(fwd_face_rect, lever_rounding, border_stroke);
            // Inner groove mark - only on FWD.
            let inner_h = layout.sc(DIRECTION_LEVER_INNER_H);
            p.rect_filled(
                Rect::from_center_size(
                    fwd_face_rect.center(),
                    Vec2::new(lever_w * DIRECTION_LEVER_INNER_W_FRAC, inner_h),
                ),
                layout.sc(DIRECTION_LEVER_INNER_ROUNDING),
                COL_BLACK,
            );
        }
        dir => {
            // Tilted - 3D perspective: outer 3-sided base frame + inner slim face rect +
            // 4 connecting perspective lines (converging from wide base to narrow face).
            let is_rev = dir == Direction::Reverse;
            let face_hw = lever_w * 0.5 * DIRECTION_FACE_W_FRAC;
            let face_h_abs = full_face_h * DIRECTION_FACE_H_FRAC;
            let base_h_abs = full_face_h * DIRECTION_BASE_H_FRAC;
            let base_near_hw = lever_w * 0.5 * DIRECTION_BASE_NEAR_W_FRAC;

            // The entire assembly lives in the protruding half of the housing.
            // REV       (bottom): assembly in bottom half - base far edge at h_mid, face at h_bot.
            // SLIP REV  (top):    assembly in top half    - base far edge at h_mid, face at h_top.
            let h_mid = (h_top + h_bot) * 0.5;
            let overshoot = (h_bot - h_top) * DIRECTION_TILT_FACE_OVERSHOOT_FRAC;
            let (base_far_y, base_near_y, face_a_y, face_b_y) = if is_rev {
                let face_bot = h_bot - h_inset + overshoot;
                let face_top = face_bot - face_h_abs;
                (h_mid, h_mid + base_h_abs, face_top, face_bot)
            } else {
                let face_top = h_top + h_inset - overshoot;
                let face_bot = face_top + face_h_abs;
                (h_mid, h_mid - base_h_abs, face_top, face_bot)
            };

            // Outer base frame corners (full-width far edge, slightly narrower near end).
            let o_far_l = Pos2::new(lever_left, base_far_y);
            let o_far_r = Pos2::new(lever_right, base_far_y);
            let o_near_l = Pos2::new(lever_cx - base_near_hw, base_near_y);
            let o_near_r = Pos2::new(lever_cx + base_near_hw, base_near_y);

            // Inner face rect corners.
            let i_a_l = Pos2::new(lever_cx - face_hw, face_a_y);
            let i_a_r = Pos2::new(lever_cx + face_hw, face_a_y);
            let i_b_l = Pos2::new(lever_cx - face_hw, face_b_y);
            let i_b_r = Pos2::new(lever_cx + face_hw, face_b_y);

            // 3-sided outer base frame (far edge + two sides, no near edge).
            p.line_segment([o_far_l, o_far_r], frame_stroke);
            p.line_segment([o_far_l, o_near_l], frame_stroke);
            p.line_segment([o_far_r, o_near_r], frame_stroke);

            // 4 perspective connecting lines - outer corners to corresponding inner face corners.
            // REV       : base-at-top, face-at-bottom → far corners to face top edge, near to face bottom.
            // SLIP REV  : mirror.
            let (far_dst_l, far_dst_r, near_dst_l, near_dst_r) = if is_rev {
                (i_a_l, i_a_r, i_b_l, i_b_r)
            } else {
                (i_b_l, i_b_r, i_a_l, i_a_r)
            };
            p.line_segment([o_far_l, far_dst_l], edge_stroke);
            p.line_segment([o_far_r, far_dst_r], edge_stroke);
            p.line_segment([o_near_l, near_dst_l], edge_stroke);
            p.line_segment([o_near_r, near_dst_r], edge_stroke);

            // Inner slim face rect - thinner border than FWD since the face is smaller.
            let face_rect = Rect::from_min_max(i_a_l, i_b_r);
            let tilt_face_stroke = egui::Stroke::new(layout.sc(1.0), COL_SILVER);
            p.rect_filled(face_rect, lever_rounding * 0.5, face_color);
            p.rect_stroke(face_rect, lever_rounding * 0.5, tilt_face_stroke);

            // Groove mark - proportional to the tilted face dimensions, offset toward
            // the protruding side (bottom for REV, top for SLIP REV).
            let groove_w = face_rect.width() * DIRECTION_LEVER_INNER_W_FRAC;
            let groove_h = face_rect.height() * DIRECTION_TILT_GROOVE_H_FRAC;
            let v_off = face_rect.height() * 0.5 * DIRECTION_TILT_GROOVE_V_OFF_FRAC;
            let groove_cy = if is_rev {
                face_rect.center().y + v_off // REV: near side is bottom → offset down
            } else {
                face_rect.center().y - v_off // SLIP REV: near side is top → offset up
            };
            p.rect_filled(
                Rect::from_center_size(
                    Pos2::new(face_rect.center().x, groove_cy),
                    Vec2::new(groove_w, groove_h),
                ),
                layout.sc(DIRECTION_LEVER_INNER_ROUNDING),
                COL_BLACK,
            );
        }
    }

    // --- Right side: mode labels, colored by current direction ---

    let right_cx_ref =
        switch_div_x_ref + (combo_ref.right() - switch_div_x_ref) * DIRECTION_SWITCH_RIGHT_FRAC;
    let label_entries: [(f32, &str, Direction); 3] = [
        (cy_ref + place.label_row_pitch, "REV", Direction::Reverse),
        (
            cy_ref - place.label_row_pitch,
            "SLIP REV",
            Direction::SlipReverse,
        ),
        (cy_ref, "FWD", Direction::Forward),
    ];
    for (y_ref, text, dir_val) in label_entries {
        let color = if app.direction == dir_val {
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
