//! USB MOUNT casing, SD slot and USB STOP, at the head of the transport column.

use cdj3k_emu_panel::Btn;
use cdj3k_emu_panel::{mosi_frame, Lamp};
use egui::{Color32, Pos2, Rect, Stroke, Vec2};

use super::super::{
    draw_bordered_rect_section, CdjApp, UiScale, COL_BLACK, COL_BTN, COL_BTN_HOT, COL_BTN_TEXT,
    COL_DARK, COL_SILVER,
};
use super::PERF_TRANSPORT_COL_REF;
use crate::app::ui::{DoubleBorderSpec, StrokeSpec};

// ── USB STOP - top of transport column ────────────────────────────────────────
/// Horizontal center fraction within PERF_TRANSPORT_COL_REF for the whole group.
pub(super) const PERF_U_USB_STOP: f32 = 0.695;
/// Vertical fractions within PERF_TRANSPORT_COL_REF.
pub(super) const PERF_V_USB_STOP_PRE_LABEL: f32 = 0.03;
pub(super) const PERF_V_USB_STOP_ICON_SCREEN: f32 = 0.017;
pub(super) const PERF_V_USB_STOP_LABEL: f32 = 0.042;
pub(super) const PERF_V_USB_STOP_BTN: f32 = 0.070;

/// Small LED-style indicator dash above the laptop icon.
pub(super) const PERF_USB_STOP_LED_W: f32 = 28.0;
pub(super) const PERF_USB_STOP_LED_H: f32 = 7.0;

/// Laptop glyph, overall width at the stand's foot.
pub(super) const PERF_USB_STOP_ICON_W: f32 = 70.0;

pub(super) const PERF_USB_STOP_LABEL_FONT_SIZE: f32 = 34.0;

pub(super) const PERF_USB_STOP_BTN_R: f32 = 42.0;
pub(super) const PERF_USB_STOP_BTN_INNER_STROKE: f32 = 4.0;
pub(super) const PERF_USB_STOP_BTN_OUTER_STROKE: f32 = 8.0;

// ── USB sign and LED cavity, left of the USB STOP button ─────────────────────
/// Center of the USB sign (logo), in fractions of PERF_TRANSPORT_COL_REF.
pub(super) const PERF_U_USB_SIGN: f32 = 0.33;
pub(super) const PERF_V_USB_SIGN: f32 = 0.0795;
/// Center of the USB MOUNT glow rectangle.
pub(super) const PERF_U_USB_MOUNT: f32 = 0.322;
pub(super) const PERF_V_USB_MOUNT: f32 = 0.040;

