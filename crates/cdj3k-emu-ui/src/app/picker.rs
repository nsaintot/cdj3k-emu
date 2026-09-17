//! Model picker screen - "choose a device emulation".
//!
//! One card per [`Model`]. The window is one frame for every step: an identity
//! strip at the top, a title block, the step's own content, an action bar at
//! the foot. Pixel sizes are multiplied by `k`, a size factor derived from the
//! window width.

use cdj3k_emu_panel::Model;
use egui::{Align2, Color32, FontId, Pos2, Rect, Rounding, Sense, Shape, Stroke, Vec2};

/// Window width the pixel sizes below are authored against; `k == 1.0` here.
const REF_WIDTH: f32 = 760.0;
const K_MIN: f32 = 0.70;
const K_MAX: f32 = 1.40;

use super::theme;
use theme::Palette;

/// The build, at the quiet end of the action bar.
const VERSION: &str = env!("CARGO_PKG_VERSION");

const ANIM_TIME: f32 = 0.13;

// ── Identity strip ────────────────────────────────────────────────────────────
/// Between the wordmark, the slot and the deck.
const STRIP_GAP: f32 = 12.0;
/// The short upright rule that separates them.
const STRIP_RULE_H: f32 = 12.0;
/// The square of deck colour before a model's name.
const STRIP_SWATCH: f32 = 6.0;
/// Between that square and the name, and between the state dot and its word.
const STRIP_MARK_GAP: f32 = 6.0;
const STRIP_STATE_DOT: f32 = 7.0;

// ── Title block ───────────────────────────────────────────────────────────────
const TITLE_TOP_GAP: f32 = 20.0;
const TITLE_SUB_GAP: f32 = 8.0;
/// Below the subtitle, where a step's own content starts.
const HEADER_BODY_GAP: f32 = 16.0;

// ── Cards ─────────────────────────────────────────────────────────────────────
const CARD_GAP: f32 = 22.0;
const CARD_PAD: f32 = 16.0;
/// Decks the picker names but cannot install: a plate that says what is
/// coming and nothing it cannot back up. No [`Model`] stands behind one, so
/// there is no firmware, no slot and no click.
const ANNOUNCED: [&str; 1] = ["CDJ-1500X"];
/// What the announced plate says in place of a display size and a year.
const SOON_SUB: &str = "";
const SOON_PILL: &str = "NOT AVAILABLE";
const SOON_CHIP: &str = "SOON";

/// A card is a plate of its own size, centred in the row with the space left
/// over around it - not a panel stretched to whatever the window has. Three
/// decks fit the window's width before the cap does anything.
const CARD_MAX_W: f32 = 250.0;
const CARD_MAX_H: f32 = 300.0;
/// Under the card row, so it does not sit on the action bar.
const CARD_ROW_BOTTOM: f32 = 20.0;
/// The band of deck colour across the head of a card - the one place the
/// window lets a hue run full width.
const CARD_CAP_H: f32 = 3.0;
/// How far a card's edge darkens towards the ink under the pointer.
const CARD_HOVER_EDGE: f32 = 0.55;

// Deck glyph: cabinet, screen in its frame, hot-cue pads, USB bay, jog and
// browse pod. Nothing touches anything; the outline and the jog carry the
// weight and everything else is drawn a shade finer.
const GLYPH_STROKE: f32 = 1.3;
const GLYPH_DETAIL_STROKE: f32 = 0.9;
/// The cabinet's corner radius, in reference units: the chassis' own, which is
/// slight - at this size all but sharp.
const GLYPH_CHASSIS_ROUND_REF: f32 = 25.0;
/// Segments per rounded corner of the cabinet silhouette.
const GLYPH_CORNER_SEGS: usize = 5;
const GLYPH_SCREEN_ROUND: f32 = 1.5;
const GLYPH_FRAME_ROUND: f32 = 1.0;
/// Hot cues, A to H.
const GLYPH_PAD_COUNT: usize = 8;

// Where the shared controls sit inside their own column, measured off
// AlphaTheta's CDJ-3000X outline drawing. Both decks give their columns the
// same size, so one set of fractions places them on either.
/// CUE and PLAY: their centre across the transport column, their diameter
/// over its width, and the two heights they sit at down it. The drawing puts
/// them at 350 x 348 reference units - round, not square - and all but
/// touching; drawn a little under that, and further apart, they read as two
/// keys at 91 px rather than a figure of eight.
const KEY_CX: f32 = 0.5288;
const KEY_D: f32 = 0.60;
const KEY_CY: (f32, f32) = (0.815, 0.945);
/// Room either side of the label on the confirmation's wide button.
const CONFIRM_BTN_PAD: f32 = 36.0;

// ── The slot chip, and the menu it opens ──────────────────────────────────────
const SLOT_CHIP_H: f32 = 22.0;
const SLOT_CHIP_PAD: f32 = 9.0;
const SLOT_CHEV_W: f32 = 8.0;
const SLOT_CHEV_H: f32 = 4.0;
const SLOT_MENU_W: f32 = 296.0;
const SLOT_MENU_ROW_H: f32 = 30.0;
const SLOT_MENU_PAD: f32 = 10.0;
const SLOT_MENU_GAP: f32 = 10.0;
const SLOT_MENU_FONT: f32 = 12.0;
/// Where a slot's deck name starts, past its own number.
const SLOT_MENU_NAME_W: f32 = 52.0;
/// Room kept at the right end for the "THIS WINDOW" note.
const SLOT_MENU_HERE_W: f32 = 74.0;
/// Between a slot's deck name and the firmware release on it.
const SLOT_MENU_REL_GAP: f32 = 7.0;
/// The delete beside the dismiss in the action bar, and the upright rule that
/// sets the slot's own action apart from the window's.
const DELETE_BTN_W: f32 = 138.0;
const FOOTER_RULE_H: f32 = 20.0;

// The bin on that button, in fractions of the square the button reserves for
// it. Lid, handle and a body that tapers to its base - no bundled face carries
// a bin, and an emoji one would be the only filled shape in the window.
const TRASH_LID_Y: f32 = 0.28;
const TRASH_LID_X: (f32, f32) = (0.04, 0.96);
const TRASH_HANDLE_X: (f32, f32) = (0.34, 0.66);
const TRASH_HANDLE_Y: f32 = 0.10;
const TRASH_BODY_TOP: (f32, f32) = (0.15, 0.85);
const TRASH_BODY_BASE: (f32, f32) = (0.26, 0.74);
const TRASH_BODY_Y: f32 = 0.94;

/// The tempo fader, down the modes column.
const FADER_X: (f32, f32) = (0.2835, 0.7427);
const FADER_Y: (f32, f32) = (0.6566, 0.9604);

// The media bay, also in transport-column fractions. The parts keep the
// proportions the drawings give them - the CDJ-3000's slot 212 x 194 against
// its 365 x 222 SD, the CDJ-3000X's two slots 228 x 223 - but the bay is centred in
// its column and spaced for a miniature 91 px wide, where the panel's own
// margins close up into a blot in the corner.
const BAY_TOP: f32 = 0.030;
/// CDJ-3000: the slot, the round stop off its bottom-right corner, the SD
/// under the pair. The panel leaves 20 reference units between slot and stop,
/// which is half a pixel here - the two strokes merge - so the stop is set
/// out clear of it.
const BAY_USB: Rect = Rect::from_min_max(Pos2::new(0.1000, BAY_TOP), Pos2::new(0.5000, 0.0775));
/// Centre across and down the column, then radius over its width. Its centre
/// sits on the slot's bottom edge.
const BAY_STOP: (f32, f32, f32) = (0.7200, 0.0775, 0.1000);
const BAY_SD: Rect = Rect::from_min_max(Pos2::new(0.1000, 0.1150), Pos2::new(0.8000, 0.1694));
const BAY_SLOTS: [Rect; 2] = [
    Rect::from_min_max(Pos2::new(0.2763, BAY_TOP), Pos2::new(0.7237, 0.0896)),
    Rect::from_min_max(Pos2::new(0.2763, 0.1646), Pos2::new(0.7237, 0.2192)),
];
/// Their stop bars, each just under its own slot.
const BAY_STOPS: [Rect; 2] = [
    Rect::from_min_max(Pos2::new(0.4147, 0.1016), Pos2::new(0.5853, 0.1098)),
    Rect::from_min_max(Pos2::new(0.4147, 0.2312), Pos2::new(0.5853, 0.2394)),
];

// Browse pod, in reference units. The block the four arc buttons fill is the
// same height on both decks; its width and the bezel ring around the rotary
// are not - see `Glyph::pod_ring_r`.
const GLYPH_POD_H_REF: f32 = 726.0;
const GLYPH_POD_ROUND_REF: f32 = 45.0;
/// Segments along each button pair's inner edge, which follows the ring.
const GLYPH_POD_ARC_SEGS: usize = 10;
const GLYPH_POD_HUB_R_REF: f32 = 100.0;
/// Reference-canvas height `GLYPH_H_FRAC` is authored against; a taller
/// cabinet draws proportionally taller.
const GLYPH_CANVAS_H: f32 = 4659.0;
/// Reference units trimmed off the cabinet's foot: none, since PLAY sits in
/// the skirt there.
const GLYPH_FOOT_TRIM_REF: f32 = 0.0;
// ── A card's contents, measured up from its foot ──────────────────────────────
const CARD_TITLE_FONT: f32 = 20.0;
const CARD_SUB_FONT: f32 = 12.0;
const CARD_ERA_FONT: f32 = 10.0;
const CARD_ERA_TRACK: f32 = 1.4;
/// Up from the pill to the era line, and from there to the subtitle and the
/// title above it.
const CARD_ERA_GAP: f32 = 18.0;
const CARD_SUB_GAP: f32 = 16.0;
const CARD_TITLE_GAP: f32 = 20.0;
/// Between the title and the foot of the miniature standing above it.
const CARD_GLYPH_GAP: f32 = 14.0;

