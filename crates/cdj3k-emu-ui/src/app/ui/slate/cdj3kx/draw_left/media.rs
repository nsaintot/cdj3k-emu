//! The USB slot stack at the head of the transport column.

use cdj3k_emu_panel::{mosi_frame, Btn, Lamp};
use egui::{Color32, FontFamily, Pos2, Rect, Stroke, Vec2};

use super::super::{
    draw_bordered_rect_section, ButtonType, CdjApp, UiScale, COL_BLACK, COL_BTN, COL_BTN_HOT,
    COL_BTN_TEXT, COL_DARK, COL_SILVER,
};
use super::PERF_TRANSPORT_COL_REF;
use crate::app::ui::{DoubleBorderSpec, StrokeSpec};

// ── USB 1 slot, at the top of the transport column ───────────────────────────
// The CDJ-3000X stacks the slot: the name, the rating, the casing, the light
// band, then a notch tab with "STOP" and a slim STOP button. USB 1 has a
// host-link computer glyph and no trident logo.
/// The centre line that the whole group shares.
pub(super) const PERF_U_USB_STOP_BTN: f32 = 0.331;
/// The button sits left of that centre line.
pub(super) const PERF_USB_STOP_BTN_OFF_X: f32 = -7.0;
/// V of each element within PERF_TRANSPORT_COL_REF.
pub(super) const PERF_V_USB_NAME: f32 = 0.014;
pub(super) const PERF_V_USB_RATING: f32 = 0.023;
pub(super) const PERF_V_USB_STOP_LABEL: f32 = 0.088;
pub(super) const PERF_V_USB_STOP_BTN: f32 = 0.098;

pub(super) const PERF_USB_NAME_FONT_SIZE: f32 = 36.0;
pub(super) const PERF_USB_RATING_FONT_SIZE: f32 = 24.0;
/// Drawn DC mark between "5 V" and "1 A": half-gap to the text, bar width,
/// stroke, and the half-separation of the solid bar from the broken one.
pub(super) const PERF_USB_DC_GAP: f32 = 21.0;
pub(super) const PERF_USB_DC_W: f32 = 20.0;
pub(super) const PERF_USB_DC_STROKE: f32 = 2.5;
pub(super) const PERF_USB_DC_SPLIT: f32 = 4.0;
pub(super) const PERF_USB_STOP_LABEL_FONT_SIZE: f32 = 30.0;

/// Placement of the dash drawn to the left of "STOP", which is
/// The notch tab is [`LEGEND_TAB_W`](super::super::LEGEND_TAB_W) wide.
pub(super) const PERF_USB_STOP_TAB_OFF_X: f32 = 54.0;
pub(super) const PERF_USB_STOP_LABEL_OFF_X: f32 = -34.0;

/// Host-link computer glyph, right of the USB 1 legend.
pub(super) const PERF_U_USB_LINK_GLYPH: f32 = 0.655;
pub(super) const PERF_V_USB_LINK_GLYPH: f32 = 0.021;
pub(super) const PERF_USB_LINK_GLYPH_W: f32 = 64.0;

/// STOP is a slim bar here, not the CDJ-3000's round eject button.
pub(super) const PERF_USB_STOP_BTN_W: f32 = 84.0;
pub(super) const PERF_USB_STOP_BTN_H: f32 = 20.0;
/// Border gap; the outer radius is this plus the corner rounding.
pub(super) const PERF_USB_STOP_BTN_ROUNDING: f32 = 1.0;
pub(super) const PERF_USB_STOP_BTN_INNER_STROKE: f32 = 1.5;
pub(super) const PERF_USB_STOP_BTN_OUTER_STROKE: f32 = 2.0;

/// Centre of the USB MOUNT casing.
pub(super) const PERF_U_USB_MOUNT: f32 = 0.331;
pub(super) const PERF_V_USB_MOUNT: f32 = 0.055;

