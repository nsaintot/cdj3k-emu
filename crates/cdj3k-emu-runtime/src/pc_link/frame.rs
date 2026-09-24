//! Wire format for the `cdj3k.usb-link` virtio-serial channel.
//!
//! Matches `guest/pc_link_bridge/` (io.c) byte-for-byte:
//!
//! ```text
//!   byte 0       : type     (FRAME_HID = 0x01, FRAME_MIDI = 0x02,
//!                             FRAME_HELLO = 0x03, FRAME_IDENTITY = 0x04)
//!   bytes 1..2   : length   (big-endian u16, payload only)
//!   bytes 3..    : payload
//! ```
//!
//! Both directions share the same encoding; "direction" is implicit in who
//! wrote the frame.  HELLO (host -> guest, empty) asks for the gadget
//! identity; IDENTITY (guest -> host) answers it, see [`super::gadget`].
//! Max payload is 4 KiB on the guest side (HID reports are as long as the
//! gadget's descriptor says, USB-MIDI packets are 4 B), but the parser
//! doesn't enforce a cap; caller decides whether to drop oversized frames.

use std::io::{self, Read, Write};

pub const FRAME_HID: u8 = 0x01;
pub const FRAME_MIDI: u8 = 0x02;
pub const FRAME_HELLO: u8 = 0x03;
pub const FRAME_IDENTITY: u8 = 0x04;

/// Read one frame.  Returns `Ok(None)` on clean EOF, `Ok(Some((kind, payload)))`
/// otherwise.  Any other error short-circuits the read loop.
pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut hdr = [0u8; 3];
    match r.read_exact(&mut hdr) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let kind = hdr[0];
    let len = u16::from_be_bytes([hdr[1], hdr[2]]) as usize;
    let mut payload = vec![0u8; len];
    if len > 0 {
        r.read_exact(&mut payload)?;
    }
    Ok(Some((kind, payload)))
}

/// Frame and write a payload.  Returns when all bytes are flushed (or errors).
pub fn write_frame<W: Write>(w: &mut W, kind: u8, payload: &[u8]) -> io::Result<()> {
    if payload.len() > u16::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pc-link frame payload exceeds u16::MAX",
        ));
    }
    let len = payload.len() as u16;
    let hdr = [kind, (len >> 8) as u8, (len & 0xff) as u8];
    w.write_all(&hdr)?;
    if !payload.is_empty() {
        w.write_all(payload)?;
    }
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_hid() {
        let payload = [0u8; 64];
        let mut buf = Vec::new();
        write_frame(&mut buf, FRAME_HID, &payload).unwrap();
        assert_eq!(buf.len(), 3 + 64);
        assert_eq!(buf[0], FRAME_HID);
        assert_eq!(u16::from_be_bytes([buf[1], buf[2]]), 64);

        let mut cur = std::io::Cursor::new(buf);
        let (kind, got) = read_frame(&mut cur).unwrap().unwrap();
        assert_eq!(kind, FRAME_HID);
        assert_eq!(got, payload);
    }

    #[test]
    fn empty_payload_is_legal() {
        let mut buf = Vec::new();
        write_frame(&mut buf, FRAME_MIDI, &[]).unwrap();
        assert_eq!(buf, [FRAME_MIDI, 0, 0]);
        let mut cur = std::io::Cursor::new(buf);
        let (kind, got) = read_frame(&mut cur).unwrap().unwrap();
        assert_eq!(kind, FRAME_MIDI);
        assert!(got.is_empty());
    }

    #[test]
    fn clean_eof_returns_none() {
        let buf: Vec<u8> = Vec::new();
        let mut cur = std::io::Cursor::new(buf);
        assert!(read_frame(&mut cur).unwrap().is_none());
    }
}