/// "INSTALLED" in the top-right corner of the slot's own model.
const CHIP_FONT: f32 = 9.0;
const CHIP_TRACK: f32 = 1.3;
const CHIP_PAD_X: f32 = 7.0;
const CHIP_H: f32 = 18.0;
/// Below the accent cap, and below the chip where the miniature starts.
const CHIP_TOP: f32 = 12.0;
const CHIP_GLYPH_GAP: f32 = 10.0;

const PILL_FONT: f32 = 11.0;
const PILL_TRACK: f32 = 1.5;

// ── The replace confirmation, shown in place of the card row ──────────────────
const CONFIRM_W: f32 = 560.0;
/// The deck-to-deck line above the plate, and the arrow in it.
const SWAP_H: f32 = 14.0;
const SWAP_GAP: f32 = 18.0;
const SWAP_ARROW_W: f32 = 22.0;
const SWAP_FONT: f32 = 11.0;
/// The plate naming what an install takes with it.
const LOSS_PAD: f32 = 16.0;
const LOSS_HEAD_H: f32 = 32.0;
const LOSS_ROW_H: f32 = 34.0;
const LOSS_FONT: f32 = 13.0;
/// The dash before each thing on it.
const LOSS_DASH_W: f32 = 8.0;
const LOSS_DASH_GAP: f32 = 12.0;

/// Picker-only presentation of a model, held by its
/// [`Slate`](crate::app::ui::slate::Slate).
pub(in crate::app) struct Card {
    pub subtitle: &'static str,
    /// Retail release year.
    pub era: &'static str,
    pub accent: Color32,
    pub glyph: Glyph,
}

/// Which media bay a deck carries at the head of its transport column - the
/// one place the two CDJ-3000s part company.
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
/// Every model's outline is drawn at one units-per-pixel, so the bigger
/// cabinet looks bigger. The screen is drawn larger than scale on purpose -
/// the two decks differ by so little else that true scale tells them apart
/// only on a ruler.
pub(in crate::app) struct Glyph {
    /// Cabinet, in the slate's reference-canvas units: its aspect, and its
    /// size next to the other model's. The block the screen sits in is part
    /// of it, so the canvas is the whole silhouette. Every vertical fraction
    /// below is of this height, not of the drawn outline, which is shorter by
    /// [`GLYPH_FOOT_TRIM_REF`].
    pub canvas: (f32, f32),
    /// Top of the body, as a fraction of the outline's height. The block the
    /// screen sits in stands above it, and the CDJ-3000X's screen is big enough that
    /// its block stands roughly twice as proud as the CDJ-3000's. Drawn
    /// deeper than the drawings have it - 8.8% and 17.0% against 6.4% and
    /// 12.4% - because that step is what the silhouette has to carry.
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
    /// row in its own block, so both decks give their own.
    pub pads_x: (f32, f32),
    pub pads_y: (f32, f32),
    /// Share of each key's pitch left as the gap to the next.
    pub pad_gap: f32,
    /// The transport column down the left and the modes column down the
    /// right, as fractions of the outline. Both decks carry the same controls
    /// in them and place them the same way, so the fractions inside are
    /// shared - see `KEY_*`, `TOP_KEY_*` and `FADER_*`.
    pub transport_col: Rect,
    pub modes_col: Rect,
    /// Which media bay stands at the head of the transport column.
    pub bay: Bay,
    /// Jog wheel: centre as fractions of the outline, radius over its width.
    pub jog: (f32, f32, f32),
    /// The jog display in the middle of it, over that radius.
    pub jog_hub: f32,
    /// Centre of the browse pod, as fractions of the outline.
    pub pod: Pos2,
    /// That pod, in reference units: half the width of the block its four arc
    /// buttons fill, and the radius of the bezel ring around the rotary. The
    /// CDJ-3000's ring stands well outside its block, which is the decoration
    /// the deck is known by; the CDJ-3000X's barely clears its own.
    pub pod_half_w: f32,
    pub pod_ring_r: f32,
}

/// What the picker is being asked to show.
pub(in crate::app) struct PickerView<'a> {
    pub slot: u32,
    /// A line under the header, e.g. a stop in progress.
    pub status: Option<&'a str>,
    /// The model installed in this slot. A slot holds one installation.
    pub installed: Option<Model>,
    /// The model whose emulation is running behind the picker.
    pub running: Option<Model>,
    /// Every card launches with nothing installed (`--no-spawn`).
    pub all_launchable: bool,
    /// Shown in place of the cards: the model that would take the slot over.
    pub confirm_replace: Option<Model>,
    /// The picker is a window over a live emulation, so it can be dismissed.
    pub dismissable: bool,
    /// The slot shown is not this window's. Its emulation runs in its own
    /// window, so the deck it holds offers a launch rather than a reinstall.
    pub foreign: bool,
    /// That slot's window is already up, so its installation is not this
    /// window's to touch.
    pub busy: bool,
    /// The firmware release installed in the slot shown.
    pub release: Option<&'a str>,
}

pub(in crate::app) enum PickerAction {
    /// Point the window at another slot.
    View(u32),
    /// Bring up a slot's own window, where its emulation runs.
    Open(u32),
    /// A card was clicked.
    Choose(Model),
    /// The running deck's card was clicked: lay its firmware down again.
    Reinstall(Model),
    /// The replace confirmation was accepted.
    Replace(Model),
    /// The replace confirmation was declined.
    CancelReplace,
    /// Leave the slot as it is.
    Dismiss,
    /// Empty the slot: no firmware, no eMMC, nothing installed in its place.
    Delete,
    /// The delete confirmation was accepted.
    ConfirmDelete,
    /// The delete confirmation was declined.
    CancelDelete,
}

/// What a card's foot pill offers for its model.
enum Pill {
    /// The slot's model, already emulating. Launching it again is no offer, so
    /// the card offers the one thing left: laying its firmware down afresh.
    Reinstall,
    /// The slot's model, ready to boot.
    Launch,
    /// An empty slot.
    Install,
    /// Another model's slot: installing takes it over.
    Replace,
}

impl Pill {
    fn of(model: Model, view: &PickerView<'_>) -> Self {
        if view.foreign {
            // Another slot's deck launches in its own window; nothing here
            // reinstalls it.
            return if view.installed == Some(model) {
                Self::Launch
            } else if view.installed.is_none() {
                Self::Install
            } else {
                Self::Replace
            };
        }
        if view.running == Some(model) {
            Self::Reinstall
        } else if view.all_launchable || view.installed == Some(model) {
            Self::Launch
        } else if view.installed.is_none() {
            Self::Install
        } else {
            Self::Replace
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Reinstall => "REINSTALL",
            Self::Launch => "LAUNCH",
            Self::Install => "INSTALL FIRMWARE",
            Self::Replace => "INSTALL & REPLACE",
        }
    }
}

/// What the identity strip and the title block say for a step.
pub(in crate::app) struct Header<'a> {
    pub slot: u32,
    /// The deck this step is about, named in the strip beside its colour.
    pub model: Option<Model>,
    /// A word at the strip's right end while something is under way.
    pub state: Option<&'a str>,
    /// The firmware release on that deck, beside its name.
    pub release: Option<&'a str>,
    pub title: &'a str,
    pub sub: &'a str,
    /// Whether the slot chip opens its menu. The window follows the slot it
    /// is pointed at, so a step in the middle of a job keeps its own.
    pub switchable: bool,
}

/// What a header drew, and what the user asked of it.
pub(in crate::app) struct HeaderOut {
    /// The area the step's own content gets, the action bar included.
    pub body: Rect,
    /// A slot picked from the chip's menu.
    pub switch_to: Option<u32>,
}