/// The USB MOUNT: `mount_rect` and its outer glow form the casing (bezel).
/// Inside the casing, a double-bordered rect forms the receptacle opening, and
/// the tongue bar sits inside that.
pub(super) const PERF_USB_MOUNT_W: f32 = 196.0;
pub(super) const PERF_USB_MOUNT_H: f32 = 180.0;
pub(super) const PERF_USB_MOUNT_ROUNDING: f32 = 8.0;
pub(super) const PERF_USB_MOUNT_GLOW_SPREAD: f32 = 50.0;
/// Peak alpha of the halo outside the casing, before the lamp's drive level.
pub(super) const PERF_USB_MOUNT_GLOW_ALPHA: f32 = 0.04;
/// Translucent housing lit by an LED at the top and bottom wall centres: the
/// share of their light on the wall and floor, and the exposure of the
/// plastic and of the LED cores (see `led_color_hot`).
pub(super) const PERF_USB_HOUSING_WALL_LIT: f32 = 0.72;
pub(super) const PERF_USB_HOUSING_FLOOR_LIT: f32 = 0.65;
pub(super) const PERF_USB_HOUSING_EXPOSURE: f32 = 1.4;
pub(super) const PERF_USB_LED_EXPOSURE: f32 = 2.5;
/// Each LED's diffused spot: half-width and half-height of its falloff.
pub(super) const PERF_USB_LED_SPOT_RX: f32 = 135.0;
pub(super) const PERF_USB_LED_SPOT_RY: f32 = 105.0;
pub(super) const PERF_USB_MOUNT_BORDER_INNER_STROKE: f32 = 12.0;
pub(super) const PERF_USB_MOUNT_BORDER_OUTER_STROKE: f32 = 4.0;
pub(super) const PERF_USB_MOUNT_BORDER_GAP: f32 = 4.0;
/// The open tray, folded against the top wall. The offsets are from the casing
/// centre.
pub(super) const PERF_USB_FLAP_W: f32 = 178.0;
pub(super) const PERF_USB_FLAP_TOP_DY: f32 = -56.0;
pub(super) const PERF_USB_FLAP_BOT_DY: f32 = -44.0;
pub(super) const PERF_USB_FLAP_STROKE: f32 = 2.0;
/// The handle dome's chord along the flap's top edge, and its height above
/// that edge.
pub(super) const PERF_USB_DOME_CHORD: f32 = 81.0;
pub(super) const PERF_USB_DOME_H: f32 = 12.0;
/// The receptacle frame inside the casing, below the tray, in reference units.
pub(super) const PERF_USB_CONNECTOR_W: f32 = 134.0;
pub(super) const PERF_USB_CONNECTOR_H: f32 = 56.0;
pub(super) const PERF_USB_CONNECTOR_V_OFF: f32 = 4.0;
pub(super) const PERF_USB_CONNECTOR_ROUNDING: f32 = 2.0;
pub(super) const PERF_USB_CONNECTOR_INNER_STROKE: f32 = 2.5;
pub(super) const PERF_USB_CONNECTOR_OUTER_STROKE: f32 = 5.0;
pub(super) const PERF_USB_CONNECTOR_BORDER_GAP: f32 = 3.0;
/// The tongue inside the receptacle opening, in ref units: its width cap, its
/// height and its inset from the top of the cavity.
pub(super) const PERF_USB_MOUNT_TONGUE_W_FRAC: f32 = 0.88;
pub(super) const PERF_USB_MOUNT_TONGUE_H: f32 = 21.0;
pub(super) const PERF_USB_MOUNT_TONGUE_TOP_PAD: f32 = 4.0;
pub(super) const PERF_USB_MOUNT_TONGUE_ROUNDING: f32 = 2.0;

/// `5V <dc> 1A` under the USB casing: centre, font, and the drawn DC mark
/// (half-gap to the text, bar width, stroke, half-separation of the bars).
pub(super) const PERF_U_USB_RATING: f32 = 0.323;
pub(super) const PERF_V_USB_RATING: f32 = 0.0698;
pub(super) const PERF_USB_RATING_FONT_SIZE: f32 = 28.0;
pub(super) const PERF_USB_DC_GAP: f32 = 22.0;
pub(super) const PERF_USB_DC_W: f32 = 30.0;
pub(super) const PERF_USB_DC_STROKE: f32 = 3.0;
pub(super) const PERF_USB_DC_SPLIT: f32 = 4.0;

// ── SD slot under the USB casing. It is decorative and drawn with its cover closed.
/// Centre of the SD casing.
pub(super) const PERF_U_SD: f32 = 0.464;
pub(super) const PERF_V_SD: f32 = 0.130;
/// Casing silhouette. The top corners are barely rounded and the bottom ones
/// are well rounded.
pub(super) const PERF_SD_W: f32 = 367.0;
pub(super) const PERF_SD_H: f32 = 223.0;
pub(super) const PERF_SD_TOP_R: f32 = 6.0;
pub(super) const PERF_SD_BOTTOM_R: f32 = 22.0;
/// Casing, outside in: a grey rim, the bezel, then a black wall at this inset.
pub(super) const PERF_SD_RIM_STROKE: f32 = 5.0;
pub(super) const PERF_SD_WALL_INSET: f32 = 13.0;
pub(super) const PERF_SD_WALL_STROKE: f32 = 5.0;
/// The cover fills the cavity down to the hinge and has rounded lower corners.
pub(super) const PERF_SD_COVER_PAD: f32 = 3.0;
pub(super) const PERF_SD_COVER_BOT_DY: f32 = 51.0;
pub(super) const PERF_SD_COVER_R: f32 = 16.0;
/// Light bar below the cover: size and offset from the casing centre.
pub(super) const PERF_SD_LED_W: f32 = 231.5;
pub(super) const PERF_SD_LED_H: f32 = 22.0;
pub(super) const PERF_SD_LED_DY: f32 = 79.0;
/// "SD" legend under the casing: card glyph and label.
pub(super) const PERF_U_SD_GLYPH: f32 = 0.405;
pub(super) const PERF_V_SD_LEGEND: f32 = 0.1682;
pub(super) const PERF_SD_GLYPH_W: f32 = 46.0;
pub(super) const PERF_SD_GLYPH_H: f32 = 55.0;
pub(super) const PERF_SD_GLYPH_STROKE: f32 = 3.5;
/// Clipped corner of the card, as a fraction of its width.
pub(super) const PERF_SD_GLYPH_NOTCH_FRAC: f32 = 0.3;
pub(super) const PERF_U_SD_LABEL: f32 = 0.493;
pub(super) const PERF_SD_LABEL_FONT_SIZE: f32 = 36.0;