/// The USB MOUNT casing (bezel) is `mount_rect`. Inside the casing, a
/// double-bordered rectangle forms the receptacle opening, and the tongue bar
/// sits inside that.
pub(super) const PERF_USB_MOUNT_W: f32 = 200.0;
pub(super) const PERF_USB_MOUNT_H: f32 = 200.0;
/// Only the casing's bottom corners are radiused; the top two stay square.
pub(super) const PERF_USB_MOUNT_BOTTOM_R: f32 = 12.0;
/// Height of the light band, which sits flush on the cavity floor and spans its
/// full width.
pub(super) const PERF_USB_STRIP_H: f32 = 13.0;
/// Casing, outside in: the panel's black, a thicker grey, then black.
pub(super) const PERF_USB_MOUNT_BORDER_INNER_STROKE: f32 = 3.0;
pub(super) const PERF_USB_MOUNT_BORDER_OUTER_STROKE: f32 = 6.0;
pub(super) const PERF_USB_MOUNT_BORDER_GAP: f32 = 2.0;
/// Vertical offset of the receptacle frame from the centre of the mount cavity
/// (ref units). The receptacle sits high in the casing, which leaves room for
/// the band below.
pub(super) const PERF_USB_CONNECTOR_V_OFF: f32 = -26.0;
/// Port housing: the opened trap around the receptacle. Its sides and its
/// top edge, which is the trap's handle, run to the casing's inner border; the bottom
/// edge sits the same distance below the port glyph as the handle sits above it.
pub(super) const PERF_USB_PORT_HOUSING_STROKE: f32 = 3.0;
pub(super) const PERF_USB_PORT_HOUSING_TOP_STROKE: f32 = 9.0;
pub(super) const PERF_USB_CONNECTOR_W: f32 = 104.0;
pub(super) const PERF_USB_CONNECTOR_H: f32 = 42.0;
pub(super) const PERF_USB_CONNECTOR_ROUNDING: f32 = 2.0;
pub(super) const PERF_USB_CONNECTOR_INNER_STROKE: f32 = 2.5;
pub(super) const PERF_USB_CONNECTOR_OUTER_STROKE: f32 = 5.0;
pub(super) const PERF_USB_CONNECTOR_BORDER_GAP: f32 = 3.0;
/// The tongue inside the receptacle opening: its width cap, its height and its
/// inset from the top of the cavity (ref units).
pub(super) const PERF_USB_MOUNT_TONGUE_W_FRAC: f32 = 0.88;
pub(super) const PERF_USB_MOUNT_TONGUE_H: f32 = 10.0;
pub(super) const PERF_USB_MOUNT_TONGUE_TOP_PAD: f32 = 6.0;
pub(super) const PERF_USB_MOUNT_TONGUE_ROUNDING: f32 = 2.0;
/// USB 1's open tray: its offsets from the casing centre, its width, its
/// handle dome and its stroke.
pub(super) const PERF_USB_TRAY_TOP_DY: f32 = -79.0;
pub(super) const PERF_USB_TRAY_BOT_DY: f32 = -67.0;
pub(super) const PERF_USB_TRAY_W: f32 = 182.0;
pub(super) const PERF_USB_TRAY_DOME_CHORD: f32 = 81.0;
pub(super) const PERF_USB_TRAY_DOME_H: f32 = 12.0;
pub(super) const PERF_USB_TRAY_STROKE: f32 = 2.0;
/// Type-C receptacle: a stadium on the USB-A receptacle's centre (outer size).
pub(super) const PERF_USB_C_W: f32 = 85.0;
pub(super) const PERF_USB_C_H: f32 = 26.0;
pub(super) const PERF_USB_C_STROKE: f32 = 3.0;

/// One slot of the USB stack. USB 2 is USB 1 moved down the column: same
/// casing, band, legends and STOP bar, with a Type-C receptacle.
pub(super) struct UsbSlot {
    /// Downward shift from USB 1, ref units.
    pub off_y: f32,
    pub name: &'static str,
    /// Current after the DC mark.
    pub amps: &'static str,
    /// DC mark offset from the slot's centre line; the rating is centred as a
    /// whole (ref units).
    pub dc_off_x: f32,
    pub type_c: bool,
    pub lamp: Lamp,
    pub stop: Btn,
    pub id: &'static str,
}

