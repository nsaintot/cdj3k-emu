//! JogLcdStream receives jog LCD frames from the guest through ivshmem.
//!
//! - On every flip, the guest's `deck_shim.so` extracts the visible 320×240
//!   region from EP122's 1280×240 stretched DRM framebuffer and writes the
//!   XRGB8888 pixels into an ivshmem BAR. The host sees the same memory as the
//!   file `{socket_dir}/jog.shm`, which this module maps read-only.
//! - Each publish increments a 32-bit `seq` counter, seqlock style: an odd
//!   value means a write is in progress and an even value means the frame is
//!   stable. This thread polls `seq` at about 60 Hz; when it advances, the
//!   thread copies the pixels under the seqlock and publishes a `JogFrame`.
//!
//! The layout must match `JOG_SHM_*` in `guest/deck_shim/core/deck_shim.h`:
//!
//! | offset | type | field |
//! |---|---|---|
//! | 0x0000 | u32 | magic, 'JOG1' (LE 0x31474F4A) |
//! | 0x0004 | u32 | seq, the seqlock counter (odd while a write is in progress) |
//! | 0x0008 | u16 | width |
//! | 0x000A | u16 | height |
//! | 0x000C | u32 | format, 1 for XRGB8888 |
//! | 0x1000 | bytes | pixels, width*height*4 bytes |

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use memmap2::Mmap;

use crate::shm::View;

/// Jog LCD width after the guest's crop, which is the physical panel's width.
pub const JOG_FB_W: usize = 320;
/// Jog LCD height, the same as the DRM jog plane's height.
pub const JOG_FB_H: usize = 240;

const FRAME_BYTES: usize = JOG_FB_W * JOG_FB_H * 4;

const SHM_MAGIC: u32 = 0x3147_4F4A; // 'JOG1' little-endian
const SHM_FMT_XRGB8888: u32 = 1;
const SHM_PIXELS_OFF: usize = 0x1000;
const SHM_OFF_MAGIC: usize = 0x0000;
const SHM_OFF_SEQ: usize = 0x0004;
const SHM_OFF_W: usize = 0x0008;
const SHM_OFF_H: usize = 0x000A;
const SHM_OFF_FMT: usize = 0x000C;

/// Poll period for `seq`. 16 ms is about 60 Hz, which matches the upstream jog
/// refresh and is well below the guest's publish rate of about 100 Hz.
const POLL_INTERVAL: Duration = Duration::from_millis(16);

/// If `seq` does not advance for this long, the stream is marked disconnected
/// so the UI can show a "wait" badge. The guest publishes on every DRM flip, so
/// a 2 s gap means EP122 has stalled or the guest has not booted yet.
const ALIVE_TIMEOUT: Duration = Duration::from_secs(2);

/// Horizontal inset from the left and right edges, as a fraction of the width.
/// It is larger than the vertical inset because the panel is 4:3.
const CORNER_SAMPLE_FRAC_X: f32 = 0.2;
/// Vertical inset from the top and bottom edges, as a fraction of the height.
const CORNER_SAMPLE_FRAC_Y: f32 = 0.1;

/// Framebuffer `(x, y)` pixel indices that [`JogFrame::corner_rgba`] samples,
/// in the same order: top-left, top-right, bottom-left and bottom-right (SLIP,
/// VINYL, SYNC and MASTER).
pub fn jog_corner_probe_fb_pixels() -> [(usize, usize); 4] {
    let dx = ((JOG_FB_W as f32) * CORNER_SAMPLE_FRAC_X).round() as usize;
    let dy = ((JOG_FB_H as f32) * CORNER_SAMPLE_FRAC_Y).round() as usize;
    let dx = dx.min(JOG_FB_W.saturating_sub(1));
    let dy = dy.min(JOG_FB_H.saturating_sub(1));
    let xm = JOG_FB_W.saturating_sub(1 + dx);
    let ym = JOG_FB_H.saturating_sub(1 + dy);
    [(dx, dy), (xm, dy), (dx, ym), (xm, ym)]
}

