//! Roofs over a top-floor outline.
//!
//! The outline is expressed in a local frame aligned with the ridge. A
//! rectilinear outline (L, T, U, ...) is decomposed into rectangles, each roofed
//! on its own (lower wings run into taller ones up to their ridge, so the roofs
//! interpenetrate into valleys). Other convex outlines get one roof over their
//! ridge-aligned bounding rectangle, clipped to the outline plus overhang.
//!
//! Every rectangle's roof is the **lower envelope of a few planes** (two slopes
//! for a gable, plus end planes for hips, four for a gambrel, ...), so a face is
//! simply the clip region intersected with the half-planes where its plane is
//! lowest. That gives exact ridges and hips for every [`RoofKind`]. The planes
//! describe the underside; the top surface sits `thickness` above it. Gable
//! walls are "fill walls" from the wall plate up to the underside along every
//! outline edge, so any gap between walls and roof is closed.

use mde_core::poly::cross;
use mde_core::{MaterialSlot as S, MeshData, Polygon, RoofKind, RoofMaterial, Rng, Vec2, Vec3};

use crate::geom::{beam, faces, jitter, poly_facing, quad_facing, sweep_box, w3, wdir, Mb};
use crate::palette::{fixed, mix, mul, scale, Linear};

/// How much geometric detail to spend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Detail {
    /// Fine tile courses, ridge crests (dwellings).
    Full,
    /// Coarse courses (lot houses).
    Lot,
    /// No courses (budget fallback).
    Low,
}

/// Gable trim on the verges (storybook style pass, module #2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Trim {
    /// No trim: the unstyled look.
    #[default]
    None,
    /// A dark bargeboard on every exposed verge.
    Boards,
}

/// A sloped outer edge of a roof face (rake), low end first.
#[derive(Clone, Copy, Debug)]
struct Verge {
    lo: Vec2,
    hi: Vec2,
    plane: Plane,
    /// Outward unit normal (plan).
    out: Vec2,
}

/// Storybook roof curve: each slope sweeps concave (steeper near the ridge,
/// flatter towards the eaves) and flares up over the overhang, so gables end
/// in a pointed apex. The lift `c·(top − h)(base − h)/(top − eave)` depends on
/// the underside height `h` alone: zero at the ridge height `top` and at the
/// wall plate `base`, a sag between them, a flare below the plate. Faces that
/// share a hip, valley or ridge therefore lift alike, and [`Roof::bottom_at`] /
/// [`Roof::top_at`] include it, so gable walls and chimneys follow.
#[derive(Clone, Copy, Debug)]
struct Curve {
    top: f32,
    base: f32,
    eave: f32,
    c: f32,
}

/// Height step of the curve's straight pieces above the wall plate, and the
/// number of pieces over the overhang.
const CURVE_STEP: f32 = 0.35;
const FLARE_STEPS: usize = 4;

impl Curve {
    fn new(roof: &Roof, p: &RoofParams) -> Option<Curve> {
        let pitch = if p.pitch_deg.is_finite() { p.pitch_deg.clamp(5.0, 70.0) } else { 40.0 };
        let overhang = if p.overhang.is_finite() { p.overhang.clamp(0.0, 1.5) } else { 0.3 };
        let top = roof
            .pieces
            .iter()
            .filter_map(|pc| pc.ridge.map(|(a, b)| pc.h(a).max(pc.h(b))))
            .fold(f32::NEG_INFINITY, f32::max);
        if !top.is_finite() || top < roof.base + 0.5 {
            return None;
        }
        let drop = (pitch.to_radians().tan() * overhang).max(0.05);
        Some(Curve { top, base: roof.base, eave: roof.base - drop, c: 0.35 })
    }

    fn at(&self, h: f32) -> f32 {
        self.c * (self.top - h) * (self.base - h) / (self.top - self.eave)
    }

    /// Heights where the piecewise-linear curve bends, highest first.
    fn levels(&self) -> Vec<f32> {
        let n = ((self.top - self.base) / CURVE_STEP).ceil().max(1.0) as usize;
        let mut v: Vec<f32> = (0..n).map(|i| self.top - (self.top - self.base) * i as f32 / n as f32).collect();
        v.extend((0..=FLARE_STEPS).map(|i| self.base - (self.base - self.eave) * i as f32 / FLARE_STEPS as f32));
        v
    }

    /// Parameters in `(0, 1)` along `a → b` where `plane` crosses a bend height.
    fn crossings(&self, a: Vec2, b: Vec2, plane: &Plane) -> Vec<f32> {
        let (ha, hb) = (plane.h(a), plane.h(b));
        let mut ts: Vec<f32> = self
            .levels()
            .into_iter()
            .filter(|&l| (l - ha) * (l - hb) < 0.0)
            .map(|l| (l - ha) / (hb - ha))
            .collect();
        ts.sort_by(f32::total_cmp);
        ts
    }

    /// `poly` cut into strips between the bend heights.
    fn strips(&self, poly: &TPoly, plane: &Plane) -> Vec<TPoly> {
        let pl = *plane;
        let mut cuts = vec![f32::INFINITY];
        cuts.extend(self.levels());
        cuts.push(f32::NEG_INFINITY);
        cuts.windows(2)
            .filter_map(|w| {
                let (hi, lo) = (w[0], w[1]);
                let mut part = poly.clone();
                if hi.is_finite() {
                    part = clip_tagged(&part, &move |q: Vec2| hi - pl.h(q), Tag::BandHigh);
                }
                if lo.is_finite() && part.len() >= 3 {
                    part = clip_tagged(&part, &move |q: Vec2| pl.h(q) - lo, Tag::BandLow);
                }
                (part.len() >= 3).then_some(part)
            })
            .collect()
    }
}

/// Look of gable (fill) walls.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GableStyle {
    pub slot: S,
    pub col: Linear,
    /// Timber trims (king post, collar) in this colour, if any.
    pub timber: Option<Linear>,
}

/// Everything the roof builder needs.
#[derive(Clone, Debug)]
pub(crate) struct RoofParams {
    pub kind: RoofKind,
    pub pitch_deg: f32,
    pub overhang: f32,
    pub ridge: Vec2,
    /// Wall plate height (underside of the roof at the walls).
    pub base: f32,
    pub material: RoofMaterial,
    pub roof: Linear,
    pub timber: Linear,
    pub gable: GableStyle,
    /// Parapet of flat roofs: slot, colour, crenellated.
    pub parapet: (S, Linear, bool),
    pub detail: Detail,
    pub seed: u64,
    pub trim: Trim,
    /// Storybook curve: concave slopes flaring at the eaves ([`Curve`];
    /// [`Detail::Full`] only, other details stay planar).
    pub curved: bool,
    /// Lay the covering as individual, slightly uneven blocks (slates or
    /// tiles) instead of whole courses ([`Detail::Full`] only).
    pub tiles: bool,
    /// Storybook front cross gable: the plan direction towards the street.
    /// Unless the front edge is a gable end already, a steep gable rises on
    /// the left half of it, its apex at the main ridge ([`Roof::cross_gable`]).
    pub front: Option<Vec2>,
}

impl RoofParams {
    pub fn slot(&self) -> S {
        roof_slot(self.material)
    }

    fn thickness(&self) -> f32 {
        match self.material {
            RoofMaterial::Tiles => 0.14,
            RoofMaterial::Slate => 0.1,
            RoofMaterial::Shingles => 0.12,
            RoofMaterial::Thatch => 0.42,
        }
    }
}

/// Material slot of a roof covering.
pub(crate) fn roof_slot(m: RoofMaterial) -> S {
    match m {
        RoofMaterial::Tiles => S::RoofTile,
        RoofMaterial::Slate => S::Slate,
        RoofMaterial::Thatch => S::Thatch,
        RoofMaterial::Shingles => S::Planks,
    }
}

/// `h(p) = g·p + c` over plan space (underside of a roof face).
#[derive(Clone, Copy, Debug)]
struct Plane {
    g: Vec2,
    c: f32,
}

impl Plane {
    #[inline]
    fn h(&self, p: Vec2) -> f32 {
        self.g.dot(p) + self.c
    }
}

/// One roofed piece: convex clip region (plan, with overhang) and its planes.
#[derive(Clone, Debug)]
struct Piece {
    clip: Vec<Vec2>,
    planes: Vec<Plane>,
    /// Junction edges (plan) where this piece's open end needs a gable wall.
    inner_gables: Vec<(Vec2, Vec2, Vec2)>,
    /// Ridge line (plan) for chimney placement, if sloped.
    ridge: Option<(Vec2, Vec2)>,
    /// Clip edge left open (buried in a neighbour): no fascia, no bargeboard.
    open_edge: Option<usize>,
    /// Convex regions (CCW) cut out of the faces, where a cross gable takes over.
    cuts: Vec<Vec<Vec2>>,
}

/// Whether `p` lies inside the convex CCW polygon `poly` (`tol` ≥ 0 grows it).
fn in_convex(poly: &[Vec2], p: Vec2, tol: f32) -> bool {
    let n = poly.len();
    n >= 3 && (0..n).all(|i| cross(poly[(i + 1) % n] - poly[i], p - poly[i]) >= -tol)
}

impl Piece {
    fn new(clip: Vec<Vec2>, planes: Vec<Plane>, inner_gables: Vec<(Vec2, Vec2, Vec2)>, ridge: Option<(Vec2, Vec2)>) -> Piece {
        Piece { clip, planes, inner_gables, ridge, open_edge: None, cuts: Vec::new() }
    }

    fn contains(&self, p: Vec2) -> bool {
        in_convex(&self.clip, p, 1e-4) && !self.cuts.iter().any(|c| in_convex(c, p, -1e-4))
    }