pub(super) const USB_1: UsbSlot = UsbSlot {
    off_y: 0.0,
    name: "USB 1",
    amps: "1 A",
    dc_off_x: 0.0,
    type_c: false,
    lamp: Lamp::Slot1,
    stop: Btn::UsbStop,
    id: "usb_stop",
};

pub(super) const USB_2: UsbSlot = UsbSlot {
    off_y: 462.0,
    name: "USB 2",
    amps: "1.5 A",
    dc_off_x: -11.8,
    type_c: true,
    lamp: Lamp::Slot2,
    stop: Btn::Usb2Stop,
    id: "usb2_stop",
};

/// Draws the casing, light band, port housing, receptacle and STOP bar of one
/// USB slot. The legends are drawn in statics.rs.
pub(super) fn draw_usb_slot(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layout: &UiScale,
    slot: &UsbSlot,
) {
    let shift = Vec2::new(0.0, layout.sc(slot.off_y));
    let (r, g, b) = app.mosi().rgb(slot.lamp).unwrap_or_default();
    let lit = app.mosi().led_color(mosi_frame::LedPart::Slot, r, g, b);
    let usb_drive = mosi_frame::led_drive_factor(r, g, b).unwrap_or(0.0);

    let mount_center =
        layout.sp_in_rect(PERF_TRANSPORT_COL_REF, PERF_U_USB_MOUNT, PERF_V_USB_MOUNT) + shift;
    let mount_size = Vec2::new(layout.sc(PERF_USB_MOUNT_W), layout.sc(PERF_USB_MOUNT_H));
    let mount_rect = Rect::from_center_size(mount_center, mount_size);

    paint_usb_casing(p, layout, mount_rect, lit, usb_drive);

    // The receptacle opening has its own double border, separate from the
    // casing border.
    let connector_size = Vec2::new(
        layout.sc(PERF_USB_CONNECTOR_W),
        layout.sc(PERF_USB_CONNECTOR_H),
    );
    let connector_rect = Rect::from_center_size(
        mount_center + Vec2::new(0.0, layout.sc(PERF_USB_CONNECTOR_V_OFF)),
        connector_size,
    );
    paint_port_housing(p, layout, mount_rect, connector_rect);

    if !slot.type_c {
        crate::app::ui::draw_usb_tray(
            p,
            crate::app::ui::UsbTraySpec {
                centre_x: mount_center.x,
                top: mount_center.y + layout.sc(PERF_USB_TRAY_TOP_DY),
                bottom: mount_center.y + layout.sc(PERF_USB_TRAY_BOT_DY),
                half_w: layout.sc(PERF_USB_TRAY_W) * 0.5,
                dome_chord: layout.sc(PERF_USB_TRAY_DOME_CHORD),
                dome_h: layout.sc(PERF_USB_TRAY_DOME_H),
                stroke: layout.sc(PERF_USB_TRAY_STROKE),
            },
        );
    }

    if slot.type_c {
        paint_usb_c(p, layout, connector_rect);
    } else {
        paint_usb_a(p, layout, connector_rect);
        // Reserve the mount's hit area; no click handler uses it yet.
        let _resp = ui.interact(
            mount_rect,
            ui.id().with("usb_mount_open"),
            egui::Sense::click(),
        );
    }
    draw_usb_stop(app, ui, layout, slot, shift);
}