/// Paint the window's ground, its identity strip and its title block, and
/// return the area a step's own content gets - the action bar included. Every
/// step draws this, so the window reads as one thing as the job moves from
/// choosing a deck to installing its firmware.
pub(in crate::app) fn draw_header(ui: &egui::Ui, head: &Header<'_>) -> HeaderOut {
    let area = ui.max_rect();
    let k = k_of(area);
    let pal = theme::palette(ui.ctx());
    let p = ui.painter();
    p.rect_filled(area, 0.0, pal.paper);

    let strip = Rect::from_min_max(
        area.left_top(),
        Pos2::new(area.right(), area.top() + theme::TOPBAR_H * k),
    );
    p.rect_filled(strip, 0.0, pal.plate);
    hairline(p, strip.left_bottom(), strip.right_bottom(), pal.line, k);

    let micro = FontId::proportional(theme::MICRO_FONT * k);
    let cy = strip.center().y;
    let mut x = area.left() + theme::GUTTER * k;
    x += tracked(
        p,
        Pos2::new(x, cy),
        cdj3k_emu_platform::app_meta::APP_DISPLAY_NAME,
        &micro,
        pal.ink,
        theme::WORDMARK_TRACK * k,
    ) + STRIP_GAP * k;
    x += strip_rule(p, x, cy, pal.line, k) + STRIP_GAP * k;
    let (chip_w, switch_to) = draw_slot_switch(ui, x, cy, head, &micro, pal, k);
    x += chip_w + STRIP_GAP * k;
    if let Some(model) = head.model {
        x += strip_rule(p, x, cy, pal.line, k) + STRIP_GAP * k;
        let s = STRIP_SWATCH * k;
        p.rect_filled(
            Rect::from_center_size(Pos2::new(x + s * 0.5, cy), Vec2::splat(s)),
            theme::HAIRLINE * k,
            crate::app::ui::slate::for_model(model).card.accent,
        );
        x += s + STRIP_MARK_GAP * k;
        x += tracked(
            p,
            Pos2::new(x, cy),
            model.title(),
            &micro,
            pal.dim,
            theme::MICRO_TRACK * k,
        );
        // The firmware it was installed from, quieter than the deck it is on.
        if let Some(release) = head.release {
            tracked(
                p,
                Pos2::new(x + STRIP_MARK_GAP * k, cy),
                release,
                &micro,
                pal.faint,
                theme::MICRO_TRACK * k,
            );
        }
    }
    if let Some(state) = head.state {
        let w = text_width(p, state, &micro, theme::MICRO_TRACK * k);
        let left = area.right() - theme::GUTTER * k - w;
        let r = STRIP_STATE_DOT * k * 0.5;
        p.circle_filled(Pos2::new(left - STRIP_MARK_GAP * k - r, cy), r, pal.ink);
        tracked(
            p,
            Pos2::new(left, cy),
            state,
            &micro,
            pal.ink,
            theme::MICRO_TRACK * k,
        );
    }

    let left = area.left() + theme::GUTTER * k;
    let mut y = strip.bottom() + TITLE_TOP_GAP * k;
    p.text(
        Pos2::new(left, y),
        Align2::LEFT_TOP,
        head.title,
        FontId::proportional(theme::TITLE_FONT * k),
        pal.ink,
    );
    y += theme::TITLE_FONT * k * 1.12 + TITLE_SUB_GAP * k;
    let galley = p.layout(
        head.sub.to_owned(),
        FontId::proportional(theme::SUB_FONT * k),
        pal.muted,
        area.width() - 2.0 * theme::GUTTER * k,
    );
    let sub_h = galley.size().y;
    p.galley(Pos2::new(left, y), galley, pal.muted);

    HeaderOut {
        body: Rect::from_min_max(
            Pos2::new(area.left(), y + sub_h + HEADER_BODY_GAP * k),
            area.max,
        ),
        switch_to,
    }
}

/// The slot chip: which slot this window is showing, and the way to the
/// others. Picking one points the window at that slot; running it stays its
/// own window's job. Returns the chip's width and any slot picked.
fn draw_slot_switch(
    ui: &egui::Ui,
    x: f32,
    cy: f32,
    head: &Header<'_>,
    font: &FontId,
    pal: &Palette,
    k: f32,
) -> (f32, Option<u32>) {
    let slot = head.slot;
    let label = format!("SLOT {slot}");
    let p = ui.painter();
    let text_w = text_width(p, &label, font, theme::MICRO_TRACK * k);
    let mut w = text_w + (SLOT_CHIP_PAD * 2.0 + STRIP_MARK_GAP) * k;
    if head.switchable {
        w += SLOT_CHEV_W * k;
    }
    let chip = Rect::from_center_size(Pos2::new(x + w * 0.5, cy), Vec2::new(w, SLOT_CHIP_H * k));
    if !head.switchable {
        p.rect_stroke(
            chip,
            theme::ROUND * k,
            Stroke::new(theme::HAIRLINE * k, pal.line_strong),
        );
        tracked(
            p,
            Pos2::new(chip.left() + SLOT_CHIP_PAD * k, cy),
            &label,
            font,
            pal.ink,
            theme::MICRO_TRACK * k,
        );
        return (w, None);
    }
    let id = ui.id().with("cdj_slot_switch");
    let resp = theme::pointer(ui.interact(chip, id, Sense::click()));
    let popup = ui.id().with("cdj_slot_menu");
    let open = ui.memory(|m| m.is_popup_open(popup));

    let edge = if open || resp.hovered() {
        pal.ink
    } else {
        pal.line_strong
    };
    p.rect_stroke(
        chip,
        theme::ROUND * k,
        Stroke::new(theme::HAIRLINE * k, edge),
    );
    tracked(
        p,
        Pos2::new(chip.left() + SLOT_CHIP_PAD * k, cy),
        &label,
        font,
        pal.ink,
        theme::MICRO_TRACK * k,
    );
    // The chevron: this opens a menu, so it points down at one.
    let c = Pos2::new(chip.right() - SLOT_CHIP_PAD * k - SLOT_CHEV_W * k * 0.5, cy);
    let (hw, hh) = (SLOT_CHEV_W * k * 0.5, SLOT_CHEV_H * k * 0.5);
    let chev = Stroke::new(theme::HAIRLINE * k * 1.2, pal.dim);
    p.line_segment(
        [Pos2::new(c.x - hw, c.y - hh), Pos2::new(c.x, c.y + hh)],
        chev,
    );
    p.line_segment(
        [Pos2::new(c.x + hw, c.y - hh), Pos2::new(c.x, c.y + hh)],
        chev,
    );

    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup));
    }
    let mut picked = None;
    egui::popup::popup_below_widget(
        ui,
        popup,
        &resp,
        egui::popup::PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            theme::apply_setup_style(ui, pal);
            ui.set_min_width(SLOT_MENU_W * k);
            ui.spacing_mut().item_spacing.y = 2.0 * k;
            for n in 1..=cdj3k_emu_platform::menu_state::MAX_INSTANCES {
                if slot_menu_row(ui, n, n == slot, pal, k).clicked() {
                    if n != slot {
                        picked = Some(n);
                    }
                    ui.memory_mut(|m| m.close_popup());
                }
            }
        },
    );
    (w, picked)
}

/// One slot in that menu: its deck colour, its name, and what it holds.
fn slot_menu_row(ui: &mut egui::Ui, n: u32, here: bool, pal: &Palette, k: f32) -> egui::Response {
    // What the slot holds, not what it last recorded: a slot keeps its model
    // in settings after its files are deleted.
    let held = cdj3k_emu_storage::slot_summary(n);
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), SLOT_MENU_ROW_H * k),
        Sense::click(),
    );
    let p = ui.painter();
    if here || resp.hovered() {
        p.rect_filled(rect, theme::ROUND_CHIP * k, pal.field);
    }
    let s = STRIP_SWATCH * k;
    let mark = Rect::from_center_size(
        Pos2::new(rect.left() + SLOT_MENU_PAD * k + s * 0.5, rect.center().y),
        Vec2::splat(s),
    );
    match held {
        Some((m, _)) => {
            p.rect_filled(
                mark,
                theme::HAIRLINE * k,
                crate::app::ui::slate::for_model(m).card.accent,
            );
        }
        None => {
            p.rect_stroke(
                mark,
                theme::HAIRLINE * k,
                Stroke::new(theme::HAIRLINE * k, pal.off_line),
            );
        }
    }
    let text = FontId::proportional(SLOT_MENU_FONT * k);
    let x = mark.right() + SLOT_MENU_GAP * k;
    p.text(
        Pos2::new(x, rect.center().y),
        Align2::LEFT_CENTER,
        format!("Slot {n}"),
        text.clone(),
        pal.ink,
    );
    let name = held.as_ref().map_or("empty", |(m, _)| m.title());
    let name_x = x + SLOT_MENU_NAME_W * k;
    p.text(
        Pos2::new(name_x, rect.center().y),
        Align2::LEFT_CENTER,
        name,
        text.clone(),
        if held.is_some() {
            pal.muted
        } else {
            pal.off_text
        },
    );
    // The firmware the slot was installed from, behind the deck it names.
    if let Some(release) = held.as_ref().and_then(|(_, r)| r.as_deref()) {
        p.text(
            Pos2::new(
                name_x + text_width(p, name, &text, 0.0) + SLOT_MENU_REL_GAP * k,
                rect.center().y,
            ),
            Align2::LEFT_CENTER,
            release,
            text,
            pal.faint,
        );
    }
    if here {
        tracked(
            p,
            Pos2::new(
                rect.right() - SLOT_MENU_PAD * k - SLOT_MENU_HERE_W * k,
                rect.center().y,
            ),
            "THIS WINDOW",
            &FontId::proportional(CHIP_FONT * k),
            pal.faint,
            CHIP_TRACK * k,
        );
    }
    theme::pointer(resp)
}

/// Split a step's area into its own space and the action bar at its foot.
pub(in crate::app) fn footer_split(area: Rect, k: f32) -> (Rect, Rect) {
    let y = area.bottom() - theme::FOOTER_H * k;
    (
        Rect::from_min_max(area.min, Pos2::new(area.right(), y)),
        Rect::from_min_max(Pos2::new(area.left(), y), area.max),
    )
}

