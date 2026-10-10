//! Tilts a slate: the panel turns back about its front edge in perspective,
//! so that the face below that edge comes into view.
//!
//! Coordinates are the slate's reference units: `x` across, `y` from the
//! back edge to the front, `z` up from the top surface. The tilt turns the
//! deck about its front edge (`y = ref_h`, `z = 0`) by `t * max_angle`, and
//! views it in a perspective whose strength grows with `t` too, so at `t = 0`
//! the projection is exactly the plan the slate draws. A 2D fit then centres
//! the tilted deck in the canvas.
//!
//! The slate draws its plan; [`project_layer`] then replaces what it
//! painted with the projection, and [`Tilt::unproject`] maps the pointer back
//! onto the panel so its controls keep working.

use egui::epaint::{Mesh, TessellationOptions, Tessellator, Vertex};
use egui::layers::ShapeIdx;
use egui::{LayerId, Pos2, Rect, Shape, Stroke, TextureId, Vec2};

use super::UiScale;

mod raised;
pub(in crate::app) use raised::{Facet, Footprint, Lift, Raised, Rib};

/// A point on the deck, in reference units.
pub(in crate::app) type P3 = [f32; 3];

/// The longest on-screen side a textured triangle may have before it is
/// split. The perspective projection is not affine, but the texture mapping
/// inside each triangle is.
const MAX_TEXTURED_EDGE_PX: f32 = 24.0;

/// How far across a crease a triangle may reach, in reference units, before
/// it is split so that it bends with the surface.
const MAX_CREASE_SPAN: f32 = 4.0;
/// How far past a crease a triangle must reach to count as crossing it, so
/// that an edge drawn on the crease, including its feathering, does not.
const CREASE_SLACK: f32 = 3.0;

/// One tilt of one slate.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(in crate::app) struct Tilt {
    ref_w: f32,
    ref_h: f32,
    sin: f32,
    cos: f32,
    /// The perspective strength: `t` divided by the camera distance.
    k: f32,
    /// The camera's y on the canvas: the middle of the turned deck, so that
    /// its front and back are seen equally squarely.
    cam_y: f32,
    fit_scale: f32,
    fit_off: Vec2,
    /// The plan's reference-to-screen mapping.
    ox: f32,
    oy: f32,
    scale: f32,
    /// The crease `(y, slope)`: behind `y`, the surface rises by `slope`
    /// units of height per unit toward the back.
    crease: Option<(f32, f32)>,
}

