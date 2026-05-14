//! MainLcdStream - reads the main LCD from the QEMU shm display backend.
//!
//! Shm file layout (written by qemu/patch/shm-display.c):
//!
//!   [0]   u32  magic       0x514D5348  ("QMS\x00")
//!   [4]   u32  generation  incremented with RELEASE after every dirty blit
//!   [8]   u32  width
//!   [12]  u32  height
//!   [16]  u32  stride      bytes per row
//!   [20]  u32  format      1 = RGBA8888  (QEMU converts from XRGB on its side)
//!   [24]  u32  dirty_x
//!   [28]  u32  dirty_y
//!   [32]  u32  dirty_w
//!   [36]  u32  dirty_h
//!   [64]  u8[] pixels      stride × height bytes
//!
//! The reader polls `generation` with Acquire semantics; when it changes,
//! dirty_x/y/w/h and pixel data are coherent.
//!
//! Pixel format: format=1 (RGBA8888, R,G,B,A byte order).
//! shm_gfx_update converts XRGB8888→RGBA8888 on the QEMU side so the host
//! can bulk-copy rows without any per-pixel channel swap.
//!
//! File path: `{socket_dir}/main.shm`  (created by QEMU at boot).

use memmap2::Mmap;
use std::fs::File;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub const LCD_W: usize = 1280;
pub const LCD_H: usize = 720;

const SHM_MAGIC: u32 = 0x514D_5348;
/// Byte offset where pixel data begins in the shm file (public for the GL upload path).
pub const SHM_PIXELS_OFFSET: usize = 64;

/// Poll interval for the shm generation counter. 500 µs comfortably tracks QEMU's
/// ~60 Hz dirty publishes without burning CPU.
const POLL_INTERVAL: Duration = Duration::from_micros(500);
/// Backoff between "shm not yet present" / "magic gone" retries.
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

/// A dirty-region notification - carries the mmap reference so the UI thread can
/// upload directly to the GPU without any intermediate pixel copy.
pub struct DisplayDirty {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Row stride in bytes (= shm header `stride` field).
    pub stride: u32,
    /// Shared reference to the shm mapping; pixels live at
    /// `mmap[SHM_PIXELS_OFFSET + y*stride + x*4 ..]`.
    pub mmap: Arc<Mmap>,
}

/// Background-thread shm reader for the main LCD.
pub struct MainLcdStream {
    slot: Arc<Mutex<Option<DisplayDirty>>>,
    connected: Arc<AtomicBool>,
    /// Monotonic count of generations observed since process start. Lets the UI
    /// gate its "still booting" overlay on actual frame production rather than
    /// just the shm being mapped.
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
            .expect("spawn main-lcd-shm thread");

        Self {
            slot,
            connected,
            frames_seen,
            shm_path,
        }
    }

    /// Total generations observed by the background reader since process start.
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
            match open_shm(shm_path) {
                Some(m) => {
                    eprintln!("[main_stream] opened {shm_path}");
                    wait_logged = false;
                    connected.store(true, Ordering::Relaxed);
                    gate.request();
                    break m;
                }
                None => {
                    if !wait_logged {
                        eprintln!("[main_stream] waiting for {shm_path}");
                        wait_logged = true;
                    }
                    thread::sleep(RECONNECT_DELAY);
                }
            }
        };

        poll_loop(&mmap, &slot, &frames_seen, &gate);

        // QEMU restarted (magic gone).
        eprintln!("[main_stream] disconnected, reconnecting");
        connected.store(false, Ordering::Relaxed);
        gate.request();
        thread::sleep(RECONNECT_DELAY);
    }
}

