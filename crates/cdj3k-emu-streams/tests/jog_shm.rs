//! The jog stream against a mapping another writer keeps publishing into.
//! Stale reads appear only in an optimised build, so run it with `--release`.

use std::time::{Duration, Instant};

use cdj3k_emu_streams::jog_stream::{JogLcdStream, JOG_FB_H, JOG_FB_W};
use cdj3k_emu_streams::RepaintGate;

#[test]
fn every_published_frame_reaches_the_ui() {
    let dir = std::env::temp_dir().join(format!("cdj3k-jog-shm-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(dir.join("jog.shm"))
        .unwrap();
    file.set_len(1 << 20).unwrap();
    let mut shm = unsafe { memmap2::MmapMut::map_mut(&file).unwrap() };
    shm[0..4].copy_from_slice(&0x3147_4F4Au32.to_le_bytes());
    shm[8..10].copy_from_slice(&(JOG_FB_W as u16).to_le_bytes());
    shm[10..12].copy_from_slice(&(JOG_FB_H as u16).to_le_bytes());
    shm[12..16].copy_from_slice(&1u32.to_le_bytes());

    // The stream maps the file only while the runtime reports a running QEMU.
    cdj3k_emu_platform::menu_state::lock().qemu_running = true;
    let gate = RepaintGate::new(egui::Context::default(), Duration::from_millis(16));
    let stream = JogLcdStream::new(dir.to_str().unwrap(), gate);

    // The test publishes each frame under the seqlock.
    // Waits up to 5 seconds for the UI to receive a frame before it publishes the next one
    const FRAMES: u32 = 20;
    let mut seq = 0u32;
    for frame in 1..=FRAMES {
        seq += 1;
        shm[4..8].copy_from_slice(&seq.to_le_bytes());
        shm[0x1000] = frame as u8;
        seq += 1;
        shm[4..8].copy_from_slice(&seq.to_le_bytes());

        let deadline = Instant::now() + Duration::from_secs(5);
        let delivered = loop {
            if let Some(f) = stream.take() {
                if f.image.pixels[0].to_array()[0] == frame as u8 {
                    break true;
                }
            }
            if Instant::now() > deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        if !delivered {
            let _ = std::fs::remove_dir_all(&dir);
            panic!("frame {frame} of {FRAMES} never reached the UI");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
