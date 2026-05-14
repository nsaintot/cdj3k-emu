//! Button widgets for [`CdjApp`]: rectangular, circular, and arc-quadrant
//! buttons. Each computes interaction state, maintains the shape cache,
//! and drives MISO press/release encoding via [`CdjApp::handle_btn_interaction`].

use egui::{Color32, Pos2, Rect};
use std::hash::Hash;

use super::ui;
use super::CdjApp;

/// Number of polygon samples per side when computing the bounding box of an
/// arc-quadrant button's interaction region.
const ARC_BTN_INTERACT_SAMPLES: usize = 16;

#[inline]
fn color_to_u32(c: Color32) -> u32 {
    u32::from_le_bytes(c.to_array())
}

impl CdjApp {
    /// Shared button press/release logic with optional ctrl-latch.
    ///
    /// On press edge: sets `held_btn` and injects.
    /// On release edge without ctrl: clears `held_btn` and injects.
    /// On release edge with ctrl: moves the button into `latched_btns` and
    /// injects (button stays on until ctrl is released).
    pub(super) fn handle_btn_interaction(&mut self, is_down: bool, ctrl: bool, btn: (usize, u8)) {
        let was_held = self.held_btn == Some(btn);
        if is_down && !was_held && !self.latched_btns.contains(&btn) {
            self.held_btn = Some(btn);
            self.inject(self.build_current_frame().finalize());
        } else if !is_down && was_held {
            self.held_btn = None;
            if ctrl {
                self.latched_btns.insert(btn);
            }
            self.inject(self.build_current_frame().finalize());
        }
    }

