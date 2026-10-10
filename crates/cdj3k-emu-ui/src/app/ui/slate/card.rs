//! What a slate tells the picker about its deck: the card's text and the
//! miniature's geometry. The picker draws them.

use egui::{Pos2, Rect, Vec2};

/// Picker-only presentation of a model, held by its [`Slate`](super::Slate).
pub(in crate::app) struct Card {
    pub subtitle: &'static str,
    /// Retail release year.
    pub era: &'static str,
    pub glyph: CardGlyph,
}

/// The miniature on a deck's card.
pub(in crate::app) enum CardGlyph {
    /// A cabinet with its screen in a block standing above the body, from the
    /// slate's own zones.
    Riser(Glyph),
    /// A plain cabinet whose top is all screen housing (e.g. the CDJ-1500X).
    Flat,
}

/// Which media bay a deck carries at the head of its transport column. The bay
/// is the only part of that column where the CDJ-3000 and the CDJ-3000X differ.
pub(in crate::app) enum Bay {
    /// CDJ-3000: one USB slot, a round stop beside it, an SD slot under the
    /// pair.
    UsbAndSd,
    /// CDJ-3000X: two identical USB slots, each with a stop bar beneath.
    TwinUsb,
}

/// Panel miniature: the cabinet, the screen in its frame, the jog and the
/// browse rotary.
///
/// Every model's outline is drawn at the same units per pixel, so the bigger
/// cabinet looks bigger. The screen is drawn larger than scale, because at
/// true scale the two decks look almost identical.
pub(in crate::app) struct Glyph {
    /// Cabinet, in the slate's reference-canvas units: its aspect, and its
    /// size next to the other model's. The block the screen sits in is part
    /// of it, so the canvas is the whole silhouette. Every vertical fraction
    /// below is of this height, not of the drawn outline, which is shorter by
    /// the picker's `GLYPH_FOOT_TRIM_REF`.
    pub canvas: (f32, f32),
    /// Top of the body, as a fraction of the outline's height. The screen
    /// block stands above it, and the CDJ-3000X's block is about twice as tall
    /// as the CDJ-3000's. The values, 8.8% and 17.0%, are deeper than the true
    /// 6.4% and 12.4% so that the step stays visible in the silhouette.
    pub body_top: f32,
    /// That block's left and right, as fractions of the outline's width.
    pub riser: (f32, f32),
    /// The screen, as fractions of the outline. Centred, as it is on the deck.
    pub screen: Rect,
    /// Gap between the screen and the frame around it, as fractions of the
    /// outline: x over its width, y over its height.
    pub bezel: Vec2,
    /// The frame's corners run in to the screen's. The CDJ-3000's screen sits
    /// in a recess the drawing looks down into, so its walls read as a
    /// trapezoid; the CDJ-3000X's frame is a flat bezel, parallel the whole way round.
    pub recessed: bool,
    /// The strip the eight hot-cue keys divide between them, as fractions of
    /// the outline: left and right, then top and bottom. The panel places the
    /// row in its own block, so each deck sets its own values.
    pub pads_x: (f32, f32),
    pub pads_y: (f32, f32),
    /// Share of each key's pitch left as the gap to the next.
    pub pad_gap: f32,
    /// The transport column down the left and the modes column down the
    /// right, as fractions of the outline. Both decks carry the same controls
    /// in them and place them the same way, so the fractions inside are
    /// shared - see `KEY_*`, `TOP_KEY_*` and `FADER`.
    pub transport_col: Rect,
    pub modes_col: Rect,
    /// Which media bay stands at the head of the transport column.
    pub bay: Bay,
    /// The jog wheel's centre as fractions of the outline, and its radius as a
    /// fraction of the outline's width.
    pub jog: (f32, f32, f32),
    /// Radius of the jog display in the middle of the jog, as a fraction of
    /// the jog radius.
    pub jog_hub: f32,
    /// Centre of the browse pod, as fractions of the outline.
    pub pod: Pos2,
    /// The browse pod in reference units: half the width of the block its four
    /// arc buttons fill, and the radius of the bezel ring around the rotary.
    /// The CDJ-3000's ring stands well outside its block and is the deck's
    /// characteristic decoration; the CDJ-3000X's ring barely clears its block.
    pub pod_half_w: f32,
    pub pod_ring_r: f32,
}