/// Inner loop: poll generation until magic disappears (QEMU gone/restarted).
/// A static display keeps the same generation indefinitely - that is normal,
/// not stale - so there is no timeout-based exit.
fn poll_loop(
    mmap: &Arc<Mmap>,
    slot: &Arc<Mutex<Option<DisplayDirty>>>,
    frames_seen: &Arc<AtomicU32>,
    gate: &crate::RepaintGate,
) {
    let mut last_gen: u32 = read_u32(mmap, 4);

    // Local dirty rect accumulator (x0, y0, x1, y1).
    // Accumulates the union of all dirty rects received since the last
    // successful publish.  Prevents cursor-ghost artifacts when the UI
    // thread is busy and cdj3k-emu misses intermediate dirty rect updates
    // (e.g. "erase old cursor" fires between two polls - without
    // accumulation the stale cursor pixels would never be uploaded).
    let mut acc: Option<(usize, usize, usize, usize)> = None;
    // Track surface dimensions to detect switches (640×480 → 1280×720).
    let mut last_w: usize = 0;
    let mut last_h: usize = 0;

    loop {
        thread::sleep(POLL_INTERVAL);

        // Magic check on every tick - disappears when QEMU exits or restarts.
        if read_u32(mmap, 0) != SHM_MAGIC {
            eprintln!("[main_stream] magic gone, reconnecting");
            return;
        }

        // Acquire load of generation - pairs with QEMU's RELEASE add.
        let gen = read_u32_acquire(mmap, 4);

        if gen == last_gen {
            continue;
        }
        puffin::profile_scope!("main_lcd_gen_bump");
        last_gen = gen;
        frames_seen.fetch_add(1, Ordering::Relaxed);

        // Re-read dimensions on every frame - the surface can switch
        // mid-session (e.g. initial 640×480 QEMU console → 1280×720 Xorg).
        let width = read_u32(mmap, 8) as usize;
        let height = read_u32(mmap, 12) as usize;
        let stride = read_u32(mmap, 16) as usize;

        if width == 0
            || height == 0
            || stride < width * 4
            || mmap.len() < SHM_PIXELS_OFFSET + stride * height
        {
            continue;
        }

        // Reset accumulator on surface dimension change.
        if width != last_w || height != last_h {
            acc = None;
            last_w = width;
            last_h = height;
        }

        // Read and validate dirty rect from header.
        let dx = read_u32(mmap, 24) as usize;
        let dy = read_u32(mmap, 28) as usize;
        let dw = read_u32(mmap, 32) as usize;
        let dh = read_u32(mmap, 36) as usize;

        if dw == 0 || dh == 0 || dx + dw > width || dy + dh > height {
            continue;
        }

        // Expand the local accumulator to cover this dirty rect.
        acc = Some(match acc {
            None => (dx, dy, dx + dw, dy + dh),
            Some((ax0, ay0, ax1, ay1)) => {
                (ax0.min(dx), ay0.min(dy), ax1.max(dx + dw), ay1.max(dy + dh))
            }
        });

        // Try to publish the accumulated region.  If the slot is still
        // occupied the accumulator keeps growing - next tick will cover
        // everything that was missed.
        if let Ok(mut g) = slot.try_lock() {
            if g.is_none() {
                if let Some((x0, y0, x1, y1)) = acc.take() {
                    let uw = x1 - x0;
                    let uh = y1 - y0;

                    // Zero-copy: share the mmap reference so the UI thread
                    // uploads directly from the shm file into the GPU texture.
                    *g = Some(DisplayDirty {
                        x: x0 as u32,
                        y: y0 as u32,
                        w: uw as u32,
                        h: uh as u32,
                        stride: stride as u32,
                        mmap: Arc::clone(mmap),
                    });
                    gate.request();
                }
            }
            // Slot busy: keep accumulating, don't take() so acc remains set.
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn open_shm(path: &str) -> Option<Arc<Mmap>> {
    let file = File::open(path).ok()?;
    let mmap = unsafe { Mmap::map(&file).ok()? };
    if mmap.len() < SHM_PIXELS_OFFSET {
        return None;
    }
    if read_u32(&mmap, 0) != SHM_MAGIC {
        return None;
    }
    Some(Arc::new(mmap))
}

/// Plain little-endian read - use for non-generation fields after the acquire.
fn read_u32(mmap: &Mmap, offset: usize) -> u32 {
    let bytes: [u8; 4] = mmap[offset..offset + 4].try_into().unwrap();
    u32::from_le_bytes(bytes)
}

/// Acquire load of a u32 - pairs with QEMU's __ATOMIC_RELEASE store.
fn read_u32_acquire(mmap: &Mmap, offset: usize) -> u32 {
    // SAFETY: `mmap.as_ptr()` is page-aligned (mmap-allocated regions
    // always are), and the header layout fixes the generation counter
    // at offset 4, which is 4-byte aligned and therefore satisfies
    // `AtomicU32`'s alignment.  The offset is well within bounds of the
    // mapped region (caller-enforced via the `Mmap` size check at
    // construction time).
    let ptr = unsafe { mmap.as_ptr().add(offset) as *const AtomicU32 };
    // SAFETY: `ptr` was just derived from a live `Mmap` borrowed for the
    // duration of this call, the dereferenced `AtomicU32` provides its
    // own synchronisation, and the QEMU side performs only atomic
    // accesses to the same word.
    unsafe { (*ptr).load(Ordering::Acquire) }
}