    /// Draw a rectangular button; inject on press edge, clear on release edge.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn btn(
        &mut self,
        ui: &mut egui::Ui,
        layout: &ui::UiScale,
        button_type: ui::ButtonType,
        rect: Rect,
        label: &str,
        font_size: f32,
        font_color: Option<Color32>,
        color: Option<Color32>,
        touchdown_color: Option<Color32>,
        label_touchdown_color: Option<Color32>,
        label_nudge: Option<egui::Vec2>,
        border_override: Option<ui::DoubleBorderSpec>,
        font_family: egui::FontFamily,
        id_src: impl Hash,
        btn: (usize, u8),
    ) {
        let border = border_override.unwrap_or_else(|| ui::default_rect_btn_border(layout));
        let id = ui.id().with(&id_src);
        let response = ui.interact(rect, id, egui::Sense::click_and_drag());
        let is_down = response.is_pointer_button_down_on();
        let latched = self.latched_btns.contains(&btn);
        let is_pressed = is_down || latched;

        // State key: mix all LED-driven colors so any change invalidates the cache.
        let nudge_u32 = label_nudge
            .map(|n| n.x.to_bits() ^ n.y.to_bits().rotate_left(17))
            .unwrap_or(0);
        let state = font_color.map_or(0u32, color_to_u32)
            ^ color.map_or(0u32, |c| color_to_u32(c).rotate_left(8))
            ^ color_to_u32(border.inner.color).rotate_left(16)
            ^ nudge_u32.rotate_left(24);

        let cache_key =
            ui::draw_cache::BtnCacheKey::new(rect, state, is_pressed, ui.ctx().pixels_per_point());
        let ctx = ui.ctx().clone();
        let painter = ui.painter().clone();
        let shapes = self.btn_cache.get_or_build(id, cache_key, |list| {
            ui::collect_button(
                list,
                &ctx,
                button_type,
                rect,
                label,
                font_size,
                font_color,
                color,
                touchdown_color,
                border,
                label_touchdown_color,
                label_nudge,
                font_family,
                is_pressed,
            );
        });
        self.frame_shape_count += shapes.len() as u64;
        painter.extend(shapes.iter().cloned());
        let ctrl = ui.input(|i| i.modifiers.ctrl);
        self.handle_btn_interaction(is_down, ctrl, btn);
    }

    /// Draw a circle button; inject on press edge, clear on release edge.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn circle_btn(
        &mut self,
        ui: &mut egui::Ui,
        layout: &ui::UiScale,
        center: Pos2,
        radius: f32,
        btn_color: Option<Color32>,
        touchdown_color: Option<Color32>,
        btn_label: &str,
        btn_label_color: Option<Color32>,
        btn_label_touchdown_color: Option<Color32>,
        btn_label_size: f32,
        btn_label_nudge: Option<egui::Vec2>,
        btn_border: Option<ui::DoubleBorderSpec>,
        id_src: impl Hash,
        btn: (usize, u8),
    ) {
        let id = ui.id().with(&id_src);
        let rect = Rect::from_center_size(center, egui::Vec2::splat(radius * 2.0));
        let response = ui.interact(rect, id, egui::Sense::click_and_drag());
        let is_down = response.is_pointer_button_down_on();
        let latched = self.latched_btns.contains(&btn);
        let is_pressed = is_down || latched;
        let border = btn_border.unwrap_or_else(|| ui::default_circle_btn_border(layout));

        let state = btn_label_color.map_or(0u32, color_to_u32)
            ^ btn_color.map_or(0u32, |c| color_to_u32(c).rotate_left(8))
            ^ color_to_u32(border.inner.color).rotate_left(16);
        let cache_key =
            ui::draw_cache::BtnCacheKey::new(rect, state, is_pressed, ui.ctx().pixels_per_point());
        let ctx = ui.ctx().clone();
        let painter = ui.painter().clone();
        let shapes = self.btn_cache.get_or_build(id, cache_key, |list| {
            ui::collect_circle_button(
                list,
                &ctx,
                center,
                radius,
                btn_color,
                touchdown_color,
                btn_label,
                btn_label_size,
                btn_label_color,
                btn_label_touchdown_color,
                btn_label_nudge,
                border,
                is_pressed,
            );
        });
        self.frame_shape_count += shapes.len() as u64;
        painter.extend(shapes.iter().cloned());
        let ctrl = ui.input(|i| i.modifiers.ctrl);
        self.handle_btn_interaction(is_down, ctrl, btn);
    }

    /// Draw an arc-shaped button; inject on press edge, clear on release.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn arc_quad_btn(
        &mut self,
        ui: &mut egui::Ui,
        center: Pos2,
        inner_r: f32,
        outer_r: f32,
        a_start: f32,
        a_end: f32,
        fill: Color32,
        press_fill: Color32,
        border: ui::DoubleBorderSpec,
        label: &str,
        font_size: f32,
        font_color: Color32,
        label_nudge: egui::Vec2,
        line_height: Option<f32>,
        font_family: egui::FontFamily,
        dec_arc: Option<ui::ArcDecorSpec>,
        notch_arc: Option<ui::ArcNotchSpec>,
        id_src: impl Hash,
        btn: (usize, u8),
    ) {
        let interact_rect = arc_quad_bounding_rect(center, inner_r, outer_r, a_start, a_end);
        let id = ui.id().with(&id_src);
        let response = ui.interact(interact_rect, id, egui::Sense::click_and_drag());
        let is_down = response.is_pointer_button_down_on();
        let latched = self.latched_btns.contains(&btn);
        let is_pressed = is_down || latched;

        let state = color_to_u32(fill);
        let cache_key = ui::draw_cache::BtnCacheKey::new(
            interact_rect,
            state,
            is_pressed,
            ui.ctx().pixels_per_point(),
        );
        let ctx = ui.ctx().clone();
        let painter = ui.painter().clone();
        let shapes = self.btn_cache.get_or_build(id, cache_key, |list| {
            ui::collect_arc_quad_button(
                list,
                &ctx,
                center,
                inner_r,
                outer_r,
                a_start,
                a_end,
                fill,
                press_fill,
                border,
                label,
                font_size,
                font_color,
                label_nudge,
                line_height,
                font_family,
                dec_arc,
                notch_arc,
                is_pressed,
            );
        });
        self.frame_shape_count += shapes.len() as u64;
        painter.extend(shapes.iter().cloned());
        let ctrl = ui.input(|i| i.modifiers.ctrl);
        self.handle_btn_interaction(is_down, ctrl, btn);
    }
}

/// Bounding rect of an arc-quadrant button - sampled along inner and outer arcs.
fn arc_quad_bounding_rect(
    center: Pos2,
    inner_r: f32,
    outer_r: f32,
    a_start: f32,
    a_end: f32,
) -> Rect {
    use std::f32::consts::PI;
    let cx = center.x;
    let cy = center.y;
    let upper = ((a_start + a_end) * 0.5).sin() < 0.0;

    let angle_at = |ia: f32, r: f32| -> f32 {
        let x = inner_r * ia.cos();
        let y_sq = r * r - x * x;
        if y_sq <= 0.0 {
            return if x < 0.0 { PI } else { 0.0 };
        }
        let y = if upper { -y_sq.sqrt() } else { y_sq.sqrt() };
        y.atan2(x)
    };
    let a_oi = angle_at(a_start, outer_r);
    let a_oe = angle_at(a_end, outer_r);

    let mk_arc = |r: f32, s: f32, e: f32| -> Vec<Pos2> {
        (0..=ARC_BTN_INTERACT_SAMPLES)
            .map(|i| {
                let t = i as f32 / ARC_BTN_INTERACT_SAMPLES as f32;
                let a = s + (e - s) * t;
                Pos2::new(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect()
    };
    let pts: Vec<Pos2> = mk_arc(inner_r, a_start, a_end)
        .into_iter()
        .chain(mk_arc(outer_r, a_oi, a_oe))
        .collect();
    Rect::from_points(&pts)
}