/// A jog LCD frame and its dirty region. `image` holds the wire-format XRGB
/// bytes (B, G, R, X); the GPU reorders them through a channel swizzle.
pub struct JogFrame {
    pub image: std::sync::Arc<egui::ColorImage>,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Non-premultiplied RGBA sampled from the full canvas at SLIP, VINYL, SYNC
    /// and MASTER (top-left, top-right, bottom-left and bottom-right).
    pub corner_rgba: [[u8; 4]; 4],
}

/// Reads the jog LCD ivshmem region on a background thread.
pub struct JogLcdStream {
    slot: Arc<Mutex<Option<JogFrame>>>,
    connected: Arc<AtomicBool>,
    shm_path: String,
}

impl JogLcdStream {
    pub fn new(socket_dir: &str, gate: crate::RepaintGate) -> Self {
        let dir = socket_dir.trim_end_matches('/').to_string();
        let shm_path = format!("{dir}/jog.shm");
        let slot: Arc<Mutex<Option<JogFrame>>> = Arc::new(Mutex::new(None));
        let slot_clone = Arc::clone(&slot);
        let connected = Arc::new(AtomicBool::new(false));
        let connected_clone = Arc::clone(&connected);
        let path = shm_path.clone();

        thread::Builder::new()
            .name("jog-lcd-stream".into())
            .spawn(move || stream_loop(&path, slot_clone, connected_clone, gate))
            .expect("the jog-lcd-stream thread could not be started");

        Self {
            slot,
            connected,
            shm_path,
        }
    }

