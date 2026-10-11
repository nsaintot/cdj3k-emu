//! Pop-out OS windows (separate viewports) for the jog LCD, the main LCD and
//! the debug pane. Each viewport shows content that the main update cycle
//! captures into a shared Arc snapshot or a shared GL texture id.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use cdj3k_emu_streams::jog_stream::{JOG_FB_H, JOG_FB_W};

use super::lcd_touch::LcdTouchCapture;
use super::ui;
use super::ui::draw_lcd::capture_lcd_touch;
use super::CdjApp;
use super::MIN_FRAME_INTERVAL;

/// Mutable state owned by the deferred debug viewport. It is separate from
/// the main viewport's state, so the debug pane measures its own frame rate.
#[derive(Clone, Copy)]
pub(crate) struct DebugViewportState {
    pub last_paint_at: Instant,
    pub fps_smooth: f32,
}

const INITIAL_FPS_ESTIMATE: f32 = 60.0;

impl Default for DebugViewportState {
    fn default() -> Self {
        Self {
            last_paint_at: Instant::now() - MIN_FRAME_INTERVAL,
            fps_smooth: INITIAL_FPS_ESTIMATE,
        }
    }
}

/// The EMA factor that smooths each viewport's fps estimate.
const FPS_ALPHA: f32 = 0.1;
/// The aspect error, in pixels, above which an `InnerSize` command is sent
/// again to restore the aspect ratio.
const ASPECT_FIX_TOL_PX: f32 = 2.0;
/// The jog LCD's aspect ratio, from its 320 x 240 framebuffer.
const JOG_LCD_ASPECT: f32 = JOG_FB_W as f32 / JOG_FB_H as f32;

const JOG_LCD_INITIAL_SIZE: [f32; 2] = [320.0, 240.0];
const JOG_LCD_MIN_SIZE: [f32; 2] = [160.0, 120.0];
const DEBUG_INITIAL_SIZE: [f32; 2] = [500.0, 540.0];
const DEBUG_BG: egui::Color32 = egui::Color32::from_rgb(20, 20, 22);
const DEBUG_INNER_MARGIN: f32 = 8.0;
const DISCONNECTED_TINT_ALPHA: u8 = 60;