    fn h(&self, p: Vec2) -> f32 {
        self.planes.iter().map(|pl| pl.h(p)).fold(f32::INFINITY, f32::min)
    }
}

/// A planned roof: geometry pieces plus the information to query its height.
pub(crate) struct Roof {
    pieces: Vec<Piece>,
    outline: Vec<Vec2>,
    base: f32,
    thickness: f32,
    flat: bool,
    curve: Option<Curve>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tag {
    Boundary,
    Shared(usize),
    BandLow,
    BandHigh,
    /// Joint between two blocks of a course.
    Cut,
    /// Edge along a cross gable's cut: a fascia but no bargeboard.
    Notch,
    /// Edge buried in a neighbouring roof: nothing drawn.
    Open,
}

type TPoly = Vec<(Vec2, Tag)>;

/// Keep the part of a tagged convex polygon where `f(p) >= 0` (`f` affine);
/// the new edge along `f = 0` gets `line_tag`.
fn clip_tagged(poly: &TPoly, f: &dyn Fn(Vec2) -> f32, line_tag: Tag) -> TPoly {
    let n = poly.len();
    let mut out: TPoly = Vec::with_capacity(n + 2);
    for i in 0..n {
        let (p, tp) = poly[i];
        let (q, _) = poly[(i + 1) % n];
        let dp = f(p);
        let dq = f(q);
        if dp >= -1e-6 {
            out.push((p, tp));
            if dq < -1e-6 && dp > 1e-6 {
                let t = dp / (dp - dq);
                out.push((p + (q - p) * t, line_tag));
            } else if dq < -1e-6 {
                // p is on the line: the edge from p leaves along the line.
                if let Some(last) = out.last_mut() {
                    last.1 = line_tag;
                }
            }
        } else if dq > 1e-6 {
            let t = dp / (dp - dq);
            out.push((p + (q - p) * t, tp));
        }
    }
    // Drop near-duplicate points (keep the later tag, which describes the next edge).
    let mut cleaned: TPoly = Vec::with_capacity(out.len());
    for (p, t) in out {
        if let Some(last) = cleaned.last_mut()
            && last.0.distance(p) < 1e-5
        {
            last.1 = t;
            continue;
        }
        cleaned.push((p, t));
    }
    while cleaned.len() > 1 && cleaned[0].0.distance(cleaned[cleaned.len() - 1].0) < 1e-5 {
        cleaned.pop();
    }
    if cleaned.len() < 3 { Vec::new() } else { cleaned }
}

fn area(p: &[Vec2]) -> f32 {
    crate::geom::signed_area(p)
}

/// Rectangle (local frame) with ridge axis and end / side flags.
#[derive(Clone, Debug)]
struct RectSpec {
    min: Vec2,
    max: Vec2,
    /// Ridge runs along local x (0) or y (1).
    axis: usize,
    /// Buried ends (no planes, no overhang): low / high end along the ridge.
    buried: [bool; 2],
    /// No overhang on side: [-x, +x, -y, +y].
    no_oh: [bool; 4],
    /// Open ends that need a gable wall at a junction: low / high.
    inner_gable: [bool; 2],
    /// Extension of buried ends into the neighbouring piece: low / high.
    ext: [f32; 2],
}

impl RectSpec {
    fn half_width(&self) -> f32 {
        (self.max[1 - self.axis] - self.min[1 - self.axis]) * 0.5
    }
}

/// Local frame: q = ((p - o)·u, (p - o)·v).
#[derive(Clone, Copy, Debug)]
struct Frame2 {
    o: Vec2,
    u: Vec2,
    v: Vec2,
}

impl Frame2 {
    fn to_local(self, p: Vec2) -> Vec2 {
        Vec2::new((p - self.o).dot(self.u), (p - self.o).dot(self.v))
    }
    fn to_plan(self, q: Vec2) -> Vec2 {
        self.o + self.u * q.x + self.v * q.y
    }
    /// Local-affine plane `gl·q + c` → plan plane.
    fn plane(self, gl: Vec2, c: f32) -> Plane {
        let g = self.u * gl.x + self.v * gl.y;
        Plane { g, c: c - g.dot(self.o) }
    }
}

fn clean_outline(outline: &Polygon) -> Option<Polygon> {
    let pts: Vec<Vec2> = outline.0.iter().copied().filter(|p| p.is_finite()).collect();
    if pts.len() < 3 {
        return None;
    }
    let mut p = Polygon(pts).simplified(1e-3);
    if p.len() < 3 || p.area() < 0.25 {
        return None;
    }
    p.make_ccw();
    Some(p)
}

/// Whether every edge of `local` is (nearly) axis aligned.
fn rectilinear(local: &[Vec2]) -> bool {
    let n = local.len();
    (0..n).all(|i| {
        let d = local[(i + 1) % n] - local[i];
        d.x.abs() <= 0.03 || d.y.abs() <= 0.03
    })
}

/// Exact cover of a rectilinear polygon by rectangles, greedily largest first.
fn decompose(local: &[Vec2]) -> Option<Vec<(Vec2, Vec2)>> {
    let merge = |mut v: Vec<f32>| {
        v.sort_by(f32::total_cmp);
        let mut out: Vec<f32> = Vec::new();
        for x in v {
            if out.last().is_none_or(|&l| x - l > 0.03) {
                out.push(x);
            }
        }
        out
    };
    let xs = merge(local.iter().map(|p| p.x).collect());
    let ys = merge(local.iter().map(|p| p.y).collect());
    let (nx, ny) = (xs.len().checked_sub(1)?, ys.len().checked_sub(1)?);
    if nx == 0 || ny == 0 || nx > 24 || ny > 24 {
        return None;
    }
    let poly = Polygon(local.to_vec());
    let mut inside = vec![false; nx * ny];
    for i in 0..nx {
        for j in 0..ny {
            inside[i * ny + j] = poly.contains(Vec2::new((xs[i] + xs[i + 1]) * 0.5, (ys[j] + ys[j + 1]) * 0.5));
        }
    }
    let mut covered = vec![false; nx * ny];
    let mut rects = Vec::new();
    loop {
        let mut best: Option<(f32, usize, usize, usize, usize)> = None;
        for i0 in 0..nx {
            for j0 in 0..ny {
                if !inside[i0 * ny + j0] || covered[i0 * ny + j0] {
                    continue;
                }
                for i1 in i0..nx {
                    if !inside[i1 * ny + j0] || covered[i1 * ny + j0] {
                        break;
                    }
                    for j1 in j0..ny {
                        let ok = (i0..=i1).all(|i| inside[i * ny + j1] && !covered[i * ny + j1]);
                        if !ok {
                            break;
                        }
                        let (w, h) = (xs[i1 + 1] - xs[i0], ys[j1 + 1] - ys[j0]);
                        // Area, with a slight preference for pieces running along
                        // the ridge (local x) so the main roof follows it.
                        let a = w * h * if w >= h { 1.05 } else { 1.0 };
                        if best.is_none_or(|b| a > b.0 + 1e-6) {
                            best = Some((a, i0, i1, j0, j1));
                        }
                    }
                }
            }
        }
        let Some((_, i0, i1, j0, j1)) = best else { break };
        for i in i0..=i1 {
            for j in j0..=j1 {
                covered[i * ny + j] = true;
            }
        }
        rects.push((Vec2::new(xs[i0], ys[j0]), Vec2::new(xs[i1 + 1], ys[j1 + 1])));
        if rects.len() > 6 {
            return None;
        }
    }
    (!rects.is_empty()).then_some(rects)
}

/// Shared boundary between two rectangles: (side of `a`: 0=-x,1=+x,2=-y,3=+y, overlap length).
fn touching(a: &(Vec2, Vec2), b: &(Vec2, Vec2)) -> Option<(usize, f32)> {
    let eps = 0.03;
    let ov_y = a.1.y.min(b.1.y) - a.0.y.max(b.0.y);
    let ov_x = a.1.x.min(b.1.x) - a.0.x.max(b.0.x);
    if (a.0.x - b.1.x).abs() < eps && ov_y > 0.1 {
        return Some((0, ov_y));
    }
    if (a.1.x - b.0.x).abs() < eps && ov_y > 0.1 {
        return Some((1, ov_y));
    }
    if (a.0.y - b.1.y).abs() < eps && ov_x > 0.1 {
        return Some((2, ov_x));
    }
    if (a.1.y - b.0.y).abs() < eps && ov_x > 0.1 {
        return Some((3, ov_x));
    }
    None
}

impl Roof {
    /// Plan the roof over `outline`. Returns `None` for degenerate outlines.
    pub fn plan(outline: &Polygon, p: &RoofParams) -> Option<Roof> {
        let mut roof = Self::plan_planar(outline, p)?;
        if p.curved && p.detail == Detail::Full && !roof.flat {
            roof.curve = Curve::new(&roof, p);
        }
        Some(roof)
    }