impl Tilt {
    /// Builds the tilt at `t`, where 0 is the plan and 1 is fully tilted to
    /// `max_angle` radians, seen from `cam_dist` reference units. The fit
    /// makes the points of `extent` fill the canvas as the plan does. With a
    /// `crease` `(y, angle)`, the panel's surface rises behind `y` at
    /// `t * angle` radians.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::app) fn new(
        layout: &UiScale,
        ref_w: f32,
        ref_h: f32,
        t: f32,
        max_angle: f32,
        cam_dist: f32,
        crease: Option<(f32, f32)>,
        extent: &[P3],
    ) -> Self {
        let (ox, oy, scale) = layout.cache_key();
        let a = t * max_angle;
        let mut tilt = Self {
            ref_w,
            ref_h,
            sin: a.sin(),
            cos: a.cos(),
            k: t / cam_dist,
            cam_y: ref_h * 0.5,
            fit_scale: 1.0,
            fit_off: Vec2::ZERO,
            ox,
            oy,
            scale,
            crease: crease.map(|(y, angle)| (y, (t * angle).tan())),
        };
        let (lo, hi) = extent.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
            let y = tilt.turn(*p).0;
            (lo.min(y), hi.max(y))
        });
        if lo <= hi {
            tilt.cam_y = (lo + hi) * 0.5;
        }
        let bounds = extent
            .iter()
            .fold(Rect::NOTHING, |r, p| r.union(Rect::from_pos(tilt.raw(*p))));
        if bounds.is_positive() {
            let f = (ref_w / bounds.width()).min(ref_h / bounds.height());
            tilt.fit_scale = f;
            tilt.fit_off =
                Pos2::new(ref_w * 0.5, ref_h * 0.5).to_vec2() - bounds.center().to_vec2() * f;
        }
        tilt
    }

    /// Returns the panel's surface height at `y`: zero in front of the
    /// crease, rising behind it.
    pub(in crate::app) fn surface_z(&self, y: f32) -> f32 {
        match self.crease {
            Some((cy, slope)) if y < cy => (cy - y) * slope,
            _ => 0.0,
        }
    }

    /// Returns the camera's x and y on the canvas.
    fn cam(&self) -> (f32, f32) {
        (self.ref_w * 0.5, self.cam_y)
    }

    /// Turns a deck point about the front edge and returns its `y` down the
    /// canvas and its height toward the camera.
    fn turn(&self, [_, y, z]: P3) -> (f32, f32) {
        let dy = y - self.ref_h;
        (
            self.ref_h + dy * self.cos - z * self.sin,
            dy * self.sin + z * self.cos,
        )
    }

    /// Turns and projects a deck point, before the fit.
    fn raw(&self, p: P3) -> Pos2 {
        let x = p[0];
        let (ty, tz) = self.turn(p);
        let s = 1.0 / (1.0 - tz * self.k);
        let (cx, cy) = self.cam();
        Pos2::new(cx + (x - cx) * s, cy + (ty - cy) * s)
    }

    /// Returns where a deck point lands on the canvas, in reference units.
    pub(in crate::app) fn canvas_point(&self, p: P3) -> Pos2 {
        self.raw(p) * self.fit_scale + self.fit_off
    }

    /// Returns the screen position of the top-surface point `(x, y)` in the
    /// untilted plan.
    pub(in crate::app) fn plan_screen(&self, p: Pos2) -> Pos2 {
        Pos2::new(self.ox + p.x * self.scale, self.oy + p.y * self.scale)
    }

    /// Returns where a deck point lands on screen.
    pub(in crate::app) fn project(&self, p: P3) -> Pos2 {
        let c = self.canvas_point(p);
        Pos2::new(self.ox + c.x * self.scale, self.oy + c.y * self.scale)
    }

    /// Returns the deck point at height `z` that projects to `screen`, as
    /// `(x, y)` in reference units, or `None` when the plane is seen edge-on
    /// or the point is behind the camera.
    pub(in crate::app) fn unproject(&self, screen: Pos2, z: f32) -> Option<Pos2> {
        self.unproject_plane(screen, z, 0.0)
    }

    /// Returns the point on the panel's surface, creased or flat, that
    /// projects to `screen`.
    pub(in crate::app) fn unproject_surface(&self, screen: Pos2) -> Option<Pos2> {
        if let Some((y, slope)) = self.crease {
            // Describe the rising plane by its height at the front edge and
            // its slope.
            let z_front = (y - self.ref_h) * slope;
            if let Some(p) = self.unproject_plane(screen, z_front, -slope) {
                if p.y < y {
                    return Some(p);
                }
            }
        }
        self.unproject(screen, 0.0)
    }

    /// Returns the point on the plane `z = z_front + m * (y - ref_h)` that
    /// projects to `screen`.
    fn unproject_plane(&self, screen: Pos2, z_front: f32, m: f32) -> Option<Pos2> {
        let c = Pos2::new(
            (screen.x - self.ox) / self.scale,
            (screen.y - self.oy) / self.scale,
        );
        let r = (c - self.fit_off) / self.fit_scale;
        let (cx, cy) = self.cam();
        // Solve the projection of the plane for `dy`, then for `x`. Both are
        // linear once the perspective divide is cleared.
        let ys = r.y - cy;
        let (sn, cs, k) = (self.sin, self.cos, self.k);
        let denom = (cs - m * sn) + ys * k * (sn + m * cs);
        if denom.abs() < 1e-6 {
            return None;
        }
        let dy = (ys - ys * k * z_front * cs - (self.ref_h - cy) + z_front * sn) / denom;
        let z = z_front + m * dy;
        let tz = dy * sn + z * cs;
        let s = 1.0 / (1.0 - tz * k);
        if s <= 0.0 {
            return None;
        }
        Some(Pos2::new(cx + (r.x - cx) / s, self.ref_h + dy))
    }
}

