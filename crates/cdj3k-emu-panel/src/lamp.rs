//! Every lamp a player can light, declared once.
//!
//! [`Lamp`] names a lamp. The player's
//! [`MosiCodec::lamp`](crate::mosi_frame::MosiCodec::lamp) knows where its
//! frame carries the lamp and whether it is a bit, a step level or a colour.
//! A slate names the lamp it draws and asks the frame. A player without the
//! lamp answers `None`.

/// A lamp of the front panel.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Lamp {
    KeySync,
    BeatSync,
    Master,
    Slip,
    Quantize,
    /// The jog ring's white light, by level.
    JogRing,
    /// The jog ring lit red, over its white.
    JogRed,
    /// The jog's ring meter, 0 off to 0x63, on a player that has one.
    JogMeter,
    Source,
    Browse,
    TagList,
    Playlist,
    Search,
    Menu,
    Play,
    Cue,
    LoopIn,
    LoopOut,
    Reloop,
    BeatJump4,
    BeatJump8,
    BeatJumpNext,
    BeatJumpPrev,
    TempoReset,
    MasterTempo,
    JogModeCdj,
    JogModeVinyl,
    /// The rotary selector's push lamp.
    Encoder,
    TrackSearch,
    Rev,
    HotA,
    HotB,
    HotC,
    HotD,
    HotE,
    HotF,
    HotG,
    HotH,
    /// Media slot 1 in panel order: SD on the CDJ-3000, USB 1 on the
    /// CDJ-3000X.
    Slot1,
    /// Media slot 2: USB on the CDJ-3000, USB 2 on the CDJ-3000X.
    Slot2,
    OnAir,
    /// The rotary selector ring.
    Rotary,
    /// The power-management lamp, red while the deck sleeps, on a player that has one.
    Standby,
    /// The media EJECT key's lamp, on a player that has one.
    Eject,
}

impl Lamp {
    /// HOT CUE pads A-H.
    pub const HOT_CUES: [Lamp; 8] = [
        Lamp::HotA,
        Lamp::HotB,
        Lamp::HotC,
        Lamp::HotD,
        Lamp::HotE,
        Lamp::HotF,
        Lamp::HotG,
        Lamp::HotH,
    ];

    /// Every lamp.
    pub const ALL: &'static [Lamp] = &[
        Lamp::KeySync,
        Lamp::BeatSync,
        Lamp::Master,
        Lamp::Slip,
        Lamp::Quantize,
        Lamp::JogRing,
        Lamp::JogRed,
        Lamp::JogMeter,
        Lamp::Source,
        Lamp::Browse,
        Lamp::TagList,
        Lamp::Playlist,
        Lamp::Search,
        Lamp::Menu,
        Lamp::Play,
        Lamp::Cue,
        Lamp::LoopIn,
        Lamp::LoopOut,
        Lamp::Reloop,
        Lamp::BeatJump4,
        Lamp::BeatJump8,
        Lamp::BeatJumpNext,
        Lamp::BeatJumpPrev,
        Lamp::TempoReset,
        Lamp::MasterTempo,
        Lamp::JogModeCdj,
        Lamp::JogModeVinyl,
        Lamp::Encoder,
        Lamp::TrackSearch,
        Lamp::Rev,
        Lamp::HotA,
        Lamp::HotB,
        Lamp::HotC,
        Lamp::HotD,
        Lamp::HotE,
        Lamp::HotF,
        Lamp::HotG,
        Lamp::HotH,
        Lamp::Slot1,
        Lamp::Slot2,
        Lamp::OnAir,
        Lamp::Rotary,
        Lamp::Standby,
        Lamp::Eject,
    ];

    /// The name the control API uses, in the style of [`Btn::name`](crate::Btn::name).
    pub const fn name(self) -> &'static str {
        NAMES[self as usize]
    }

    /// Look up a lamp by [`name`](Self::name), case-insensitive.
    pub fn from_name(name: &str) -> Option<Lamp> {
        Lamp::ALL
            .iter()
            .copied()
            .find(|l| l.name().eq_ignore_ascii_case(name))
    }
}

/// [`Lamp::name`] of each lamp, in declaration order.
const NAMES: [&str; Lamp::ALL.len()] = [
    "KEY_SYNC",
    "BEAT_SYNC",
    "MASTER",
    "SLIP",
    "QUANTIZE",
    "JOG_RING",
    "JOG_RED",
    "JOG_METER",
    "SOURCE",
    "BROWSE",
    "TAG_LIST",
    "PLAYLIST",
    "SEARCH",
    "MENU",
    "PLAY",
    "CUE",
    "LOOP_IN",
    "LOOP_OUT",
    "RELOOP",
    "BEATJUMP_4",
    "BEATJUMP_8",
    "BEATJUMP_NEXT",
    "BEATJUMP_PREV",
    "TEMPO_RESET",
    "MASTER_TEMPO",
    "JOG_MODE_CDJ",
    "JOG_MODE_VINYL",
    "ENCODER",
    "TRACK_SEARCH",
    "REV",
    "HOT_A",
    "HOT_B",
    "HOT_C",
    "HOT_D",
    "HOT_E",
    "HOT_F",
    "HOT_G",
    "HOT_H",
    "SLOT_1",
    "SLOT_2",
    "ON_AIR",
    "ROTARY",
    "STANDBY",
    "EJECT",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_lists_the_lamps_in_declaration_order() {
        for (i, lamp) in Lamp::ALL.iter().enumerate() {
            assert_eq!(*lamp as usize, i, "{lamp:?}");
        }
    }

    #[test]
    fn every_lamp_has_its_own_name() {
        for lamp in Lamp::ALL {
            assert_eq!(Lamp::from_name(lamp.name()), Some(*lamp));
            assert_eq!(Lamp::from_name(&lamp.name().to_lowercase()), Some(*lamp));
        }
        assert_eq!(Lamp::from_name("NOPE"), None);
    }
}