/// Draws the USB MOUNT: the LED-tinted glow, the casing and the USB-A cavity.
pub(super) fn draw_usb_mount(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layout: &UiScale,
) {
    let (r, g, b) = app.mosi().rgb(Lamp::Slot2).unwrap_or_default();
    let lit = app.mosi().led_color(mosi_frame::LedPart::Slot, r, g, b);
    let usb_drive = mosi_frame::led_drive_factor(r, g, b).unwrap_or(0.0);

    let mount_center =
        layout.sp_in_rect(PERF_TRANSPORT_COL_REF, PERF_U_USB_MOUNT, PERF_V_USB_MOUNT);
    let mount_size = Vec2::new(layout.sc(PERF_USB_MOUNT_W), layout.sc(PERF_USB_MOUNT_H));
    let mount_rect = Rect::from_center_size(mount_center, mount_size);
    let rounding = layout.sc(PERF_USB_MOUNT_ROUNDING);

    if let Some(led_color) = lit {
        paint_usb_glow(p, layout, mount_rect, rounding, led_color, usb_drive);
    }
    paint_usb_housing(app, p, layout, mount_center, mount_size, (r, g, b));

    // The open tray, folded up against the top wall.
    crate::app::ui::draw_usb_tray(
        p,
        crate::app::ui::UsbTraySpec {
            centre_x: mount_center.x,
            top: mount_center.y + layout.sc(PERF_USB_FLAP_TOP_DY),
            bottom: mount_center.y + layout.sc(PERF_USB_FLAP_BOT_DY),
            half_w: layout.sc(PERF_USB_FLAP_W) * 0.5,
            dome_chord: layout.sc(PERF_USB_DOME_CHORD),
            dome_h: layout.sc(PERF_USB_DOME_H),
            stroke: layout.sc(PERF_USB_FLAP_STROKE),
        },
    );

    paint_usb_receptacle(p, layout, mount_center);

    // Reserve the mount's hit area; no click handler uses it yet.
    let _resp = ui.interact(
        mount_rect,
        ui.id().with("usb_mount_open"),
        egui::Sense::click(),
    );
}

/// Paints a soft glow halo, tinted by the USB LED color, while the LED is
/// lit. More layers and a smooth falloff give a wider, softer spread. The
/// alpha is gamma-lifted so that whites read as bright on screen.
fn paint_usb_glow(
    p: &egui::Painter,
    layout: &UiScale,
    mount_rect: Rect,
    rounding: f32,
    led_color: Color32,
    usb_drive: f32,
) {
    let max_expand = layout.sc(PERF_USB_MOUNT_GLOW_SPREAD);
    const N: usize = 12;
    for layer in 0..N {
        // t is 1.0 at the rect edge and 0.0 at the outermost layer.
        let t = 1.0 - layer as f32 / (N - 1) as f32;
        // A quadratic ease-out softens the outer falloff.
        let falloff = t * t;
        // Each layer extends progressively farther from the rect.
        let expand = (1.0 - t) * max_expand;
        let alpha_linear: f32 = falloff * PERF_USB_MOUNT_GLOW_ALPHA * usb_drive;
        let alpha = alpha_linear.powf(1.0 / mosi_frame::LED_GAMMA);
        if alpha < 0.005 {
            continue;
        }

        let g_rect = mount_rect.expand(expand);
        p.rect_filled(g_rect, rounding + expand, led_color.gamma_multiply(alpha));
    }
}

