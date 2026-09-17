//! Every button a player can have, declared once.
//!
//! [`Btn`] is a name, not an address: its [`bit`](Btn::bit) is in the
//! CDJ-3000's frame coordinates and only [`button`](crate::miso_frame::button)
//! turns it into an offset, by shifting it into the player's own frame. A
//! slate therefore names the control it draws and gets the right bit for
//! whichever player is on screen - naming another player's button is not
//! something the type allows.
//!
//! The macro below declares the variant, its script name and its bit together
//! so the three can't drift. `shared` are the CDJ-3000's, which every player
//! inherits; `extra` are later additions, which a player claims in
//! [`MisoMap::extra`](crate::frame::MisoMap::extra).

use crate::frame::FrameBit;

macro_rules! buttons {
    (
        shared { $($(#[$sm:meta])* $sv:ident = $sn:literal => $sb:expr),+ $(,)? }
        extra  { $($(#[$em:meta])* $ev:ident = $en:literal => $eb:expr),+ $(,)? }
    ) => {
        /// A button of the front panel.
        #[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
        pub enum Btn {
            $($(#[$sm])* $sv,)+
            $($(#[$em])* $ev,)+
        }

        impl Btn {
            /// The buttons the CDJ-3000 has, which every player inherits.
            pub const SHARED: &'static [Btn] = &[$(Btn::$sv),+];
            /// The buttons added after the CDJ-3000; a player has one only if
            /// its map claims it.
            pub const EXTRA: &'static [Btn] = &[$(Btn::$ev),+];

            /// Its place in the CDJ-3000's frame - the shared coordinates
            /// every player's map shifts from, not an address in any frame.
            pub const fn bit(self) -> FrameBit {
                match self {
                    $(Btn::$sv => $sb,)+
                    $(Btn::$ev => $eb,)+
                }
            }

            /// The name scripts and the CLI use.
            pub const fn name(self) -> &'static str {
                match self {
                    $(Btn::$sv => $sn,)+
                    $(Btn::$ev => $en,)+
                }
            }

            /// True for a button every player has.
            pub const fn is_shared(self) -> bool {
                matches!(self, $(Btn::$sv)|+)
            }

            /// Look up a button by [`name`](Self::name), case-insensitive.
            pub fn from_name(name: &str) -> Option<Btn> {
                let upper = name.to_ascii_uppercase();
                Btn::SHARED
                    .iter()
                    .chain(Btn::EXTRA)
                    .copied()
                    .find(|b| b.name() == upper)
            }
        }
    };
}

buttons! {
    shared {
        Play = "PLAY" => (5, 0x01),
        Cue = "CUE" => (5, 0x02),
        SearchNext = "SEARCH_NEXT" => (5, 0x04),
        SearchPrev = "SEARCH_PREV" => (5, 0x08),
        TrackNext = "TRACK_NEXT" => (5, 0x10),
        TrackPrev = "TRACK_PREV" => (5, 0x20),
        BeatjumpNext = "BEATJUMP_NEXT" => (5, 0x40),
        BeatjumpPrev = "BEATJUMP_PREV" => (5, 0x80),

        TempoReset = "TEMPO_RESET" => (6, 0x01),
        MasterTempo = "MASTER_TEMPO" => (6, 0x02),
        TempoRange = "TEMPO_RANGE" => (6, 0x04),
        AutoCue = "AUTO_CUE" => (6, 0x08),
        KeySync = "KEY_SYNC" => (6, 0x10),
        BeatSync = "BEAT_SYNC" => (6, 0x20),
        Master = "MASTER" => (6, 0x40),

        LoopIn = "LOOP_IN" => (7, 0x01),
        LoopOut = "LOOP_OUT" => (7, 0x02),
        Reloop = "RELOOP" => (7, 0x04),
        BeatloopHalf = "BEATLOOP_HALF" => (7, 0x10),
        Beatloop2x = "BEATLOOP_2X" => (7, 0x20),
        Slip = "SLIP" => (7, 0x80),

        Memory = "MEMORY" => (8, 0x01),
        Delete = "DELETE" => (8, 0x02),
        CallNext = "CALL_NEXT" => (8, 0x04),
        CallPrev = "CALL_PREV" => (8, 0x08),
        CallDelete = "CALL_DELETE" => (8, 0x20),

        HotA = "HOT_A" => (9, 0x01),
        HotB = "HOT_B" => (9, 0x02),
        HotC = "HOT_C" => (9, 0x04),
        HotD = "HOT_D" => (9, 0x08),
        HotE = "HOT_E" => (9, 0x10),
        HotF = "HOT_F" => (9, 0x20),
        HotG = "HOT_G" => (9, 0x40),
        HotH = "HOT_H" => (9, 0x80),

        Source = "SOURCE" => (10, 0x01),
        Browse = "BROWSE" => (10, 0x02),
        TagList = "TAG_LIST" => (10, 0x04),
        Playlist = "PLAYLIST" => (10, 0x08),
        SearchMenu = "SEARCH_MENU" => (10, 0x10),
        Menu = "MENU" => (10, 0x20),
        JogMode = "JOG_MODE" => (10, 0x80),

        Back = "BACK" => (11, 0x01),
        TagTrack = "TAG_TRACK" => (11, 0x02),
        TrackFilter = "TRACK_FILTER" => (11, 0x04),
        Shortcut = "SHORTCUT" => (11, 0x08),
        RotaryPress = "ROTARY_PRESS" => (11, 0x10),
        Sleep = "SLEEP" => (11, 0x20),
        TimeMode = "TIME_MODE" => (11, 0x40),
        Quantize = "QUANTIZE" => (11, 0x80),

        /// The USB eject button - USB on the CDJ-3000, USB 1 on the
        /// CDJ-3000X.
        UsbStop = "USB_STOP" => (12, 0x02),
        PowerOn = "POWER_ON" => (12, 0x80),
    }
    extra {
        /// The second USB port's eject button - USB 2 on the CDJ-3000X.
        /// Takes the bit that is the SD-cover state on the CDJ-3000. Not yet
        /// confirmed against the CDJ-3000X.
        Usb2Stop = "USB2_STOP" => (12, 0x01),
    }
}

/// Device state bit: the CDJ-3000's SD slot cover is closed. Set in its idle
/// frame; a player without an SD slot makes the bit a button ([`Btn::Usb2Stop`]).
pub const STATE_SD_CLOSED: FrameBit = (12, 0x01);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for b in Btn::SHARED.iter().chain(Btn::EXTRA) {
            assert_eq!(Btn::from_name(b.name()), Some(*b));
            assert_eq!(Btn::from_name(&b.name().to_ascii_lowercase()), Some(*b));
        }
        assert_eq!(Btn::from_name("NOT_A_BUTTON"), None);
    }

    /// Two buttons on one bit would make a press ambiguous, and a name reused
    /// would make [`Btn::from_name`] answer with whichever came first.
    #[test]
    fn every_button_is_its_own_bit_and_name() {
        let all: Vec<Btn> = Btn::SHARED.iter().chain(Btn::EXTRA).copied().collect();
        for (i, a) in all.iter().enumerate() {
            assert!(a.is_shared() == Btn::SHARED.contains(a));
            for b in &all[i + 1..] {
                assert_ne!(a.bit(), b.bit(), "{a:?} and {b:?} share a bit");
                assert_ne!(a.name(), b.name(), "{a:?} and {b:?} share a name");
            }
        }
    }

    #[test]
    fn the_cdj3000x_reuses_the_sd_cover_bit() {
        assert_eq!(Btn::Usb2Stop.bit(), STATE_SD_CLOSED);
        assert!(!Btn::Usb2Stop.is_shared());
    }
}