/// Paint the action bar - a plate under a hairline, the build at its quiet
/// end - and return the area the step's own buttons get.
pub(in crate::app) fn draw_footer(ui: &egui::Ui, bar: Rect, pal: &Palette, k: f32) -> Rect {
    let p = ui.painter();
    p.rect_filled(bar, 0.0, pal.plate);
    hairline(p, bar.left_top(), bar.right_top(), pal.line, k);
    p.text(
        Pos2::new(bar.left() + theme::GUTTER * k, bar.center().y),
        Align2::LEFT_CENTER,
        format!("v{VERSION}"),
        FontId::monospace(theme::MICRO_FONT * k),
        pal.faint,
    );
    Rect::from_center_size(
        bar.center(),
        Vec2::new(
            bar.width() - 2.0 * theme::GUTTER * k,
            theme::BUTTON_SIZE[1] * k,
        ),
    )
}

/// The size every action button in the bar takes, so a row of them lines up
/// whatever the labels say.
pub(in crate::app) fn button_size(k: f32) -> [f32; 2] {
    [theme::BUTTON_SIZE[0] * k, theme::BUTTON_SIZE[1] * k]
}

/// The window's size factor: every pixel size is authored against
/// [`REF_WIDTH`].
pub(in crate::app) fn k_of(area: Rect) -> f32 {
    (area.width() / REF_WIDTH).clamp(K_MIN, K_MAX)
}

/// A step with nothing to do but wait: the emulation stopping before its
/// installation may be touched.
pub(in crate::app) fn draw_busy(
    ui: &mut egui::Ui,
    head: &Header<'_>,
    headline: &str,
    detail: &str,
) {
    let body = draw_header(ui, head).body;
    let k = k_of(ui.max_rect());
    let pal = theme::palette(ui.ctx());
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(body), |ui| {
        theme::apply_setup_style(ui, pal);
        ui.vertical_centered(|ui| {
            ui.add_space(body.height() * 0.28);
            ui.spinner();
            ui.add_space(14.0 * k);
            ui.label(egui::RichText::new(headline).size(16.0 * k).color(pal.ink));
            ui.add_space(6.0 * k);
            ui.label(
                egui::RichText::new(detail)
                    .size(theme::BODY_FONT * k)
                    .color(pal.muted),
            );
        });
    });
}

/// Draw the picker into `ui`; returns what the user asked for.
pub(super) fn draw_picker(ui: &mut egui::Ui, view: &PickerView<'_>) -> Option<PickerAction> {
    let k = k_of(ui.max_rect());
    let pal = theme::palette(ui.ctx());

    if let Some(model) = view.confirm_replace {
        return draw_confirm(ui, view, model, k, pal);
    }

    let sub = match view.status {
        Some(status) => status.to_owned(),
        None => "emulation model for the current slot".to_owned(),
    };
    let head = Header {
        slot: view.slot,
        // The deck the slot holds, running or not - the strip says which slot
        // this is, and what is in it.
        model: view.running.or(view.installed),
        release: view.release,
        state: match (view.running, view.busy) {
            (Some(_), _) => Some("RUNNING"),
            (None, true) => Some("OPEN"),
            _ => None,
        },
        title: if view.dismissable {
            "Choose an emulation"
        } else {
            "Install an emulation"
        },
        sub: &sub,
        switchable: true,
    };
    let out = draw_header(ui, &head);
    let body = out.body;
    let (space, bar) = footer_split(body, k);
    let mut action = out.switch_to.map(PickerAction::View);

    let row = Rect::from_min_max(
        Pos2::new(space.left() + theme::GUTTER * k, space.top()),
        Pos2::new(
            space.right() - theme::GUTTER * k,
            space.bottom() - CARD_ROW_BOTTOM * k,
        ),
    );
    let n = (Model::ALL.len() + ANNOUNCED.len()) as f32;
    let gap = CARD_GAP * k;
    let cw = ((row.width() - gap * (n - 1.0)) / n).min(CARD_MAX_W * k);
    let ch = row.height().min(CARD_MAX_H * k);
    let x0 = row.center().x - (cw * n + gap * (n - 1.0)) * 0.5;
    let y0 = row.center().y - ch * 0.5;
    for (i, model) in Model::ALL.iter().enumerate() {
        let model = *model;
        let rect =
            Rect::from_min_size(Pos2::new(x0 + i as f32 * (cw + gap), y0), Vec2::new(cw, ch));
        // A slot whose own window is up keeps its installation: only the deck
        // it holds is offered, and only to bring that window forward.
        let inert = view.busy && view.installed != Some(model);
        let resp = ui.interact(rect, ui.id().with(("cdj_model", i)), Sense::click());
        let resp = if inert { resp } else { theme::pointer(resp) };
        let hover_t = ui.ctx().animate_bool_with_time(
            ui.id().with(("cdj_hover", i)),
            resp.hovered() && !inert,
            ANIM_TIME,
        );
        draw_card(
            ui.painter(),
            rect,
            model,
            Pill::of(model, view),
            view.installed == Some(model),
            inert,
            hover_t,
            k,
            pal,
        );
        if resp.clicked() && !inert {
            action = Some(if view.foreign && view.installed == Some(model) {
                PickerAction::Open(view.slot)
            } else if !view.foreign && view.running == Some(model) {
                PickerAction::Reinstall(model)
            } else {
                PickerAction::Choose(model)
            });
        }
    }
    for (i, name) in ANNOUNCED.iter().enumerate() {
        draw_soon_card(
            ui.painter(),
            Rect::from_min_size(
                Pos2::new(x0 + (Model::ALL.len() + i) as f32 * (cw + gap), y0),
                Vec2::new(cw, ch),
            ),
            name,
            k,
            pal,
        );
    }

    let inner = draw_footer(ui, bar, pal, k);
    // Leaving the slot as it is: a named button in the bar, where the window's
    // other actions are, rather than a second cross under the one the desktop
    // already draws.
    // A slot whose own window has it keeps its installation; and only a window
    // standing over an emulation has anything to go back to.
    let deletable = view.installed.is_some() && !view.busy;
    if view.dismissable || deletable {
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
            theme::apply_setup_style(ui, pal);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if view.dismissable && theme::secondary(ui, button_size(k), "Close", pal).clicked()
                {
                    action = Some(PickerAction::Dismiss);
                }
                // Beside the dismiss, behind a rule and outlined rather than
                // filled: destructive, but not the thing this step is for.
                if deletable {
                    if view.dismissable {
                        footer_rule(ui, pal, k);
                    }
                    // Which slot is the strip's job, at the top of the window;
                    // the button says what it does, not where.
                    if theme::destructive_line(
                        ui,
                        [DELETE_BTN_W * k, theme::BUTTON_SIZE[1] * k],
                        "Delete slot",
                        pal,
                        |p, icon, col| trash_glyph(p, icon, col, k),
                    )
                    .clicked()
                    {
                        action = Some(PickerAction::Delete);
                    }
                }
            });
        });
    }
    action
}