    fn plan_planar(outline: &Polygon, p: &RoofParams) -> Option<Roof> {
        let outline = clean_outline(outline)?;
        let pitch = if p.pitch_deg.is_finite() { p.pitch_deg.clamp(5.0, 70.0) } else { 40.0 };
        let t = pitch.to_radians().tan();
        let overhang = if p.overhang.is_finite() { p.overhang.clamp(0.0, 1.5) } else { 0.3 };
        let base = if p.base.is_finite() { p.base } else { 0.0 };
        let longest = {
            let i = outline.longest_edge();
            (outline.at(i as isize + 1) - outline.at(i as isize)).normalize_or(Vec2::X)
        };
        let ridge = if p.ridge.is_finite() && p.ridge.length_squared() > 1e-6 { p.ridge.normalize() } else { longest };
        let centre = outline.centroid();
        let mk = |u: Vec2| Frame2 { o: centre, u, v: Vec2::new(-u.y, u.x) };
        let mut frame = mk(ridge);
        let mut local: Vec<Vec2> = outline.0.iter().map(|&q| frame.to_local(q)).collect();
        if !rectilinear(&local) {
            let alt = mk(longest);
            let alt_local: Vec<Vec2> = outline.0.iter().map(|&q| alt.to_local(q)).collect();
            if rectilinear(&alt_local) {
                frame = alt;
                local = alt_local;
            }
        }
        let flat = p.kind == RoofKind::Flat;
        let thickness = if flat { 0.25 } else { p.thickness() };
        let mut roof = Roof { pieces: Vec::new(), outline: outline.0.clone(), base, thickness, flat, curve: None };
        if flat {
            return Some(roof);
        }
        let ridge_axis_hint = 0usize; // local x is the ridge direction
        let convex = outline.is_convex();
        if p.kind == RoofKind::Tent && convex {
            roof.pieces.push(pyramid(&outline, overhang, t, base));
            return Some(roof);
        }
        let rects = if rectilinear(&local) { decompose(&local) } else { None };
        match rects {
            Some(rects) if rects.len() > 1 => {
                let specs = junction_specs(&rects, ridge_axis_hint, t);
                let kind = if p.kind == RoofKind::Tent { RoofKind::Hip } else { p.kind };
                for s in &specs {
                    roof.pieces.extend(rect_pieces(&frame, s, kind, t, overhang, base, None));
                }
            }
            other => {
                // Single rectangle: the ridge-aligned bounds.
                let (mut min, mut max) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
                for q in &local {
                    min = min.min(*q);
                    max = max.max(*q);
                }
                let axis = if max.x - min.x >= (max.y - min.y) * 0.6 { 0 } else { 1 };
                let spec = RectSpec { min, max, axis, buried: [false; 2], no_oh: [false; 4], inner_gable: [false; 2], ext: [0.0; 2] };
                // Exact rectangle: clip to the expanded rect; otherwise to the
                // outline grown by the overhang (convex) or the bounds.
                let clip = if other.is_some() {
                    None
                } else if convex {
                    outline.inset(-overhang).map(|g| g.0)
                } else {
                    None
                };
                roof.pieces.extend(rect_pieces(&frame, &spec, p.kind, t, overhang, base, clip));
            }
        }
        if let Some(front) = p.front
            && p.kind != RoofKind::Tent
        {
            roof.cross_gable(front, t, overhang);
        }
        Some(roof)
    }

    /// Planar underside height at `p` (highest covering piece), before any curve.
    fn planar_at(&self, p: Vec2) -> Option<f32> {
        self.pieces.iter().filter(|pc| pc.contains(p)).map(|pc| pc.h(p)).fold(None, |a: Option<f32>, h| Some(a.map_or(h, |x| x.max(h))))
    }

    /// Storybook front cross gable (style pass #4). On the outline edge facing
    /// `front`, unless it is a gable end already, add a steep gable wing over
    /// the left half (as seen from the street), running back to the highest
    /// roof line behind it. Its pitch brings its apex up to that ridge (at
    /// most 74°); its side overhang is cut so its eaves meet the main eaves'
    /// height. The main roof's eave strip under the wing is cut away, so the
    /// wing's gable wall (the outline fill wall) rises straight from the facade.
    fn cross_gable(&mut self, front: Vec2, t: f32, overhang: f32) {
        let front = if front.is_finite() { front.normalize_or_zero() } else { Vec2::ZERO };
        let n = self.outline.len();
        if front == Vec2::ZERO || n < 3 || self.pieces.is_empty() {
            return;
        }
        let outline = Polygon(self.outline.clone());
        // Street-facing, then long edges.
        let edge = |i: usize| {
            let (a, b) = (self.outline[i], self.outline[(i + 1) % n]);
            let d = (b - a).normalize_or(Vec2::X);
            (a, d, a.distance(b), Vec2::new(d.y, -d.x))
        };
        let Some(i) = (0..n)
            .filter(|&i| edge(i).3.dot(front) > 0.7)
            .max_by(|&i, &j| {
                let score = |k: usize| edge(k).3.dot(front) * 10.0 + edge(k).2.min(12.0) * 0.1;
                score(i).total_cmp(&score(j))
            })
        else {
            return;
        };
        let (a, d, len, out) = edge(i);
        // Wing across the left half, kept off the corner (its side eaves too).
        let margin = (len * 0.05).max(overhang + 0.1);
        let mut w = (len * 0.45).clamp(2.2, 5.5).min(len - 2.0 * margin);
        if w < 1.8 {
            return;
        }
        // Depth: back along the centre line to the highest roof line, while
        // the wing's centre and both sides stay over the house.
        let lateral = |w: f32| (margin + w * 0.5).min(len - margin - w * 0.5);
        let inside = |c: f32, x: f32| outline.contains(a + d * c - out * x);
        let reach = |c: f32| (1..400).map(|k| k as f32 * 0.1).take_while(|&x| inside(c, x)).last().unwrap_or(0.0);
        let c = lateral(w);
        let limit = reach(c - w * 0.5 + 0.05).min(reach(c)).min(reach(c + w * 0.5 - 0.05));
        let best = |xs: &mut dyn Iterator<Item = f32>| {
            xs.filter_map(|x| self.planar_at(a + d * c - out * x).map(|h| (x, h)))
                .fold((0.0f32, self.base), |acc, (x, h)| if h > acc.1 + 1e-4 { (x, h) } else { acc })
        };
        let (coarse, _) = best(&mut (1..=(limit / 0.1) as usize).map(|k| k as f32 * 0.1));
        // Refine around the best sample (1 cm).
        let (depth, top) = best(&mut (-10..=10).map(|k| coarse + k as f32 * 0.01).filter(|&x| x > 0.0 && x <= limit));
        let rise = top - self.base;
        if rise < 0.8 || depth < 1.0 {
            return;
        }
        // A gable end already faces the street (an eave on a skewed front
        // rises a little along it, a gable end most of the way to the ridge).
        let gable_end = (0..=16).any(|k| {
            let q = a + d * (len * k as f32 / 16.0) - out * 0.05;
            self.planar_at(q).is_some_and(|h| h > self.base + 0.5 * rise)
        });
        if gable_end {
            return;
        }
        // Widen up to the steepest pitch if the ridge is high; the apex then
        // still reaches it.
        let steepest = 74f32.to_radians().tan();
        w = w.max(2.0 * rise / steepest).min(len - 2.0 * margin);
        let c = lateral(w);
        let tw = (rise / (w * 0.5)).clamp(t, steepest);
        let side_oh = overhang * t / tw;
        let half = w * 0.5 + side_oh;
        // Local frame: x from the wall line into the house, y across (CCW).
        let u = -out;
        let frame = Frame2 { o: a + d * c, u, v: Vec2::new(-u.y, u.x) };
        // A little lower than the main roof, as for junction wings.
        let wb = self.base - 0.02;
        let planes = [1.0f32, -1.0]
            .iter()
            .map(|&sgn| frame.plane(Vec2::new(0.0, -tw * sgn), wb + tw * w * 0.5))
            .collect();
        let rect = |x0: f32, x1: f32, y: f32| {
            [Vec2::new(x1, -y), Vec2::new(x1, y), Vec2::new(x0, y), Vec2::new(x0, -y)].iter().map(|&q| frame.to_plan(q)).collect::<Vec<_>>()
        };
        // Main roofs lose their eave strip under the wing (first cut edge on the
        // wall line, so the split seams stay inside the strip).
        let cut = rect(-(overhang + 1.0), 0.0, half);
        for pc in &mut self.pieces {
            pc.cuts.push(cut.clone());
        }
        // Wing clip; edge 0 (at the ridge) is buried in the main roof.
        let mut wing = Piece::new(rect(-overhang, depth, half), planes, Vec::new(), None);
        wing.open_edge = Some(0);
        self.pieces.push(wing);
    }

    /// Height of the roof's underside above plan point `p` (highest of the
    /// pieces covering it), or `None` outside the roof.
    pub fn bottom_at(&self, p: Vec2) -> Option<f32> {
        if self.flat {
            return Polygon(self.outline.clone()).contains(p).then_some(self.base);
        }
        self.pieces
            .iter()
            .filter(|pc| pc.contains(p))
            .map(|pc| pc.h(p))
            .fold(None, |acc: Option<f32>, h| Some(acc.map_or(h, |a| a.max(h))))
            .map(|h| h + self.lift(h))
    }

    /// The storybook curve's lift at underside height `h` (0 when planar).
    fn lift(&self, h: f32) -> f32 {
        self.curve.map_or(0.0, |c| c.at(h))
    }

    /// Height of the roof's top surface above `p`.
    pub fn top_at(&self, p: Vec2) -> Option<f32> {
        self.bottom_at(p).map(|h| h + self.thickness)
    }

    /// Ridge segments (plan) of the sloped pieces, longest first.
    pub fn ridges(&self) -> Vec<(Vec2, Vec2)> {
        let mut r: Vec<(Vec2, Vec2)> = self.pieces.iter().filter_map(|p| p.ridge).collect();
        r.sort_by(|a, b| b.0.distance(b.1).total_cmp(&a.0.distance(a.1)));
        r
    }

