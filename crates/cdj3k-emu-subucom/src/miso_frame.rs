// The button constants below are read by name from the UI crate; cargo's
// dead-code lint can't see those cross-crate string-style references and
// would otherwise warn about every unused-this-build constant. Silenced
// at file scope so the warning isn't whack-a-mole.
#![allow(dead_code)]

use crate::crc::crc16_x25;
use crate::Direction;

pub const MISO_SIZE: usize = 64;

// Idle frame payload (bytes 0..62, before CRC).
pub const IDLE_PAYLOAD: [u8; 62] = {
    let mut f = [0u8; 62];
    // b02-b04: header constant
    f[2] = 0x01;
    f[3] = 0x04;
    f[4] = 0x03;
    // b12: device state (0x80=power_on | 0x01=sdcard_closed)
    f[12] = 0x81;
    // b14-b15: rotary encoder idle (0xfffe LE)
    f[14] = 0xfe;
    f[15] = 0xff;
    // b16-b19: LCD touch (0x0000 = no touch, LE)
    // all zero - already set by default
    // b20-b21: unknown
    f[21] = 0x7f;
    // b22-b23: tempo slider idle (0x0000 BE = 0%)
    // all zero - already set by default
    // b24: vinyl speed rotary idle
    // zero - already set by default
    // b26-b27: jog position idle (0xffff LE)
    f[26] = 0xff;
    f[27] = 0xff;
    // b28-b29: jog velocity (16-bit LE, INVERSE: 0xffff=stopped, 0x0000=max speed)
    f[28] = 0xff;
    f[29] = 0xff;
    // b30: jog touch state (0x00=none, 0x03=press, 0x04=baseline/idle, 0x0c=turning)
    f[30] = 0x04;
    // b32-b43: capacitive sensor
    f[32] = 0x51;
    f[33] = 0x01;
    f[34] = 0xd5;
    f[35] = 0xd6;
    f[36] = 0xd6;
    f[37] = 0xd5;
    f[38] = 0xd5;
    f[39] = 0xd6;
    f[40] = 0xd6;
    f[41] = 0xd5;
    f[42] = 0xd5;
    f[43] = 0xd6;
    f
};

// Button constants: (byte_offset, bitmask)
pub const BTN_PLAY: (usize, u8) = (5, 0x01);
pub const BTN_CUE: (usize, u8) = (5, 0x02);
pub const BTN_SEARCH_NEXT: (usize, u8) = (5, 0x04);
pub const BTN_SEARCH_PREV: (usize, u8) = (5, 0x08);
pub const BTN_TRACK_NEXT: (usize, u8) = (5, 0x10);
pub const BTN_TRACK_PREV: (usize, u8) = (5, 0x20);
pub const BTN_BEATJUMP_NEXT: (usize, u8) = (5, 0x40);
pub const BTN_BEATJUMP_PREV: (usize, u8) = (5, 0x80);

pub const BTN_TEMPO_RESET: (usize, u8) = (6, 0x01);
pub const BTN_MASTER_TEMPO: (usize, u8) = (6, 0x02);
pub const BTN_TEMPO_RANGE: (usize, u8) = (6, 0x04);
pub const BTN_AUTO_CUE: (usize, u8) = (6, 0x08);
pub const BTN_KEY_SYNC: (usize, u8) = (6, 0x10);
pub const BTN_BEAT_SYNC: (usize, u8) = (6, 0x20);
pub const BTN_MASTER: (usize, u8) = (6, 0x40);

pub const BTN_LOOP_IN: (usize, u8) = (7, 0x01);
pub const BTN_LOOP_OUT: (usize, u8) = (7, 0x02);
pub const BTN_RELOOP: (usize, u8) = (7, 0x04);
pub const BTN_BEATLOOP_HALF: (usize, u8) = (7, 0x10);
pub const BTN_BEATLOOP_2X: (usize, u8) = (7, 0x20);
pub const BTN_SLIP: (usize, u8) = (7, 0x80);

pub const BTN_MEMORY: (usize, u8) = (8, 0x01);
pub const BTN_DELETE: (usize, u8) = (8, 0x02);
pub const BTN_CALL_PREV: (usize, u8) = (8, 0x08);
pub const BTN_CALL_NEXT: (usize, u8) = (8, 0x04);
pub const BTN_CALL_DELETE: (usize, u8) = (8, 0x20);

