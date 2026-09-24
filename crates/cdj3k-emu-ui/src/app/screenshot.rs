//! Headless one-frame capture for layout work and regression diffs.
//!
//! When `CDJ3K_SCREENSHOT=<path.png>` is set, the app lets the UI settle for
//! `CDJ3K_SCREENSHOT_FRAME` frames (default 30) and, when
//! `CDJ3K_SCREENSHOT_SECS` is set, for that many seconds as well (a booting
//! guest), requests an egui framebuffer capture, writes it to `<path>` as
//! RGBA PNG and closes the window. Combine with `--no-spawn` to capture the
//! chassis without booting a guest.

use std::sync::Arc;
use std::time::Instant;

const DEFAULT_SETTLE_FRAMES: u64 = 30;
/// Frames to wait for a scripted capture before asking for it again.
const SCRIPTED_RETRY_FRAMES: u32 = 8;

pub(super) struct ScreenshotDriver {
    path: Option<String>,
    frame: u64,
    started: Instant,
    requested: bool,
    /// A capture asked for by the input script; written without closing.
    scripted: Option<String>,
    /// Frames since the scripted capture was requested with no image yet.
    scripted_wait: u32,
}

impl ScreenshotDriver {
    pub(super) fn from_env() -> Self {
        Self {
            path: std::env::var("CDJ3K_SCREENSHOT")
                .ok()
                .filter(|s| !s.is_empty()),
            frame: 0,
            started: Instant::now(),
            requested: false,
            scripted: None,
            scripted_wait: 0,
        }
    }

    /// Capture the window to `path` on the next frame and keep running.
    pub(super) fn request_capture(&mut self, ctx: &egui::Context, path: String) {
        self.scripted = Some(path);
        self.scripted_wait = 0;
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
    }

    /// Call once per frame after drawing. Drives the capture + exit.
    pub(super) fn tick(&mut self, ctx: &egui::Context) {
        let shot: Option<Arc<egui::ColorImage>> = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            // A scripted capture comes first; the env one-shot closes the window.
            let (path, close) = match self.scripted.take() {
                Some(p) => (p, false),
                None => match self.path.clone() {
                    Some(p) => (p, true),
                    None => return,
                },
            };
            match write_png(&path, &img) {
                Ok(()) => eprintln!(
                    "cdj3k-emu: screenshot {}x{} -> {path}",
                    img.size[0], img.size[1]
                ),
                Err(e) => eprintln!("cdj3k-emu: screenshot write failed: {e}"),
            }
            if close {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        // A request can go unanswered (no frame was rendered for it); ask
        // again after a few frames rather than lose the capture.
        if self.scripted.is_some() {
            self.scripted_wait += 1;
            if self.scripted_wait >= SCRIPTED_RETRY_FRAMES {
                self.scripted_wait = 0;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            ctx.request_repaint();
        }
        if self.path.is_none() {
            return;
        }
        self.frame += 1;

        let settle: u64 = std::env::var("CDJ3K_SCREENSHOT_FRAME")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_SETTLE_FRAMES);
        let settle_secs: f64 = std::env::var("CDJ3K_SCREENSHOT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        if settle_secs > 0.0 {
            // A booting guest repaints only when its LCD changes: keep the
            // clock running rather than waiting for frames.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        if !self.requested
            && self.frame > settle
            && self.started.elapsed().as_secs_f64() >= settle_secs
        {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
        }
        ctx.request_repaint();
    }
}

fn write_png(path: &str, img: &egui::ColorImage) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(
        std::io::BufWriter::new(file),
        img.size[0] as u32,
        img.size[1] as u32,
    );
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header()?;
    let mut buf = Vec::with_capacity(img.pixels.len() * 4);
    for px in &img.pixels {
        buf.extend_from_slice(&px.to_array());
    }
    w.write_image_data(&buf)?;
    Ok(())
}