/// The placement of a face of the deck. The point `(u, v)` of the face's own
/// drawing, in reference units, lies at `origin + u * u_axis + v * v_axis`.
#[derive(Copy, Clone, Debug)]
pub(in crate::app) struct Face {
    pub origin: P3,
    pub u_axis: P3,
    pub v_axis: P3,
}

impl Face {
    pub(in crate::app) fn at(&self, u: f32, v: f32) -> P3 {
        let [ox, oy, oz] = self.origin;
        let [ux, uy, uz] = self.u_axis;
        let [vx, vy, vz] = self.v_axis;
        [
            ox + u * ux + v * vx,
            oy + u * uy + v * vy,
            oz + u * uz + v * vz,
        ]
    }
}

fn tessellator(ctx: &egui::Context, pixels_per_point: f32) -> Tessellator {
    let options: TessellationOptions = ctx.tessellation_options(|o| *o);
    let font_tex_size = ctx.fonts_mut(|f| f.font_image_size());
    Tessellator::new(pixels_per_point, options, font_tex_size, Vec::new())
}

/// Bisects each triangle that `split` selects along its longest side, until
/// `split` selects none, so that a texture follows the perspective and a
/// surface follows its crease.
fn subdivide(mesh: &mut Mesh, split: impl Fn([Pos2; 3]) -> bool) {
    let mut out: Vec<[Vertex; 3]> = Vec::with_capacity(mesh.indices.len() / 3);
    let mut stack: Vec<[Vertex; 3]> = mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| t.map(|i| mesh.vertices[i as usize]))
        .collect();
    let mut split_any = false;
    while let Some(tri) = stack.pop() {
        if !split(tri.map(|v| v.pos)) {
            out.push(tri);
            continue;
        }
        split_any = true;
        // Split the longest side at its middle.
        let len = |i: usize| tri[i].pos.distance(tri[(i + 1) % 3].pos);
        let i = (0..3)
            .max_by(|&a, &b| len(a).total_cmp(&len(b)))
            .unwrap_or(0);
        let (a, b, c) = (tri[i], tri[(i + 1) % 3], tri[(i + 2) % 3]);
        let m = Vertex {
            pos: a.pos.lerp(b.pos, 0.5),
            uv: a.uv.lerp(b.uv, 0.5),
            color: a.color,
        };
        stack.push([a, m, c]);
        stack.push([m, b, c]);
    }
    if split_any {
        mesh.vertices = out.iter().flatten().copied().collect();
        mesh.indices = (0..mesh.vertices.len() as u32).collect();
    }
}

/// Projected meshes kept from frame to frame, keyed by each shape's
/// tessellated mesh and its lift. While the tilt stays the same, each shape
/// costs a tessellation and a hash instead of a projection. A new tilt clears
/// the cache, and the cache fills again once the tilt is the same on two
/// consecutive frames.
#[derive(Default)]
pub(in crate::app) struct TiltCache {
    tilt: Option<Tilt>,
    /// Whether the tilt equals the previous frame's; only then are the meshes
    /// that this frame projects cached.
    steady: bool,
    meshes: std::collections::HashMap<u64, Mesh>,
    seen: std::collections::HashSet<u64>,
}

impl TiltCache {
    /// Starts a frame under `tilt`. When the tilt is unchanged, it drops the
    /// meshes that the last frame did not draw; otherwise it clears them all.
    fn begin(&mut self, tilt: &Tilt) {
        self.steady = self.tilt == Some(*tilt);
        if self.steady {
            let seen = &self.seen;
            self.meshes.retain(|k, _| seen.contains(k));
        } else {
            self.meshes.clear();
            self.tilt = Some(*tilt);
        }
        self.seen.clear();
    }
}