    pub fn take(&self) -> Option<JogFrame> {
        self.slot.lock().ok()?.take()
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    #[allow(dead_code)]
    pub fn addr_str(&self) -> &str {
        &self.shm_path
    }
}

/// Map the ivshmem-backed file once QEMU runs. cdj3k-emu-runtime sets it to
/// 1 MiB before it launches QEMU, so the file exists once the instance has
/// spawned.
fn open_shm(shm_path: &str) -> Mmap {
    let mut log_attempts: u32 = 0;
    loop {
        if crate::guest::wait_until_running() {
            log_attempts = 0;
        }
        match std::fs::OpenOptions::new().read(true).open(shm_path) {
            Ok(f) => match unsafe { Mmap::map(&f) } {
                Ok(m) if m.len() >= SHM_PIXELS_OFF + FRAME_BYTES => return m,
                Ok(m) => {
                    eprintln!(
                        "[jog_stream] {shm_path} is too small ({} bytes); waiting.",
                        m.len()
                    );
                }
                Err(e) => eprintln!("[jog_stream] Mapping {shm_path} failed: {e}"),
            },
            Err(e) => {
                log_attempts += 1;
                if log_attempts == 1 || log_attempts.is_multiple_of(10) {
                    eprintln!(
                        "[jog_stream] Opening {shm_path} failed: {e} (attempt {log_attempts})"
                    );
                }
            }
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn stream_loop(
    shm_path: &str,
    slot: Arc<Mutex<Option<JogFrame>>>,
    connected: Arc<AtomicBool>,
    gate: crate::RepaintGate,
) {
    let sink = JogSink {
        slot,
        connected,
        gate,
    };
    let mut bufs = FrameBuffers::new();

    // Map the file again whenever the QEMU worker exits or restarts. The
    // runtime unlinks jog.shm on cleanup and creates it again, with a new
    // inode, on spawn, so a held mapping would freeze on the previous
    // session's last frame. The cleanup hook zeros the magic before the
    // unlink, which tells this loop to drop the mapping and reopen it.
    // The mapping is logged once, and again after each session that carried
    // frames. Until the guest publishes (a deck without a jog LCD never does),
    // it is remapped silently.
    let mut announce = true;
    loop {
        let shm = open_shm(shm_path);
        if announce {
            eprintln!("[jog_stream] Mapped {shm_path} ({} bytes).", shm.len());
            announce = false;
        }

        // Before the first publish, the file is remapped at the reconnect
        // pace below, which picks up the new file after a restart.
        if poll_session(View::new(&shm), &mut bufs, &sink) {
            eprintln!("[jog_stream] The shm magic was cleared; reconnecting.");
            announce = true;
        }
        sink.disconnect();

        // Drop the old Mmap explicitly before reopening so the kernel can
        // reclaim the stale inode's pages.
        drop(shm);
        thread::sleep(Duration::from_millis(500));
    }
}

/// Where frames go: the slot the UI takes them from, the connected flag and
/// the repaint request.
struct JogSink {
    slot: Arc<Mutex<Option<JogFrame>>>,
    connected: Arc<AtomicBool>,
    gate: crate::RepaintGate,
}

impl JogSink {
    fn disconnect(&self) {
        if self.connected.swap(false, Ordering::Relaxed) {
            self.gate.request();
        }
    }

    fn publish(&self, image: Arc<egui::ColorImage>, corner_rgba: [[u8; 4]; 4]) {
        if !self.connected.load(Ordering::Relaxed) {
            self.connected.store(true, Ordering::Relaxed);
        }
        if let Ok(mut g) = self.slot.try_lock() {
            *g = Some(JogFrame {
                image,
                x: 0,
                y: 0,
                w: JOG_FB_W as u32,
                h: JOG_FB_H as u32,
                corner_rgba,
            });
            self.gate.request();
        }
    }
}

/// Two `Arc<ColorImage>` buffers used in turn, so the receiver can read one
/// while the next frame is written to the other, without reallocating the
/// 76 800-pixel Vec.
struct FrameBuffers {
    bufs: [Arc<egui::ColorImage>; 2],
    next: usize,
}

impl FrameBuffers {
    fn new() -> Self {
        Self {
            bufs: [blank_frame(), blank_frame()],
            next: 0,
        }
    }

    /// Returns a buffer that no reader holds: the next one if it is free, else
    /// the other one, else a fresh buffer that replaces the other one.
    fn writable(&mut self) -> &mut egui::ColorImage {
        if Arc::get_mut(&mut self.bufs[self.next]).is_none() {
            self.next ^= 1;
            if Arc::get_mut(&mut self.bufs[self.next]).is_none() {
                self.bufs[self.next] = blank_frame();
            }
        }
        Arc::get_mut(&mut self.bufs[self.next]).expect("buffer is unshared")
    }

    /// Hand out the buffer just written and move on to the other.
    fn publish(&mut self) -> Arc<egui::ColorImage> {
        let image = Arc::clone(&self.bufs[self.next]);
        self.next ^= 1;
        image
    }
}

fn blank_frame() -> Arc<egui::ColorImage> {
    Arc::new(egui::ColorImage::filled(
        [JOG_FB_W, JOG_FB_H],
        egui::Color32::BLACK,
    ))
}

/// Per-mapping poll state.
#[derive(Default)]
struct Session {
    last_seq: u32,
    last_change_at: Option<Instant>,
    frame_count: u64,
}

/// Poll one mapping until its magic goes. Returns whether the magic was ever
/// seen, i.e. whether the guest published into this mapping.
fn poll_session(shm: View<'_>, bufs: &mut FrameBuffers, sink: &JogSink) -> bool {
    let mut session = Session::default();
    let mut seen_magic = false;
    while shm.u32_at(SHM_OFF_MAGIC, Ordering::Relaxed) == SHM_MAGIC {
        seen_magic = true;
        session.tick(shm, bufs, sink);
        thread::sleep(POLL_INTERVAL);
    }
    seen_magic
}

impl Session {
    /// One poll: publish the frame the guest wrote since the last one, if any.
    fn tick(&mut self, shm: View<'_>, bufs: &mut FrameBuffers, sink: &JogSink) {
        let seq = shm.u32_at(SHM_OFF_SEQ, Ordering::Acquire);
        if seq == self.last_seq || (seq & 1) != 0 {
            if self
                .last_change_at
                .is_some_and(|t| t.elapsed() > ALIVE_TIMEOUT)
            {
                sink.disconnect();
            }
            return;
        }
        if !header_is_xrgb_panel(shm) {
            return;
        }

        let canvas = color_image_bytes_mut(bufs.writable());
        let Some(seq_now) = copy_stable(shm, canvas) else {
            return;
        };
        if seq_now == self.last_seq {
            return;
        }
        self.last_seq = seq_now;
        self.last_change_at = Some(Instant::now());

        let corners = corner_colors_from_canvas(canvas);
        if self.frame_count == 0 {
            let nonzero = canvas.iter().any(|&b| b != 0);
            eprintln!("[jog_stream] The first frame arrived (nonzero={nonzero}, seq={seq_now}).");
        }
        self.frame_count += 1;
        sink.publish(bufs.publish(), corners);
    }
}

/// One copy of the jog LCD as opaque RGBA rows: `(width, height, bytes)`,
/// read from `{socket_dir}/jog.shm` under its seqlock.
pub fn capture(socket_dir: &str) -> Result<(u32, u32, Vec<u8>), String> {
    let path = format!("{}/jog.shm", socket_dir.trim_end_matches('/'));
    let file = std::fs::File::open(&path).map_err(|_| "the jog LCD is not up")?;
    // SAFETY: the mapping is read-only, and every read goes through `View`.
    let mmap = unsafe { Mmap::map(&file) }.map_err(|e| format!("mapping {path} failed: {e}"))?;
    if mmap.len() < SHM_PIXELS_OFF + FRAME_BYTES {
        return Err("the jog LCD is not up".into());
    }
    let shm = View::new(&mmap);
    if shm.u32_at(SHM_OFF_MAGIC, Ordering::Relaxed) != SHM_MAGIC || !header_is_xrgb_panel(shm) {
        return Err("the guest has not drawn the jog LCD yet".into());
    }
    let mut out = vec![0u8; FRAME_BYTES];
    copy_stable(shm, &mut out).ok_or("the jog LCD changed during every copy")?;
    for px in out.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
        px[3] = 0xff;
    }
    Ok((JOG_FB_W as u32, JOG_FB_H as u32, out))
}

/// Whether the header announces XRGB8888 at the panel's size; logs what it
/// announces otherwise.
fn header_is_xrgb_panel(shm: View<'_>) -> bool {
    let fmt = shm.u32_at(SHM_OFF_FMT, Ordering::Relaxed);
    if fmt != SHM_FMT_XRGB8888 {
        eprintln!("[jog_stream] The frame format {fmt} is not XRGB8888.");
        return false;
    }
    let w = shm.u16_at(SHM_OFF_W) as usize;
    let h = shm.u16_at(SHM_OFF_H) as usize;
    if w != JOG_FB_W || h != JOG_FB_H {
        eprintln!("[jog_stream] The frame size {w}x{h} does not match the panel.");
        return false;
    }
    true
}

/// Copies the pixels into `canvas` under the seqlock. Returns the `seq` of the
/// copy, or `None` if none of four attempts found `seq` stable.
fn copy_stable(shm: View<'_>, canvas: &mut [u8]) -> Option<u32> {
    for _ in 0..4 {
        let seq_before = shm.u32_at(SHM_OFF_SEQ, Ordering::Acquire);
        if seq_before & 1 != 0 {
            std::hint::spin_loop();
            continue;
        }
        shm.copy_to(SHM_PIXELS_OFF, canvas);
        if shm.u32_at(SHM_OFF_SEQ, Ordering::Relaxed) == seq_before {
            return Some(seq_before);
        }
    }
    None
}

/// Sample the canvas at (x, y) and return RGBA bytes for the UI. The canvas
/// holds wire-format XRGB bytes (B, G, R, X), which the GPU reorders through a
/// channel swizzle; `corner_rgba` goes to `Color32::from_rgba_unmultiplied`,
/// so the bytes are swapped here.
fn sample_corner_rgba(canvas: &[u8], x: usize, y: usize) -> [u8; 4] {
    let i = (y * JOG_FB_W + x) * 4;
    if i + 4 <= canvas.len() {
        [canvas[i + 2], canvas[i + 1], canvas[i], 255]
    } else {
        [0, 0, 0, 255]
    }
}

fn corner_colors_from_canvas(canvas: &[u8]) -> [[u8; 4]; 4] {
    let [(x0, y0), (x1, y1), (x2, y2), (x3, y3)] = jog_corner_probe_fb_pixels();
    [
        sample_corner_rgba(canvas, x0, y0),
        sample_corner_rgba(canvas, x1, y1),
        sample_corner_rgba(canvas, x2, y2),
        sample_corner_rgba(canvas, x3, y3),
    ]
}

/// Reinterpret a `ColorImage`'s pixel storage as a mutable byte slice. This is
/// sound because [`egui::Color32`] is `#[repr(C)] [u8; 4]`.
fn color_image_bytes_mut(img: &mut egui::ColorImage) -> &mut [u8] {
    let len = img.pixels.len() * 4;
    unsafe { std::slice::from_raw_parts_mut(img.pixels.as_mut_ptr() as *mut u8, len) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A published 'JOG1' region at `seq` whose pixels are all `fill`.
    fn shm(seq: u32, fill: u8) -> Vec<u8> {
        let mut b = vec![fill; SHM_PIXELS_OFF + FRAME_BYTES];
        b[..SHM_PIXELS_OFF].fill(0);
        b[SHM_OFF_MAGIC..SHM_OFF_MAGIC + 4].copy_from_slice(&SHM_MAGIC.to_le_bytes());
        b[SHM_OFF_SEQ..SHM_OFF_SEQ + 4].copy_from_slice(&seq.to_le_bytes());
        b[SHM_OFF_W..SHM_OFF_W + 2].copy_from_slice(&(JOG_FB_W as u16).to_le_bytes());
        b[SHM_OFF_H..SHM_OFF_H + 2].copy_from_slice(&(JOG_FB_H as u16).to_le_bytes());
        b[SHM_OFF_FMT..SHM_OFF_FMT + 4].copy_from_slice(&SHM_FMT_XRGB8888.to_le_bytes());
        b
    }

    #[test]
    fn a_stable_seq_copies_the_pixels() {
        let mut canvas = vec![0u8; FRAME_BYTES];
        assert_eq!(copy_stable(View::new(&shm(4, 0x7f)), &mut canvas), Some(4));
        assert!(canvas.iter().all(|&b| b == 0x7f));

        let mut canvas = vec![0u8; FRAME_BYTES];
        assert_eq!(copy_stable(View::new(&shm(5, 0x7f)), &mut canvas), None);
        assert!(
            canvas.iter().all(|&b| b == 0),
            "a write in progress is not read"
        );
    }

    #[test]
    fn only_an_xrgb_panel_sized_header_is_read() {
        assert!(header_is_xrgb_panel(View::new(&shm(2, 0))));
        let mut b = shm(2, 0);
        b[SHM_OFF_FMT] = 2;
        assert!(!header_is_xrgb_panel(View::new(&b)));
        let mut b = shm(2, 0);
        b[SHM_OFF_W] ^= 1;
        assert!(!header_is_xrgb_panel(View::new(&b)));
    }

    #[test]
    fn a_held_buffer_is_never_written() {
        let mut bufs = FrameBuffers::new();
        bufs.writable().pixels[0] = egui::Color32::WHITE;
        let first = bufs.publish();
        let second_ptr = Arc::as_ptr(&bufs.bufs[1]);

        // The UI still holds the first buffer, so the second is written.
        bufs.writable().pixels[0] = egui::Color32::RED;
        let second = bufs.publish();
        assert_eq!(Arc::as_ptr(&second), second_ptr);

        // With both held, a fresh buffer replaces one of them.
        let fresh: *const egui::ColorImage = bufs.writable();
        assert!(fresh != Arc::as_ptr(&first) && fresh != Arc::as_ptr(&second));
        assert_eq!(first.pixels[0], egui::Color32::WHITE);
        assert_eq!(second.pixels[0], egui::Color32::RED);
    }
}