/// The card row replaced by the question the replacement asks: the slot's
/// installation, its eMMC and everything the guest wrote to it all go.
fn draw_confirm(
    ui: &mut egui::Ui,
    view: &PickerView<'_>,
    model: Model,
    k: f32,
    pal: &Palette,
) -> Option<PickerAction> {
    let held = view.installed.map(|m| m.title()).unwrap_or("nothing");
    let title = format!("Replace the emulation in slot {}?", view.slot);
    let sub = if view.running.is_some() {
        format!(
            "Slot {} holds the {held}. Installing the {} replaces it, and the running \
             emulation stops first.",
            view.slot,
            model.title()
        )
    } else {
        format!(
            "Slot {} holds the {held}. Installing the {} replaces it.",
            view.slot,
            model.title()
        )
    };
    let head = Header {
        slot: view.slot,
        model: view.installed,
        release: view.release,
        state: None,
        title: &title,
        sub: &sub,
        switchable: false,
    };
    let body = draw_header(ui, &head).body;
    let (space, bar) = footer_split(body, k);

    let w = (CONFIRM_W * k).min(space.width() - 2.0 * theme::GUTTER * k);
    let plate_h = (LOSS_HEAD_H + 3.0 * LOSS_ROW_H) * k;
    let block_h = (SWAP_H + SWAP_GAP) * k + plate_h;
    let left = space.center().x - w * 0.5;
    let top = space.center().y - block_h * 0.5;
    let p = ui.painter();

    // The two decks, in the order the slot sees them.
    let swap_font = FontId::proportional(SWAP_FONT * k);
    let cy = top + SWAP_H * k * 0.5;
    let mut x = left;
    for (i, deck) in [view.installed, Some(model)]
        .into_iter()
        .flatten()
        .enumerate()
    {
        if i > 0 {
            let a = SWAP_ARROW_W * k;
            let arrow = Stroke::new(theme::HAIRLINE * k, pal.faint);
            p.line_segment([Pos2::new(x, cy), Pos2::new(x + a, cy)], arrow);
            for dy in [-1.0, 1.0] {
                p.line_segment(
                    [
                        Pos2::new(x + a, cy),
                        Pos2::new(x + a - a * 0.16, cy + a * 0.16 * dy),
                    ],
                    arrow,
                );
            }
            x += a + STRIP_GAP * k;
        }
        let s = STRIP_SWATCH * k;
        p.rect_filled(
            Rect::from_center_size(Pos2::new(x + s * 0.5, cy), Vec2::splat(s)),
            theme::HAIRLINE * k,
            crate::app::ui::slate::for_model(deck).card.accent,
        );
        x += s + STRIP_MARK_GAP * k;
        x += tracked(
            p,
            Pos2::new(x, cy),
            deck.title(),
            &swap_font,
            pal.muted,
            theme::MICRO_TRACK * k,
        ) + STRIP_GAP * k;
    }

    // What an install takes with it.
    let plate = Rect::from_min_size(
        Pos2::new(left, top + (SWAP_H + SWAP_GAP) * k),
        Vec2::new(w, plate_h),
    );
    let round = theme::ROUND * k;
    p.rect_filled(plate, round, pal.plate);
    p.rect_stroke(plate, round, Stroke::new(theme::HAIRLINE * k, pal.line));
    let band = Rect::from_min_size(plate.min, Vec2::new(w, LOSS_HEAD_H * k));
    p.rect_filled(
        band,
        Rounding {
            nw: round,
            ne: round,
            sw: 0.0,
            se: 0.0,
        },
        pal.field,
    );
    hairline(p, band.left_bottom(), band.right_bottom(), pal.line, k);
    tracked(
        p,
        Pos2::new(band.left() + LOSS_PAD * k, band.center().y),
        "DELETED, AND NOT RECOVERABLE",
        &FontId::proportional(theme::MICRO_FONT * k),
        pal.danger,
        theme::MICRO_TRACK * k,
    );
    let rows = [
        format!("The {held} firmware in this slot"),
        "Its eMMC image".to_owned(),
        "Every setting stored on it".to_owned(),
    ];
    for (i, row) in rows.iter().enumerate() {
        let ry = band.bottom() + (i as f32 + 0.5) * LOSS_ROW_H * k;
        let dx = plate.left() + LOSS_PAD * k;
        p.line_segment(
            [Pos2::new(dx, ry), Pos2::new(dx + LOSS_DASH_W * k, ry)],
            Stroke::new(theme::HAIRLINE * k, pal.line_strong),
        );
        p.text(
            Pos2::new(dx + (LOSS_DASH_W + LOSS_DASH_GAP) * k, ry),
            Align2::LEFT_CENTER,
            row,
            FontId::proportional(LOSS_FONT * k),
            pal.ink,
        );
    }

    let inner = draw_footer(ui, bar, pal, k);
    let mut action = None;
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
        theme::apply_setup_style(ui, pal);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = format!("Install the {}", model.title());
            let wide = [
                text_width(
                    ui.painter(),
                    &label,
                    &FontId::proportional(theme::BUTTON_FONT * k),
                    0.0,
                ) + CONFIRM_BTN_PAD * k,
                theme::BUTTON_SIZE[1] * k,
            ];
            if theme::destructive(ui, wide, &label, pal).clicked() {
                action = Some(PickerAction::Replace(model));
            }
            if theme::secondary(ui, button_size(k), "Keep it", pal).clicked() {
                action = Some(PickerAction::CancelReplace);
            }
        });
    });
    action
}

#[allow(clippy::too_many_arguments)]
fn draw_card(
    p: &egui::Painter,
    rect: Rect,
    model: Model,
    pill_kind: Pill,
    installed: bool,
    // The card is shown but offers nothing: the slot is another window's.
    inert: bool,
    hover_t: f32,
    k: f32,
    pal: &Palette,
) {
    let card = &crate::app::ui::slate::for_model(model).card;
    let round = theme::ROUND * k;
    let pad = CARD_PAD * k;

    // A flat plate under a hairline. Under the pointer the plate lifts and the
    // edge darkens towards the ink; nothing grows and nothing glows.
    p.rect_filled(rect, round, lerp_color(pal.plate, pal.plate_hover, hover_t));
    p.rect_stroke(
        rect,
        round,
        Stroke::new(
            theme::HAIRLINE * k,
            lerp_color(pal.line, pal.ink, hover_t * CARD_HOVER_EDGE),
        ),
    );

    // The deck's own colour, across the head of its card.
    let cap = Rect::from_min_size(rect.min, Vec2::new(rect.width(), CARD_CAP_H * k));
    p.rect_filled(
        cap,
        Rounding {
            nw: round,
            ne: round,
            sw: 0.0,
            se: 0.0,
        },
        card.accent,
    );

    if installed {
        let font = FontId::proportional(CHIP_FONT * k);
        let w = text_width(p, "INSTALLED", &font, CHIP_TRACK * k) + 2.0 * CHIP_PAD_X * k;
        let chip = Rect::from_min_size(
            Pos2::new(rect.right() - pad - w, cap.bottom() + CHIP_TOP * k),
            Vec2::new(w, CHIP_H * k),
        );
        p.rect_stroke(
            chip,
            theme::ROUND_CHIP * k,
            Stroke::new(theme::HAIRLINE * k, pal.line_strong),
        );
        tracked(
            p,
            Pos2::new(chip.left() + CHIP_PAD_X * k, chip.center().y),
            "INSTALLED",
            &font,
            pal.muted,
            CHIP_TRACK * k,
        );
    }

    // Everything below the miniature is measured up from the card's foot, so
    // the two cards agree line for line whatever the window's size factor is.
    let pill = Rect::from_min_max(
        Pos2::new(rect.left() + pad, rect.bottom() - pad - theme::PILL_H * k),
        Pos2::new(rect.right() - pad, rect.bottom() - pad),
    );
    let cx = rect.center().x;
    let era_y = pill.top() - CARD_ERA_GAP * k;
    let sub_y = era_y - CARD_SUB_GAP * k;
    let title_y = sub_y - CARD_TITLE_GAP * k;

    tracked_center(
        p,
        Pos2::new(cx, era_y),
        card.era,
        &FontId::proportional(CARD_ERA_FONT * k),
        pal.faint,
        CARD_ERA_TRACK * k,
    );
    p.text(
        Pos2::new(cx, sub_y),
        Align2::CENTER_CENTER,
        card.subtitle,
        FontId::proportional(CARD_SUB_FONT * k),
        pal.muted,
    );
    p.text(
        Pos2::new(cx, title_y),
        Align2::CENTER_CENTER,
        model.title(),
        FontId::proportional(CARD_TITLE_FONT * k),
        pal.ink,
    );

    // The miniature stands on whatever the chip above and the title below
    // leave it.
    draw_deck_glyph(
        p,
        Rect::from_min_max(
            Pos2::new(
                rect.left() + pad,
                cap.bottom() + (CHIP_TOP + CHIP_H + CHIP_GLYPH_GAP) * k,
            ),
            Pos2::new(
                rect.right() - pad,
                title_y - CARD_TITLE_FONT * 0.5 * k - CARD_GLYPH_GAP * k,
            ),
        ),
        card,
        pal,
        lerp_color(pal.plate, pal.plate_hover, hover_t),
        k,
    );

    // Foot pill: what clicking the card does to the slot, and how much it
    // costs. Nothing at risk stays neutral until the pointer is on it; a run
    // that has to stop first is amber; an installation that goes is red. The
    // deck accents stay on the cap - they say which deck, never what the
    // button does.
    let (rest_fill, rest_edge, rest_text, hot) = if inert {
        (pal.off, pal.off_line, pal.off_text, pal.off)
    } else {
        match pill_kind {
            Pill::Launch => (pal.ink, pal.ink, pal.on_ink, pal.ink_hover),
            Pill::Install => (pal.plate, pal.line_strong, pal.ink, pal.ink),
            Pill::Reinstall => (pal.plate, pal.warn, pal.warn, pal.warn),
            Pill::Replace => (pal.plate, pal.danger, pal.danger, pal.danger),
        }
    };
    let fill = lerp_color(rest_fill, hot, hover_t);
    let edge = lerp_color(rest_edge, hot, hover_t);
    // The one already filled keeps its text; the outlines flip as they fill.
    let text = match pill_kind {
        _ if inert => rest_text,
        Pill::Launch => rest_text,
        _ => lerp_color(rest_text, pal.on_ink, hover_t),
    };
    p.rect_filled(pill, round, fill);
    p.rect_stroke(pill, round, Stroke::new(theme::HAIRLINE * k, edge));
    tracked_center(
        p,
        pill.center(),
        pill_kind.label(),
        &FontId::proportional(PILL_FONT * k),
        text,
        PILL_TRACK * k,
    );
}

