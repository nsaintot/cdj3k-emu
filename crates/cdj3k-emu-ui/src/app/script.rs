//! Scripted panel input for headless checks: `CDJ3K_SCRIPT=<file>` plays
//! timed actions against the running panel so a model's control mapping can
//! be exercised and captured without a hand on the UI.
//!
//! One action per line, `<seconds since launch> <action> [args]`; blank
//! lines and `#` comments are skipped:
//!
//! ```text
//! 60   press PLAY            # a miso_frame BTN_* name, held 200 ms
//! 61   press 12:0x01 500     # a raw (byte, mask) frame bit, held 500 ms
//! 61.5 clear 12:0x80 500     # force a frame bit low (one the idle frame sets)
//! 62   touch 0.5 0.5 300     # LCD tap at fractions of the display, 300 ms
//! 63   shot /tmp/after.png   # capture the window (the panel keeps running)
//! 70   quit                  # close the window (graceful shutdown)
//! ```

use std::time::{Duration, Instant};

use egui::{Pos2, Rect};

use super::screenshot::ScreenshotDriver;
use super::{CdjApp, LcdTouchCapture};

const DEFAULT_HOLD_MS: u64 = 200;

enum Action {
    Press { btn: String, hold: Duration },
    Clear { bit: String, hold: Duration },
    Touch { x: f32, y: f32, hold: Duration },
    Shot(String),
    Quit,
}

enum Release {
    Btn((usize, u8)),
    Cleared((usize, u8)),
    Touch,
}

pub(super) struct ScriptDriver {
    started: Instant,
    /// Remaining actions, earliest first.
    actions: Vec<(f64, Action)>,
    releases: Vec<(Instant, Release)>,
}

impl ScriptDriver {
    pub(super) fn from_env() -> Self {
        let mut actions = Vec::new();
        if let Some(path) = std::env::var_os("CDJ3K_SCRIPT") {
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    for (n, line) in text.lines().enumerate() {
                        match parse_line(line) {
                            Ok(Some(a)) => actions.push(a),
                            Ok(None) => {}
                            Err(e) => eprintln!("cdj3k-emu: script line {}: {e}", n + 1),
                        }
                    }
                }
                Err(e) => eprintln!("cdj3k-emu: script {}: {e}", path.to_string_lossy()),
            }
        }
        actions.sort_by(|a, b| a.0.total_cmp(&b.0));
        actions.reverse();
        Self {
            started: Instant::now(),
            actions,
            releases: Vec::new(),
        }
    }

    /// Call once per frame while the panel is on screen.
    pub(super) fn tick(
        &mut self,
        app: &mut CdjApp,
        ctx: &egui::Context,
        shot: &mut ScreenshotDriver,
    ) {
        if self.actions.is_empty() && self.releases.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut i = 0;
        while i < self.releases.len() {
            if self.releases[i].0 <= now {
                match self.releases.swap_remove(i).1 {
                    Release::Btn(btn) => app.handle_btn_interaction(false, false, btn),
                    Release::Cleared(bit) => app.set_bit_cleared(bit, false),
                    Release::Touch => {
                        app.script_touch = false;
                        app.apply_lcd_touch(touch_capture(None));
                    }
                }
            } else {
                i += 1;
            }
        }
        let elapsed = self.started.elapsed().as_secs_f64();
        while self.actions.last().is_some_and(|(t, _)| *t <= elapsed) {
            let (t, action) = self.actions.pop().unwrap();
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let t = format!("{t}s @{ms}");
            match action {
                Action::Press { btn: name, hold } => {
                    let Some(btn) = app.btn_by_name(&name) else {
                        eprintln!(
                            "cdj3k-emu: script {t}s press {name}: not a button of this model"
                        );
                        continue;
                    };
                    eprintln!(
                        "cdj3k-emu: script {t}s press {name} = {}:{:#04x}",
                        btn.0, btn.1
                    );
                    app.handle_btn_interaction(true, false, btn);
                    self.releases.push((now + hold, Release::Btn(btn)));
                }
                Action::Clear { bit: name, hold } => {
                    let Some(bit) = app.btn_by_name(&name) else {
                        eprintln!("cdj3k-emu: script {t}s clear {name}: not a bit of this model");
                        continue;
                    };
                    eprintln!(
                        "cdj3k-emu: script {t}s clear {name} = {}:{:#04x}",
                        bit.0, bit.1
                    );
                    app.set_bit_cleared(bit, true);
                    self.releases.push((now + hold, Release::Cleared(bit)));
                }
                Action::Touch { x, y, hold } => {
                    eprintln!("cdj3k-emu: script {t}s touch {x} {y}");
                    app.script_touch = false;
                    app.apply_lcd_touch(touch_capture(Some(Pos2::new(x, y))));
                    app.script_touch = true;
                    self.releases.push((now + hold, Release::Touch));
                }
                Action::Shot(path) => {
                    eprintln!("cdj3k-emu: script {t}s shot {path}");
                    shot.request_capture(ctx, path);
                }
                Action::Quit => {
                    eprintln!("cdj3k-emu: script {t}s quit");
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        // Keep the clock running: a quiet guest does not repaint on its own.
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}

/// A synthetic LCD touch: the display is the unit square, so fractions map
/// straight to the frame's normalised coordinates.
fn touch_capture(pos: Option<Pos2>) -> LcdTouchCapture {
    LcdTouchCapture {
        hovered: true,
        scroll_y: 0.0,
        pointer_moved: true,
        is_down: pos.is_some(),
        ctrl: false,
        right_down: false,
        interact_pos: pos,
        display_rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
    }
}

fn parse_line(line: &str) -> Result<Option<(f64, Action)>, String> {
    let line = line.split('#').next().unwrap_or("").trim();
    if line.is_empty() {
        return Ok(None);
    }
    let mut w = line.split_whitespace();
    let t: f64 = w
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or("expected a time in seconds")?;
    let hold = |s: Option<&str>| -> Result<Duration, String> {
        match s {
            None => Ok(Duration::from_millis(DEFAULT_HOLD_MS)),
            Some(s) => s
                .parse::<u64>()
                .map(Duration::from_millis)
                .map_err(|_| format!("bad hold {s:?}")),
        }
    };
    let action = match w.next() {
        Some("press") => Action::Press {
            btn: w.next().ok_or("press needs a button")?.to_string(),
            hold: hold(w.next())?,
        },
        Some("clear") => Action::Clear {
            bit: w.next().ok_or("clear needs <byte>:<mask>")?.to_string(),
            hold: hold(w.next())?,
        },
        Some("touch") => {
            let x: f32 = w
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or("touch needs x y")?;
            let y: f32 = w
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or("touch needs x y")?;
            Action::Touch {
                x,
                y,
                hold: hold(w.next())?,
            }
        }
        Some("shot") => Action::Shot(w.next().ok_or("shot needs a path")?.to_string()),
        Some("quit") => Action::Quit,
        other => return Err(format!("unknown action {other:?}")),
    };
    Ok(Some((t, action)))
}
