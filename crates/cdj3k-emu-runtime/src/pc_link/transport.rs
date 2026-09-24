//! Unix-socket transport for the `cdj3k.usb-link` virtio-serial channel.
//!
//! QEMU runs the chardev as `server=on,wait=off`, so we (the host) act as
//! the client: we connect to `{sock_dir}/usb-link.sock`, then the kernel's
//! virtio_console driver marks the guest port as `HOST_CONNECTED`, the
//! guest `cdj3k-pc-link-bridge` daemon starts pumping, and bytes flow.
//!
//! Threading model: two threads per connection: one **reader** (socket →
//! demux → `FrameSink`) and one **writer** (mpsc channel → frame → socket).
//! The caller owns the mpsc::channel pair so the backends can hold a
//! `Sender<OutFrame>` independently of the transport's own lifetime.  When
//! the connection drops, both threads exit; [`Transport::shutdown_into_rx`]
//! joins them and hands the `Receiver` back, so a caller keeping the backends
//! alive can re-dial and reuse the same channel.  Dropping the `Transport`
//! without reclaiming discards the `Receiver`.
//!
//! `stop`, `alive` and both join handles are per connection, constructed only
//! in [`Transport::spawn`].

use std::io::{self, BufWriter, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::pc_link::frame::{read_frame, write_frame, FRAME_HID, FRAME_MIDI};

/// One frame to send out the writer half: `(kind, payload)`.
pub type OutFrame = (u8, Vec<u8>);

/// How long the writer parks on the channel before re-checking `stop`.
/// Bounds how long `shutdown_into_rx` stalls waiting for the writer.
const RECV_TIMEOUT: Duration = Duration::from_millis(100);

/// Callback handed to the reader thread; one demuxed frame per call.  Must be
/// `Send + Sync` and not block.  The same sink outlives any single connection.
pub trait FrameSink: Send + Sync {
    fn on_hid(&self, payload: &[u8]);
    fn on_midi(&self, payload: &[u8]);
}

/// Live transport bound to a connected stream.  Drop to stop both threads.
pub struct Transport {
    stop: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
    /// Clone of the connection, used by `shutdown_into_rx` to `shutdown()`
    /// and unblock the reader/writer without waiting for the guest to close.
    shutdown_handle: UnixStream,
    /// Returns the outbound channel on exit, so a reconnect can reuse it.
    writer_thread: Option<thread::JoinHandle<Receiver<OutFrame>>>,
    reader_thread: Option<thread::JoinHandle<()>>,
}

impl Transport {
    /// Connect to `sock_path`, returning the stream without starting any I/O.
    ///
    /// Split from [`Transport::spawn`] so a caller can find out the guest side
    /// is absent *before* registering OS-visible endpoints: the host retries
    /// the connect at the worker poll cadence, and building the backends first
    /// would register and tear down a virtual HID device many times a second
    /// for the whole of guest boot.
    pub fn dial(sock_path: &Path) -> io::Result<UnixStream> {
        let stream = UnixStream::connect(sock_path)?;
        stream.set_nonblocking(false)?;
        Ok(stream)
    }

    /// Spawn the I/O threads on an already-dialed stream.
    ///
    /// - `rx`: writer thread drains this for host→guest frames.  The caller
    ///   keeps the matching `Sender` alive (typically cloned into each
    ///   backend), and can take `rx` back with
    ///   [`Transport::shutdown_into_rx`].
    /// - `sink`: reader thread dispatches guest→host frames here.  Shared, so
    ///   one sink serves every connection the caller dials.
    pub fn spawn(
        stream: UnixStream,
        rx: Receiver<OutFrame>,
        sink: Arc<dyn FrameSink>,
    ) -> io::Result<Self> {
        let reader_stream = stream.try_clone()?;
        let shutdown_handle = stream.try_clone()?;
        let writer_stream = stream;

        let stop = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));

        let writer_stop = stop.clone();
        let writer_alive = alive.clone();
        let writer_thread = thread::Builder::new()
            .name("pc-link-writer".into())
            .spawn(move || writer_loop(writer_stream, rx, writer_stop, writer_alive))?;

        let reader_stop = stop.clone();
        let reader_alive = alive.clone();
        let reader_thread = match thread::Builder::new()
            .name("pc-link-reader".into())
            .spawn(move || reader_loop(reader_stream, sink, reader_stop, reader_alive))
        {
            Ok(h) => h,
            Err(e) => {
                // The writer is already running and owns `rx`.  Stop and join
                // it; `rx` is lost with it, which the caller treats as fatal.
                stop.store(true, Ordering::Release);
                alive.store(false, Ordering::Release);
                let _ = shutdown_handle.shutdown(std::net::Shutdown::Both);
                let _ = writer_thread.join();
                return Err(e);
            }
        };

        Ok(Self {
            stop,
            alive,
            shutdown_handle,
            writer_thread: Some(writer_thread),
            reader_thread: Some(reader_thread),
        })
    }

    /// Whether both I/O threads are still running.  Transport liveness only:
    /// with `server=on,wait=off`, the guest can exit while QEMU keeps the port
    /// open, and neither thread observes it.
    pub fn is_transport_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    /// Stop both threads and hand back the outbound channel.  Reclaiming the
    /// `Receiver` lets a caller re-dial without disturbing the backends'
    /// `Sender` clones.
    ///
    /// `None` means the writer thread panicked and dropped the `Receiver`; the
    /// channel is gone and every `send` fails.  Callers treat that as fatal.
    ///
    /// Idempotent: handles are `Option`s, so a second call joins nothing.
    pub fn shutdown_into_rx(&mut self) -> Option<Receiver<OutFrame>> {
        self.stop.store(true, Ordering::Release);
        self.alive.store(false, Ordering::Release);
        // Must precede the reader join: the reader is parked in read_exact,
        // and only a shutdown of the socket unblocks it.
        let _ = self.shutdown_handle.shutdown(std::net::Shutdown::Both);
        let rx = self.writer_thread.take().and_then(|h| h.join().ok());
        if let Some(h) = self.reader_thread.take() {
            let _ = h.join();
        }
        rx
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        let _ = self.shutdown_into_rx();
    }
}