    /// Emit the roof (covering, underside, caps), the gable fill walls and,
    /// for flat roofs, the slab and parapet.
    pub fn build(&self, mb: &mut Mb, p: &RoofParams) {
        let mut rng = Rng::new(p.seed ^ 0x005E_ED0F_200F);
        if self.flat {
            self.build_flat(mb, p, &mut rng);
            return;
        }
        let slot = p.slot();
        let pitch = if p.pitch_deg.is_finite() { p.pitch_deg.clamp(5.0, 70.0) } else { 40.0 };
        let tiles = p.tiles && p.detail == Detail::Full && p.material != RoofMaterial::Thatch;
        let (course_len, step) = match (p.material, p.detail) {
            (_, Detail::Low) => (0.0, 0.0),
            // Chunky blocks: deeper courses, thick butts.
            (RoofMaterial::Slate, Detail::Full) if tiles => (0.34, 0.045),
            (_, Detail::Full) if tiles => (0.3, 0.045),
            (RoofMaterial::Thatch, Detail::Full) => (0.55, 0.05),
            (RoofMaterial::Thatch, _) => (1.0, 0.06),
            (RoofMaterial::Tiles, Detail::Full) => (0.3, 0.032),
            (RoofMaterial::Slate, Detail::Full) => (0.26, 0.016),
            (RoofMaterial::Shingles, Detail::Full) => (0.3, 0.026),
            (RoofMaterial::Tiles, Detail::Lot) => (0.55, 0.035),
            (RoofMaterial::Slate, Detail::Lot) => (0.5, 0.02),
            (RoofMaterial::Shingles, Detail::Lot) => (0.55, 0.03),
        };
        let dh = course_len * pitch.to_radians().sin();
        let ref_h = self.base - 3.0;
        let curve = self.curve;
        // Thatch wraps over its verges: no bargeboards.
        let boards = p.trim != Trim::None && p.material != RoofMaterial::Thatch;
        let mut verges: Vec<Verge> = Vec::new();
        for (pi, piece) in self.pieces.iter().enumerate() {
            let mut piece_rng = rng.fork(pi as u64);
            for (i, plane) in piece.planes.iter().enumerate() {
                let mut poly: TPoly =
                    piece.clip.iter().enumerate().map(|(k, &q)| (q, if piece.open_edge == Some(k) { Tag::Open } else { Tag::Boundary })).collect();
                for (j, other) in piece.planes.iter().enumerate() {
                    if j == i {
                        continue;
                    }
                    let (a, b) = (*plane, *other);
                    poly = clip_tagged(&poly, &move |q: Vec2| b.h(q) - a.h(q), Tag::Shared(j));
                    if poly.is_empty() {
                        break;
                    }
                }
                if poly.len() < 3 || area(&poly.iter().map(|x| x.0).collect::<Vec<_>>()).abs() < 1e-4 {
                    continue;
                }
                if boards {
                    collect_verges(&poly, plane, &mut verges);
                }
                for part in subtract(poly, &piece.cuts) {
                    self.face(mb, p, slot, plane, &part, i, dh, ref_h, step, curve, tiles, &mut piece_rng);
                }
            }
        }
        // Fill (gable) walls along every outline edge, plus inner gables.
        let n = self.outline.len();
        for i in 0..n {
            let a = self.outline[i];
            let b = self.outline[(i + 1) % n];
            let d = (b - a).normalize_or(Vec2::X);
            let out = Vec2::new(d.y, -d.x);
            self.fill_wall(mb, p, a, b, out, &|q| self.bottom_at(q), &mut rng);
        }
        for piece in &self.pieces {
            for &(a, b, out) in &piece.inner_gables {
                self.fill_wall(mb, p, a, b, out, &|q| Some(piece.h(q) + self.lift(piece.h(q))), &mut rng);
            }
        }
        if boards {
            self.trim(mb, p, &verges, curve);
        }
    }

    /// A bargeboard on every exposed verge. The boards follow the roof height
    /// sampled along each verge, so the storybook curve bends them too.
    fn trim(&self, mb: &mut Mb, p: &RoofParams, verges: &[Verge], curve: Option<Curve>) {
        let th = self.thickness;
        let lite = p.detail != Detail::Full;
        let col = scale(p.timber, 0.8);
        // Board depth (across the slope) and thickness; its top stands proud of the covering.
        let (depth, thick, proud) = (0.3, 0.08, 0.06);
        let lift = |h: f32| curve.map_or(0.0, |c| c.at(h));
        for v in verges {
            let top = |q: Vec2| {
                let h = v.plane.h(q);
                w3(q, h + th + lift(h))
            };
            let mid = (v.lo + v.hi) * 0.5;
            // Skip verges inside a taller roof (wings running into a neighbour).
            let own = v.plane.h(mid);
            if self.top_at(mid).is_some_and(|h| h > own + lift(own) + th + 0.05) {
                continue;
            }
            let t = (top(v.hi) - top(v.lo)).normalize_or(Vec3::Y);
            let up = (Vec3::Y - t * t.y).normalize_or(Vec3::Y);
            let side = wdir(v.out);
            // Centre line: overlapping the covering's edge by 1.5 cm.
            let centre = |q: Vec2| top(q) + up * (proud - depth * 0.5) + side * (thick * 0.5 - 0.015);
            let n = if lite { 1 } else { (v.lo.distance(v.hi) / 0.6).ceil().max(1.0) as usize };
            // Every ~0.6 m, plus the curve's bends so the board follows it.
            let mut ts: Vec<f32> = (0..=n).map(|i| i as f32 / n as f32).collect();
            if let Some(c) = curve {
                ts.extend(c.crossings(v.lo, v.hi, &v.plane));
                ts.sort_by(f32::total_cmp);
                ts.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
            }
            let path: Vec<Vec3> = ts.iter().map(|&t| centre(v.lo + (v.hi - v.lo) * t)).collect();
            sweep_box(mb.m(S::Timber), &path, thick, depth, col);
            if !lite {
                // Squared foot block hanging below the eave end.
                let along = wdir((v.hi - v.lo).normalize_or(Vec2::X));
                let c = top(v.lo) + side * (thick * 0.5 - 0.015) + along * 0.12;
                beam(mb.m(S::Timber), c + Vec3::Y * 0.08, c - Vec3::Y * (depth + 0.25), along, thick + 0.03, 0.24, scale(col, 0.9), 0);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn face(
        &self,
        mb: &mut Mb,
        p: &RoofParams,
        slot: S,
        plane: &Plane,
        poly: &TPoly,
        index: usize,
        dh: f32,
        ref_h: f32,
        step: f32,
        curve: Option<Curve>,
        tiles: bool,
        rng: &mut Rng,
    ) {
        let th = self.thickness;
        let up_n = Vec3::new(-plane.g.x, 1.0, plane.g.y).normalize();
        let lift_of = |h: f32| curve.map_or(0.0, |c| c.at(h));
        let bottom = |q: Vec2| {
            let h = plane.h(q);
            w3(q, h + lift_of(h))
        };
        let top = |q: Vec2| {
            let h = plane.h(q);
            w3(q, h + th + lift_of(h))
        };
        let pts: Vec<Vec2> = poly.iter().map(|x| x.0).collect();
        // Underside (in strips over a curve, so it bends with the covering).
        let under_col = jitter(scale(p.timber, 0.8), rng, 0.04);
        let parts = match curve {
            Some(c) => c.strips(poly, plane),
            None => vec![poly.clone()],
        };
        for part in &parts {
            let under: Vec<Vec3> = part.iter().map(|x| bottom(x.0)).collect();
            poly_facing(mb.m(S::Timber), &under, -up_n, under_col);
        }
        // Edges.
        let n = poly.len();
        for k in 0..n {
            let (a, tag) = poly[k];
            let b = poly[(k + 1) % n].0;
            if a.distance(b) < 1e-4 {
                continue;
            }
            let d = (b - a).normalize();
            let out2 = Vec2::new(d.y, -d.x);
            match tag {
                Tag::Boundary | Tag::Notch => {
                    // Split at the curve's bends so the edge follows it.
                    let mut cuts = vec![a];
                    if let Some(c) = curve {
                        cuts.extend(c.crossings(a, b, plane).into_iter().map(|t| a + (b - a) * t));
                    }
                    cuts.push(b);
                    for w in cuts.windows(2) {
                        let (a, b) = (w[0], w[1]);
                        if p.material == RoofMaterial::Thatch {
                            thatch_roll(mb.m(slot), bottom(a), bottom(b), th, wdir(out2), scale(p.roof, 0.82));
                        } else {
                            quad_facing(mb.m(slot), bottom(a), bottom(b), top(b), top(a), wdir(out2), scale(p.roof, 0.7));
                        }
                    }
                }
                Tag::Shared(j) if j > index => {
                    // Hips bend with the curve: one cap piece per bend, all in
                    // the colour a single cap would get.
                    let mut cuts = vec![a];
                    if let Some(c) = curve {
                        cuts.extend(c.crossings(a, b, plane).into_iter().map(|t| a + (b - a) * t));
                    }
                    cuts.push(b);
                    let start = rng.clone();
                    for w in cuts.windows(2) {
                        *rng = start.clone();
                        ridge_cap(mb, p, slot, top(w[0]), top(w[1]), step, rng);
                    }
                }
                _ => {}
            }
        }
        // Covering: flat polygon or courses.
        let slope = plane.g.length();
        if dh <= 1e-4 || slope < 0.05 {
            let tp: Vec<Vec3> = pts.iter().map(|&q| top(q)).collect();
            poly_facing(mb.m(slot), &tp, up_n, jitter(p.roof, rng, 0.05));
            return;
        }
        let hs: Vec<f32> = pts.iter().map(|&q| plane.h(q)).collect();
        let hmin = hs.iter().copied().fold(f32::INFINITY, f32::min);
        let hmax = hs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let k0 = ((hmin - ref_h) / dh).floor() as i64;
        let k1 = ((hmax - ref_h) / dh).ceil() as i64;
        if k1 - k0 > 400 {
            return;
        }
        let downslope = wdir(-plane.g / slope);
        for k in k0..k1 {
            let ylo = ref_h + k as f32 * dh;
            let yhi = ylo + dh;
            let pl = *plane;
            let mut band = clip_tagged(poly, &move |q: Vec2| pl.h(q) - ylo, Tag::BandLow);
            if band.is_empty() {
                continue;
            }
            band = clip_tagged(&band, &move |q: Vec2| yhi - pl.h(q), Tag::BandHigh);
            if band.len() < 3 {
                continue;
            }
            // Per-course colour: random plus a gentle alternation.
            let tone = match (p.material, k % 2 == 0) {
                (_, true) => 1.0,
                (RoofMaterial::Thatch, false) => 0.9,
                _ => 0.95,
            };
            let course_col = jitter(scale(p.roof, tone), rng, match p.material {
                RoofMaterial::Shingles => 0.1,
                RoofMaterial::Thatch => 0.06,
                _ => 0.07,
            });
            // The course as one piece, or as blocks of random width with
            // staggered joints, each with its own tint, height and tilt.
            let along = Vec2::new(-plane.g.y, plane.g.x) / slope;
            let mut blocks: Vec<(TPoly, Linear, f32, f32, f32)> = Vec::new();
            if tiles {
                let (s0, s1) = band.iter().map(|x| x.0.dot(along)).fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)));
                let (w0, w1) = match p.material {
                    RoofMaterial::Slate => (0.5, 0.9),
                    RoofMaterial::Tiles => (0.35, 0.55),
                    _ => (0.3, 0.5),
                };
                let mut a = s0 - rng.range_f32(0.0, w1);
                while a < s1 {
                    let b = a + rng.range_f32(w0, w1);
                    let (ca, cb) = (a, b);
                    a = b;
                    if cb <= s0 {
                        continue;
                    }
                    let mut tile = clip_tagged(&band, &move |q: Vec2| q.dot(along) - ca, Tag::Cut);
                    if tile.len() >= 3 {
                        tile = clip_tagged(&tile, &move |q: Vec2| cb - q.dot(along), Tag::Cut);
                    }
                    if tile.len() < 3 {
                        continue;
                    }
                    let col = jitter(course_col, rng, 0.09);
                    let raise = rng.range_f32(-0.2, 0.6) * step;
                    let tilt = rng.range_f32(-0.5, 0.5) * step / (cb - ca);
                    blocks.push((tile, col, raise, tilt, (ca + cb) * 0.5));
                }
            } else {
                blocks.push((band, course_col, 0.0, 0.0, 0.0));
            }
            for (block, col, raise, tilt, mid) in &blocks {
                let lift = |q: Vec2| {
                    let mut d = ((yhi - plane.h(q)) / dh).clamp(0.0, 1.0) * step;
                    if tiles {
                        d += raise + tilt * (q.dot(along) - mid);
                    }
                    top(q) + up_n * d
                };
                let tp: Vec<Vec3> = block.iter().map(|x| lift(x.0)).collect();
                poly_facing(mb.m(slot), &tp, up_n, *col);
                if step <= 0.0 {
                    continue;
                }
                let m = block.len();
                for e in 0..m {
                    let (a, tag) = block[e];
                    let b = block[(e + 1) % m].0;
                    if a.distance(b) < 1e-4 {
                        continue;
                    }
                    match tag {
                        Tag::BandLow => {
                            quad_facing(mb.m(slot), top(a), top(b), lift(b), lift(a), downslope - Vec3::Y * 0.2, scale(*col, 0.62));
                        }
                        Tag::BandHigh => {}
                        _ => {
                            let d = (b - a).normalize();
                            let out2 = Vec2::new(d.y, -d.x);
                            let pts = [top(a), top(b), lift(b), lift(a)];
                            poly_facing(mb.m(slot), &pts, wdir(out2), scale(*col, 0.7));
                        }
                    }
                }
            }
        }
    }

