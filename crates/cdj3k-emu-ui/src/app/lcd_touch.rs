//! Main LCD touch input handling - capture from any viewport (main or popout)
//! and apply to the per-frame MISO state (touch coords, navigation rotary,
//! ctrl-latched touch).

use cdj3k_emu_subucom::miso_frame;

use super::frame_inject::LCD_TOUCH_RANGE;
use super::CdjApp;

/// Raw LCD touch input captured from a viewport frame, applied via
/// [`CdjApp::apply_lcd_touch`].
pub(crate) struct LcdTouchCapture {
    pub hovered: bool,
    pub scroll_y: f32,
    pub pointer_moved: bool,
    pub is_down: bool,
    pub ctrl: bool,
    pub right_down: bool,
    pub interact_pos: Option<egui::Pos2>,
    pub display_rect: egui::Rect,
}

impl CdjApp {
    /// Apply raw LCD touch input captured from any viewport (main or popout).
    pub(crate) fn apply_lcd_touch(&mut self, cap: LcdTouchCapture) {
        use super::ui::{NAV_DETENT_COUNT, NAV_SCROLL_PX_PER_TICK};

        if cap.hovered {
            if cap.scroll_y != 0.0 {
                self.consume_lcd_scroll(cap.scroll_y, NAV_DETENT_COUNT, NAV_SCROLL_PX_PER_TICK);
            }
            if cap.pointer_moved {
                self.lcd_nav_mode = false;
            }
        }

        if self.lcd_nav_mode {
            self.handle_btn_interaction(cap.is_down, cap.ctrl, miso_frame::BTN_ROTARY_PRESS);
            self.handle_btn_interaction(cap.right_down, false, miso_frame::BTN_BACK);
            return;
        }

        if cap.is_down {
            self.handle_lcd_press(&cap);
        } else if self.lcd_touch_ctrl_latched {
            // Mouse released while Ctrl is still held: keep the latched touch
            // alive until Ctrl is released.
            if !cap.ctrl {
                self.clear_lcd_touch();
            }
        } else if self.lcd_touch.is_some() {
            self.clear_lcd_touch();
        }
    }

    fn consume_lcd_scroll(&mut self, scroll_y: f32, detent_count: f32, px_per_tick: f32) {
        self.nav_scroll_accum += scroll_y;
        let detent_rad = std::f32::consts::TAU / detent_count;
        while self.nav_scroll_accum >= px_per_tick {
            self.nav_scroll_accum -= px_per_tick;
            self.nav_angle += detent_rad;
            self.rotary = self.rotary.wrapping_sub(1);
            self.inject_rotary();
        }
        while self.nav_scroll_accum <= -px_per_tick {
            self.nav_scroll_accum += px_per_tick;
            self.nav_angle -= detent_rad;
            self.rotary = self.rotary.wrapping_add(1);
            self.inject_rotary();
        }
        self.lcd_nav_mode = true;
    }

    fn handle_lcd_press(&mut self, cap: &LcdTouchCapture) {
        if self.lcd_touch_ctrl_latched {
            if !cap.ctrl {
                self.clear_lcd_touch();
            }
            return;
        }
        if let Some(pos) = cap.interact_pos {
            // X is mirrored: the device's origin is at the right edge.
            let nx = (LCD_TOUCH_RANGE
                - (pos.x - cap.display_rect.left()) / cap.display_rect.width() * LCD_TOUCH_RANGE)
                as u16;
            let ny = ((pos.y - cap.display_rect.top()) / cap.display_rect.height()
                * LCD_TOUCH_RANGE) as u16;
            if self.lcd_touch != Some((nx, ny)) {
                self.lcd_touch = Some((nx, ny));
                self.inject_touch(nx, ny);
            }
        }
        if cap.ctrl {
            self.lcd_touch_ctrl_latched = true;
        }
    }

    fn clear_lcd_touch(&mut self) {
        self.lcd_touch = None;
        self.lcd_touch_ctrl_latched = false;
        self.inject_touch(0, 0);
    }
}
