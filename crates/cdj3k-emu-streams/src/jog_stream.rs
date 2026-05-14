//! JogLcdStream - receives jog LCD frames from the guest via ivshmem.
//!
//! Transport (zero-copy, polling):
//!   * The guest's `ep122_shim.so` extracts the visible 320×240 region from
//!     EP122's 1280×240 stretched DRM framebuffer on every flip and writes the
//!     XRGB8888 pixels directly into an ivshmem BAR. The same memory is
//!     visible on the host as the file at `{socket_dir}/jog.shm` (mapped here
//!     read-only).
//!   * Each publish bumps a 32-bit `seq` counter (seqlock-style: odd = write
//!     in progress, even = stable). This thread polls `seq` at ~60 Hz; when
//!     it advances we read pixels via the seqlock and publish a `JogFrame`.
//!
//! Layout - must match `JOG_SHM_*` in `guest/shim/shim.h`:
//!   0x0000 u32 magic = 'JOG1' (LE 0x31474F4A)
//!   0x0004 u32 seq      seqlock counter (odd = write in progress)
//!   0x0008 u16 width
//!   0x000A u16 height
//!   0x000C u32 format   1 = XRGB8888
//!   0x1000 pixels       width*height*4 bytes

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use memmap2::Mmap;

/// Logical jog LCD width after guest-side crop (physical panel).
pub const JOG_FB_W: usize = 320;
/// Logical jog LCD height (matches DRM jog plane height).
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

/// Poll period for `seq`. 16 ms ≈ 60 Hz, which matches the upstream jog
/// refresh and is well below the guest's ~100 Hz publish rate.
const POLL_INTERVAL: Duration = Duration::from_millis(16);

/// If `seq` doesn't advance for this long we mark the stream disconnected so
/// the UI can show a "wait" badge. The guest publishes on every DRM flip - a
/// 2 s gap means EP122 has stalled or the guest hasn't booted yet.
const ALIVE_TIMEOUT: Duration = Duration::from_secs(2);

/// Horizontal inset from left/right edges as a fraction of width (4:3 panel - slightly larger than Y).
const CORNER_SAMPLE_FRAC_X: f32 = 0.2;
/// Vertical inset from top/bottom edges as a fraction of height.
const CORNER_SAMPLE_FRAC_Y: f32 = 0.1;

/// Framebuffer `(x, y)` pixel indices used for [`JogFrame::corner_rgba`], in the same order:
/// top-left, top-right, bottom-left, bottom-right (SLIP, VINYL, SYNC, MASTER).
pub fn jog_corner_probe_fb_pixels() -> [(usize, usize); 4] {
    let dx = ((JOG_FB_W as f32) * CORNER_SAMPLE_FRAC_X).round() as usize;
    let dy = ((JOG_FB_H as f32) * CORNER_SAMPLE_FRAC_Y).round() as usize;
    let dx = dx.min(JOG_FB_W.saturating_sub(1));
    let dy = dy.min(JOG_FB_H.saturating_sub(1));
    let xm = JOG_FB_W.saturating_sub(1 + dx);
    let ym = JOG_FB_H.saturating_sub(1 + dy);
    [(dx, dy), (xm, dy), (dx, ym), (xm, ym)]
}