    /// Wall from the wall plate up to the roof underside along `a → b`.
    #[allow(clippy::too_many_arguments)]
    fn fill_wall(
        &self,
        mb: &mut Mb,
        p: &RoofParams,
        a: Vec2,
        b: Vec2,
        out: Vec2,
        field: &dyn Fn(Vec2) -> Option<f32>,
        rng: &mut Rng,
    ) {
        let len = a.distance(b);
        if len < 0.05 {
            return;
        }
        // Candidate parameters: plane crossings of every piece plus a fine grid.
        let mut ts: Vec<f32> = vec![0.0, 1.0];
        let samples = (len / 0.25).ceil() as usize;
        for k in 1..samples {
            ts.push(k as f32 / samples as f32);
        }
        for pc in &self.pieces {
            for i in 0..pc.planes.len() {
                for j in i + 1..pc.planes.len() {
                    let (pi, pj) = (pc.planes[i], pc.planes[j]);
                    let fa = pi.h(a) - pj.h(a);
                    let fb = pi.h(b) - pj.h(b);
                    if (fa > 0.0) != (fb > 0.0) && (fa - fb).abs() > 1e-9 {
                        ts.push(fa / (fa - fb));
                    }
                }
            }
        }
        ts.sort_by(f32::total_cmp);
        ts.dedup_by(|x, y| (*x - *y).abs() < 1e-5);
        let base = self.base;
        let hts: Vec<(f32, f32)> = ts
            .iter()
            .map(|&t| {
                let q = a + (b - a) * t;
                // Nudge inwards so the point is inside the clip of its piece.
                let qi = q - out * 0.01;
                (t, field(qi).unwrap_or(base).max(base))
            })
            .collect();
        // Runs where the roof is above the plate.
        let eps = 0.01;
        let mut runs: Vec<Vec<(f32, f32)>> = Vec::new();
        let mut cur: Vec<(f32, f32)> = Vec::new();
        for w in 0..hts.len() {
            let (t, h) = hts[w];
            let above = h > base + eps;
            if above {
                if cur.is_empty() && w > 0 {
                    let (t0, h0) = hts[w - 1];
                    let s = ((base + eps - h0) / (h - h0)).clamp(0.0, 1.0);
                    cur.push((t0 + (t - t0) * s, base));
                }
                cur.push((t, h));
            } else if !cur.is_empty() {
                let (t0, h0) = hts[w - 1];
                let s = ((h0 - base - eps) / (h0 - h)).clamp(0.0, 1.0);
                cur.push((t0 + (t - t0) * s, base));
                runs.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            runs.push(cur);
        }
        let facing = wdir(out);
        for run in runs {
            if run.len() < 2 {
                continue;
            }
            let (t_first, t_last) = (run[0].0, run[run.len() - 1].0);
            let mut pts: Vec<Vec3> = vec![w3(a + (b - a) * t_first, base), w3(a + (b - a) * t_last, base)];
            for &(t, h) in run.iter().rev() {
                pts.push(w3(a + (b - a) * t, h.max(base)));
            }
            pts.dedup_by(|x, y| x.distance(*y) < 1e-4);
            poly_facing(mb.m(p.gable.slot), &pts, facing, jitter(p.gable.col, rng, 0.03));
            // Timber trims: king post at the apex and a collar beam.
            if let Some(tc) = p.gable.timber {
                let (ta, ha) = run.iter().copied().fold((0.0, f32::NEG_INFINITY), |acc, x| if x.1 > acc.1 { x } else { acc });
                let rise = ha - base;
                if rise > 1.0 {
                    let apex = a + (b - a) * ta;
                    let dir = (b - a).normalize_or(Vec2::X);
                    let fo = crate::walls::WallFrame { origin: apex, dir, out };
                    let m = mb.m(S::Timber);
                    let c = jitter(tc, rng, 0.06);
                    fo.block(m, -0.09, 0.09, base + 0.18, ha - 0.05, 0.0, 0.045, c, faces::NZ);
                    let yc = base + rise * 0.42;
                    // Collar spans where the roof is above it.
                    let inside: Vec<f32> =
                        run.iter().filter(|x| x.1 > yc + 0.2).map(|x| (x.0 - ta) * len).collect();
                    if let (Some(&s0), Some(&s1)) = (inside.first(), inside.last())
                        && s1 - s0 > 0.6
                    {
                        fo.block(m, s0 + 0.1, s1 - 0.1, yc - 0.09, yc + 0.09, 0.0, 0.04, c, faces::NZ);
                    }
                    // Sill beam under the gable. With a cross gable the run also
                    // takes in the eave a nudge above the plate on either side,
                    // where the beam would poke through the roof: stop there.
                    let (mut s0, mut s1) = (run[0].0, t_last);
                    if p.front.is_some() {
                        let ia = run.iter().position(|x| x.0 >= ta).unwrap_or(0);
                        let lo = run[..=ia].iter().rposition(|x| x.1 <= base + 0.03).unwrap_or(0);
                        let hi = run[ia..].iter().position(|x| x.1 <= base + 0.03).map_or(run.len() - 1, |i| ia + i);
                        (s0, s1) = (run[lo].0, run[hi].0);
                    }
                    fo.block(m, (s0 - ta) * len, (s1 - ta) * len, base, base + 0.18, 0.0, 0.05, scale(c, 0.95), faces::NZ);
                }
            }
        }
    }

    fn build_flat(&self, mb: &mut Mb, p: &RoofParams, rng: &mut Rng) {
        let outline = &self.outline;
        let slot = p.slot();
        let y0 = self.base;
        let y1 = self.base + self.thickness;
        // Underside only: the parapet's outer face closes the sides.
        let under: Vec<Vec3> = outline.iter().map(|&q| w3(q, y0)).collect();
        poly_facing(mb.m(S::Timber), &under, -Vec3::Y, scale(p.timber, 0.7));
        // Lead / flagstone terrace: the roof colour, muted towards grey.
        let top: Vec<Vec3> = outline.iter().map(|&q| w3(q, y1)).collect();
        let lead = mix(scale(p.roof, 0.85), crate::palette::lin(0.55, 0.55, 0.56), 0.6);
        poly_facing(mb.m(slot), &top, Vec3::Y, jitter(lead, rng, 0.04));
        // Parapet: a mitred ring inside the outline, a slightly wider coping on
        // top and merlons along the outer edge, kept clear of the corners.
        let (pslot, pcol, crenel) = p.parapet;
        let pt = 0.32;
        let ph = 0.75;
        let poly = Polygon(outline.clone());
        let (Some(inner), Some(cope_out), Some(cope_in)) = (poly.inset(pt), poly.inset(-0.04), poly.inset(pt + 0.04)) else {
            return;
        };
        let m = mb.m(pslot);
        crate::geom::ring_wall(m, outline, &inner.0, y0, y1 + ph, jitter(pcol, rng, 0.04), false, false);
        crate::geom::ring_wall(m, &cope_out.0, &cope_in.0, y1 + ph, y1 + ph + 0.08, scale(pcol, 1.05), true, true);
        if crenel {
            let n = outline.len();
            let perimeter: f32 = (0..n).map(|i| outline[i].distance(outline[(i + 1) % n])).sum();
            let mw = 0.55;
            let gap = (perimeter / 60.0 - mw).max(0.45);
            for i in 0..n {
                let a = outline[i];
                let b = outline[(i + 1) % n];
                let len = a.distance(b);
                let free = len - 2.0 * (pt + 0.1);
                if free < mw {
                    continue;
                }
                let d = (b - a) / len;
                let f = crate::walls::WallFrame { origin: a, dir: d, out: Vec2::new(d.y, -d.x) };
                let count = ((free + gap) / (mw + gap)).floor().max(1.0) as usize;
                let used = count as f32 * mw + (count - 1) as f32 * gap;
                let start = (len - used) * 0.5;
                for k in 0..count {
                    let s0 = start + k as f32 * (mw + gap);
                    f.block(m, s0, s0 + mw, y1 + ph + 0.08, y1 + ph + 0.6, -pt + 0.02, 0.02, jitter(pcol, rng, 0.05), faces::NY);
                }
            }
        }
    }
}

/// `poly` minus the convex CCW regions `cuts`, as convex parts. The pieces
/// split off along a cut's edge lines get [`Tag::Notch`] there, except along
/// its first edge (the wall line), a seam inside the covering ([`Tag::Cut`]).
fn subtract(poly: TPoly, cuts: &[Vec<Vec2>]) -> Vec<TPoly> {
    let mut parts = vec![poly];
    for cut in cuts {
        let mut next = Vec::new();
        for mut rest in parts {
            let m = cut.len();
            for k in 0..m {
                let (c0, c1) = (cut[k], cut[(k + 1) % m]);
                let tag = if k == 0 { Tag::Cut } else { Tag::Notch };
                let outside = clip_tagged(&rest, &move |q: Vec2| -cross(c1 - c0, q - c0), tag);
                if outside.len() >= 3 {
                    next.push(outside);
                }
                rest = clip_tagged(&rest, &move |q: Vec2| cross(c1 - c0, q - c0), tag);
                if rest.len() < 3 {
                    break;
                }
            }
        }
        parts = next;
    }
    parts
}

/// Collect the sloped [`Tag::Boundary`] edges (verges) of a roof face.
fn collect_verges(poly: &TPoly, plane: &Plane, out: &mut Vec<Verge>) {
    let n = poly.len();
    for k in 0..n {
        let (a, tag) = poly[k];
        let b = poly[(k + 1) % n].0;
        let len = a.distance(b);
        if tag != Tag::Boundary || len < 0.3 {
            continue;
        }
        let (ha, hb) = (plane.h(a), plane.h(b));
        // Level (or nearly level) boundary edges are eaves.
        if (hb - ha).abs() < 0.3 * len {
            continue;
        }
        let d = (b - a) / len;
        let (lo, hi) = if ha <= hb { (a, b) } else { (b, a) };
        out.push(Verge { lo, hi, plane: *plane, out: Vec2::new(d.y, -d.x) });
    }
}

/// Underside planes and clip regions of one ridge-aligned rectangle: the main
/// piece (with overhangs) plus, for buried ends, an extension piece running
/// into the neighbouring roof without any overhang.
fn rect_pieces(frame: &Frame2, s: &RectSpec, kind: RoofKind, t: f32, oh: f32, base: f32, clip: Option<Vec<Vec2>>) -> Vec<Piece> {
    let ax = s.axis;
    let ac = 1 - ax;
    let w = s.half_width().max(0.05);
    let a_mid = (s.min[ac] + s.max[ac]) * 0.5;
    let (l0, l1) = (s.min[ax], s.max[ax]);
    let lg = |gl_along: f32, gl_across: f32| {
        let mut g = Vec2::ZERO;
        g[ax] = gl_along;
        g[ac] = gl_across;
        g
    };
    let mut planes = Vec::new();
    // Wings that run into a neighbour sit a little lower, so where their slopes
    // coincide with the neighbour's (e.g. a wing flush with a hipped end) the
    // neighbour's surface wins instead of z-fighting.
    let base = if s.buried.iter().any(|&b| b) { base - 0.02 } else { base };
    // Side slopes: h = base + t*(W - s*(a - a_mid)).
    let side = |tt: f32, sgn: f32, rise0: f32, w_eff: f32| frame.plane(lg(0.0, -tt * sgn), base + rise0 + tt * w_eff + tt * sgn * a_mid);
    let ridge_h = match kind {
        RoofKind::Gambrel => {
            let tl = (t.atan() + 25f32.to_radians()).min(72f32.to_radians()).tan();
            let tu = (t.atan() - 18f32.to_radians()).max(16f32.to_radians()).tan();
            let k = 0.4 * w;
            for sgn in [1.0, -1.0] {
                planes.push(side(tl, sgn, 0.0, w));
                planes.push(side(tu, sgn, tl * k, w - k));
            }
            tl * k + tu * (w - k)
        }
        _ => {
            for sgn in [1.0, -1.0] {
                planes.push(side(t, sgn, 0.0, w));
            }
            t * w
        }
    };
    // End planes.
    let hip_end = |end: usize, tt: f32, rise0: f32| {
        if end == 0 {
            frame.plane(lg(tt, 0.0), base + rise0 - tt * l0)
        } else {
            frame.plane(lg(-tt, 0.0), base + rise0 + tt * l1)
        }
    };
    for end in 0..2 {
        if s.buried[end] {
            continue;
        }
        match kind {
            RoofKind::Hip | RoofKind::Tent => planes.push(hip_end(end, t, 0.0)),
            RoofKind::HalfHip => {
                let th = (t.atan() + 18f32.to_radians()).min(78f32.to_radians()).tan();
                planes.push(hip_end(end, th, ridge_h * 0.58));
            }
            _ => {}
        }
    }
    // Clip region.
    let ohs = |side: usize, end_buried: bool| if s.no_oh[side] || end_buried { 0.0 } else { oh };
    let mut mn = s.min;
    let mut mx = s.max;
    // sides: 0=-x,1=+x,2=-y,3=+y; buried ends along the ridge axis.
    let buried_side = |side: usize| -> bool {
        let axis_of_side = side / 2;
        axis_of_side == ax && s.buried[side % 2]
    };
    mn.x -= ohs(0, buried_side(0));
    mx.x += ohs(1, buried_side(1));
    mn.y -= ohs(2, buried_side(2));
    mx.y += ohs(3, buried_side(3));
    let clip = clip.unwrap_or_else(|| {
        vec![
            frame.to_plan(Vec2::new(mn.x, mn.y)),
            frame.to_plan(Vec2::new(mx.x, mn.y)),
            frame.to_plan(Vec2::new(mx.x, mx.y)),
            frame.to_plan(Vec2::new(mn.x, mx.y)),
        ]
    });
    // Ridge line (for chimneys).
    let ridge = {
        let mut r0 = Vec2::ZERO;
        let mut r1 = Vec2::ZERO;
        r0[ax] = l0 + if matches!(kind, RoofKind::Hip | RoofKind::Tent) && !s.buried[0] { w.min((l1 - l0) * 0.5) } else { 0.0 };
        r1[ax] = l1 - if matches!(kind, RoofKind::Hip | RoofKind::Tent) && !s.buried[1] { w.min((l1 - l0) * 0.5) } else { 0.0 };
        r0[ac] = a_mid;
        r1[ac] = a_mid;
        Some((frame.to_plan(r0), frame.to_plan(r1)))
    };
    // Gable walls at open junction ends.
    let mut inner_gables = Vec::new();
    for end in 0..2 {
        if !s.inner_gable[end] {
            continue;
        }
        let l = if end == 0 { l0 } else { l1 };
        let mut a = Vec2::ZERO;
        let mut b = Vec2::ZERO;
        a[ax] = l;
        b[ax] = l;
        a[ac] = s.min[ac];
        b[ac] = s.max[ac];
        let mut outward = Vec2::ZERO;
        outward[ax] = if end == 0 { -1.0 } else { 1.0 };
        let o = frame.u * outward.x + frame.v * outward.y;
        inner_gables.push((frame.to_plan(a), frame.to_plan(b), o));
    }
    let mut out = Vec::with_capacity(3);
    for end in 0..2 {
        if s.ext[end] <= 1e-3 {
            continue;
        }
        let (e0, e1) = if end == 0 { (l0 - s.ext[0], l0) } else { (l1, l1 + s.ext[1]) };
        let corner = |along: f32, across: f32| {
            let mut q = Vec2::ZERO;
            q[ax] = along;
            q[ac] = across;
            frame.to_plan(q)
        };
        // Slightly narrower than the wing so its eaves stay behind the
        // neighbour's walls (no fascia showing through a gable).
        let inset = (0.15f32).min((s.max[ac] - s.min[ac]) * 0.2);
        let (c0, c1) = (s.min[ac] + inset, s.max[ac] - inset);
        let mut ring = vec![corner(e0, c0), corner(e1, c0), corner(e1, c1), corner(e0, c1)];
        if area(&ring) < 0.0 {
            ring.reverse();
        }
        out.push(Piece::new(ring, planes.clone(), Vec::new(), None));
    }
    out.insert(0, Piece::new(clip, planes, inner_gables, ridge));
    out
}

/// Pyramid over a convex outline: one plane per edge through the apex.
fn pyramid(outline: &Polygon, oh: f32, t: f32, base: f32) -> Piece {
    let apex = outline.centroid();
    let n = outline.len();
    let mut dmin = f32::INFINITY;
    let mut lines = Vec::with_capacity(n);
    for i in 0..n {
        let a = outline.0[i];
        let b = outline.0[(i + 1) % n];
        let d = (b - a).normalize_or(Vec2::X);
        let inward = Vec2::new(-d.y, d.x);
        let dist = (apex - a).dot(inward).max(0.05);
        dmin = dmin.min(dist);
        lines.push((a, inward, dist));
    }
    let height = t * dmin;
    let planes = lines
        .iter()
        .map(|&(a, inward, dist)| {
            // h = base + height * ((p - a)·inward) / dist
            let g = inward * (height / dist);
            Plane { g, c: base - g.dot(a) }
        })
        .collect();
    let clip = outline.inset(-oh).map(|g| g.0).unwrap_or_else(|| outline.0.clone());
    Piece::new(clip, planes, Vec::new(), Some((apex, apex)))
}

/// Ridge axes, junction extensions and overhang flags for a decomposition.
fn junction_specs(rects: &[(Vec2, Vec2)], _hint: usize, t: f32) -> Vec<RectSpec> {
    let n = rects.len();
    let mut specs: Vec<RectSpec> = rects
        .iter()
        .map(|&(min, max)| RectSpec { min, max, axis: 0, buried: [false; 2], no_oh: [false; 4], inner_gable: [false; 2], ext: [0.0; 2] })
        .collect();
    // Ridge axes: the largest rect along its longer side; the others
    // perpendicular to their junction with an earlier (bigger) rect.
    for i in 0..n {
        let (min, max) = rects[i];
        let longer = if max.x - min.x >= max.y - min.y { 0 } else { 1 };
        specs[i].axis = longer;
        if i > 0 {
            for j in 0..i {
                if let Some((side, _)) = touching(&rects[i], &rects[j]) {
                    specs[i].axis = side / 2;
                    break;
                }
            }
        }
    }
    let height = |s: &RectSpec| s.half_width() * t;
    let original = specs.clone();
    for i in 0..n {
        for j in 0..n {
            if i == j {
                continue;
            }
            let Some((side, _)) = touching(&rects[i], &rects[j]) else { continue };
            let (hi, hj) = (height(&original[i]), height(&original[j]));
            let lower = hi < hj - 1e-3 || ((hi - hj).abs() <= 1e-3 && i > j);
            let side_axis = side / 2;
            let end = side % 2;
            if lower {
                if side_axis == original[i].axis {
                    // Our ridge runs into the neighbour: extend to its ridge line.
                    let jr = &original[j];
                    let reach = if jr.axis == side_axis {
                        // Neighbour's ridge is parallel to ours: stop halfway.
                        (jr.max[side_axis] - jr.min[side_axis]) * 0.5
                    } else {
                        jr.half_width()
                    };
                    let reach = reach.min(jr.max[side_axis] - jr.min[side_axis]);
                    specs[i].ext[end] = specs[i].ext[end].max(reach);
                    specs[i].buried[end] = true;
                } else {
                    specs[i].no_oh[side] = true;
                }
            } else if side_axis == original[i].axis {
                // Taller roof's end faces a lower wing: close it with a gable
                // wall and drop the overhang there.
                specs[i].inner_gable[end] = true;
                specs[i].no_oh[side] = true;
            }
        }
    }
    specs
}

/// Ridge / hip cap along a shared top edge.
fn ridge_cap(mb: &mut Mb, p: &RoofParams, slot: S, a: Vec3, b: Vec3, step: f32, rng: &mut Rng) {
    let d = b - a;
    let len = d.length();
    if len < 0.05 {
        return;
    }
    let dir = d / len;
    // Run a little past both ends so the cap's end faces never lie in a verge.
    let (a, b) = (a - dir * 0.03, b + dir * 0.03);
    let d = b - a;
    let len = d.length();
    let mut side = dir.cross(Vec3::Y);
    if side.length_squared() < 1e-6 {
        side = Vec3::X;
    }
    let side = side.normalize();
    let col = jitter(scale(p.roof, 0.78), rng, 0.04);
    let m = mb.m(slot);
    match p.material {
        RoofMaterial::Thatch => {
            // Rounded roll: hexagonal prism.
            let r = 0.24;
            let ring: Vec<Vec3> = (0..6)
                .map(|k| {
                    let ang = k as f32 / 6.0 * std::f32::consts::TAU;
                    side * (ang.cos() * r) + Vec3::Y * (ang.sin() * r * 0.8 + 0.02)
                })
                .collect();
            for k in 0..6 {
                let (u, v) = (ring[k], ring[(k + 1) % 6]);
                quad_facing(m, a + u, b + u, b + v, a + v, (u + v) * 0.5, col);
            }
            let ca: Vec<Vec3> = ring.iter().map(|&u| a + u).collect();
            let cb: Vec<Vec3> = ring.iter().map(|&u| b + u).collect();
            poly_facing(m, &ca, -dir, col);
            poly_facing(m, &cb, dir, col);
        }
        _ => {
            let w = match p.material {
                RoofMaterial::Slate => 0.16,
                _ => 0.22,
            };
            let hgt = match p.material {
                RoofMaterial::Slate => 0.06,
                RoofMaterial::Shingles => 0.07,
                _ => 0.1,
            } + step;
            let lift = Vec3::Y * (step * 0.5);
            let l = side * (w * 0.5) - Vec3::Y * 0.03 + lift;
            let r = -side * (w * 0.5) - Vec3::Y * 0.03 + lift;
            let top = Vec3::Y * hgt + lift;
            let mid_l = side * (w * 0.32) + Vec3::Y * (hgt * 0.75) + lift;
            let mid_r = -side * (w * 0.32) + Vec3::Y * (hgt * 0.75) + lift;
            let prof = [l, mid_l, top, mid_r, r];
            for k in 0..prof.len() - 1 {
                let (u, v) = (prof[k], prof[k + 1]);
                let facing = ((u + v) * 0.5 - Vec3::Y * 0.0).normalize_or(Vec3::Y);
                quad_facing(m, a + u, b + u, b + v, a + v, facing, col);
            }
            let ca: Vec<Vec3> = prof.iter().map(|&u| a + u).collect();
            let cb: Vec<Vec3> = prof.iter().map(|&u| b + u).collect();
            poly_facing(m, &ca, -dir, col);
            poly_facing(m, &cb, dir, col);
            // Crest: little raised tiles along level ridges (tiles, full detail).
            if p.material == RoofMaterial::Tiles && p.detail == Detail::Full && dir.y.abs() < 0.05 {
                let count = (len / 0.55).floor() as usize;
                for k in 1..count {
                    let c = a + d * (k as f32 / count as f32) + top;
                    let half = side * 0.05 + dir * 0.06;
                    let pts = [c - half, c + dir * 0.06 - side * 0.05, c + half, c - dir * 0.06 + side * 0.05];
                    let _ = pts;
                    let h = Vec3::Y * 0.09;
                    let q0 = c - dir * 0.07 - side * 0.035;
                    let q1 = c + dir * 0.07 - side * 0.035;
                    let q2 = c + dir * 0.07 + side * 0.035;
                    let q3 = c - dir * 0.07 + side * 0.035;
                    quad_facing(m, q0, q1, q1 + h, q0 + h, -side, col);
                    quad_facing(m, q2, q3, q3 + h, q2 + h, side, col);
                    quad_facing(m, q0 + h, q1 + h, q2 + h, q3 + h, Vec3::Y, col);
                    quad_facing(m, q3, q0, q0 + h, q3 + h, -dir, col);
                    quad_facing(m, q1, q2, q2 + h, q1 + h, dir, col);
                }
            }
        }
    }
}

/// Rounded thatch eave: a half-roll bulging outwards between the underside
/// edge `a → b` and the top edge `th` above.
fn thatch_roll(m: &mut MeshData, a: Vec3, b: Vec3, th: f32, out: Vec3, col: Linear) {
    let segs = 4;
    let r = th * 0.42;
    let prof: Vec<Vec3> = (0..=segs)
        .map(|k| {
            let phi = std::f32::consts::PI * k as f32 / segs as f32;
            Vec3::Y * (th * 0.5 - th * 0.5 * phi.cos()) + out * (r * phi.sin())
        })
        .collect();
    for k in 0..segs {
        let (u, v) = (prof[k], prof[k + 1]);
        let mid = (u + v) * 0.5 - Vec3::Y * (th * 0.5);
        quad_facing(m, a + u, b + u, b + v, a + v, mid.normalize_or(out), col);
    }
    // Close the ends of the roll.
    let ca: Vec<Vec3> = prof.iter().map(|&u| a + u).collect();
    let cb: Vec<Vec3> = prof.iter().map(|&u| b + u).collect();
    let d = (b - a).normalize_or(Vec3::X);
    poly_facing(m, &ca, -d, col);
    poly_facing(m, &cb, d, col);
}

/// Stone chimney stack through the roof at plan point `pos`, aligned with `dir`.
/// Returns false if the point is not under the roof.
#[allow(clippy::too_many_arguments)]
pub(crate) fn chimney(
    mb: &mut Mb,
    roof: &Roof,
    pos: Vec2,
    dir: Vec2,
    stone: Linear,
    brick_top: bool,
    rng: &mut Rng,
) -> bool {
    let dir = dir.normalize_or(Vec2::X);
    let side = Vec2::new(-dir.y, dir.x);
    let (hx, hz) = (0.45, 0.35);
    let corners = [pos - dir * hx - side * hz, pos + dir * hx - side * hz, pos + dir * hx + side * hz, pos - dir * hx + side * hz];
    let mut top = f32::NEG_INFINITY;
    for &c in &corners {
        match roof.top_at(c) {
            Some(h) => top = top.max(h),
            None => return false,
        }
    }
    let top = top.max(roof.top_at(pos).unwrap_or(top)) + 0.75;
    let y0 = roof.base;
    let m = mb.m(S::Stone);
    let c = jitter(stone, rng, 0.05);
    let shaft_top = if brick_top { top - 0.4 } else { top };
    crate::geom::prism(m, &corners, y0, shaft_top, c, false, false);
    // Masonry courses: a slightly proud band every ~0.6 m.
    let band_out = |d: f32| [pos - dir * (hx + d) - side * (hz + d), pos + dir * (hx + d) - side * (hz + d), pos + dir * (hx + d) + side * (hz + d), pos - dir * (hx + d) + side * (hz + d)];
    if brick_top {
        let brick = mul(stone, fixed::brick());
        crate::geom::prism(m, &corners, shaft_top, top, jitter(brick, rng, 0.05), false, false);
        crate::geom::prism(m, &band_out(0.04), shaft_top - 0.08, shaft_top, scale(c, 1.05), true, true);
    }
    crate::geom::prism(m, &band_out(0.07), top, top + 0.1, mix(c, fixed::dressed_stone(), 0.4), true, true);
    // Flue: dark recess on the cap.
    let flue = [pos - dir * 0.2 - side * 0.14, pos + dir * 0.2 - side * 0.14, pos + dir * 0.2 + side * 0.14, pos - dir * 0.2 + side * 0.14];
    let fp: Vec<Vec3> = flue.iter().map(|&q| w3(q, top + 0.101)).collect();
    poly_facing(m, &fp, Vec3::Y, fixed::soot());
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::WHITE;

    fn params(kind: RoofKind) -> RoofParams {
        RoofParams {
            kind,
            pitch_deg: 40.0,
            overhang: 0.4,
            ridge: Vec2::X,
            base: 5.0,
            material: RoofMaterial::Tiles,
            roof: WHITE,
            timber: WHITE,
            gable: GableStyle { slot: S::Plaster, col: WHITE, timber: Some(WHITE) },
            parapet: (S::Stone, WHITE, true),
            detail: Detail::Full,
            seed: 1,
            trim: Trim::None,
            curved: false,
            tiles: false,
            front: None,
        }
    }

    #[test]
    fn decomposes_l_shape() {
        let l = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 6.0),
            Vec2::new(4.0, 6.0),
            Vec2::new(4.0, 11.0),
            Vec2::new(0.0, 11.0),
        ];
        let r = decompose(&l).unwrap();
        assert_eq!(r.len(), 2);
        let total: f32 = r.iter().map(|(a, b)| (b.x - a.x) * (b.y - a.y)).sum();
        assert!((total - Polygon(l).area()).abs() < 1e-3);
    }