/// Paints the port housing under the receptacle. Three of its edges run to
/// the casing's inner border, so the trap and the casing read as one piece.
/// Strokes are centred on their edge, so each edge is placed half its own
/// width outside the face it presents.
fn paint_port_housing(p: &egui::Painter, layout: &UiScale, mount_rect: Rect, connector_rect: Rect) {
    let inset = layout.sc(PERF_USB_MOUNT_BORDER_INNER_STROKE);
    let side_w = layout.sc(PERF_USB_PORT_HOUSING_STROKE);
    let top_w = layout.sc(PERF_USB_PORT_HOUSING_TOP_STROKE);
    let top_y = mount_rect.top() + inset + top_w * 0.5;
    let clearance = connector_rect.top() - (top_y + top_w * 0.5);
    let housing = Rect::from_min_max(
        Pos2::new(mount_rect.left() + inset, top_y),
        Pos2::new(
            mount_rect.right() - inset,
            connector_rect.bottom() + clearance + side_w * 0.5,
        ),
    );
    p.rect_stroke(
        housing,
        0.0,
        Stroke::new(side_w, COL_BTN_HOT),
        egui::StrokeKind::Middle,
    );
    p.line_segment(
        [housing.left_top(), housing.right_top()],
        Stroke::new(top_w, COL_BTN_HOT),
    );
}

fn paint_usb_casing(
    p: &egui::Painter,
    layout: &UiScale,
    mount_rect: Rect,
    lit: Option<Color32>,
    usb_drive: f32,
) {
    // Casing, outside in: black, a thicker grey, black, then the cavity.
    // The outermost black is the panel the casing sits on; the grey and the
    // inner black are the spec's two strokes. Only the bottom corners are
    // radiused, so these are painted with a per-corner rounding instead of
    // draw_bordered_rect_section's single value.
    let mount_border = DoubleBorderSpec::from_strokes_with_gap(
        StrokeSpec {
            width: layout.sc(PERF_USB_MOUNT_BORDER_INNER_STROKE),
            color: COL_BLACK,
        },
        StrokeSpec {
            width: layout.sc(PERF_USB_MOUNT_BORDER_OUTER_STROKE),
            color: COL_SILVER,
        },
        layout.sc(PERF_USB_MOUNT_BORDER_GAP),
    );
    let br = layout.sc(PERF_USB_MOUNT_BOTTOM_R);
    let round = egui::epaint::CornerRadiusF32 {
        nw: 0.0,
        ne: 0.0,
        sw: br,
        se: br,
    };
    let round_outer = egui::epaint::CornerRadiusF32 {
        nw: 0.0,
        ne: 0.0,
        sw: br + mount_border.gap,
        se: br + mount_border.gap,
    };
    p.rect_filled(mount_rect, round, COL_DARK);
    p.rect_stroke(
        mount_rect.expand(mount_border.gap),
        round_outer,
        mount_border.outer.stroke(),
        egui::StrokeKind::Middle,
    );
    p.rect_stroke(
        mount_rect,
        round,
        mount_border.inner.stroke(),
        egui::StrokeKind::Middle,
    );

    // The light band sits flush on the cavity floor, across its full width.
    {
        let inset = layout.sc(PERF_USB_MOUNT_BORDER_INNER_STROKE);
        let band_h = layout.sc(PERF_USB_STRIP_H);
        let band = Rect::from_min_max(
            Pos2::new(
                mount_rect.left() + inset,
                mount_rect.bottom() - inset - band_h,
            ),
            Pos2::new(mount_rect.right() - inset, mount_rect.bottom() - inset),
        );
        // The band is dim while the slot is idle and lit while the lamp is on.
        let strip_col = lit
            .map(|c| c.gamma_multiply(usb_drive))
            .unwrap_or(COL_SILVER.gamma_multiply(0.30));
        p.rect_filled(
            band,
            egui::epaint::CornerRadiusF32 {
                nw: 0.0,
                ne: 0.0,
                sw: br * 0.55,
                se: br * 0.55,
            },
            strip_col,
        );
    }
}

/// Type-C: a stadium on the USB-A receptacle's centre.
fn paint_usb_c(p: &egui::Painter, layout: &UiScale, connector_rect: Rect) {
    let stroke_w = layout.sc(PERF_USB_C_STROKE);
    let pill = Rect::from_center_size(
        connector_rect.center(),
        Vec2::new(layout.sc(PERF_USB_C_W), layout.sc(PERF_USB_C_H)),
    )
    .shrink(stroke_w * 0.5);
    let pill_r = pill.height() * 0.5;
    p.rect_filled(pill, pill_r, COL_DARK);
    p.rect_stroke(
        pill,
        pill_r,
        Stroke::new(stroke_w, COL_SILVER),
        egui::StrokeKind::Middle,
    );
}

