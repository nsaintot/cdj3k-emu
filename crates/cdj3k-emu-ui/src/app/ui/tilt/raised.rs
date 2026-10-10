//! Raised parts of a slate: how each stands off the panel, and the walls,
//! ribs and facets of its side that show when the deck tilts.

use egui::epaint::Mesh;
use egui::{Color32, Pos2, Shape, Stroke, Vec2};

use super::{Tilt, P3};

/// How a raised part stands off the panel.
#[derive(Copy, Clone, Debug)]
pub(in crate::app) enum Lift {
    /// The top is flat at this height.
    Flat(f32),
    /// A round part that is flat at `height` out to `r_top`, then slopes down
    /// to height `foot` at `r_foot`, like a grip whose side is a cone.
    Cone {
        center: Pos2,
        r_top: f32,
        r_foot: f32,
        height: f32,
        foot: f32,
    },
}

impl Lift {
    /// Returns the part's height at a point of its plan.
    pub(super) fn z(&self, p: Pos2) -> f32 {
        match *self {
            Lift::Flat(h) => h,
            Lift::Cone {
                center,
                r_top,
                r_foot,
                height,
                foot,
            } => {
                let t = ((p.distance(center) - r_top) / (r_foot - r_top)).clamp(0.0, 1.0);
                height + (foot - height) * t
            }
        }
    }

    /// Returns the lift's parameters as bits, used to key a cached projection.
    pub(super) fn bits(&self) -> [u32; 6] {
        match *self {
            Lift::Flat(h) => [0, h.to_bits(), 0, 0, 0, 0],
            Lift::Cone {
                center,
                r_top,
                r_foot,
                height,
                foot,
            } => [
                center.x.to_bits() ^ center.y.to_bits().rotate_left(16),
                r_top.to_bits(),
                r_foot.to_bits(),
                height.to_bits(),
                foot.to_bits(),
                1,
            ],
        }
    }

    /// Returns the radii at which the height stops being one plane. A triangle
    /// that reaches across one of them must be split.
    pub(super) fn creases(&self) -> Option<(Pos2, [f32; 2])> {
        match *self {
            Lift::Flat(_) => None,
            Lift::Cone {
                center,
                r_top,
                r_foot,
                ..
            } => Some((center, [r_top, r_foot])),
        }
    }
}

/// One wall of a raised part: its outline at its foot and at its top, and
/// their heights. A top smaller than the base slopes the wall inward, and
/// walls stacked on one another make a curved side.
pub(in crate::app) struct Footprint {
    pub base: Vec<Pos2>,
    pub base_z: f32,
    pub top: Vec<Pos2>,
    pub top_z: f32,
    /// Whether the wall's silhouette is outlined. A wall stacked inside a
    /// curve is not outlined, so the curve reads as one surface.
    pub outlined: bool,
    /// The wall's own colour, used where a part's walls change shade along a
    /// curve.
    pub fill: Option<Color32>,
}

impl Footprint {
    /// Returns a straight wall with the same outline at the panel and at `z`.
    pub(in crate::app) fn upright(outline: Vec<Pos2>, z: f32) -> Self {
        Self {
            base: outline.clone(),
            base_z: 0.0,
            top: outline,
            top_z: z,
            outlined: true,
            fill: None,
        }
    }
}

/// A line moulded into a part's side, between two points on it. It shows only
/// where the side faces the front; `normal` is the side's outward direction
/// at the line.
pub(in crate::app) struct Rib {
    pub a: P3,
    pub b: P3,
    pub normal: Vec2,
}

/// A flat patch of a part's side, with its corners in order and its own
/// colour. Like a [`Rib`], it shows only where it faces the front.
pub(in crate::app) struct Facet {
    pub quad: [P3; 4],
    pub normal: Vec2,
    pub fill: Color32,
}

/// A raised part of a slate: the shapes it painted (indices into the slate's
/// layer), how it stands off the panel, and its walls in reference units,
/// which show when the deck tilts.
pub(in crate::app) struct Raised {
    pub shapes: std::ops::Range<usize>,
    pub lift: Lift,
    pub walls: Vec<Footprint>,
    pub wall: Color32,
    pub ribs: Vec<Rib>,
    pub rib: Stroke,
    /// Patches drawn over the walls where a side is not one surface.
    pub facets: Vec<Facet>,
    /// The tops a pointer may land on, with their heights.
    pub tops: Vec<(Vec<Pos2>, f32)>,
}

/// The convex hull of `pts`, clockwise on screen.
fn hull(mut pts: Vec<Pos2>) -> Vec<Pos2> {
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup_by(|a, b| a.distance(*b) < 0.01);
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: Pos2, a: Pos2, b: Pos2| (a - o).x * (b - o).y - (a - o).y * (b - o).x;
    let mut lower: Vec<Pos2> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<Pos2> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

impl Raised {
    /// Returns the part's walls on screen, and the facets and ribs on their
    /// front-facing side.
    pub(super) fn walls(&self, tilt: &Tilt, edge: Stroke) -> Vec<Shape> {
        let mut out: Vec<Shape> = self
            .walls
            .iter()
            .map(|f| {
                let pts = f
                    .base
                    .iter()
                    .map(|p| tilt.project([p.x, p.y, f.base_z]))
                    .chain(f.top.iter().map(|p| tilt.project([p.x, p.y, f.top_z])))
                    .collect();
                Shape::convex_polygon(
                    hull(pts),
                    f.fill.unwrap_or(self.wall),
                    if f.outlined { edge } else { Stroke::NONE },
                )
            })
            .collect();
        // Draw the facets as one unfeathered mesh. They tile the side edge to
        // edge, and a feathered sliver seen edge-on spikes far past its corners.
        let mut facets = Mesh::default();
        for f in self.facets.iter().filter(|f| f.normal.y > 0.0) {
            let i = facets.vertices.len() as u32;
            for &p in &f.quad {
                facets.colored_vertex(tilt.project(p), f.fill);
            }
            facets.add_triangle(i, i + 1, i + 2);
            facets.add_triangle(i, i + 2, i + 3);
        }
        if !facets.is_empty() {
            out.push(Shape::mesh(facets));
        }
        out.extend(
            self.ribs
                .iter()
                .filter(|r| r.normal.y > 0.0)
                .map(|r| Shape::line_segment([tilt.project(r.a), tilt.project(r.b)], self.rib)),
        );
        out
    }

    /// Returns the part's tops on screen, for hit-testing.
    pub(in crate::app) fn screen_tops(&self, tilt: &Tilt) -> Vec<(Vec<Pos2>, f32)> {
        self.tops
            .iter()
            .map(|(f, z)| (f.iter().map(|p| tilt.project([p.x, p.y, *z])).collect(), *z))
            .collect()
    }
}