pub const BTN_HOT_A: (usize, u8) = (9, 0x01);
pub const BTN_HOT_B: (usize, u8) = (9, 0x02);
pub const BTN_HOT_C: (usize, u8) = (9, 0x04);
pub const BTN_HOT_D: (usize, u8) = (9, 0x08);
pub const BTN_HOT_E: (usize, u8) = (9, 0x10);
pub const BTN_HOT_F: (usize, u8) = (9, 0x20);
pub const BTN_HOT_G: (usize, u8) = (9, 0x40);
pub const BTN_HOT_H: (usize, u8) = (9, 0x80);

pub const BTN_SOURCE: (usize, u8) = (10, 0x01);
pub const BTN_BROWSE: (usize, u8) = (10, 0x02);
pub const BTN_TAG_LIST: (usize, u8) = (10, 0x04);
pub const BTN_PLAYLIST: (usize, u8) = (10, 0x08);
pub const BTN_SEARCH_MENU: (usize, u8) = (10, 0x10);
pub const BTN_MENU: (usize, u8) = (10, 0x20);
pub const BTN_JOG_MODE: (usize, u8) = (10, 0x80);

pub const BTN_BACK: (usize, u8) = (11, 0x01);
pub const BTN_TAG_TRACK: (usize, u8) = (11, 0x02);
pub const BTN_TRACK_FILTER: (usize, u8) = (11, 0x04);
pub const BTN_SHORTCUT: (usize, u8) = (11, 0x08);
pub const BTN_ROTARY_PRESS: (usize, u8) = (11, 0x10);
pub const BTN_SLEEP: (usize, u8) = (11, 0x20);
pub const BTN_TIME_MODE: (usize, u8) = (11, 0x40);
pub const BTN_QUANTIZE: (usize, u8) = (11, 0x80);

pub const BTN_USB_STOP: (usize, u8) = (12, 0x02);
pub const BTN_POWER_ON: (usize, u8) = (12, 0x80);

pub struct MisoFrame([u8; MISO_SIZE]);

impl MisoFrame {
    pub fn idle() -> Self {
        let mut frame = [0u8; MISO_SIZE];
        frame[..62].copy_from_slice(&IDLE_PAYLOAD);
        Self(frame)
    }

    pub fn set_btn(&mut self, btn: (usize, u8), pressed: bool) {
        let (byte, mask) = btn;
        if pressed {
            self.0[byte] |= mask;
        } else {
            self.0[byte] &= !mask;
        }
    }

    /// Set touch coordinates (16-bit LE each). x=0, y=0 means no touch.
    /// b16-b17: LCD touch X (LE), b18-b19: LCD touch Y (LE)
    pub fn set_touch(&mut self, x: u16, y: u16) {
        self.0[16..18].copy_from_slice(&x.to_le_bytes());
        self.0[18..20].copy_from_slice(&y.to_le_bytes());
    }

    /// Rotary encoder counter (16-bit LE). b14-b15.
    pub fn set_rotary(&mut self, counter: u16) {
        self.0[14..16].copy_from_slice(&counter.to_le_bytes());
    }

    /// Jog wheel: pos (16-bit LE), vel (16-bit LE, INVERSE: 0xffff=stopped 0x0000=max),
    /// touch byte (0x00=none, 0x03=press, 0x04=idle/baseline, 0x0c=turning).
    /// b26-b27: pos LE, b28-b29: vel LE, b30: touch state.
    pub fn set_jog(&mut self, pos: u16, vel: u16, touch: u8) {
        self.0[26..28].copy_from_slice(&pos.to_le_bytes());
        self.0[28..30].copy_from_slice(&vel.to_le_bytes());
        self.0[30] = touch;
    }

    /// 3-position rocker switch (REV / SLIP_REV / FWD). b4.
    pub fn set_direction(&mut self, direction: Direction) {
        self.0[4] = direction.as_byte();
    }

    /// Tempo slider (16-bit LE, range 0x0000-0xffff, dead zone ~0x7F50-0x7FD0). b22-b23.
    pub fn set_tempo(&mut self, raw: u16) {
        self.0[22..24].copy_from_slice(&raw.to_le_bytes());
    }

    /// Vinyl speed rotary (0x00-0xff). b24.
    pub fn set_vinyl(&mut self, v: u8) {
        self.0[24] = v;
    }

    pub fn set_power(&mut self, on: bool) {
        let (byte, mask) = BTN_POWER_ON;
        if on {
            self.0[byte] |= mask;
        } else {
            self.0[byte] &= !mask;
        }
    }

    pub fn finalize(mut self) -> [u8; MISO_SIZE] {
        let crc = crc16_x25(&self.0[..62]);
        self.0[62..64].copy_from_slice(&crc.to_le_bytes());
        self.0
    }

    pub fn as_bytes(&self) -> &[u8; MISO_SIZE] {
        &self.0
    }
}