/// USB-A: the receptacle opening with its own double border, and the
/// tongue inside it.
fn paint_usb_a(p: &egui::Painter, layout: &UiScale, connector_rect: Rect) {
    let connector_r = layout.sc(PERF_USB_CONNECTOR_ROUNDING);
    let connector_border = DoubleBorderSpec::from_strokes_with_gap(
        StrokeSpec {
            width: layout.sc(PERF_USB_CONNECTOR_INNER_STROKE),
            color: COL_SILVER,
        },
        StrokeSpec {
            width: layout.sc(PERF_USB_CONNECTOR_OUTER_STROKE),
            color: COL_BTN_TEXT,
        },
        layout.sc(PERF_USB_CONNECTOR_BORDER_GAP),
    );
    draw_bordered_rect_section(
        p,
        connector_rect,
        Some(connector_r),
        Some(COL_DARK),
        connector_border,
    );

    // Draw the tongue inside the receptacle cavity, centred at the top.
    let cavity_inset = layout.sc(PERF_USB_CONNECTOR_INNER_STROKE) * 0.5 + layout.sc(3.0);
    let cavity = connector_rect.shrink(cavity_inset);
    let tongue_top = cavity.top() + layout.sc(PERF_USB_MOUNT_TONGUE_TOP_PAD);
    let max_tongue_h = (cavity.bottom() - tongue_top).max(1.0);
    let tongue_h = layout
        .sc(PERF_USB_MOUNT_TONGUE_H)
        .min(cavity.height() * 0.45)
        .min(max_tongue_h);
    let tongue_w = (cavity.width() * PERF_USB_MOUNT_TONGUE_W_FRAC).min(cavity.width());
    let tongue_rect = Rect::from_min_size(
        Pos2::new(cavity.center().x - 0.5 * tongue_w, tongue_top),
        Vec2::new(tongue_w, tongue_h),
    );
    let tongue_r = layout.sc(PERF_USB_MOUNT_TONGUE_ROUNDING);
    p.rect_filled(tongue_rect, tongue_r, COL_SILVER);
    p.rect_stroke(
        tongue_rect,
        tongue_r,
        Stroke::new(layout.sc(1.5), COL_DARK.gamma_multiply(0.65)),
        egui::StrokeKind::Middle,
    );
}

/// USB STOP: a slim bar under the slot's light band.
fn draw_usb_stop(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    layout: &UiScale,
    slot: &UsbSlot,
    shift: Vec2,
) {
    let usb_stop_center = layout.sp_in_rect(
        PERF_TRANSPORT_COL_REF,
        PERF_U_USB_STOP_BTN,
        PERF_V_USB_STOP_BTN,
    ) + Vec2::new(layout.sc(PERF_USB_STOP_BTN_OFF_X), 0.0)
        + shift;
    let usb_stop_rect = Rect::from_center_size(
        usb_stop_center,
        Vec2::new(
            layout.sc(PERF_USB_STOP_BTN_W),
            layout.sc(PERF_USB_STOP_BTN_H),
        ),
    );
    let usb_stop_border = DoubleBorderSpec::from_strokes_with_gap(
        StrokeSpec {
            width: layout.sc(PERF_USB_STOP_BTN_INNER_STROKE),
            color: COL_BTN,
        },
        StrokeSpec {
            width: layout.sc(PERF_USB_STOP_BTN_OUTER_STROKE),
            color: COL_SILVER,
        },
        layout.sc(PERF_USB_STOP_BTN_ROUNDING),
    );
    app.btn(
        ui,
        layout,
        ButtonType::Basic,
        usb_stop_rect,
        "",
        layout.sc(0.0),
        None,
        Some(COL_BTN),
        None,
        None,
        None,
        Some(usb_stop_border),
        FontFamily::Proportional,
        slot.id,
        slot.stop,
    );
}