/// Hashes a mesh's vertices, indices and texture together with `salt`.
fn mesh_key(mesh: &Mesh, salt: &[u32]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytemuck::cast_slice::<Vertex, u8>(&mesh.vertices).hash(&mut h);
    mesh.indices.hash(&mut h);
    mesh.texture_id.hash(&mut h);
    salt.hash(&mut h);
    h.finish()
}

/// Tessellates `shape`, splits the triangles that `split` selects (it is told
/// whether the mesh is textured), and passes every vertex through `map`. With
/// a cache, a mesh already projected under `salt` is reused.
fn project_shape(
    tess: &mut Tessellator,
    shape: Shape,
    map: &impl Fn(Pos2) -> Pos2,
    split: &impl Fn([Pos2; 3], bool) -> bool,
    mut cache: Option<(&mut TiltCache, &[u32])>,
) -> Shape {
    if let Shape::Callback(_) = shape {
        return shape;
    }
    let mut mesh = Mesh::default();
    tess.tessellate_shape(shape, &mut mesh);
    let key = cache.as_ref().map(|(_, salt)| mesh_key(&mesh, salt));
    if let (Some((cache, _)), Some(key)) = (cache.as_mut(), key) {
        if let Some(hit) = cache.meshes.get(&key) {
            let hit = hit.clone();
            cache.seen.insert(key);
            return Shape::mesh(hit);
        }
    }
    let textured = mesh.texture_id != TextureId::default();
    subdivide(&mut mesh, |t| split(t, textured));
    for v in &mut mesh.vertices {
        v.pos = map(v.pos);
    }
    if let (Some((cache, _)), Some(key)) = (cache, key) {
        if cache.steady {
            cache.seen.insert(key);
            cache.meshes.insert(key, mesh.clone());
        }
    }
    Shape::mesh(mesh)
}

/// Whether a textured triangle is long on screen under `map`.
fn long_on_screen(t: [Pos2; 3], map: &impl Fn(Pos2) -> Pos2) -> bool {
    let p = t.map(map);
    (0..3).any(|i| p[i].distance(p[(i + 1) % 3]) > MAX_TEXTURED_EDGE_PX)
}

/// Replaces the shapes that `layer` painted from `start` onward, which are
/// the slate's plan in screen coordinates, with their projection under
/// `tilt`. Shapes lie on the panel's surface, except the `raised` parts,
/// which stand on their walls at their own height.
pub(in crate::app) fn project_layer(
    ctx: &egui::Context,
    layer: LayerId,
    start: ShapeIdx,
    layout: &UiScale,
    tilt: &Tilt,
    raised: &[Raised],
    wall_edge: Stroke,
    cache: &mut TiltCache,
) {
    cache.begin(tilt);
    let mut tess = tessellator(ctx, ctx.pixels_per_point());
    let clip = ctx.content_rect();
    let (ox, oy, scale) = layout.cache_key();
    let to_ref = |p: Pos2| Pos2::new((p.x - ox) / scale, (p.y - oy) / scale);
    // A raised part stands off the flat panel by its lift; the rest lies on
    // the surface, bent at its crease.
    let map = |p: Pos2, lift: Option<Lift>| {
        let r = to_ref(p);
        let z = lift.map_or_else(|| tilt.surface_z(r.y), |l| l.z(r));
        tilt.project([r.x, r.y, z])
    };
    let crease_y = tilt.crease.map(|(y, _)| y);
    ctx.graphics_mut(|g| {
        let list = g.entry(layer);
        let end = list.next_idx().0;
        for i in start.0..end {
            let part = raised.iter().find(|r| r.shapes.contains(&i));
            let lift = part.map(|r| r.lift);
            let walls = part
                .filter(|r| r.shapes.start == i)
                .map(|r| r.walls(tilt, wall_edge));
            list.mutate_shape(ShapeIdx(i), |cs| {
                let shape = std::mem::replace(&mut cs.shape, Shape::Noop);
                let at = |p: Pos2| map(p, lift);
                // Split any triangle that reaches across a line where the
                // height bends, such as the surface's crease or a cone's rims,
                // until no triangle does.
                let split = |t: [Pos2; 3], textured: bool| {
                    let across = |v: [f32; 3], at: f32| {
                        let (lo, hi) = (v[0].min(v[1]).min(v[2]), v[0].max(v[1]).max(v[2]));
                        lo < at - CREASE_SLACK
                            && at + CREASE_SLACK < hi
                            && hi - lo > MAX_CREASE_SPAN
                    };
                    let refs = t.map(to_ref);
                    let straddles = match lift {
                        Some(l) => l.creases().is_some_and(|(c, radii)| {
                            let rs = refs.map(|p| p.distance(c));
                            radii.iter().any(|&r| across(rs, r))
                        }),
                        None => crease_y.is_some_and(|cy| across(refs.map(|p| p.y), cy)),
                    };
                    straddles || (textured && long_on_screen(t, &at))
                };
                let salt = lift.map_or([u32::MAX; 6], |l| l.bits());
                let top = project_shape(&mut tess, shape, &at, &split, Some((&mut *cache, &salt)));
                cs.shape = match walls {
                    Some(mut w) => {
                        w.push(top);
                        Shape::Vec(w)
                    }
                    None => top,
                };
                cs.clip_rect = clip;
            });
        }
    });
}

