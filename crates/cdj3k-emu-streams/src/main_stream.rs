//! MainLcdStream reads the main LCD from the QEMU shm display backend.
//!
//! QEMU creates the file `{socket_dir}/main.shm` at boot, and `ui/shm-display.c`
//! (added by `qemu/patches/06-shm-display-source.patch`) writes it in this
//! layout:
//!
//! | offset | type | field |
//! |---|---|---|
//! | 0 | u32 | magic, 0x514D5348, stored little-endian as the bytes "HSMQ" |
//! | 4 | u32 | generation, a sequence counter that is odd while QEMU writes an update |
//! | 8 | u32 | width |
//! | 12 | u32 | height |
//! | 16 | u32 | stride, in bytes per row |
//! | 20 | u32 | format, 1 for RGBA8888 |
//! | 24 | u32 | dirty_x |
//! | 28 | u32 | dirty_y |
//! | 32 | u32 | dirty_w |
//! | 36 | u32 | dirty_h |
//! | 64 | u8[] | pixels, stride × height bytes |
//!
//! QEMU adds 1 to `generation` before it writes an update, which makes it odd,
//! and adds 1 with Release ordering when the update is written, which makes it
//! even again. The reader polls `generation` with Acquire ordering and skips
//! the tick while it is odd. When it is even and has changed, the pixels of
//! the dirty rect are in place. A step of 2 is exactly one update. The header
//! holds only the latest rect, so the reader uploads the whole surface when
//! the step is larger (it missed an update) or the generation moved while it
//! read the rect.
//!
//! Pixel format 1 is RGBA8888, in R, G, B, A byte order. `shm_gfx_update`
//! converts XRGB8888 to RGBA8888 on the QEMU side, so the host can copy whole
//! rows without swapping channels.

use memmap2::Mmap;

