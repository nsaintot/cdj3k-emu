//! The panel window's frame, kept by us on hosts whose window system keeps
//! none (AppKit's frame autosave does it on macOS).
//!
//! One line per slot and model in the app data directory: the inner size in
//! points, and the outer position where the window system reports one. A
//! Wayland client is never told where its window is, nor allowed to place it,
//! so there only the size comes back.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a frame has to stay put before it is written: a drag writes once,
/// at the end.
const SAVE_SETTLE: Duration = Duration::from_millis(500);

const FILE_NAME: &str = "window-frames.txt";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Frame {
    pub size: egui::Vec2,
    pub pos: Option<egui::Pos2>,
}

/// The frame being watched for the panel on screen.
struct Tracking {
    key: String,
    /// The size when tracking began: the picker's, until the panel's own
    /// size lands. Nothing is saved before the window leaves it.
    start_size: egui::Vec2,
    armed: bool,
    last: Option<Frame>,
    changed_at: Instant,
    saved: Option<Frame>,
}

static TRACKING: Mutex<Option<Tracking>> = Mutex::new(None);

fn path() -> PathBuf {
    crate::app_dirs::app_data_dir().join(FILE_NAME)
}

/// The saved frame for `key`, if there is one.
pub(super) fn load(key: &str) -> Option<Frame> {
    let text = std::fs::read_to_string(path()).ok()?;
    parse(&text)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, f)| f)
}

/// Start watching the window as `key`'s panel.
pub(super) fn track(ctx: &egui::Context, key: String, saved: Option<Frame>) {
    *TRACKING.lock().unwrap_or_else(|e| e.into_inner()) = Some(Tracking {
        key,
        start_size: ctx.screen_rect().size(),
        armed: false,
        last: None,
        changed_at: Instant::now(),
        saved,
    });
}

/// Stop watching: the window is no longer a panel.
pub(super) fn untrack() {
    *TRACKING.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Note the window's frame this frame, and write it once it has settled.
/// A maximized or full-screen window is not a frame to come back to.
pub(super) fn observe(ctx: &egui::Context) {
    let mut guard = TRACKING.lock().unwrap_or_else(|e| e.into_inner());
    let Some(t) = guard.as_mut() else {
        return;
    };
    let (pos, whole_screen) = ctx.input(|i| {
        let v = i.viewport();
        (
            v.outer_rect.map(|r| r.min),
            v.maximized == Some(true) || v.fullscreen == Some(true),
        )
    });
    if whole_screen {
        return;
    }
    let frame = Frame {
        size: ctx.screen_rect().size(),
        pos,
    };
    if !t.armed {
        if frame.size == t.start_size {
            return;
        }
        t.armed = true;
    }
    if t.last != Some(frame) {
        t.last = Some(frame);
        t.changed_at = Instant::now();
        ctx.request_repaint_after(SAVE_SETTLE);
        return;
    }
    if t.saved != t.last && t.changed_at.elapsed() >= SAVE_SETTLE {
        if let Err(e) = store(&t.key, frame) {
            eprintln!("cdj3k-emu: saving the window frame failed: {e}");
        }
        t.saved = Some(frame);
    }
}

fn store(key: &str, frame: Frame) -> std::io::Result<()> {
    let path = path();
    let mut entries = std::fs::read_to_string(&path)
        .map(|t| parse(&t))
        .unwrap_or_default();
    entries.retain(|(k, _)| k != key);
    entries.push((key.to_string(), frame));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, render(&entries))?;
    std::fs::rename(tmp, path)
}

/// `key width height [x y]` per line; anything else is skipped.
fn parse(text: &str) -> Vec<(String, Frame)> {
    text.lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let key = it.next()?.to_string();
            let nums: Vec<f32> = it.map(str::parse).collect::<Result<_, _>>().ok()?;
            let size = match nums[..] {
                [w, h, ..] if w > 0.0 && h > 0.0 => egui::vec2(w, h),
                _ => return None,
            };
            let pos = match nums[..] {
                [_, _] => None,
                [_, _, x, y] => Some(egui::pos2(x, y)),
                _ => return None,
            };
            Some((key, Frame { size, pos }))
        })
        .collect()
}

fn render(entries: &[(String, Frame)]) -> String {
    let mut out = String::new();
    for (key, f) in entries {
        out.push_str(&format!("{key} {} {}", f.size.x.round(), f.size.y.round()));
        if let Some(p) = f.pos {
            out.push_str(&format!(" {} {}", p.x.round(), p.y.round()));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{parse, render, Frame};

    #[test]
    fn a_frame_survives_the_file() {
        let entries = vec![
            (
                "cdj3k-emu-instance-1".to_string(),
                Frame {
                    size: egui::vec2(1000.0, 780.0),
                    pos: Some(egui::pos2(-20.0, 64.0)),
                },
            ),
            (
                "cdj3k-emu-instance-2-cdj3kx".to_string(),
                Frame {
                    size: egui::vec2(900.0, 700.0),
                    pos: None,
                },
            ),
        ];
        assert_eq!(parse(&render(&entries)), entries);
    }

    #[test]
    fn a_damaged_line_is_skipped_not_fatal() {
        let got = parse("a 10 20\nb ten 20\nc 0 20\nd 1 2 3\ne 5 6 7 8\n");
        let keys: Vec<&str> = got.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["a", "e"]);
    }
}
