//! The strip's glyphs, each the design canvas's own SVG in its own
//! coordinates.

use egui::{Color32, CornerRadius, Pos2, Rect, Stroke, Ui, Vec2};

use super::paint::paint_tracked;
use super::style::{OK_GREEN, TEXT_DISABLED, TEXT_MUTED, WARN_AMBER};
use crate::menu::model::{MenuIcon, NetKind};

/// Draws a design icon in the coordinates it was drawn in.
///
/// Every glyph in the strip comes from the canvas as an SVG on a viewBox; this
/// maps that box onto the button so the proportions survive, and keeps the
/// stroke widths in the same units.
pub(super) struct Pen<'a> {
    p: &'a egui::Painter,
    origin: Pos2,
    k: f32,
    color: Color32,
}

impl<'a> Pen<'a> {
    /// `vb` is the SVG viewBox size, `draw` the size it is drawn at.
    pub(super) fn new(ui: &'a Ui, rect: Rect, vb: Vec2, draw: Vec2, color: Color32) -> Self {
        let k = draw.x / vb.x;
        Self {
            p: ui.painter(),
            origin: rect.center() - draw * 0.5,
            k,
            color,
        }
    }

    fn at(&self, x: f32, y: f32) -> Pos2 {
        self.origin + Vec2::new(x, y) * self.k
    }

    fn stroke(&self, w: f32) -> Stroke {
        Stroke::new(w * self.k, self.color)
    }

    pub(super) fn line(&self, a: (f32, f32), b: (f32, f32), w: f32) {
        self.p
            .line_segment([self.at(a.0, a.1), self.at(b.0, b.1)], self.stroke(w));
    }

    pub(super) fn poly(&self, pts: &[(f32, f32)], w: f32) {
        self.p.add(egui::Shape::line(
            pts.iter().map(|(x, y)| self.at(*x, *y)).collect(),
            self.stroke(w),
        ));
    }

    fn closed_poly(&self, pts: &[(f32, f32)], w: f32) {
        self.p.add(egui::Shape::closed_line(
            pts.iter().map(|(x, y)| self.at(*x, *y)).collect(),
            self.stroke(w),
        ));
    }

    pub(super) fn rect(&self, x: f32, y: f32, w: f32, h: f32, r: f32, sw: f32) {
        self.p.rect_stroke(
            Rect::from_min_size(self.at(x, y), Vec2::new(w, h) * self.k),
            CornerRadius::from(r * self.k),
            self.stroke(sw),
            egui::StrokeKind::Middle,
        );
    }

    fn fill_rect(&self, x: f32, y: f32, w: f32, h: f32) {
        self.p.rect_filled(
            Rect::from_min_size(self.at(x, y), Vec2::new(w, h) * self.k),
            0.0,
            self.color,
        );
    }

    fn fill_circle(&self, x: f32, y: f32, r: f32) {
        self.p.circle_filled(self.at(x, y), r * self.k, self.color);
    }