use crate::shm::View;
use std::fs::File;
use std::sync::atomic::{fence, AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const SHM_MAGIC: u32 = 0x514D_5348;
/// Byte offset of the pixel data in the shm file. The GL upload path uses it.
pub const SHM_PIXELS_OFFSET: usize = 64;

/// Poll interval for the shm generation counter. 500 µs keeps up with QEMU's
/// dirty publishes, about 60 a second, at little CPU cost.
const POLL_INTERVAL: Duration = Duration::from_micros(500);
/// The wait before retrying when the shm file is missing or its magic is gone.
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

/// A dirty-region notification. It carries the mmap reference, so the UI
/// thread can upload straight to the GPU without copying the pixels first.
pub struct DisplayDirty {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Row stride in bytes, from the shm header's `stride` field.
    pub stride: u32,
    /// Shared reference to the shm mapping. The pixels are at
    /// `mmap[SHM_PIXELS_OFFSET + y*stride + x*4 ..]`.
    pub mmap: Arc<Mmap>,
}

/// Reads the main LCD's shm on a background thread.
pub struct MainLcdStream {
    slot: Arc<Mutex<Option<DisplayDirty>>>,
    connected: Arc<AtomicBool>,
    /// The number of generations seen since the process started. The UI keeps
    /// its "still booting" overlay up until frames arrive, even if the shm is
    /// already mapped.
    frames_seen: Arc<AtomicU32>,
    shm_path: String,
}

impl MainLcdStream {
    pub fn new(socket_dir: &str, gate: crate::RepaintGate) -> Self {
        let shm_path = format!("{}/main.shm", socket_dir.trim_end_matches('/'));
        let slot: Arc<Mutex<Option<DisplayDirty>>> = Arc::new(Mutex::new(None));
        let slot_clone = Arc::clone(&slot);
        let connected = Arc::new(AtomicBool::new(false));
        let connected_clone = Arc::clone(&connected);
        let frames_seen = Arc::new(AtomicU32::new(0));
        let frames_seen_clone = Arc::clone(&frames_seen);
        let path = shm_path.clone();

        thread::Builder::new()
            .name("main-lcd-shm".into())
            .spawn(move || shm_loop(&path, slot_clone, connected_clone, frames_seen_clone, gate))
            .expect("the main-lcd-shm thread could not be started");

        Self {
            slot,
            connected,
            frames_seen,
            shm_path,
        }
    }

    /// The number of generations the background reader has seen since the
    /// process started.
    pub fn frames_seen(&self) -> u32 {
        self.frames_seen.load(Ordering::Relaxed)
    }

    /// Take the latest dirty notification, if any.
    pub fn take(&self) -> Option<DisplayDirty> {
        self.slot.lock().ok()?.take()
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub fn addr_str(&self) -> &str {
        &self.shm_path
    }
}

// ---------------------------------------------------------------------------
// Background thread
// ---------------------------------------------------------------------------

fn shm_loop(
    shm_path: &str,
    slot: Arc<Mutex<Option<DisplayDirty>>>,
    connected: Arc<AtomicBool>,
    frames_seen: Arc<AtomicU32>,
    gate: crate::RepaintGate,
) {
    let mut wait_logged = false;
    loop {
        // Wait for the shm file to appear and contain a valid header.
        let mmap = loop {
            if crate::guest::wait_until_running() {
                wait_logged = false;
            }
            match open_shm(shm_path) {
                Some(m) => {
                    eprintln!("[main_stream] Opened {shm_path}.");
                    wait_logged = false;
                    connected.store(true, Ordering::Relaxed);
                    gate.request();
                    break m;
                }
                None => {
                    if !wait_logged {
                        eprintln!("[main_stream] Waiting for {shm_path}.");
                        wait_logged = true;
                    }
                    thread::sleep(RECONNECT_DELAY);
                }
            }
        };

        poll_loop(&mmap, &slot, &frames_seen, &gate);

        // The magic is gone, so QEMU has exited or restarted.
        eprintln!("[main_stream] The main LCD mapping closed; reconnecting.");
        connected.store(false, Ordering::Relaxed);
        gate.request();
        thread::sleep(RECONNECT_DELAY);
    }
}

/// Polls the generation until the magic disappears, which happens when QEMU
/// exits or restarts. A static display keeps the same generation
/// indefinitely, so the loop has no timeout.
fn poll_loop(
    mmap: &Arc<Mmap>,
    slot: &Arc<Mutex<Option<DisplayDirty>>>,
    frames_seen: &Arc<AtomicU32>,
    gate: &crate::RepaintGate,
) {
    let shm = View::new(mmap);
    let mut last_gen: u32 = shm.u32_at(4, Ordering::Acquire);
    let mut dirty = DirtyUnion::default();
    let mut stride = 0;

    loop {
        thread::sleep(POLL_INTERVAL);

        // Check the magic on every tick; it disappears when QEMU exits or
        // restarts.
        if shm.u32_at(0, Ordering::Relaxed) != SHM_MAGIC {
            eprintln!("[main_stream] The shm magic was cleared; reconnecting.");
            return;
        }

        // Load the generation with Acquire ordering, which pairs with QEMU's
        // Release add. An odd value means that QEMU is writing an update; the
        // next tick reads it.
        let gen = shm.u32_at(4, Ordering::Acquire);
        if gen != last_gen && gen.is_multiple_of(2) {
            puffin::profile_scope!("main_lcd_gen_bump");
            frames_seen.fetch_add(1, Ordering::Relaxed);
            // Read the dimensions again on every frame, because the surface
            // can change during a session: from the initial 640×480 QEMU
            // console to Xorg at the model's main LCD mode.
            if let Some(surface) = Surface::read(shm) {
                dirty.resize(surface.width, surface.height);
                dirty.add(surface.changed_region(shm, last_gen, gen));
                stride = surface.stride;
            }
            last_gen = gen;
        }

        // Retry on every tick while the union is not empty, because the UI
        // may not have taken the previous notification yet.
        if publish(slot, &mut dirty, stride, mmap) {
            gate.request();
        }
    }
}

/// Move the union into `slot` if the UI has taken the previous notification.
/// Returns `true` when it published. The union is kept when the slot is busy.
fn publish(
    slot: &Mutex<Option<DisplayDirty>>,
    dirty: &mut DirtyUnion,
    stride: usize,
    mmap: &Arc<Mmap>,
) -> bool {
    if dirty.rect.is_none() {
        return false;
    }
    let Ok(mut g) = slot.try_lock() else {
        return false;
    };
    if g.is_some() {
        return false;
    }
    let Some((x0, y0, x1, y1)) = dirty.take() else {
        return false;
    };
    // Share the mmap reference, so the UI thread uploads straight from the
    // shm file into the GPU texture.
    *g = Some(DisplayDirty {
        x: x0 as u32,
        y: y0 as u32,
        w: (x1 - x0) as u32,
        h: (y1 - y0) as u32,
        stride: stride as u32,
        mmap: Arc::clone(mmap),
    });
    true
}

/// The surface the header describes, in pixels and bytes per row.
struct Surface {
    width: usize,
    height: usize,
    stride: usize,
}

impl Surface {
    /// `None` for an empty surface, or one whose rows the mapping does not
    /// hold.
    fn read(shm: View<'_>) -> Option<Self> {
        let width = shm.u32_at(8, Ordering::Relaxed) as usize;
        let height = shm.u32_at(12, Ordering::Relaxed) as usize;
        let stride = shm.u32_at(16, Ordering::Relaxed) as usize;
        if width == 0
            || height == 0
            || stride < width * 4
            || shm.len() < SHM_PIXELS_OFFSET + stride * height
        {
            return None;
        }
        Some(Self {
            width,
            height,
            stride,
        })
    }

    /// The header's dirty rect as `(x, y, w, h)`; `None` when it is empty or
    /// leaves the surface.
    fn dirty_rect(&self, shm: View<'_>) -> Option<(usize, usize, usize, usize)> {
        let dx = shm.u32_at(24, Ordering::Relaxed) as usize;
        let dy = shm.u32_at(28, Ordering::Relaxed) as usize;
        let dw = shm.u32_at(32, Ordering::Relaxed) as usize;
        let dh = shm.u32_at(36, Ordering::Relaxed) as usize;
        if dw == 0 || dh == 0 || dx + dw > self.width || dy + dh > self.height {
            return None;
        }
        Some((dx, dy, dw, dh))
    }

    /// The region that changed between `last_gen` and `gen`, as
    /// `(x, y, w, h)`.
    ///
    /// The header holds only the rect of the latest update. The header rect is
    /// used only when exactly one update landed (a step of 2) and the
    /// generation did not move while the rect was read; otherwise the whole
    /// surface is returned.
    fn changed_region(
        &self,
        shm: View<'_>,
        last_gen: u32,
        gen: u32,
    ) -> (usize, usize, usize, usize) {
        let rect = self.dirty_rect(shm);
        // Keep the second generation load after the rect loads.
        fence(Ordering::Acquire);
        let one_update = gen.wrapping_sub(last_gen) == 2 && shm.u32_at(4, Ordering::Relaxed) == gen;
        match rect {
            Some(r) if one_update => r,
            _ => (0, 0, self.width, self.height),
        }
    }
}

/// The union of the dirty rects received since the last successful publish,
/// as `(x0, y0, x1, y1)`.
///
/// The union prevents cursor ghosts when the UI thread is busy and misses
/// intermediate dirty rects. For example, if an "erase old cursor" update
/// lands between two polls, the stale cursor pixels would otherwise never be
/// uploaded.
#[derive(Default)]
struct DirtyUnion {
    rect: Option<(usize, usize, usize, usize)>,
    /// Surface dimensions, used to detect a switch such as the one from the
    /// 640×480 console to the X server's mode.
    size: (usize, usize),
}

impl DirtyUnion {
    /// Drop the union when the surface changes size.
    fn resize(&mut self, width: usize, height: usize) {
        if (width, height) != self.size {
            self.rect = None;
            self.size = (width, height);
        }
    }

    /// Grow the union to cover `(x, y, w, h)`.
    fn add(&mut self, (dx, dy, dw, dh): (usize, usize, usize, usize)) {
        self.rect = Some(match self.rect {
            None => (dx, dy, dx + dw, dy + dh),
            Some((ax0, ay0, ax1, ay1)) => {
                (ax0.min(dx), ay0.min(dy), ax1.max(dx + dw), ay1.max(dy + dh))
            }
        });
    }

    fn take(&mut self) -> Option<(usize, usize, usize, usize)> {
        self.rect.take()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// One copy of the main LCD as opaque RGBA rows: `(width, height, bytes)`,
/// read from `{socket_dir}/main.shm`. A copy is taken while the generation is
/// even, and taken again when an update overlapped it. An odd generation that
/// lasts past [`CAPTURE_ODD_WAIT`] is accepted, for a QEMU that counts each
/// update once.
pub fn capture(socket_dir: &str) -> Result<(u32, u32, Vec<u8>), String> {
    let path = format!("{}/main.shm", socket_dir.trim_end_matches('/'));
    let mmap = open_shm(&path).ok_or("the main LCD is not up")?;
    let shm = View::new(&mmap);
    let mut out = Vec::new();
    let odd_until = std::time::Instant::now() + CAPTURE_ODD_WAIT;
    for _ in 0..CAPTURE_ATTEMPTS {
        let gen = shm.u32_at(4, Ordering::Acquire);
        if !gen.is_multiple_of(2) && std::time::Instant::now() < odd_until {
            thread::sleep(Duration::from_millis(1));
            continue;
        }
        let surface = Surface::read(shm).ok_or("the main LCD has no surface yet")?;
        let row = surface.width * 4;
        out.resize(row * surface.height, 0);
        for (y, dst) in out.chunks_exact_mut(row).enumerate() {
            shm.copy_to(SHM_PIXELS_OFFSET + y * surface.stride, dst);
        }
        let size = (surface.width as u32, surface.height as u32);
        if shm.u32_at(4, Ordering::Relaxed) == gen {
            for px in out.as_chunks_mut::<4>().0 {
                px[3] = 0xff;
            }
            return Ok((size.0, size.1, out));
        }
    }
    Err("the main LCD changed during every copy".into())
}

/// How many times [`capture`] tries for a copy no update overlapped.
const CAPTURE_ATTEMPTS: usize = 64;
/// How long [`capture`] waits for QEMU to finish an update in progress.
const CAPTURE_ODD_WAIT: Duration = Duration::from_millis(40);

fn open_shm(path: &str) -> Option<Arc<Mmap>> {
    let file = File::open(path).ok()?;
    let mmap = unsafe { Mmap::map(&file).ok()? };
    if mmap.len() < SHM_PIXELS_OFFSET {
        return None;
    }
    if View::new(&mmap).u32_at(0, Ordering::Relaxed) != SHM_MAGIC {
        return None;
    }
    Some(Arc::new(mmap))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A header for a `width`×`height` surface with `stride`-byte rows and
    /// the dirty rect `(x, y, w, h)`, followed by its pixels.
    fn shm(width: u32, height: u32, stride: u32, dirty: [u32; 4]) -> Vec<u8> {
        let mut b = vec![0u8; SHM_PIXELS_OFFSET + (stride * height) as usize];
        for (off, v) in [(0, SHM_MAGIC), (8, width), (12, height), (16, stride)]
            .into_iter()
            .chain([24, 28, 32, 36].into_iter().zip(dirty))
        {
            b[off..off + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    #[test]
    fn a_surface_needs_rows_the_mapping_holds() {
        let s = Surface::read(View::new(&shm(4, 2, 16, [0; 4]))).unwrap();
        assert_eq!((s.width, s.height, s.stride), (4, 2, 16));
        assert!(Surface::read(View::new(&shm(0, 2, 16, [0; 4]))).is_none());
        assert!(Surface::read(View::new(&shm(4, 2, 15, [0; 4]))).is_none());
        let mut short = shm(4, 2, 16, [0; 4]);
        short.pop();
        assert!(Surface::read(View::new(&short)).is_none());
    }

    #[test]
    fn a_dirty_rect_must_be_inside_the_surface() {
        let rect = |d: [u32; 4]| {
            let b = shm(4, 2, 16, d);
            Surface::read(View::new(&b))
                .unwrap()
                .dirty_rect(View::new(&b))
        };
        assert_eq!(rect([1, 0, 3, 2]), Some((1, 0, 3, 2)));
        assert_eq!(rect([1, 0, 0, 2]), None);
        assert_eq!(rect([2, 0, 3, 2]), None);
        assert_eq!(rect([0, 1, 1, 2]), None);
    }

    /// `shm` with the generation counter set to `gen`.
    fn shm_at(gen: u32, dirty: [u32; 4]) -> Vec<u8> {
        let mut b = shm(4, 2, 16, dirty);
        b[4..8].copy_from_slice(&gen.to_le_bytes());
        b
    }

    #[test]
    fn one_update_uses_the_header_rect() {
        let b = shm_at(8, [1, 0, 2, 1]);
        let s = Surface::read(View::new(&b)).unwrap();
        assert_eq!(s.changed_region(View::new(&b), 6, 8), (1, 0, 2, 1));
        let b = shm_at(0, [1, 0, 2, 1]);
        assert_eq!(
            s.changed_region(View::new(&b), u32::MAX - 1, 0),
            (1, 0, 2, 1),
            "the generation wraps"
        );
    }

    #[test]
    fn a_missed_update_or_a_moving_generation_uploads_the_whole_surface() {
        let b = shm_at(10, [1, 0, 2, 1]);
        let s = Surface::read(View::new(&b)).unwrap();
        assert_eq!(s.changed_region(View::new(&b), 6, 10), (0, 0, 4, 2));
        assert_eq!(
            s.changed_region(View::new(&b), 6, 8),
            (0, 0, 4, 2),
            "the generation moved from 8 to 10 during the read"
        );
        let b = shm_at(9, [1, 0, 2, 1]);
        assert_eq!(
            s.changed_region(View::new(&b), 6, 8),
            (0, 0, 4, 2),
            "QEMU started the next update during the read"
        );
        let b = shm_at(8, [3, 0, 2, 1]);
        assert_eq!(
            s.changed_region(View::new(&b), 6, 8),
            (0, 0, 4, 2),
            "a rect outside the surface"
        );
    }

    #[test]
    fn a_busy_slot_keeps_the_union_for_the_next_tick() {
        let mmap = Arc::new(
            memmap2::MmapMut::map_anon(64)
                .unwrap()
                .make_read_only()
                .unwrap(),
        );
        let slot = Mutex::new(None);
        let mut u = DirtyUnion::default();
        assert!(!publish(&slot, &mut u, 16, &mmap), "nothing to publish");

        u.add((0, 0, 1, 1));
        assert!(publish(&slot, &mut u, 16, &mmap));
        u.add((2, 1, 1, 1));
        assert!(
            !publish(&slot, &mut u, 16, &mmap),
            "the UI has not taken it"
        );
        assert_eq!(u.rect, Some((2, 1, 3, 2)));

        let first = slot.lock().unwrap().take().unwrap();
        assert_eq!((first.x, first.y, first.w, first.h), (0, 0, 1, 1));
        assert!(publish(&slot, &mut u, 16, &mmap));
        let second = slot.lock().unwrap().take().unwrap();
        assert_eq!(
            (second.x, second.y, second.w, second.h, second.stride),
            (2, 1, 1, 1, 16)
        );
    }

    #[test]
    fn the_union_grows_until_taken_and_resets_on_resize() {
        let mut u = DirtyUnion::default();
        u.resize(4, 2);
        u.add((1, 1, 1, 1));
        u.add((0, 0, 1, 1));
        assert_eq!(u.take(), Some((0, 0, 2, 2)));
        assert_eq!(u.take(), None);

        u.add((1, 1, 1, 1));
        u.resize(4, 2);
        assert_eq!(u.take(), Some((1, 1, 2, 2)), "same size keeps the union");
        u.add((1, 1, 1, 1));
        u.resize(8, 4);
        assert_eq!(u.take(), None, "a new size drops it");
    }
}
