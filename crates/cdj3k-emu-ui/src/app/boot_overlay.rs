//! The boot and idle shade: a translucent scrim and a 12-spoke spinner, drawn
//! while QEMU starts and the LCD texture catches up.

const SHADE_BG: egui::Color32 = egui::Color32::from_rgb(25, 25, 25);
/// Maximum scrim opacity at full fade-in. The chassis stays visible behind the
/// scrim; the LCD content is blanked at the source.
const SHADE_MAX_ALPHA: f32 = 225.0;

const SPOKE_COUNT: usize = 12;
/// Spokes within this many positions of `leading` get the bright ramp.
const FADE_TAIL: f32 = 6.0;
/// Floor brightness for spokes outside the ramp.
const FADE_FLOOR: f32 = 0.2;
/// Spinner speed, in spoke positions per second.
const TICK_RATE: f32 = 24.0;
/// Polygon segments in each spoke's semicircular outer cap.
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
/// The shade is dark in either system theme, so its error colours come from
/// the dark palette.
const BROKEN_ICON: egui::Color32 = super::theme::DARK.danger;
const BROKEN_TEXT: egui::Color32 = super::theme::DARK.ink;
const BROKEN_DETAIL: egui::Color32 = super::theme::DARK.muted;
const BROKEN_TITLE: &str = "This emulation cannot boot";
/// Repaint interval while the spinner animates, capped at 60 Hz.
const REPAINT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);

/// Paint the boot and idle shade on top of [`egui::Order::Foreground`].
///
/// `alpha` is the current ramp value in `[0, 1]`. `booting` shows the spinner;
/// with it off during the fade-out, the chassis appears without a spinner over
/// it. `broken` shows a warning that the slot needs a reinstall instead.
pub(super) fn paint_boot_shade(ctx: &egui::Context, alpha: f32, booting: bool, broken: bool) {
    // The deck is what is booting, so the shade covers the deck and stops at
    // the in-window menu strip: those controls work throughout.
    let mut screen = ctx.content_rect();
    screen.min.y += cdj3k_emu_platform::menu::in_window_bar_height();
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

            if broken {
                paint_broken(painter, screen, alpha);
            } else if booting {
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

/// Draw an error triangle above the title and the reinstall hint.
fn paint_broken(painter: &egui::Painter, screen: egui::Rect, alpha: f32) {
    let size = (screen.width().min(screen.height()) * SPOKE_OUTER_RADIUS_FRAC)
        .clamp(SPOKE_OUTER_RADIUS_MIN, SPOKE_OUTER_RADIUS_MAX);
    let center = screen.center() - egui::vec2(0.0, size);
    let icon = BROKEN_ICON.gamma_multiply(alpha);
    let top = center - egui::vec2(0.0, size * 0.8);
    let left = center + egui::vec2(-size * 0.92, size * 0.8);
    let right = center + egui::vec2(size * 0.92, size * 0.8);
    let stroke = egui::Stroke::new((size * SPOKE_STROKE_FRAC).max(SPOKE_STROKE_MIN), icon);
    painter.add(egui::Shape::closed_line(vec![top, right, left], stroke));
    // The exclamation mark: a bar and a dot.
    painter.line_segment(
        [
            center - egui::vec2(0.0, size * 0.3),
            center + egui::vec2(0.0, size * 0.25),
        ],
        stroke,
    );
    painter.circle_filled(
        center + egui::vec2(0.0, size * 0.52),
        stroke.width * 0.6,
        icon,
    );

    let title = egui::FontId::proportional((size * 0.5).max(15.0));
    let hint = egui::FontId::proportional((size * 0.34).max(12.0));
    let mut y = center.y + size * 1.4;
    let title_rect = painter.text(
        egui::pos2(center.x, y),
        egui::Align2::CENTER_TOP,
        BROKEN_TITLE,
        title,
        BROKEN_TEXT.gamma_multiply(alpha),
    );
    y = title_rect.bottom() + size * 0.25;
    painter.text(
        egui::pos2(center.x, y),
        egui::Align2::CENTER_TOP,
        super::picker::BROKEN,
        hint,
        BROKEN_DETAIL.gamma_multiply(alpha),
    );
}

/// Each spoke is one convex polygon: a rectangle joined to a semicircular outer
/// cap. A separate line and circle would leave anti-aliased edge pixels that
/// double the alpha at the join; one convex polygon is tessellated and filled
/// once.
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

        // The perpendicular is (-sn, cs).
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
        // Sweep from the inner_l flank (theta+π/2) through the tip (theta) to
        // the inner_r flank (theta−π/2). On screen, where y points down, this
        // runs counter-clockwise.
        for k in 0..=ARC_SEGS {
            let a = theta + FRAC_PI_2 - PI * (k as f32 / ARC_SEGS as f32);
            let (asn, acs) = a.sin_cos();
            pts.push(egui::pos2(cap_cx + half_w * acs, cap_cy + half_w * asn));
        }
        pts.push(inner_r);

        painter.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
    }
}
