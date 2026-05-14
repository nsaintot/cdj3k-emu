//! Pop-out OS windows (separate viewports) for jog LCD, main LCD, and the
//! debug pane. Each viewport mirrors content captured by the main update
//! cycle into a borrowed Arc snapshot or shared GL texture id.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use cdj3k_emu_streams::jog_stream::{JOG_FB_H, JOG_FB_W};

use super::lcd_touch::LcdTouchCapture;
use super::ui;
use super::CdjApp;
use super::MIN_FRAME_INTERVAL;

/// Mutable state owned by the deferred debug viewport. Independent of the
/// main viewport so the debug pane measures its own frame rate.
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

/// EMA factor used to smooth per-viewport fps estimates.
const FPS_ALPHA: f32 = 0.1;
/// Aspect-tolerance (px) before we re-issue an `InnerSize` to enforce ratio.
const ASPECT_FIX_TOL_PX: f32 = 2.0;
/// Main LCD aspect ratio (16:9 pixel grid).
const MAIN_LCD_ASPECT: f32 = 1280.0 / 720.0;
/// Jog LCD aspect ratio (square framebuffer).
const JOG_LCD_ASPECT: f32 = JOG_FB_W as f32 / JOG_FB_H as f32;

const MAIN_LCD_INITIAL_SIZE: [f32; 2] = [1280.0, 720.0];
const MAIN_LCD_MIN_SIZE: [f32; 2] = [640.0, 360.0];
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
        let tex_id = self.jog_tex_id;
        let close = Arc::new(AtomicBool::new(false));
        let close_inner = close.clone();
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("cdj_jog_screen"),
            egui::ViewportBuilder::default()
                .with_title(format!(
                    "{} — Jog Screen",
                    cdj3k_emu_platform::app_meta::DEVICE_NAME
                ))
                .with_inner_size(JOG_LCD_INITIAL_SIZE)
                .with_min_inner_size(JOG_LCD_MIN_SIZE),
            |inner_ctx, _class| {
                enforce_aspect_ratio(inner_ctx, JOG_LCD_ASPECT);
                handle_close_request(inner_ctx, &close_inner);
                egui::CentralPanel::default()
                    .frame(egui::Frame::none().fill(egui::Color32::BLACK))
                    .show(inner_ctx, |ui| {
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
        // Deferred viewport: own update cycle, can't borrow `&self`. Reads
        // from shared Arcs populated each main update.
        let snapshot = Arc::clone(&self.debug_snapshot);
        let vp_state = Arc::clone(&self.debug_viewport_state);
        let wants_close = Arc::clone(&self.debug_wants_close);
        // Live MOSI peek bypasses the snapshot path so debug shows real-time
        // wire bytes even when the main viewport's repaint filter is quiet.
        let live_mosi = self.ctrl_stream.latest_mosi_arc();

        ctx.show_viewport_deferred(
            egui::ViewportId::from_hash_of("cdj_debug"),
            egui::ViewportBuilder::default()
                .with_title("Debug Window")
                .with_inner_size(DEBUG_INITIAL_SIZE),
            move |inner_ctx, _class| {
                let local_fps = update_local_fps(&vp_state);

                if inner_ctx.input(|i| i.viewport().close_requested()) {
                    wants_close.store(true, Ordering::Relaxed);
                    inner_ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }

                let mut snap = snapshot.lock().map(|g| g.clone()).unwrap_or_default();
                if let Ok(g) = live_mosi.lock() {
                    snap.led_frame = *g;
                }
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::none()
                            .fill(DEBUG_BG)
                            .inner_margin(egui::Margin::same(DEBUG_INNER_MARGIN)),
                    )
                    .show(inner_ctx, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui::draw_debug_content(ui, &snap, local_fps);
                        });
                    });

                // Self-driven 60 Hz tick. We can't use cross-thread repaint
                // hints from the metronome here: on macOS, `request_repaint`
                // from a background thread doesn't reliably wake a *deferred*
                // viewport's event loop. Calling `request_repaint_after` from
                // *inside* this closure goes through the in-thread scheduling
                // path (capped only by ProMotion vsync ≤120 Hz, but reliable).
                inner_ctx.request_repaint_after(MIN_FRAME_INTERVAL);
            },
        );

        if self.debug_wants_close.swap(false, Ordering::Relaxed) {
            self.debug_screen_popped = false;
        }
    }

    pub(super) fn show_main_viewport(&mut self, ctx: &egui::Context) {
        if !self.main_screen_popped {
            return;
        }
        let tex_id = self.display_tex_id;
        let connected = self.display_stream.is_connected();
        let close = Arc::new(AtomicBool::new(false));
        let close_inner = close.clone();
        let mut popout_touch: Option<LcdTouchCapture> = None;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("cdj_main_screen"),
            egui::ViewportBuilder::default()
                .with_title(format!(
                    "{} — Main Screen",
                    cdj3k_emu_platform::app_meta::DEVICE_NAME
                ))
                .with_inner_size(MAIN_LCD_INITIAL_SIZE)
                .with_min_inner_size(MAIN_LCD_MIN_SIZE),
            |inner_ctx, _class| {
                // Use the *inner* viewport's Context (not the outer captured
                // one) so input/screen_rect read from the popout window.
                enforce_aspect_ratio(inner_ctx, MAIN_LCD_ASPECT);
                handle_close_request(inner_ctx, &close_inner);
                egui::CentralPanel::default()
                    .frame(egui::Frame::none().fill(egui::Color32::BLACK))
                    .show(inner_ctx, |ui| {
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
                            let rect = paint_aspect_fit_image(ui, tex_id, MAIN_LCD_ASPECT, tint);
                            popout_touch = Some(capture_lcd_touch(ui, rect));
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

/// Re-issue an `InnerSize` viewport command if the current size's aspect
/// ratio drifts from `aspect` by more than [`ASPECT_FIX_TOL_PX`].
fn enforce_aspect_ratio(ctx: &egui::Context, aspect: f32) {
    let size = ctx.screen_rect().size();
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

fn capture_lcd_touch(ui: &mut egui::Ui, display_rect: egui::Rect) -> LcdTouchCapture {
    let lcd_resp = ui.interact(
        display_rect,
        ui.id().with("lcd_touch_popout"),
        egui::Sense::click_and_drag(),
    );
    LcdTouchCapture {
        hovered: lcd_resp.hovered(),
        scroll_y: ui.input(|i| i.raw_scroll_delta.y),
        pointer_moved: ui.input(|i| i.pointer.delta().length_sq() > 0.0),
        is_down: lcd_resp.is_pointer_button_down_on(),
        ctrl: ui.input(|i| i.modifiers.ctrl),
        right_down: ui.input(|i| {
            i.pointer.button_down(egui::PointerButton::Secondary)
                && i.pointer
                    .hover_pos()
                    .map_or(false, |p| display_rect.contains(p))
        }),
        interact_pos: lcd_resp.interact_pointer_pos(),
        display_rect,
    }
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
