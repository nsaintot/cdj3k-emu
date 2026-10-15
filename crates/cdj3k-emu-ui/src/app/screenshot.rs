//! Captures one frame of the window for layout work and regression diffs,
//! and for the control API's `shot`.
//!
//! When `CDJ3K_SCREENSHOT=<path.png>` is set, the app lets the UI settle for
//! `CDJ3K_SCREENSHOT_FRAME` frames (default 30) and, when
//! `CDJ3K_SCREENSHOT_SECS` is set, for that many seconds as well, which suits
//! a booting guest. It then requests an egui framebuffer capture, writes it
//! to `<path>` as an RGBA PNG and closes the window. Combine with `--no-spawn` to capture the
//! chassis without booting a guest.

use std::sync::Arc;
use std::time::Instant;

const DEFAULT_SETTLE_FRAMES: u64 = 30;
/// Frames to wait for an API capture before asking for it again.
const API_RETRY_FRAMES: u32 = 8;

pub(super) struct ScreenshotDriver {
    path: Option<String>,
    frame: u64,
    started: Instant,
    requested: bool,
    /// Whether the control API asked for a capture that has not arrived.
    api_wanted: bool,
    /// Frames since the API capture was requested without an image
    /// arriving.
    api_wait: u32,
    /// The API capture, until the control host takes it.
    api_image: Option<Arc<egui::ColorImage>>,
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
            api_wanted: false,
            api_wait: 0,
            api_image: None,
        }
    }

    /// Capture the window on the next frame for the control API; see
    /// [`Self::take_image`].
    pub(super) fn request_image(&mut self, ctx: &egui::Context) {
        self.api_wanted = true;
        self.api_wait = 0;
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
    }

    /// The capture that [`Self::request_image`] asked for, once it arrived.
    pub(super) fn take_image(&mut self) -> Option<Arc<egui::ColorImage>> {
        self.api_image.take()
    }

    /// Call once per frame after drawing. It drives the capture and the exit.
    pub(super) fn tick(&mut self, ctx: &egui::Context) {
        let shot: Option<Arc<egui::ColorImage>> = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            if std::mem::take(&mut self.api_wanted) {
                self.api_image = Some(img);
            } else {
                self.write_capture(ctx, &img);
            }
            return;
        }
        // A request can go unanswered when no frame is rendered for it, so
        // ask again after a few frames.
        if self.api_wanted {
            self.api_wait += 1;
            if self.api_wait >= API_RETRY_FRAMES {
                self.api_wait = 0;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            ctx.request_repaint();
        }
        if self.path.is_some() {
            self.settle_env_capture(ctx);
        }
    }

    /// Write the capture requested by `CDJ3K_SCREENSHOT`, then close the
    /// window.
    fn write_capture(&mut self, ctx: &egui::Context, img: &egui::ColorImage) {
        let Some(path) = self.path.clone() else {
            return;
        };
        match write_png(&path, img) {
            Ok(()) => eprintln!(
                "cdj3k-emu: Wrote a {}x{} screenshot to {path}.",
                img.size[0], img.size[1]
            ),
            Err(e) => eprintln!("cdj3k-emu: Could not write the screenshot: {e}"),
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Count the env capture's settle frames and seconds, and request it once
    /// both have passed.
    fn settle_env_capture(&mut self, ctx: &egui::Context) {
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
            // A booting guest repaints only when its LCD changes, so keep
            // requesting frames to let the clock advance.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        if !self.requested
            && self.frame > settle
            && self.started.elapsed().as_secs_f64() >= settle_secs
        {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
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