/// Paints the translucent housing, lit by an LED at the centre of its top wall
/// and one at the centre of its bottom wall.
fn paint_usb_housing(
    app: &CdjApp,
    p: &egui::Painter,
    layout: &UiScale,
    mount_center: Pos2,
    mount_size: Vec2,
    (r, g, b): (u8, u8, u8),
) {
    let usb_drive = mosi_frame::led_drive_factor(r, g, b).unwrap_or(0.0);
    let mount_rect = Rect::from_center_size(mount_center, mount_size);
    let rounding = layout.sc(PERF_USB_MOUNT_ROUNDING);
    let wall = layout.sc(PERF_USB_MOUNT_BORDER_INNER_STROKE) * 0.5;
    let mosi = app.mosi();
    let hot_at = |exposure: f32| mosi.led_color_hot(mosi_frame::LedPart::Slot, r, g, b, exposure);
    let hot = hot_at(PERF_USB_HOUSING_EXPOSURE);
    let lit_at = |k: f32| {
        hot.map(|c| mix(COL_DARK, c, k * usb_drive))
            .unwrap_or(COL_DARK)
    };
    let floor = lit_at(PERF_USB_HOUSING_FLOOR_LIT);
    let mount_border = DoubleBorderSpec::from_strokes_with_gap(
        StrokeSpec {
            width: layout.sc(PERF_USB_MOUNT_BORDER_INNER_STROKE),
            color: lit_at(PERF_USB_HOUSING_WALL_LIT),
        },
        StrokeSpec {
            width: layout.sc(PERF_USB_MOUNT_BORDER_OUTER_STROKE),
            color: COL_SILVER,
        },
        layout.sc(PERF_USB_MOUNT_BORDER_GAP),
    );
    draw_bordered_rect_section(p, mount_rect, Some(rounding), Some(floor), mount_border);

    if let Some(core) = hot_at(PERF_USB_LED_EXPOSURE) {
        let core = mix(COL_DARK, core, usb_drive);
        let spot = Vec2::new(
            layout.sc(PERF_USB_LED_SPOT_RX),
            layout.sc(PERF_USB_LED_SPOT_RY),
        );
        let clip = p.with_clip_rect(mount_rect.intersect(p.clip_rect()));
        for y in [mount_rect.top() + wall, mount_rect.bottom() - wall] {
            radial_glow(&clip, Pos2::new(mount_center.x, y), spot, core);
        }
    }
}

