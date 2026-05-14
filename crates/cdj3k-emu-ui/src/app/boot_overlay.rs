//! Boot/idle shade overlay: translucent scrim plus a 12-spoke "dents"
//! spinner rendered while QEMU is starting and the LCD texture is still
//! catching up.

const SHADE_BG: egui::Color32 = egui::Color32::from_rgb(25, 25, 25);
/// Maximum scrim opacity at full fade-in. The chassis chrome stays visible
/// behind it on purpose - LCD content is masked at the source.
const SHADE_MAX_ALPHA: f32 = 225.0;

const SPOKE_COUNT: usize = 12;
/// Spokes within this many positions of `leading` get the bright ramp.
const FADE_TAIL: f32 = 6.0;
/// Floor brightness for spokes outside the ramp.
const FADE_FLOOR: f32 = 0.2;
/// Spinner angular speed (positions per second).
const TICK_RATE: f32 = 24.0;
/// Polygon segments used per spoke's outer cap (semicircle).
const ARC_SEGS: usize = 12;
/// Outer radius as a fraction of `min(width, height)` (clamped below).
const SPOKE_OUTER_RADIUS_FRAC: f32 = 0.06;
const SPOKE_OUTER_RADIUS_MIN: f32 = 22.0;
const SPOKE_OUTER_RADIUS_MAX: f32 = 56.0;
/// Inner radius as a fraction of the outer radius.
const SPOKE_INNER_RATIO: f32 = 0.55;
/// Stroke width as a fraction of the outer radius (with a 1.5 px floor).
const SPOKE_STROKE_FRAC: f32 = 0.13;
const SPOKE_STROKE_MIN: f32 = 1.5;
/// Repaint cadence while the spinner is animating (60 Hz cap).
const REPAINT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);

/// Paint the boot/idle shade on top of [`egui::Order::Foreground`].
///
/// `alpha` is the current ramp value in `[0, 1]`. `booting` toggles the
/// spinner; chrome reveals cleanly when it's off during fade-out.
pub(super) fn paint_boot_shade(ctx: &egui::Context, alpha: f32, booting: bool) {
    let screen = ctx.screen_rect();
    egui::Area::new(egui::Id::new("qemu_boot_shade"))
        .order(egui::Order::Foreground)
        .interactable(false)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let painter = ui.painter();
            let base_a = (SHADE_MAX_ALPHA * alpha) as u8;
            painter.rect_filled(
                screen,
                0.0,
                egui::Color32::from_rgba_unmultiplied(
                    SHADE_BG.r(),
                    SHADE_BG.g(),
                    SHADE_BG.b(),
                    base_a,
                ),
            );

            if booting {
                let center = screen.center();
                let outer = (screen.width().min(screen.height()) * SPOKE_OUTER_RADIUS_FRAC)
                    .clamp(SPOKE_OUTER_RADIUS_MIN, SPOKE_OUTER_RADIUS_MAX);
                let inner = outer * SPOKE_INNER_RATIO;
                let stroke_w = (outer * SPOKE_STROKE_FRAC).max(SPOKE_STROKE_MIN);
                let t = ui.input(|i| i.time) as f32;
                paint_spinner(painter, center, inner, outer, stroke_w, t, alpha);
                ctx.request_repaint_after(REPAINT_INTERVAL);
            }
        });
}

/// Each spoke is built as one convex polygon - a rectangle joined to a
/// semicircular outer cap. Drawing them as separate line + circle leaves
/// anti-aliased edge pixels that double the alpha at the join; one
/// convex_polygon goes through one tessellation pass with one fill.
fn paint_spinner(
    painter: &egui::Painter,
    center: egui::Pos2,
    inner: f32,
    outer: f32,
    stroke_w: f32,
    t: f32,
    alpha: f32,
) {
    use std::f32::consts::{FRAC_PI_2, PI, TAU};

    let leading = t * TICK_RATE;
    let half_w = stroke_w * 0.5;

    for i in 0..SPOKE_COUNT {
        let dist = (leading - i as f32).rem_euclid(SPOKE_COUNT as f32);
        let fade = if dist <= FADE_TAIL {
            1.0 - (1.0 - FADE_FLOOR) * (dist / FADE_TAIL)
        } else {
            FADE_FLOOR
        };
        let a = (255.0 * fade * alpha) as u8;
        let color = egui::Color32::from_white_alpha(a);
        let theta = (i as f32 / SPOKE_COUNT as f32) * TAU - FRAC_PI_2;
        let (sn, cs) = theta.sin_cos();

        // perp = (-sn, cs).
        let inner_l = egui::pos2(
            center.x + cs * inner - sn * half_w,
            center.y + sn * inner + cs * half_w,
        );
        let inner_r = egui::pos2(
            center.x + cs * inner + sn * half_w,
            center.y + sn * inner - cs * half_w,
        );
        let cap_cx = center.x + cs * (outer - half_w);
        let cap_cy = center.y + sn * (outer - half_w);

        let mut pts = Vec::with_capacity(3 + ARC_SEGS);
        pts.push(inner_l);
        // Sweep clockwise from theta+π/2 (left flank) through theta (tip)
        // to theta−π/2 (right flank).
        for k in 0..=ARC_SEGS {
            let a = theta + FRAC_PI_2 - PI * (k as f32 / ARC_SEGS as f32);
            let (asn, acs) = a.sin_cos();
            pts.push(egui::pos2(cap_cx + half_w * acs, cap_cy + half_w * asn));
        }
        pts.push(inner_r);

        painter.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
    }
}
