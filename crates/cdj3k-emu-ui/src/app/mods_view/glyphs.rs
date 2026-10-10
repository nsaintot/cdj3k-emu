//! The Mods screens' line icons, each drawn on a 14-unit grid (the cube on
//! 28) with the window's stroke.

use super::*;

#[derive(Clone, Copy)]
pub(super) enum Glyph {
    Doc,
    Eject,
    Trash,
    Plus,
    Lock,
    Warn,
    Handle,
    Cube,
    Folder,
}

/// Maps a glyph's grid units to points inside its rect, and holds the
/// glyph's stroke.
struct Grid<'a> {
    p: &'a egui::Painter,
    /// The grid's top-left corner.
    o: Pos2,
    /// Points per grid unit.
    s: f32,
    stroke: Stroke,
}

impl Grid<'_> {
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        Pos2::new(self.o.x + x * self.s, self.o.y + y * self.s)
    }

    /// A line through `pts`, closed back to the first when `closed`.
    fn poly(&self, pts: &[(f32, f32)], closed: bool) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        if closed {
            self.p.add(Shape::closed_line(v, self.stroke));
        } else {
            self.p.add(Shape::line(v, self.stroke));
        }
    }
}

/// Paint `g` into `rect`, drawn on a 14-unit grid (the cube on 28).
pub(super) fn glyph(p: &egui::Painter, g: Glyph, rect: Rect, col: Color32, k: f32) {
    let grid = if matches!(g, Glyph::Cube) { 28.0 } else { 14.0 };
    let s = rect.width().min(rect.height()) / grid;
    let grid = Grid {
        p,
        o: rect.center() - Vec2::splat(grid * 0.5 * s),
        s,
        stroke: Stroke::new(1.1 * k, col),
    };
    match g {
        Glyph::Doc => doc(&grid),
        Glyph::Eject => {
            grid.poly(&[(7.0, 2.4), (12.0, 8.2), (2.0, 8.2)], true);
            grid.poly(&[(2.2, 11.2), (11.8, 11.2)], false);
        }
        Glyph::Trash => trash_glyph(p, rect.shrink(rect.width() * 0.05), col, k),
        Glyph::Plus => {
            grid.poly(&[(7.0, 1.5), (7.0, 12.5)], false);
            grid.poly(&[(1.5, 7.0), (12.5, 7.0)], false);
        }
        Glyph::Lock => lock(&grid),
        Glyph::Warn => warn(&grid, col, k),
        Glyph::Handle => {
            for (x, y) in [
                (5.0, 3.0),
                (9.0, 3.0),
                (5.0, 7.0),
                (9.0, 7.0),
                (5.0, 11.0),
                (9.0, 11.0),
            ] {
                p.circle_filled(grid.pt(x, y), 1.1 * s, col);
            }
        }
        Glyph::Cube => cube(&grid, col, k),
        Glyph::Folder => folder(&grid),
    }
}

/// The mods icon, a cube, used by the Mods bar and the replace warning.
pub(in crate::app) fn cube_glyph(p: &egui::Painter, rect: Rect, col: Color32, k: f32) {
    glyph(p, Glyph::Cube, rect, col, k);
}

/// A page with a folded corner and two lines of text.
fn doc(g: &Grid<'_>) {
    g.poly(
        &[
            (3.0, 1.8),
            (8.4, 1.8),
            (11.0, 4.4),
            (11.0, 12.2),
            (3.0, 12.2),
        ],
        true,
    );
    g.poly(&[(8.2, 1.8), (8.2, 4.6), (11.0, 4.6)], false);
    g.poly(&[(5.0, 7.0), (9.0, 7.0)], false);
    g.poly(&[(5.0, 9.4), (9.0, 9.4)], false);
}

/// A padlock: its body and a half-round shackle.
fn lock(g: &Grid<'_>) {
    g.poly(&[(2.5, 6.0), (11.5, 6.0), (11.5, 12.5), (2.5, 12.5)], true);
    let arc: Vec<(f32, f32)> = (0..=12)
        .map(|i| {
            let a = std::f32::consts::PI * (i as f32 / 12.0);
            (7.0 - 2.5 * a.cos(), 4.3 - 2.5 * a.sin())
        })
        .collect();
    let mut shackle = vec![(4.5, 6.0)];
    shackle.extend(arc);
    shackle.push((9.5, 6.0));
    g.poly(&shackle, false);
}

/// A warning triangle with an exclamation mark.
fn warn(g: &Grid<'_>, col: Color32, k: f32) {
    g.poly(&[(7.0, 1.4), (13.3, 12.6), (0.7, 12.6)], true);
    g.p.line_segment([g.pt(7.0, 5.4), g.pt(7.0, 8.8)], Stroke::new(1.3 * k, col));
    g.p.circle_filled(g.pt(7.0, 10.6), 0.8 * g.s, col);
}

/// The cube, on the 28-unit grid, a shade heavier than the other glyphs.
fn cube(g: &Grid<'_>, col: Color32, k: f32) {
    let stroke = Stroke::new(1.3 * k, col);
    let v = |pts: &[(f32, f32)]| pts.iter().map(|&(x, y)| g.pt(x, y)).collect::<Vec<_>>();
    g.p.add(Shape::closed_line(
        v(&[
            (4.0, 9.5),
            (14.0, 4.0),
            (24.0, 9.5),
            (24.0, 18.5),
            (14.0, 24.0),
            (4.0, 18.5),
        ]),
        stroke,
    ));
    g.p.add(Shape::line(
        v(&[(4.0, 9.5), (14.0, 15.0), (24.0, 9.5)]),
        stroke,
    ));
    g.p.add(Shape::line(v(&[(14.0, 15.0), (14.0, 24.0)]), stroke));
}

/// A folder with a plus on it.
fn folder(g: &Grid<'_>) {
    g.poly(
        &[
            (2.0, 3.2),
            (6.0, 3.2),
            (7.2, 4.6),
            (12.0, 4.6),
            (12.0, 11.2),
            (2.0, 11.2),
        ],
        true,
    );
    g.poly(&[(7.0, 6.6), (7.0, 9.6)], false);
    g.poly(&[(5.5, 8.1), (8.5, 8.1)], false);
}