/// Emptying the slot: the same plate the replacement shows, for a step that
/// puts nothing in its place.
pub(in crate::app) fn draw_delete_confirm(
    ui: &mut egui::Ui,
    view: &PickerView<'_>,
) -> Option<PickerAction> {
    let k = k_of(ui.max_rect());
    let pal = theme::palette(ui.ctx());
    let held = view.installed.map(|m| m.title()).unwrap_or("nothing");
    let title = format!("Delete the emulation in slot {}?", view.slot);
    let sub = if view.running.is_some() {
        "The emulation stops first - QEMU holds the eMMC open while it runs - and the slot goes \
         back to empty. Nothing is installed in its place."
    } else {
        "The slot goes back to empty. Nothing is installed in its place."
    };
    let head = Header {
        slot: view.slot,
        model: view.installed,
        release: view.release,
        state: None,
        title: &title,
        sub,
        switchable: false,
    };
    let body = draw_header(ui, &head).body;
    let (space, bar) = footer_split(body, k);

    let w = (CONFIRM_W * k).min(space.width() - 2.0 * theme::GUTTER * k);
    let plate_h = (LOSS_HEAD_H + 3.0 * LOSS_ROW_H) * k;
    let plate = Rect::from_min_size(
        Pos2::new(space.center().x - w * 0.5, space.center().y - plate_h * 0.5),
        Vec2::new(w, plate_h),
    );
    let p = ui.painter();
    let round = theme::ROUND * k;
    p.rect_filled(plate, round, pal.plate);
    p.rect_stroke(plate, round, Stroke::new(theme::HAIRLINE * k, pal.line));
    let band = Rect::from_min_size(plate.min, Vec2::new(w, LOSS_HEAD_H * k));
    p.rect_filled(
        band,
        Rounding {
            nw: round,
            ne: round,
            sw: 0.0,
            se: 0.0,
        },
        pal.field,
    );
    hairline(p, band.left_bottom(), band.right_bottom(), pal.line, k);
    tracked(
        p,
        Pos2::new(band.left() + LOSS_PAD * k, band.center().y),
        "DELETED, AND NOT RECOVERABLE",
        &FontId::proportional(theme::MICRO_FONT * k),
        pal.danger,
        theme::MICRO_TRACK * k,
    );
    let rows = [
        format!("The {held} firmware in this slot"),
        "Its eMMC image".to_owned(),
        "Everything stored on it - settings, library, cues".to_owned(),
    ];
    for (i, row) in rows.iter().enumerate() {
        let ry = band.bottom() + (i as f32 + 0.5) * LOSS_ROW_H * k;
        let dx = plate.left() + LOSS_PAD * k;
        p.line_segment(
            [Pos2::new(dx, ry), Pos2::new(dx + LOSS_DASH_W * k, ry)],
            Stroke::new(theme::HAIRLINE * k, pal.line_strong),
        );
        p.text(
            Pos2::new(dx + (LOSS_DASH_W + LOSS_DASH_GAP) * k, ry),
            Align2::LEFT_CENTER,
            row,
            FontId::proportional(LOSS_FONT * k),
            pal.ink,
        );
    }

    let inner = draw_footer(ui, bar, pal, k);
    let mut action = None;
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
        theme::apply_setup_style(ui, pal);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = format!("Delete slot {}", view.slot);
            let wide = [
                text_width(
                    ui.painter(),
                    &label,
                    &FontId::proportional(theme::BUTTON_FONT * k),
                    0.0,
                ) + CONFIRM_BTN_PAD * k,
                theme::BUTTON_SIZE[1] * k,
            ];
            if theme::destructive(ui, wide, &label, pal).clicked() {
                action = Some(PickerAction::ConfirmDelete);
            }
            if theme::secondary(ui, button_size(k), "Keep it", pal).clicked() {
                action = Some(PickerAction::CancelDelete);
            }
        });
    });
    action
}

// ── The announced deck ────────────────────────────────────────────────────────
// Measured off AlphaTheta's own CDJ-1500X outline drawing. The cabinet is a
// plain rectangle - no riser block, the screen assembly is the whole of the
// top - with the tempo fader down the right and CUE and PLAY down the left,
// where the CDJ-3000 has neither on its miniature. Its true size against the
// other decks is not known, so it is drawn at their height and its own
// measured width; that lands properly once a panel is measured.
/// Its canvas, in the same reference units as the other two, so it is drawn
/// at the same one-unit-per-pixel and comes out its own size beside them.
/// Taken off AlphaTheta's own size overlay, where a dashed CDJ-3000X stands
/// behind the CDJ-1500X on the same foot line: the 1500X measures 0.863 of
/// the CDJ-3000X's height and 0.822 of its width, which is about 283 x 423 mm against
/// the CDJ-3000X's 344.6 x 490.4. The aspect that falls out, 0.673, is the spec
/// outline's own to within 1.3 %.
const SOON_CANVAS: (f32, f32) = (2706.0, 4020.0);
const SOON_SCREEN_FRAME: (f32, f32, f32, f32) = (0.012, 0.012, 0.988, 0.455);
/// The glass, not the frame around it. Sized off the CDJ-3000X's, which the
/// vector drawing puts at 217.0 x 134.2 mm - the 10.1-inch panel both decks
/// carry - rather than read off this drawing, where the nested frames are a
/// pixel apart at the size it was published.
const SOON_SCREEN: (f32, f32, f32, f32) = (0.1166, 0.0749, 0.8834, 0.3921);
const SOON_PADS: (f32, f32, f32, f32) = (0.172, 0.478, 0.833, 0.505);
/// Share of each key's pitch left as the gap, read off the same drawing.
const SOON_PAD_GAP: f32 = 0.34;
/// Jog: centre as fractions of the outline, radius over its width.
const SOON_JOG: (f32, f32, f32) = (0.495, 0.755, 0.305);
const SOON_JOG_HUB: f32 = 0.120;
/// The browse rotary, which stands alone where the CDJ-3000 has a pod.
const SOON_BROWSE: (f32, f32, f32) = (0.912, 0.570, 0.037);
const SOON_LOOP: (f32, f32, f32) = (0.086, 0.570, 0.040);
/// Tempo fader, down the right.
const SOON_TEMPO: (f32, f32, f32, f32) = (0.855, 0.635, 0.955, 0.955);
/// CUE and PLAY, down the left. Same radius, one above the other.
const SOON_TRANSPORT_X: f32 = 0.100;
const SOON_TRANSPORT_R: f32 = 0.053;
const SOON_TRANSPORT_Y: (f32, f32) = (0.845, 0.945);

/// A deck the picker names but cannot offer: the same plate, gone quiet. It
/// takes no pointer - there is nothing behind it to click.
fn draw_soon_card(p: &egui::Painter, rect: Rect, name: &str, k: f32, pal: &Palette) {
    let round = theme::ROUND * k;
    let pad = CARD_PAD * k;

    p.rect_filled(rect, round, pal.off);
    p.rect_stroke(rect, round, Stroke::new(theme::HAIRLINE * k, pal.line));

    let cap = Rect::from_min_size(rect.min, Vec2::new(rect.width(), CARD_CAP_H * k));
    p.rect_filled(
        cap,
        Rounding {
            nw: round,
            ne: round,
            sw: 0.0,
            se: 0.0,
        },
        pal.off_line,
    );

    let font = FontId::proportional(CHIP_FONT * k);
    let w = text_width(p, SOON_CHIP, &font, CHIP_TRACK * k) + 2.0 * CHIP_PAD_X * k;
    let chip = Rect::from_min_size(
        Pos2::new(rect.right() - pad - w, cap.bottom() + CHIP_TOP * k),
        Vec2::new(w, CHIP_H * k),
    );
    p.rect_stroke(
        chip,
        theme::ROUND_CHIP * k,
        Stroke::new(theme::HAIRLINE * k, pal.off_line),
    );
    tracked(
        p,
        Pos2::new(chip.left() + CHIP_PAD_X * k, chip.center().y),
        SOON_CHIP,
        &font,
        pal.off_text,
        CHIP_TRACK * k,
    );

    // The same lines as the cards beside it, measured up from the same foot.
    let pill = Rect::from_min_max(
        Pos2::new(rect.left() + pad, rect.bottom() - pad - theme::PILL_H * k),
        Pos2::new(rect.right() - pad, rect.bottom() - pad),
    );
    let cx = rect.center().x;
    let sub_y = pill.top() - CARD_ERA_GAP * k - CARD_SUB_GAP * k;
    let title_y = sub_y - CARD_TITLE_GAP * k;
    p.text(
        Pos2::new(cx, sub_y),
        Align2::CENTER_CENTER,
        SOON_SUB,
        FontId::proportional(CARD_SUB_FONT * k),
        pal.off_text,
    );
    p.text(
        Pos2::new(cx, title_y),
        Align2::CENTER_CENTER,
        name,
        FontId::proportional(CARD_TITLE_FONT * k),
        pal.off_text,
    );

    draw_soon_glyph(
        p,
        Rect::from_min_max(
            Pos2::new(
                rect.left() + pad,
                cap.bottom() + (CHIP_TOP + CHIP_H + CHIP_GLYPH_GAP) * k,
            ),
            Pos2::new(
                rect.right() - pad,
                title_y - CARD_TITLE_FONT * 0.5 * k - CARD_GLYPH_GAP * k,
            ),
        ),
        k,
        pal,
    );

    p.rect_stroke(pill, round, Stroke::new(theme::HAIRLINE * k, pal.off_line));
    tracked_center(
        p,
        pill.center(),
        SOON_PILL,
        &FontId::proportional(PILL_FONT * k),
        pal.off_text,
        PILL_TRACK * k,
    );
}