    #[test]
    fn gable_heights() {
        let outline = Polygon::rect(Vec2::new(5.0, 3.0), Vec2::new(10.0, 6.0));
        let roof = Roof::plan(&outline, &params(RoofKind::Gable)).unwrap();
        // Ridge at y = 3: 3 m half-width × tan 40°.
        let h = roof.bottom_at(Vec2::new(5.0, 3.0)).unwrap();
        assert!((h - (5.0 + 3.0 * 40f32.to_radians().tan())).abs() < 1e-3, "{h}");
        assert!((roof.bottom_at(Vec2::new(5.0, 0.0)).unwrap() - 5.0).abs() < 1e-3);
        let hip = Roof::plan(&outline, &params(RoofKind::Hip)).unwrap();
        assert!((hip.bottom_at(Vec2::new(0.0, 3.0)).unwrap() - 5.0).abs() < 1e-3);
    }

    /// Highest underside along the outline edge facing `front`, and the
    /// highest underside anywhere over the outline (the main ridge).
    fn front_and_ridge(roof: &Roof, outline: &Polygon, front: Vec2) -> (f32, f32) {
        let n = outline.len();
        let (mut fill, mut ridge) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        for i in 0..n {
            let (a, b) = (outline.0[i], outline.0[(i + 1) % n]);
            let d = (b - a).normalize();
            let out = Vec2::new(d.y, -d.x);
            for k in 0..=2000 {
                let q = a + (b - a) * (k as f32 / 2000.0) - out * 0.02;
                let h = roof.bottom_at(q).unwrap_or(f32::NEG_INFINITY);
                if out.dot(front) > 0.7 {
                    fill = fill.max(h);
                }
            }
        }
        let (min, max) = outline.0.iter().fold((Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)), |(a, b), &q| (a.min(q), b.max(q)));
        for i in 0..=60 {
            for j in 0..=60 {
                let q = min + (max - min) * Vec2::new(i as f32 / 60.0, j as f32 / 60.0);
                if outline.contains(q) {
                    ridge = ridge.max(roof.bottom_at(q).unwrap_or(f32::NEG_INFINITY));
                }
            }
        }
        (fill, ridge)
    }

    fn build_ok(roof: &Roof, p: &RoofParams) {
        let mut mb = Mb::new();
        roof.build(&mut mb, p);
        let m = mb.into_model("roof");
        assert!(m.triangle_count() > 0);
        for part in &m.parts {
            assert!(part.mesh.positions.iter().all(|v| v.iter().all(|c| c.is_finite())));
        }
    }

    #[test]
    fn cross_gable_on_any_footprint() {
        let base = 5.0;
        let rect = Polygon::rect(Vec2::ZERO, Vec2::new(10.0, 7.0));
        let l = Polygon(vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(12.0, 0.0),
            Vec2::new(12.0, 7.0),
            Vec2::new(5.0, 7.0),
            Vec2::new(5.0, 12.0),
            Vec2::new(0.0, 12.0),
        ]);
        let trapezoid = Polygon(vec![Vec2::new(-5.0, -3.5), Vec2::new(5.0, -3.5), Vec2::new(4.0, 3.8), Vec2::new(-4.4, 3.2)]);
        let pentagon = Polygon(vec![Vec2::new(-5.0, -3.0), Vec2::new(5.5, -3.4), Vec2::new(6.0, 1.5), Vec2::new(0.5, 4.5), Vec2::new(-5.4, 2.0)]);
        let mut rng = Rng::new(9);
        let mut cases = vec![(rect, -Vec2::Y), (l, -Vec2::Y), (trapezoid, -Vec2::Y), (pentagon, -Vec2::Y)];
        // Random convex quads, roughly 8–12 × 6–8 m, slightly skewed.
        for _ in 0..12 {
            let (w, h) = (rng.range_f32(8.0, 12.0), rng.range_f32(6.0, 8.0));
            let j = |rng: &mut Rng| Vec2::new(rng.range_f32(-0.6, 0.6), rng.range_f32(-0.6, 0.6));
            let pts = vec![
                Vec2::new(-w * 0.5, -h * 0.5) + j(&mut rng),
                Vec2::new(w * 0.5, -h * 0.5) + j(&mut rng),
                Vec2::new(w * 0.5, h * 0.5) + j(&mut rng),
                Vec2::new(-w * 0.5, h * 0.5) + j(&mut rng),
            ];
            cases.push((Polygon(pts), -Vec2::Y));
        }
        for (i, (outline, front)) in cases.into_iter().enumerate() {
            for pitch in [52.0, 60.0, 68.0] {
                let mut p = params(RoofKind::Gable);
                (p.pitch_deg, p.overhang, p.base, p.trim, p.ridge) = (pitch, 0.6, base, Trim::Boards, Vec2::X);
                let plain = Roof::plan(&outline, &p).unwrap();
                p.front = Some(front);
                let roof = Roof::plan(&outline, &p).unwrap();
                assert_eq!(roof.pieces.len(), plain.pieces.len() + 1, "case {i} pitch {pitch}: no wing");
                let (fill, ridge) = front_and_ridge(&roof, &outline, front);
                let (plain_fill, plain_ridge) = front_and_ridge(&plain, &outline, front);
                // The front edge was an eave; now a gable rises on it, up to
                // the main ridge.
                assert!(plain_fill < base + 0.5 * (plain_ridge - base), "case {i}: front was a gable end");
                assert!((fill - ridge).abs() < 0.1, "case {i} pitch {pitch}: apex {fill} vs ridge {ridge}");
                build_ok(&roof, &p);
                p.curved = true;
                build_ok(&Roof::plan(&outline, &p).unwrap(), &p);
            }
        }
    }

    #[test]
    fn no_cross_gable_on_gable_ends() {
        let outline = Polygon::rect(Vec2::ZERO, Vec2::new(10.0, 7.0));
        let mut p = params(RoofKind::Gable);
        p.front = Some(Vec2::X);
        let roof = Roof::plan(&outline, &p).unwrap();
        assert_eq!(roof.pieces.len(), 1);
        // A narrow front is too small for a wing.
        let narrow = Polygon::rect(Vec2::ZERO, Vec2::new(2.5, 2.0));
        p.front = Some(-Vec2::Y);
        assert_eq!(Roof::plan(&narrow, &p).unwrap().pieces.len(), 1);
    }
}
