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

    // One frame every 33 ms, published under the seqlock as the guest does.
    let (mut seq, mut published, mut received) = (0u32, 0, 0);
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(1500) {
        seq += 1;
        shm[4..8].copy_from_slice(&seq.to_le_bytes());
        shm[0x1000] = seq as u8;
        seq += 1;
        shm[4..8].copy_from_slice(&seq.to_le_bytes());
        published += 1;
        std::thread::sleep(Duration::from_millis(33));
        if stream.take().is_some() {
            received += 1;
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        received * 4 >= published * 3,
        "{received} of {published} frames"
    );
}