/// Projects shapes drawn in `face`'s own reference units onto the screen.
/// Call it after [`project_layer`] in the same frame, because that call
/// starts the frame in `cache`.
pub(in crate::app) fn project_face(
    ctx: &egui::Context,
    layout: &UiScale,
    tilt: &Tilt,
    face: Face,
    shapes: Vec<Shape>,
    cache: &mut TiltCache,
) -> Vec<Shape> {
    let (_, _, scale) = layout.cache_key();
    // A face unit is a reference unit. Passing the physical pixels per
    // reference unit keeps the edges' feathering one pixel wide.
    let mut tess = tessellator(ctx, ctx.pixels_per_point() * scale);
    let map = |p: Pos2| tilt.project(face.at(p.x, p.y));
    let split = |t: [Pos2; 3], textured: bool| textured && long_on_screen(t, &map);
    let salt: Vec<u32> = [face.origin, face.u_axis, face.v_axis]
        .iter()
        .flatten()
        .map(|v| v.to_bits())
        .collect();
    shapes
        .into_iter()
        .map(|s| project_shape(&mut tess, s, &map, &split, Some((&mut *cache, &salt))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 2440.0;
    const H: f32 = 3626.0;

    fn tilt(t: f32) -> (UiScale, Tilt) {
        let layout = UiScale::fit(
            Rect::from_min_size(Pos2::ZERO, Vec2::new(1158.0, 1724.0)),
            W,
            H,
        );
        let extent = [
            [0.0, 0.0, 0.0],
            [W, 0.0, 0.0],
            [0.0, H, -600.0],
            [W, H, -600.0],
        ];
        let tilt = Tilt::new(
            &layout,
            W,
            H,
            t,
            0.73,
            2.5 * H,
            Some((1652.0, 0.15)),
            &extent,
        );
        (layout, tilt)
    }

    /// When the deck is flat, the projection matches the plan the slate draws.
    #[test]
    fn untilted_is_the_plan() {
        let (layout, tilt) = tilt(0.0);
        for (x, y) in [(0.0, 0.0), (W, H), (1220.0, 1800.0), (300.0, 3000.0)] {
            let p = tilt.project([x, y, 0.0]);
            assert!(p.distance(layout.sp(x, y)) < 1e-3, "{x},{y}: {p:?}");
        }
    }

    /// A pointer over a projected point maps back to that point on its plane.
    /// The tilted panel relies on this to remap input.
    #[test]
    fn unproject_inverts_project() {
        for t in [0.3, 1.0] {
            let (_, tilt) = tilt(t);
            for z in [0.0, 25.0, 150.0] {
                for (x, y) in [(10.0, 10.0), (W - 10.0, H - 10.0), (1220.0, 1800.0)] {
                    let back = tilt.unproject(tilt.project([x, y, z]), z).unwrap();
                    assert!(
                        back.distance(Pos2::new(x, y)) < 0.05,
                        "t {t} z {z}: {back:?}"
                    );
                }
            }
        }
    }
}