/// The announced deck's miniature, standing on the same foot as its
/// neighbours and drawn in the plate's own quiet greys.
fn draw_soon_glyph(p: &egui::Painter, area: Rect, k: f32, pal: &Palette) {
    // The other two are drawn at one unit-per-pixel against a 4659-unit
    // canvas; this one has no measured canvas, so it takes the area it is
    // given and its own aspect.
    // The same units-per-pixel the other two are drawn at: the area holds the
    // tallest cabinet, and this one takes the share its canvas asks for.
    let u = area.height() / (GLYPH_CANVAS_H - GLYPH_FOOT_TRIM_REF);
    let (w, h) = (SOON_CANVAS.0 * u, SOON_CANVAS.1 * u);
    let outline = Rect::from_min_size(
        Pos2::new(area.center().x - w * 0.5, area.bottom() - h),
        Vec2::new(w, h),
    );
    let at = |x: f32, y: f32| {
        Pos2::new(
            outline.left() + x * outline.width(),
            outline.top() + y * outline.height(),
        )
    };
    let place = |r: (f32, f32, f32, f32)| Rect::from_min_max(at(r.0, r.1), at(r.2, r.3));

    let col = pal.off_text;
    let stroke = Stroke::new(GLYPH_STROKE * k, col);
    let detail = Stroke::new(GLYPH_DETAIL_STROKE * k, pal.off_line);

    p.rect_stroke(outline, GLYPH_CHASSIS_ROUND_REF * k * 0.05, stroke);
    p.rect_stroke(place(SOON_SCREEN_FRAME), GLYPH_FRAME_ROUND * k, detail);
    p.rect_filled(place(SOON_SCREEN), GLYPH_SCREEN_ROUND * k, pal.off_line);
    button_row(
        p,
        place(SOON_PADS),
        GLYPH_PAD_COUNT,
        SOON_PAD_GAP,
        pal.off_line,
        k,
    );
    p.rect_stroke(place(SOON_TEMPO), GLYPH_FRAME_ROUND * k, detail);

    let (jx, jy, jr) = SOON_JOG;
    let jog_c = at(jx, jy);
    p.circle_stroke(jog_c, jr * outline.width(), stroke);
    p.circle_filled(jog_c, SOON_JOG_HUB * outline.width(), pal.off_line);
    for (x, y, r) in [SOON_BROWSE, SOON_LOOP] {
        p.circle_stroke(at(x, y), r * outline.width(), detail);
    }
    for y in [SOON_TRANSPORT_Y.0, SOON_TRANSPORT_Y.1] {
        p.circle_stroke(
            at(SOON_TRANSPORT_X, y),
            SOON_TRANSPORT_R * outline.width(),
            detail,
        );
    }
}

/// Draw the model's miniature into `area`, standing on its foot.
fn draw_deck_glyph(
    p: &egui::Painter,
    area: Rect,
    card: &Card,
    pal: &Palette,
    fill: Color32,
    k: f32,
) {
    let g = &card.glyph;
    // Reference units → glyph pixels, shared by both models, so a part given
    // in reference units comes out the same size on either card and the taller
    // cabinet is the one that fills `area`.
    let u = area.height() / (GLYPH_CANVAS_H - GLYPH_FOOT_TRIM_REF);
    // Vertical fractions below are of the whole reference canvas, not of the
    // outline, so trimming the foot leaves everything above it where it was.
    let vh = g.canvas.1 * u;
    let foot = area.bottom();
    let outline = Rect::from_min_max(
        Pos2::new(
            area.center().x - g.canvas.0 * u * 0.5,
            foot - vh + GLYPH_FOOT_TRIM_REF * u,
        ),
        Pos2::new(area.center().x + g.canvas.0 * u * 0.5, foot),
    );
    let at =
        |x: f32, y: f32| Pos2::new(outline.left() + x * outline.width(), outline.top() + y * vh);
    let place = |r: Rect| Rect::from_min_max(at(r.left(), r.top()), at(r.right(), r.bottom()));

    // The cabinet and the jog carry the weight; everything else is a shade
    // finer. The deck's colour is on the card's cap, not in here.
    let col = pal.glyph;
    let stroke = Stroke::new(GLYPH_STROKE * k, col);
    let detail = Stroke::new(GLYPH_DETAIL_STROKE * k, pal.glyph_detail);

    // The cabinet: the body, with the screen's block standing above it. The
    // two corners at the block's foot turn inward and are left sharp, as the
    // chassis draws them.
    let body_top = outline.top() + g.body_top * vh;
    let (riser_l, riser_r) = (
        outline.left() + g.riser.0 * outline.width(),
        outline.left() + g.riser.1 * outline.width(),
    );
    let r = GLYPH_CHASSIS_ROUND_REF * u;
    p.add(Shape::closed_line(
        rounded_polygon(&[
            (Pos2::new(outline.left(), body_top), r),
            (Pos2::new(riser_l, body_top), 0.0),
            (Pos2::new(riser_l, outline.top()), r),
            (Pos2::new(riser_r, outline.top()), r),
            (Pos2::new(riser_r, body_top), 0.0),
            (Pos2::new(outline.right(), body_top), r),
            (outline.right_bottom(), r),
            (outline.left_bottom(), r),
        ]),
        stroke,
    ));

    // The screen, and the frame it sits in.
    let screen = place(g.screen);
    let frame = screen.expand2(Vec2::new(g.bezel.x * outline.width(), g.bezel.y * vh));
    p.rect_stroke(frame, GLYPH_FRAME_ROUND * k, detail);
    if g.recessed {
        for (wall, floor) in [
            (frame.left_top(), screen.left_top()),
            (frame.right_top(), screen.right_top()),
            (frame.right_bottom(), screen.right_bottom()),
            (frame.left_bottom(), screen.left_bottom()),
        ] {
            p.line_segment([wall, floor], detail);
        }
    }
    p.rect_filled(screen, GLYPH_SCREEN_ROUND * k, col);

    // Hot cues, filled so a row this small still reads as eight of them, in
    // the block the panel puts them in.
    button_row(
        p,
        Rect::from_min_max(at(g.pads_x.0, g.pads_y.0), at(g.pads_x.1, g.pads_y.1)),
        GLYPH_PAD_COUNT,
        g.pad_gap,
        col,
        k,
    );

    // What the two columns carry: a pair of keys at the head of the left one,
    // CUE and PLAY at its foot, and the tempo fader down the right. These are
    // what tell a CDJ-3000 from the CDJ-1500X beside it; the two 3000s carry
    // them alike, which is the point.
    let tcol = place(g.transport_col);
    let mcol = place(g.modes_col);
    let key_r = KEY_D * tcol.width() * 0.5;
    for cy in [KEY_CY.0, KEY_CY.1] {
        p.circle_stroke(
            Pos2::new(
                tcol.left() + KEY_CX * tcol.width(),
                tcol.top() + cy * tcol.height(),
            ),
            key_r,
            detail,
        );
    }
    // The media bay, laid out in the column rather than traced into the
    // corner the panel puts it in.
    let cell = |r: Rect| {
        Rect::from_min_max(
            Pos2::new(
                tcol.left() + r.left() * tcol.width(),
                tcol.top() + r.top() * tcol.height(),
            ),
            Pos2::new(
                tcol.left() + r.right() * tcol.width(),
                tcol.top() + r.bottom() * tcol.height(),
            ),
        )
    };
    match g.bay {
        Bay::UsbAndSd => {
            p.rect_stroke(cell(BAY_USB), GLYPH_FRAME_ROUND * k, detail);
            p.circle_stroke(
                Pos2::new(
                    tcol.left() + BAY_STOP.0 * tcol.width(),
                    tcol.top() + BAY_STOP.1 * tcol.height(),
                ),
                BAY_STOP.2 * tcol.width(),
                detail,
            );
            p.rect_stroke(cell(BAY_SD), GLYPH_FRAME_ROUND * k, detail);
        }
        Bay::TwinUsb => {
            for (slot, stop) in BAY_SLOTS.iter().zip(BAY_STOPS.iter()) {
                p.rect_stroke(cell(*slot), GLYPH_FRAME_ROUND * k, detail);
                // Filled: a bar this thin closes up as an outline.
                let bar = cell(*stop);
                p.rect_filled(bar, bar.height() * 0.5, detail.color);
            }
        }
    }
    p.rect_stroke(
        Rect::from_min_max(
            Pos2::new(
                mcol.left() + FADER_X.0 * mcol.width(),
                mcol.top() + FADER_Y.0 * mcol.height(),
            ),
            Pos2::new(
                mcol.left() + FADER_X.1 * mcol.width(),
                mcol.top() + FADER_Y.1 * mcol.height(),
            ),
        ),
        GLYPH_FRAME_ROUND * k,
        detail,
    );

    let (jx, jy, jr) = g.jog;
    let jog_c = at(jx, jy);
    let jog_r = jr * outline.width();
    p.circle_stroke(jog_c, jog_r, stroke);
    p.circle_filled(jog_c, jog_r * g.jog_hub, col);

    // Browse pod: two arc buttons above the rotary and two below, their inner
    // edge the bezel ring itself, which is why the ring shows between them at
    // the sides - wide of the buttons on the CDJ-3000, flush on the CDJ-3000X.
    let pod_c = at(g.pod.x, g.pod.y);
    let (hw, ring_r) = (g.pod_half_w * u, g.pod_ring_r * u);
    let half_h = GLYPH_POD_H_REF * 0.5 * u;
    let round = GLYPH_POD_ROUND_REF * u;
    if hw < ring_r {
        // The ring stands wider than the buttons, so each pair's inner edge is
        // the ring itself and no cut face crosses it.
        let dy = (ring_r * ring_r - hw * hw).sqrt();
        let a = dy.atan2(hw);
        let pi = std::f32::consts::PI;
        for (edge, join, arc) in [
            (pod_c.y - half_h, pod_c.y - dy, (-a, a - pi)),
            (pod_c.y + half_h, pod_c.y + dy, (a, pi - a)),
        ] {
            let mut pts = rounded_polygon(&[
                (Pos2::new(pod_c.x - hw, join), 0.0),
                (Pos2::new(pod_c.x - hw, edge), round),
                (Pos2::new(pod_c.x + hw, edge), round),
                (Pos2::new(pod_c.x + hw, join), 0.0),
            ]);
            for i in 0..=GLYPH_POD_ARC_SEGS {
                let t = i as f32 / GLYPH_POD_ARC_SEGS as f32;
                pts.push(pod_c + Vec2::angled(arc.0 + (arc.1 - arc.0) * t) * ring_r);
            }
            p.add(Shape::closed_line(pts, detail));
            p.line_segment([Pos2::new(pod_c.x, edge), Pos2::new(pod_c.x, join)], detail);
        }
    } else {
        // The ring fits inside the block: one plain outline, and the seam runs
        // from each end down to the ring.
        let pod = Rect::from_center_size(pod_c, Vec2::new(2.0 * hw, 2.0 * half_h));
        p.rect_stroke(pod, round, detail);
        for (y0, y1) in [
            (pod.top(), pod_c.y - ring_r),
            (pod_c.y + ring_r, pod.bottom()),
        ] {
            p.line_segment([Pos2::new(pod_c.x, y0), Pos2::new(pod_c.x, y1)], detail);
        }
    }
    // Filled first: the button block's own edges run in behind the ring, and
    // the rotary stands on top of them.
    p.circle_filled(pod_c, ring_r, fill);
    p.circle_stroke(pod_c, ring_r, detail);
    p.circle_filled(pod_c, GLYPH_POD_HUB_R_REF * u, col);
}