/// Paints the receptacle opening, with its own double border, and the tongue
/// inside it.
fn paint_usb_receptacle(p: &egui::Painter, layout: &UiScale, mount_center: Pos2) {
    let connector_size = Vec2::new(
        layout.sc(PERF_USB_CONNECTOR_W),
        layout.sc(PERF_USB_CONNECTOR_H),
    );
    let connector_rect = Rect::from_center_size(
        mount_center + Vec2::new(0.0, layout.sc(PERF_USB_CONNECTOR_V_OFF)),
        connector_size,
    );
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

    // The tongue sits inside the receptacle cavity, centered at the top.
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

/// Draws the SD slot: its casing, its closed cover and its light bar.
pub(super) fn draw_sd_slot(app: &CdjApp, p: &egui::Painter, layout: &UiScale) {
    let (r, g, b) = app.mosi().rgb(Lamp::Slot1).unwrap_or_default();
    let lit = app.mosi().led_color(mosi_frame::LedPart::Slot, r, g, b);
    let drive = mosi_frame::led_drive_factor(r, g, b).unwrap_or(0.0);

    let centre = layout.sp_in_rect(PERF_TRANSPORT_COL_REF, PERF_U_SD, PERF_V_SD);
    let outer = Rect::from_center_size(
        centre,
        Vec2::new(layout.sc(PERF_SD_W), layout.sc(PERF_SD_H)),
    );
    let corners = |inset: f32| egui::epaint::CornerRadiusF32 {
        nw: (layout.sc(PERF_SD_TOP_R) - inset).max(0.0),
        ne: (layout.sc(PERF_SD_TOP_R) - inset).max(0.0),
        sw: (layout.sc(PERF_SD_BOTTOM_R) - inset).max(0.0),
        se: (layout.sc(PERF_SD_BOTTOM_R) - inset).max(0.0),
    };
    let rim_w = layout.sc(PERF_SD_RIM_STROKE);
    p.rect_filled(outer, corners(0.0), COL_DARK);
    p.rect_stroke(
        outer.shrink(rim_w * 0.5),
        corners(rim_w * 0.5),
        Stroke::new(rim_w, COL_SILVER),
        egui::StrokeKind::Middle,
    );

    let wall_w = layout.sc(PERF_SD_WALL_STROKE);
    let wall_mid = layout.sc(PERF_SD_WALL_INSET) + wall_w * 0.5;
    p.rect_stroke(
        outer.shrink(wall_mid),
        corners(wall_mid),
        Stroke::new(wall_w, COL_BLACK),
        egui::StrokeKind::Middle,
    );
    let cavity_inset = layout.sc(PERF_SD_WALL_INSET) + wall_w;
    let cavity = outer.shrink(cavity_inset);
    p.rect_filled(cavity, corners(cavity_inset), COL_BLACK);

    let pad = layout.sc(PERF_SD_COVER_PAD);
    let cover = Rect::from_min_max(
        cavity.left_top() + Vec2::splat(pad),
        Pos2::new(
            cavity.right() - pad,
            centre.y + layout.sc(PERF_SD_COVER_BOT_DY),
        ),
    );
    let cover_r = layout.sc(PERF_SD_COVER_R);
    let cover_round = egui::epaint::CornerRadiusF32 {
        nw: 0.0,
        ne: 0.0,
        sw: cover_r,
        se: cover_r,
    };
    p.rect_filled(cover, cover_round, COL_BTN_HOT);
    p.rect_stroke(
        cover,
        cover_round,
        Stroke::new(layout.sc(PERF_USB_FLAP_STROKE), COL_SILVER),
        egui::StrokeKind::Middle,
    );

    // The bar is a diffuser: it is silver while the slot is idle and takes the
    // lamp colour while the lamp is driven.
    let led = Rect::from_center_size(
        centre + Vec2::new(0.0, layout.sc(PERF_SD_LED_DY)),
        Vec2::new(layout.sc(PERF_SD_LED_W), layout.sc(PERF_SD_LED_H)),
    );
    let led_col = lit.map(|c| c.gamma_multiply(drive)).unwrap_or(COL_SILVER);
    p.rect_filled(led, led.height() * 0.5, led_col);
}

/// Draws the USB STOP button at the top of the transport column.
pub(super) fn draw_usb_stop(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    let usb_stop_center =
        layout.sp_in_rect(PERF_TRANSPORT_COL_REF, PERF_U_USB_STOP, PERF_V_USB_STOP_BTN);
    let usb_stop_border = DoubleBorderSpec::from_strokes_with_gap(
        StrokeSpec {
            width: layout.sc(PERF_USB_STOP_BTN_INNER_STROKE),
            color: COL_BTN,
        },
        StrokeSpec {
            width: layout.sc(PERF_USB_STOP_BTN_OUTER_STROKE),
            color: COL_SILVER,
        },
        layout.sc(10.0),
    );
    app.circle_btn(
        ui,
        layout,
        usb_stop_center,
        layout.sc(PERF_USB_STOP_BTN_R),
        Some(COL_BLACK),
        None,
        "",
        None,
        None,
        layout.sc(0.0),
        None,
        Some(usb_stop_border),
        "usb_stop",
        Btn::UsbStop,
    );
}

/// Mixes `a` toward `b` by `t` (0..1), per sRGB channel.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(ch(a.r(), b.r()), ch(a.g(), b.g()), ch(a.b(), b.b()))
}

/// A soft elliptical spot of `col` at `centre`, opaque there and fading to
/// nothing at `radii` with a quadratic falloff.
fn radial_glow(p: &egui::Painter, centre: Pos2, radii: Vec2, col: Color32) {
    const RINGS: usize = 8;
    const SEGMENTS: usize = 40;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, col);
    for ring in 1..=RINGS {
        let t = ring as f32 / RINGS as f32;
        let a = (1.0 - t) * (1.0 - t);
        let c = Color32::from_rgba_premultiplied(
            (col.r() as f32 * a) as u8,
            (col.g() as f32 * a) as u8,
            (col.b() as f32 * a) as u8,
            (255.0 * a) as u8,
        );
        for k in 0..SEGMENTS {
            let th = k as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(
                centre + Vec2::new(radii.x * t * th.cos(), radii.y * t * th.sin()),
                c,
            );
        }
    }
    let at = |ring: usize, k: usize| (1 + (ring - 1) * SEGMENTS + k % SEGMENTS) as u32;
    for k in 0..SEGMENTS {
        mesh.add_triangle(0, at(1, k), at(1, k + 1));
    }
    for ring in 1..RINGS {
        for k in 0..SEGMENTS {
            mesh.add_triangle(at(ring, k), at(ring + 1, k), at(ring + 1, k + 1));
            mesh.add_triangle(at(ring, k), at(ring + 1, k + 1), at(ring, k + 1));
        }
    }
    p.add(egui::Shape::mesh(mesh));
}