    /// An arc, angles in turns measured the way SVG does (clockwise on screen).
    fn arc(&self, cx: f32, cy: f32, r: f32, from: f32, to: f32, w: f32) {
        let n = 32;
        let pts = (0..=n)
            .map(|i| {
                let t = from + (to - from) * i as f32 / n as f32;
                let a = t * std::f32::consts::TAU;
                self.at(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        self.p.add(egui::Shape::line(pts, self.stroke(w)));
    }

    fn bezier(&self, pts: [(f32, f32); 4], w: f32) {
        self.p
            .add(egui::epaint::CubicBezierShape::from_points_stroke(
                [
                    self.at(pts[0].0, pts[0].1),
                    self.at(pts[1].0, pts[1].1),
                    self.at(pts[2].0, pts[2].1),
                    self.at(pts[3].0, pts[3].1),
                ],
                false,
                Color32::TRANSPARENT,
                self.stroke(w),
            ));
    }
}

/// The square viewBox most glyphs are drawn on.
const SQ: Vec2 = Vec2::splat(15.0);

/// This provider picks the glyph for a state; the state itself comes from the
/// model. A disabled menu's glyph is drawn whole, in the
/// disabled grey.
pub(super) fn paint_icon(ui: &Ui, rect: Rect, icon: MenuIcon, enabled: bool) {
    let ink = |c: Color32| if enabled { c } else { TEXT_DISABLED };
    match icon {
        MenuIcon::Restart { armed } => restart(ui, rect, armed),
        MenuIcon::Storage { mounted } => storage(ui, rect, ink(TEXT_MUTED), mounted && enabled),
        MenuIcon::Network(kind) => network(ui, rect, kind, ink(TEXT_MUTED)),
        MenuIcon::Audio { on } => audio(
            ui,
            rect,
            on,
            ink(if on { TEXT_MUTED } else { TEXT_DISABLED }),
        ),
        MenuIcon::View | MenuIcon::Instances | MenuIcon::Emulation => {
            windows(ui, rect, ink(TEXT_MUTED))
        }
    }
}

/// A circular arrow, greyed while there is nothing to restart.
fn restart(ui: &Ui, rect: Rect, armed: bool) {
    let pen = Pen::new(
        ui,
        rect,
        SQ,
        SQ,
        if armed { TEXT_MUTED } else { TEXT_DISABLED },
    );
    pen.arc(7.5, 7.5, 4.7, 0.0, 0.8690, 1.2);
    pen.poly(&[(11.4, 1.6), (11.4, 4.5), (8.5, 4.5)], 1.2);
}

/// A USB stick, with a green light while an image is mounted.
fn storage(ui: &Ui, rect: Rect, ink: Color32, lit: bool) {
    let vb = Vec2::new(11.0, 16.0);
    let pen = Pen::new(ui, rect, vb, vb, ink);
    // The connector shell and its two contacts, over the body.
    pen.rect(2.5, 0.5, 6.0, 4.0, 0.0, 1.0);
    pen.fill_rect(4.0, 2.0, 1.0, 1.0);
    pen.fill_rect(6.0, 2.0, 1.0, 1.0);
    pen.rect(0.5, 4.5, 10.0, 11.0, 2.0, 1.0);
    if lit {
        pen.p
            .circle_filled(pen.at(5.5, 11.0), 1.75 * pen.k, OK_GREEN);
    }
}

/// The network mode: NAT spelled out, two linked boxes for link-local, a
/// tree of three for bridged.
fn network(ui: &Ui, rect: Rect, kind: NetKind, ink: Color32) {
    match kind {
        NetKind::Nat => paint_tracked(
            ui,
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "NAT",
            9.0,
            0.12,
            ink,
        ),
        NetKind::LinkLocal => {
            let pen = Pen::new(ui, rect, SQ, SQ, ink);
            pen.rect(1.6, 4.6, 4.8, 5.8, 1.0, 1.1);
            pen.rect(8.6, 4.6, 4.8, 5.8, 1.0, 1.1);
            pen.line((6.4, 7.5), (8.6, 7.5), 1.1);
        }
        NetKind::Bridged => {
            let pen = Pen::new(ui, rect, SQ, SQ, ink);
            pen.rect(5.1, 1.5, 4.8, 3.6, 0.8, 1.1);
            pen.rect(1.3, 9.9, 4.8, 3.6, 0.8, 1.1);
            pen.rect(8.9, 9.9, 4.8, 3.6, 0.8, 1.1);
            pen.line((7.5, 5.1), (7.5, 7.5), 1.1);
            pen.poly(&[(3.7, 9.9), (3.7, 7.5), (11.3, 7.5), (11.3, 9.9)], 1.1);
        }
    }
}

/// A speaker, sounding or crossed out.
fn audio(ui: &Ui, rect: Rect, on: bool, ink: Color32) {
    let pen = Pen::new(ui, rect, SQ, SQ, ink);
    pen.closed_poly(
        &[
            (2.6, 5.8),
            (5.0, 5.8),
            (8.0, 3.2),
            (8.0, 11.8),
            (5.0, 9.2),
            (2.6, 9.2),
        ],
        1.1,
    );
    if on {
        pen.bezier([(10.3, 5.5), (11.3, 6.7), (11.3, 8.3), (10.3, 9.5)], 1.1);
        pen.bezier([(12.1, 4.0), (13.9, 6.1), (13.9, 8.9), (12.1, 11.0)], 1.1);
    } else {
        pen.line((10.0, 6.0), (13.2, 9.2), 1.1);
        pen.line((13.2, 6.0), (10.0, 9.2), 1.1);
    }
}

/// Two overlapping windows.
fn windows(ui: &Ui, rect: Rect, ink: Color32) {
    let pen = Pen::new(ui, rect, SQ, SQ, ink);
    pen.rect(1.8, 2.8, 8.6, 6.4, 1.1, 1.1);
    pen.rect(4.8, 5.8, 8.6, 6.4, 1.1, 1.1);
}

/// The notice badge's gauge.
pub(super) fn paint_gauge(ui: &Ui, rect: Rect) {
    let pen = Pen::new(ui, rect, SQ, SQ, WARN_AMBER);
    pen.arc(7.5, 9.0, 6.0, 0.4286, 1.0714, 1.1);
    pen.line((7.5, 9.0), (4.0, 8.2), 1.3);
    pen.fill_circle(7.5, 9.0, 1.1);
}
