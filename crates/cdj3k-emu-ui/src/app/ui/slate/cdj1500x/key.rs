//! The CDJ-1500X's keys, in the CDJ-3000's key style: a silver ring at the
//! drawing's outline, a clear gap where the panel shows, and the face inside
//! it with a dark line on its edge. The legends and lamps are their callers'.

use std::f32::consts::PI;

use egui::{Color32, Pos2, Rect, Shape, Stroke};

use crate::app::ui::draw_cache::ShapeList;
use crate::app::ui::tilt::{Footprint, Lift, Raised};
use crate::app::ui::{UiScale, COL_BLACK, COL_BTN, COL_BTN_HOT, COL_SILVER};

/// How far a key's cap stands above the panel, and a round transport key's.
pub const HEIGHT: f32 = 14.0;
pub const ROUND_HEIGHT: f32 = 20.0;

/// A circle's outline in reference units, for a round part's footprint.
pub fn circle(c: Pos2, r: f32) -> Vec<Pos2> {
    (0..48)
        .map(|i| c + egui::Vec2::angled(i as f32 * std::f32::consts::TAU / 48.0) * r)
        .collect()
}

/// An unlit LED legend.
pub const COL_LED_OFF: Color32 = Color32::from_rgb(82, 82, 90);

/// An LED legend lit at `level` (0 dark, 255 full) in its one tint.
pub fn led(tint: Color32, level: u8) -> Color32 {
    if level == 0 {
        return COL_LED_OFF;
    }
    let k = 0.4 + 0.6 * level as f32 / 255.0;
    let f = |c: u8| (c as f32 * k).round() as u8;
    Color32::from_rgb(f(tint.r()), f(tint.g()), f(tint.b()))
}

/// The key's edge, reference units: the silver ring's width, the gap inside
/// it, and the face's edge line - the CDJ-3000's hot cue keys' widths.
const RING: f32 = 4.0;
const GAP: f32 = 2.0;
const EDGE: f32 = 1.0;

/// Samples per quarter turn of a corner.
const CORNER_SEGS: usize = 10;

/// A key's outline: a rectangle in reference units with its own radius at
/// each corner, clockwise from the top-left `[nw, ne, se, sw]`.
#[derive(Copy, Clone, Debug)]
pub struct Outline {
    pub rect: Rect,
    pub radii: [f32; 4],
}

impl Outline {
    pub fn new(rect: Rect, radii: [f32; 4]) -> Self {
        Self { rect, radii }
    }

    /// The same shape `d` further in: every edge moves in, every radius
    /// shrinks by as much.
    pub fn inset(self, d: f32) -> Self {
        Self {
            rect: self.rect.shrink(d),
            radii: self.radii.map(|r| (r - d).max(0.0)),
        }
    }

    /// The outline as screen points, clockwise from the top edge.
    pub fn points(&self, layout: &UiScale) -> Vec<Pos2> {
        self.ref_points()
            .into_iter()
            .map(|p| layout.sp(p.x, p.y))
            .collect()
    }

    /// The outline in reference units, clockwise from the top edge.
    pub fn ref_points(&self) -> Vec<Pos2> {
        let r = self.rect;
        let [nw, ne, se, sw] = self.radii;
        let mut pts = Vec::with_capacity(4 * (CORNER_SEGS + 1));
        let mut corner = |c: Pos2, rad: f32, a0: f32| {
            for i in 0..=CORNER_SEGS {
                let a = a0 + 0.5 * PI * i as f32 / CORNER_SEGS as f32;
                pts.push(Pos2::new(c.x + rad * a.cos(), c.y + rad * a.sin()));
            }
        };
        corner(Pos2::new(r.right() - ne, r.top() + ne), ne, 1.5 * PI);
        corner(Pos2::new(r.right() - se, r.bottom() - se), se, 0.0);
        corner(Pos2::new(r.left() + sw, r.bottom() - sw), sw, 0.5 * PI);
        corner(Pos2::new(r.left() + nw, r.top() + nw), nw, PI);
        // Where a straight run has no length, two corners share a point; a
        // repeated point has no direction, and the stroke spikes at it.
        pts.dedup_by(|b, a| a.distance(*b) < 0.01);
        if pts.len() > 1 && pts[0].distance(pts[pts.len() - 1]) < 0.01 {
            pts.pop();
        }
        pts
    }

    /// The hit area on screen.
    pub fn screen_rect(&self, layout: &UiScale) -> Rect {
        Rect::from_min_max(
            layout.sp(self.rect.left(), self.rect.top()),
            layout.sp(self.rect.right(), self.rect.bottom()),
        )
    }
}

/// The face inside a key's outline: what stands off the panel once the deck
/// tilts.
pub fn face(outline: Outline) -> Outline {
    outline.inset(RING + GAP + EDGE * 0.5)
}

/// Collect a key: the silver ring, then the face with its edge line.
pub fn collect_key(list: &mut ShapeList, layout: &UiScale, outline: Outline, pressed: bool) {
    list.add(Shape::closed_line(
        outline.inset(RING * 0.5).points(layout),
        Stroke::new(layout.sc(RING), COL_SILVER),
    ));
    let fill = if pressed { COL_BTN_HOT } else { COL_BTN };
    list.add(Shape::convex_polygon(
        face(outline).points(layout),
        fill,
        Stroke::new(layout.sc(EDGE), COL_BLACK),
    ));
}

/// A key's side, seen once the deck tilts.
const COL_WALL: Color32 = Color32::from_rgb(14, 14, 18);

/// Keys with these cap outlines (reference units), standing `height` off the
/// panel on straight sides.
pub fn raised(outlines: Vec<Vec<Pos2>>, height: f32) -> Raised {
    Raised {
        shapes: 0..0,
        lift: Lift::Flat(height),
        walls: outlines
            .iter()
            .map(|o| Footprint::upright(o.clone(), height))
            .collect(),
        wall: COL_WALL,
        ribs: Vec::new(),
        rib: Stroke::NONE,
        facets: Vec::new(),
        tops: outlines.into_iter().map(|o| (o, height)).collect(),
    }
}