/// Dirty RGBA8 region for the jog LCD (same semantics as [`crate::display::DisplayFrame`]).
pub struct JogFrame {
    pub image: std::sync::Arc<egui::ColorImage>,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Non-premultiplied RGBA from full canvas: SLIP, VINYL, SYNC, MASTER (top-left, top-right,
    /// bottom-left, bottom-right).
    pub corner_rgba: [[u8; 4]; 4],
}

/// Background-thread reader for the jog LCD ivshmem region.
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
            .expect("spawn jog-lcd-stream thread");

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

/// Map the ivshmem-backed file. cdj3k-emu-runtime truncates it to 1 MiB before
/// QEMU launch, so the file always exists once the instance has spawned;
/// loop in case the UI runs ahead of the emulator.
fn open_shm(shm_path: &str) -> Mmap {
    let mut log_attempts: u32 = 0;
    loop {
        match std::fs::OpenOptions::new().read(true).open(shm_path) {
            Ok(f) => match unsafe { Mmap::map(&f) } {
                Ok(m) if m.len() >= SHM_PIXELS_OFF + FRAME_BYTES => return m,
                Ok(m) => {
                    eprintln!(
                        "[jog_stream] {shm_path}: too small ({} bytes), waiting…",
                        m.len()
                    );
                }
                Err(e) => eprintln!("[jog_stream] mmap {shm_path}: {e}"),
            },
            Err(e) => {
                log_attempts += 1;
                if log_attempts == 1 || log_attempts % 10 == 0 {
                    eprintln!("[jog_stream] open {shm_path}: {e} (attempt {log_attempts})");
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
    // Two `Arc<ColorImage>` buffers - rotate so the receiver can read while we
    // write the next frame without reallocating the 76 800-element pixel Vec.
    let mut bufs: [Arc<egui::ColorImage>; 2] = [
        Arc::new(egui::ColorImage::new(
            [JOG_FB_W, JOG_FB_H],
            egui::Color32::BLACK,
        )),
        Arc::new(egui::ColorImage::new(
            [JOG_FB_W, JOG_FB_H],
            egui::Color32::BLACK,
        )),
    ];
    let mut next_idx: usize = 0;

    // Outer loop: re-mmap whenever the QEMU worker exits/restarts. The
    // runtime unlinks jog.shm on cleanup and re-creates it on spawn (new
    // inode), so a held mapping would freeze on the previous session's last
    // frame. The cleanup hook zeros the magic before unlink, giving us a
    // reliable in-band signal to drop and re-open.
    loop {
        let shm = open_shm(shm_path);
        eprintln!("[jog_stream] shm mapped: {shm_path} ({} bytes)", shm.len());

        let mut last_seq: u32 = 0;
        let mut last_change_at: Option<Instant> = None;
        let mut frame_count: u64 = 0;

        // Inner poll loop - exits on magic loss to trigger a re-mmap.
        loop {
            let magic = read_u32(&shm, SHM_OFF_MAGIC);
            if magic != SHM_MAGIC {
                eprintln!("[jog_stream] magic gone, reconnecting");
                if connected.swap(false, Ordering::Relaxed) {
                    gate.request();
                }
                break;
            }

            let seq = read_u32(&shm, SHM_OFF_SEQ);
            if seq == last_seq || (seq & 1) != 0 {
                if let Some(t) = last_change_at {
                    if t.elapsed() > ALIVE_TIMEOUT && connected.swap(false, Ordering::Relaxed) {
                        gate.request();
                    }
                }
                thread::sleep(POLL_INTERVAL);
                continue;
            }

            let fmt = read_u32(&shm, SHM_OFF_FMT);
            if fmt != SHM_FMT_XRGB8888 {
                eprintln!("[jog_stream] unexpected format {fmt}");
                thread::sleep(POLL_INTERVAL);
                continue;
            }
            let w = read_u16(&shm, SHM_OFF_W) as usize;
            let h = read_u16(&shm, SHM_OFF_H) as usize;
            if w != JOG_FB_W || h != JOG_FB_H {
                eprintln!("[jog_stream] unexpected size {w}x{h}");
                thread::sleep(POLL_INTERVAL);
                continue;
            }

            let img: &mut egui::ColorImage = loop {
                if Arc::get_mut(&mut bufs[next_idx]).is_some() {
                    break Arc::get_mut(&mut bufs[next_idx]).unwrap();
                }
                next_idx ^= 1;
                if Arc::get_mut(&mut bufs[next_idx]).is_some() {
                    break Arc::get_mut(&mut bufs[next_idx]).unwrap();
                }
                bufs[next_idx] = Arc::new(egui::ColorImage::new(
                    [JOG_FB_W, JOG_FB_H],
                    egui::Color32::BLACK,
                ));
                break Arc::get_mut(&mut bufs[next_idx]).unwrap();
            };
            let canvas = color_image_bytes_mut(img);

            let mut got_seq: Option<u32> = None;
            for _ in 0..4 {
                let seq_before = read_u32(&shm, SHM_OFF_SEQ);
                if seq_before & 1 != 0 {
                    std::hint::spin_loop();
                    continue;
                }
                canvas.copy_from_slice(&shm[SHM_PIXELS_OFF..SHM_PIXELS_OFF + FRAME_BYTES]);
                let seq_after = read_u32(&shm, SHM_OFF_SEQ);
                if seq_after == seq_before {
                    got_seq = Some(seq_after);
                    break;
                }
            }
            let Some(seq_now) = got_seq else {
                thread::sleep(POLL_INTERVAL);
                continue;
            };
            if seq_now == last_seq {
                thread::sleep(POLL_INTERVAL);
                continue;
            }
            last_seq = seq_now;
            last_change_at = Some(Instant::now());

            let corners = corner_colors_from_canvas(canvas);
            if frame_count == 0 {
                let nonzero = canvas.iter().any(|&b| b != 0);
                eprintln!("[jog_stream] first frame: nonzero={nonzero} seq={seq_now}");
            }
            if !connected.load(Ordering::Relaxed) {
                connected.store(true, Ordering::Relaxed);
            }
            frame_count += 1;

            let publish = Arc::clone(&bufs[next_idx]);
            next_idx ^= 1;
            if let Ok(mut g) = slot.try_lock() {
                *g = Some(JogFrame {
                    image: publish,
                    x: 0,
                    y: 0,
                    w: JOG_FB_W as u32,
                    h: JOG_FB_H as u32,
                    corner_rgba: corners,
                });
                gate.request();
            }

            thread::sleep(POLL_INTERVAL);
        }

        // Drop the old Mmap explicitly before reopening so the kernel can
        // reclaim the stale inode's pages.
        drop(shm);
        thread::sleep(Duration::from_millis(500));
    }
}

#[inline]
fn read_u32(shm: &Mmap, off: usize) -> u32 {
    u32::from_le_bytes([shm[off], shm[off + 1], shm[off + 2], shm[off + 3]])
}

#[inline]
fn read_u16(shm: &Mmap, off: usize) -> u16 {
    u16::from_le_bytes([shm[off], shm[off + 1]])
}

/// Sample the canvas at (x, y) and return RGBA bytes for UI consumption.
/// The canvas stores raw wire-format XRGB bytes (B, G, R, X) - the GPU
/// re-orders them via channel-swizzle, but `corner_rgba` is consumed by
/// `Color32::from_rgba_unmultiplied` on the host side and so needs the
/// CPU-visible RGBA layout. Swap on read.
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

/// Reinterpret a `ColorImage`'s pixel storage as a mutable byte slice. Sound
/// because [`egui::Color32`] is `#[repr(C)] [u8; 4]`.
fn color_image_bytes_mut(img: &mut egui::ColorImage) -> &mut [u8] {
    let len = img.pixels.len() * 4;
    unsafe { std::slice::from_raw_parts_mut(img.pixels.as_mut_ptr() as *mut u8, len) }
}