fn writer_loop(
    stream: UnixStream,
    rx: Receiver<OutFrame>,
    stop: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
) -> Receiver<OutFrame> {
    use std::sync::mpsc::RecvTimeoutError;
    // BufWriter so the 3-byte header and payload reach the kernel in one syscall.
    let mut w = BufWriter::new(stream);
    loop {
        // Timeout so the loop observes `stop`: a blocking recv() would hang
        // the join in `shutdown_into_rx`.
        if stop.load(Ordering::Acquire) {
            break;
        }
        match rx.recv_timeout(RECV_TIMEOUT) {
            Ok((kind, payload)) => {
                if let Err(e) = write_frame(&mut w, kind, &payload) {
                    eprintln!("pc-link: writer error ({e:?}) — exiting");
                    // Clear `alive` too: a write failure kills the host→guest
                    // half before the reader sees EOF.
                    alive.store(false, Ordering::Release);
                    stop.store(true, Ordering::Release);
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                // Unreachable while the backends hold a `Sender`; kept so a
                // future change to that ownership cannot busy-loop here.
                alive.store(false, Ordering::Release);
                break;
            }
        }
    }
    let _ = w.flush();
    rx
}

fn reader_loop(
    stream: UnixStream,
    sink: Arc<dyn FrameSink>,
    stop: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
) {
    let mut r = std::io::BufReader::new(stream);
    while !stop.load(Ordering::Acquire) {
        match read_frame(&mut r) {
            Ok(Some((FRAME_HID, payload))) => sink.on_hid(&payload),
            Ok(Some((FRAME_MIDI, payload))) => sink.on_midi(&payload),
            // Unknown frame type; not fatal.
            Ok(Some((other, _))) => eprintln!("pc-link: unknown frame type 0x{other:02x}"),
            Ok(None) => break, // clean EOF
            Err(e) => {
                eprintln!("pc-link: reader error ({e:?}) — exiting");
                break;
            }
        }
    }
    alive.store(false, Ordering::Release);
    // Wake the writer so it stops holding queued frames.  Parked in
    // recv_timeout, so the join costs <= RECV_TIMEOUT.
    stop.store(true, Ordering::Release);
}

/// The socket QEMU exposes for the cdj3k.usb-link virtserialport.
pub fn usb_link_sock_path(sock_dir: &Path) -> PathBuf {
    sock_dir.join("usb-link.sock")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, Sender};

    struct NullSink;

    impl FrameSink for NullSink {
        fn on_hid(&self, _payload: &[u8]) {}
        fn on_midi(&self, _payload: &[u8]) {}
    }

    fn spawn_pair() -> (Transport, UnixStream, Sender<OutFrame>) {
        let (host, peer) = UnixStream::pair().expect("socketpair");
        let (tx, rx) = channel::<OutFrame>();
        let transport =
            Transport::spawn(host, rx, Arc::new(NullSink)).expect("spawn transport");
        (transport, peer, tx)
    }

    #[test]
    fn reclaim_returns_the_same_channel() {
        let (mut transport, _peer, tx) = spawn_pair();
        assert!(transport.is_transport_alive());

        let rx = transport.shutdown_into_rx().expect("receiver reclaimed");
        assert!(!transport.is_transport_alive());

        // The backends' Sender clones stay valid across the reclaim.
        tx.send((FRAME_MIDI, vec![0x90, 0x40, 0x7f])).unwrap();
        assert_eq!(rx.try_recv().unwrap(), (FRAME_MIDI, vec![0x90, 0x40, 0x7f]));

        // `transport` drops here, running shutdown_into_rx a second time over
        // already-joined handles.
    }

    #[test]
    fn peer_close_clears_alive_without_an_explicit_shutdown() {
        let (transport, peer, _tx) = spawn_pair();
        assert!(transport.is_transport_alive());

        drop(peer);

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while transport.is_transport_alive() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        // This is the signal the worker polls for on a QEMU respawn.
        assert!(!transport.is_transport_alive(), "reader missed the peer close");
    }
}