/// The hot-cue strip: `count` keys filling `rect`, filled because an outline
/// this small would close up.
fn button_row(p: &egui::Painter, rect: Rect, count: usize, gap: f32, col: Color32, k: f32) {
    let pitch = rect.width() / count as f32;
    let w = pitch * (1.0 - gap);
    for i in 0..count {
        p.rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.left() + i as f32 * pitch, rect.top()),
                Vec2::new(w, rect.height()),
            ),
            GLYPH_SCREEN_ROUND * k * 0.5,
            col,
        );
    }
}

/// Round each corner of `pts` by its own radius - a quadratic Bezier through
/// the corner, which at this size is indistinguishable from a fillet and needs
/// no centres. A radius of zero leaves the corner sharp.
fn rounded_polygon(pts: &[(Pos2, f32)]) -> Vec<Pos2> {
    let n = pts.len();
    let mut out = Vec::with_capacity(n * (GLYPH_CORNER_SEGS + 1));
    for i in 0..n {
        let (v, r) = pts[i];
        if r <= 0.0 {
            out.push(v);
            continue;
        }
        let (prev, next) = (pts[(i + n - 1) % n].0, pts[(i + 1) % n].0);
        let (to_prev, to_next) = (prev - v, next - v);
        let d = r.min(to_prev.length() * 0.5).min(to_next.length() * 0.5);
        let a = v + to_prev.normalized() * d;
        let b = v + to_next.normalized() * d;
        for seg in 0..=GLYPH_CORNER_SEGS {
            let t = seg as f32 / GLYPH_CORNER_SEGS as f32;
            let u = 1.0 - t;
            out.push(Pos2::new(
                u * u * a.x + 2.0 * u * t * v.x + t * t * b.x,
                u * u * a.y + 2.0 * u * t * v.y + t * t * b.y,
            ));
        }
    }
    out
}
/// Draw `text` with `track` extra pixels between glyphs (egui's painter has no
/// letter-spacing), left-aligned on `pos` and centred on its y. Returns the
/// width it took.
pub(in crate::app) fn tracked(
    p: &egui::Painter,
    pos: Pos2,
    text: &str,
    font: &FontId,
    color: Color32,
    track: f32,
) -> f32 {
    let mut x = pos.x;
    for c in text.chars() {
        p.text(
            Pos2::new(x, pos.y),
            Align2::LEFT_CENTER,
            c,
            font.clone(),
            color,
        );
        x += glyph_width(p, c, font) + track;
    }
    (x - pos.x - track).max(0.0)
}

/// The same, centred on `pos`.
pub(in crate::app) fn tracked_center(
    p: &egui::Painter,
    pos: Pos2,
    text: &str,
    font: &FontId,
    color: Color32,
    track: f32,
) {
    let w = text_width(p, text, font, track);
    tracked(
        p,
        Pos2::new(pos.x - w * 0.5, pos.y),
        text,
        font,
        color,
        track,
    );
}

/// What [`tracked`] takes for `text`.
pub(in crate::app) fn text_width(p: &egui::Painter, text: &str, font: &FontId, track: f32) -> f32 {
    let n = text.chars().count();
    if n == 0 {
        return 0.0;
    }
    text.chars().map(|c| glyph_width(p, c, font)).sum::<f32>() + track * (n - 1) as f32
}

fn glyph_width(p: &egui::Painter, c: char, font: &FontId) -> f32 {
    p.ctx().fonts(|f| {
        f.layout_no_wrap(c.to_string(), font.clone(), Color32::WHITE)
            .size()
            .x
    })
}

/// A rule one pixel thick, whatever the window's size factor.
fn hairline(p: &egui::Painter, a: Pos2, b: Pos2, col: Color32, k: f32) {
    p.line_segment([a, b], Stroke::new(theme::HAIRLINE * k, col));
}

/// The upright rule in the action bar, between the window's own action and
/// the slot's. It takes its own place in the row rather than being painted
/// into the gap, so the spacing either side of it is the bar's.
fn footer_rule(ui: &mut egui::Ui, pal: &Palette, k: f32) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(theme::HAIRLINE * k, FOOTER_RULE_H * k),
        Sense::hover(),
    );
    let x = rect.center().x;
    hairline(
        ui.painter(),
        Pos2::new(x, rect.top()),
        Pos2::new(x, rect.bottom()),
        pal.line,
        k,
    );
}

/// The bin beside the delete's label, drawn to the window's hairline.
fn trash_glyph(p: &egui::Painter, rect: Rect, col: Color32, k: f32) {
    let x = |t: f32| rect.left() + rect.width() * t;
    let y = |t: f32| rect.top() + rect.height() * t;
    let stroke = Stroke::new(theme::HAIRLINE * k, col);
    let line = |a: Pos2, b: Pos2| p.line_segment([a, b], stroke);

    line(
        Pos2::new(x(TRASH_LID_X.0), y(TRASH_LID_Y)),
        Pos2::new(x(TRASH_LID_X.1), y(TRASH_LID_Y)),
    );
    line(
        Pos2::new(x(TRASH_HANDLE_X.0), y(TRASH_LID_Y)),
        Pos2::new(x(TRASH_HANDLE_X.0), y(TRASH_HANDLE_Y)),
    );
    line(
        Pos2::new(x(TRASH_HANDLE_X.0), y(TRASH_HANDLE_Y)),
        Pos2::new(x(TRASH_HANDLE_X.1), y(TRASH_HANDLE_Y)),
    );
    line(
        Pos2::new(x(TRASH_HANDLE_X.1), y(TRASH_HANDLE_Y)),
        Pos2::new(x(TRASH_HANDLE_X.1), y(TRASH_LID_Y)),
    );
    line(
        Pos2::new(x(TRASH_BODY_TOP.0), y(TRASH_LID_Y)),
        Pos2::new(x(TRASH_BODY_BASE.0), y(TRASH_BODY_Y)),
    );
    line(
        Pos2::new(x(TRASH_BODY_TOP.1), y(TRASH_LID_Y)),
        Pos2::new(x(TRASH_BODY_BASE.1), y(TRASH_BODY_Y)),
    );
    line(
        Pos2::new(x(TRASH_BODY_BASE.0), y(TRASH_BODY_Y)),
        Pos2::new(x(TRASH_BODY_BASE.1), y(TRASH_BODY_Y)),
    );
}

/// The short upright rule between the identity strip's parts. Returns its
/// width, so the caller can go on laying the strip out left to right.
fn strip_rule(p: &egui::Painter, x: f32, cy: f32, col: Color32, k: f32) -> f32 {
    let h = STRIP_RULE_H * k * 0.5;
    p.line_segment(
        [Pos2::new(x, cy - h), Pos2::new(x, cy + h)],
        Stroke::new(theme::HAIRLINE * k, col),
    );
    theme::HAIRLINE * k
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| lerp(x as f32, y as f32, t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(
        m(a.r(), b.r()),
        m(a.g(), b.g()),
        m(a.b(), b.b()),
        m(a.a(), b.a()),
    )
}