impl CdjApp {
    pub(super) fn show_jog_viewport(&mut self, ctx: &egui::Context) {
        if !self.jog_screen_popped {
            return;
        }
        let tex_id = self.textures.jog_id;
        let close = Arc::new(AtomicBool::new(false));
        let close_inner = close.clone();
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("cdj_jog_screen"),
            egui::ViewportBuilder::default()
                .with_title(format!("{} — Jog Screen", self.model.title()))
                .with_inner_size(JOG_LCD_INITIAL_SIZE)
                .with_min_inner_size(JOG_LCD_MIN_SIZE),
            |root, _class| {
                enforce_aspect_ratio(root, JOG_LCD_ASPECT);
                handle_close_request(root, &close_inner);
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
                    .show(root, |ui| {
                        if let Some(tex_id) = tex_id {
                            paint_aspect_fit_image(
                                ui,
                                tex_id,
                                JOG_LCD_ASPECT,
                                egui::Color32::WHITE,
                            );
                        }
                    });
            },
        );
        if close.load(Ordering::Relaxed) {
            self.jog_screen_popped = false;
        }
    }

    pub(super) fn show_debug_viewport(&mut self, ctx: &egui::Context) {
        if !self.debug_screen_popped {
            return;
        }
        // A deferred viewport has its own update cycle and cannot borrow
        // `&self`, so it reads from shared Arcs that each main update fills.
        let snapshot = Arc::clone(&self.debug.snapshot);
        let vp_state = Arc::clone(&self.debug.viewport_state);
        let wants_close = Arc::clone(&self.debug.wants_close);
        // Read the live MOSI frame directly, bypassing the snapshot, so the
        // debug pane shows the current wire bytes while the main viewport is
        // not repainting.
        let live_mosi = self.ctrl_stream.latest_mosi_arc();

        ctx.show_viewport_deferred(
            egui::ViewportId::from_hash_of("cdj_debug"),
            egui::ViewportBuilder::default()
                .with_title("Debug Window")
                .with_inner_size(DEBUG_INITIAL_SIZE),
            move |root, _class| {
                let local_fps = update_local_fps(&vp_state);

                if root.input(|i| i.viewport().close_requested()) {
                    wants_close.store(true, Ordering::Relaxed);
                    root.send_viewport_cmd(egui::ViewportCommand::Close);
                }

                let mut snap = snapshot.lock().map(|g| g.clone()).unwrap_or_default();
                if let Ok(g) = live_mosi.lock() {
                    snap.led_frame = *g;
                }
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::NONE
                            .fill(DEBUG_BG)
                            .inner_margin(egui::Margin::from(DEBUG_INNER_MARGIN)),
                    )
                    .show(root, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui::draw_debug_content(ui, &snap, local_fps);
                        });
                    });

                // Schedule the next frame at 60 Hz from inside the viewport.
                // On macOS, `request_repaint` from a background thread such as
                // the metronome does not reliably wake a deferred viewport's
                // event loop. `request_repaint_after` inside this closure uses
                // the in-thread scheduling path, which is reliable and limited
                // only by the ProMotion refresh rate of up to 120 Hz.
                root.request_repaint_after(MIN_FRAME_INTERVAL);
            },
        );

        if self.debug.wants_close.swap(false, Ordering::Relaxed) {
            self.debug_screen_popped = false;
        }
    }

    pub(super) fn show_main_viewport(&mut self, ctx: &egui::Context) {
        if !self.main_screen_popped {
            return;
        }
        let tex_id = self.textures.main_id;
        let connected = self.display_stream.is_connected();
        // The pop-out opens at the framebuffer's native size and keeps its
        // aspect ratio.
        let (fb_w, fb_h) = self.model.main_lcd();
        let main_aspect = fb_w as f32 / fb_h as f32;
        let main_initial = [fb_w as f32, fb_h as f32];
        let main_min = [fb_w as f32 * 0.5, fb_h as f32 * 0.5];
        let close = Arc::new(AtomicBool::new(false));
        let close_inner = close.clone();
        let mut popout_touch: Option<LcdTouchCapture> = None;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("cdj_main_screen"),
            egui::ViewportBuilder::default()
                .with_title(format!("{} — Main Screen", self.model.title()))
                .with_inner_size(main_initial)
                .with_min_inner_size(main_min),
            |root, _class| {
                // Use the pop-out's own root, so input and the window size
                // come from the pop-out window.
                enforce_aspect_ratio(root, main_aspect);
                handle_close_request(root, &close_inner);
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
                    .show(root, |ui| {
                        if let Some(tex_id) = tex_id {
                            let tint = if connected {
                                egui::Color32::WHITE
                            } else {
                                egui::Color32::from_rgba_unmultiplied(
                                    255,
                                    255,
                                    255,
                                    DISCONNECTED_TINT_ALPHA,
                                )
                            };
                            let rect = paint_aspect_fit_image(ui, tex_id, main_aspect, tint);
                            popout_touch = Some(capture_lcd_touch(ui, rect, "lcd_touch_popout"));
                        }
                    });
            },
        );
        if close.load(Ordering::Relaxed) {
            self.main_screen_popped = false;
        }
        if let Some(cap) = popout_touch {
            self.apply_lcd_touch(cap);
        }
    }
}

/// Send an `InnerSize` viewport command again if the current size's aspect
/// ratio drifts from `aspect` by more than [`ASPECT_FIX_TOL_PX`].
fn enforce_aspect_ratio(ctx: &egui::Context, aspect: f32) {
    let size = ctx.content_rect().size();
    let ideal_h = size.x / aspect;
    if (ideal_h - size.y).abs() > ASPECT_FIX_TOL_PX {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::Vec2::new(
            size.x, ideal_h,
        )));
    }
}

fn handle_close_request(ctx: &egui::Context, close_flag: &Arc<AtomicBool>) {
    if ctx.input(|i| i.viewport().close_requested()) {
        close_flag.store(true, Ordering::Relaxed);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

/// Paint `tex_id` centred in the available space, aspect-fit to `aspect`.
/// Returns the actual rect drawn so the caller can attach interaction.
fn paint_aspect_fit_image(
    ui: &mut egui::Ui,
    tex_id: egui::TextureId,
    aspect: f32,
    tint: egui::Color32,
) -> egui::Rect {
    let avail = ui.available_size();
    let (w, h) = if avail.x / avail.y > aspect {
        (avail.y * aspect, avail.y)
    } else {
        (avail.x, avail.x / aspect)
    };
    let rect = egui::Rect::from_center_size(
        ui.available_rect_before_wrap().center(),
        egui::Vec2::new(w, h),
    );
    ui.painter().image(
        tex_id,
        rect,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
        tint,
    );
    rect
}

fn update_local_fps(vp_state: &Arc<Mutex<DebugViewportState>>) -> f32 {
    let mut state = vp_state.lock().unwrap();
    let now = Instant::now();
    let dt = now
        .saturating_duration_since(state.last_paint_at)
        .as_secs_f32()
        .max(1.0e-4);
    state.fps_smooth = FPS_ALPHA * (1.0 / dt) + (1.0 - FPS_ALPHA) * state.fps_smooth;
    state.last_paint_at = now;
    state.fps_smooth
}
