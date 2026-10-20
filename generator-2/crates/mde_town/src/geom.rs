//! Plan-space geometry helpers on top of `mde_core::poly`, plus a uniform-grid
//! spatial index used for clearance tests.

use mde_core::Vec2;
use mde_core::poly::{closest_point_on_segment, cross};

/// Signed area of a ring (positive = CCW).
pub(crate) fn signed_area(pts: &[Vec2]) -> f32 {
    let n = pts.len();
    (0..n).map(|i| cross(pts[i], pts[(i + 1) % n])).sum::<f32>() * 0.5
}

/// Area centroid of a ring (vertex mean for degenerate rings).
pub(crate) fn centroid(pts: &[Vec2]) -> Vec2 {
    mde_core::Polygon(pts.to_vec()).centroid()
}

pub(crate) fn point_seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    p.distance(closest_point_on_segment(p, a, b))
}

#[inline]
fn orient(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    cross(b - a, c - a)
}

/// Proper (strict) segment intersection.
pub(crate) fn segs_cross(a1: Vec2, a2: Vec2, b1: Vec2, b2: Vec2) -> bool {
    let d1 = orient(b1, b2, a1);
    let d2 = orient(b1, b2, a2);
    let d3 = orient(a1, a2, b1);
    let d4 = orient(a1, a2, b2);
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// Intersection point of two segments that properly cross.
pub(crate) fn seg_cross_point(a1: Vec2, a2: Vec2, b1: Vec2, b2: Vec2) -> Option<Vec2> {
    if !segs_cross(a1, a2, b1, b2) {
        return None;
    }
    let r = a2 - a1;
    let s = b2 - b1;
    let d = cross(r, s);
    if d.abs() < 1e-12 {
        return None;
    }
    Some(a1 + r * (cross(b1 - a1, s) / d))
}

/// Closest points `(on a, on b)` of two segments that do not cross.
pub(crate) fn seg_seg_closest(a1: Vec2, a2: Vec2, b1: Vec2, b2: Vec2) -> (Vec2, Vec2) {
    let cands = [
        (a1, closest_point_on_segment(a1, b1, b2)),
        (a2, closest_point_on_segment(a2, b1, b2)),
        (closest_point_on_segment(b1, a1, a2), b1),
        (closest_point_on_segment(b2, a1, a2), b2),
    ];
    let mut best = cands[0];
    for c in &cands[1..] {
        if c.0.distance_squared(c.1) < best.0.distance_squared(best.1) {
            best = *c;
        }
    }
    best
}

pub(crate) fn seg_seg_dist(a1: Vec2, a2: Vec2, b1: Vec2, b2: Vec2) -> f32 {
    if segs_cross(a1, a2, b1, b2) {
        return 0.0;
    }
    let (p, q) = seg_seg_closest(a1, a2, b1, b2);
    p.distance(q)
}

/// Even-odd point in polygon.
pub(crate) fn contains(pts: &[Vec2], p: Vec2) -> bool {
    let n = pts.len();
    let mut inside = false;
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        if (a.y > p.y) != (b.y > p.y) {
            let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if p.x < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// Distance from `p` to the boundary of a ring.
pub(crate) fn boundary_dist(pts: &[Vec2], p: Vec2) -> f32 {
    let n = pts.len();
    (0..n).map(|i| point_seg_dist(p, pts[i], pts[(i + 1) % n])).fold(f32::INFINITY, f32::min)
}

/// Distance between a polygon (area) and a segment; 0 if they touch.
pub(crate) fn poly_seg_dist(pts: &[Vec2], a: Vec2, b: Vec2) -> f32 {
    if pts.len() < 3 || contains(pts, a) || contains(pts, b) {
        return 0.0;
    }
    let n = pts.len();
    (0..n).map(|i| seg_seg_dist(pts[i], pts[(i + 1) % n], a, b)).fold(f32::INFINITY, f32::min)
}

/// Closest points `(on polygon, on segment)` for a polygon and a segment that
/// do not intersect.
pub(crate) fn poly_seg_closest(pts: &[Vec2], a: Vec2, b: Vec2) -> (Vec2, Vec2) {
    let n = pts.len();
    let mut best = (pts[0], a);
    let mut bd = f32::INFINITY;
    for i in 0..n {
        let (p, q) = seg_seg_closest(pts[i], pts[(i + 1) % n], a, b);
        let d = p.distance_squared(q);
        if d < bd {
            bd = d;
            best = (p, q);
        }
    }
    best
}

/// Do two convex polygons overlap by more than `tol` (separating-axis test)?
pub(crate) fn convex_overlap(a: &[Vec2], b: &[Vec2], tol: f32) -> bool {
    for (p, q) in [(a, b), (b, a)] {
        let n = p.len();
        for i in 0..n {
            let e = p[(i + 1) % n] - p[i];
            let axis = Vec2::new(-e.y, e.x).normalize_or_zero();
            if axis == Vec2::ZERO {
                continue;
            }
            let (mut amin, mut amax) = (f32::INFINITY, f32::NEG_INFINITY);
            for v in p {
                let d = v.dot(axis);
                amin = amin.min(d);
                amax = amax.max(d);
            }
            let (mut bmin, mut bmax) = (f32::INFINITY, f32::NEG_INFINITY);
            for v in q {
                let d = v.dot(axis);
                bmin = bmin.min(d);
                bmax = bmax.max(d);
            }
            if amax <= bmin + tol || bmax <= amin + tol {
                return false;
            }
        }
    }
    true
}

/// Clip a convex ring to the half-plane `(q - p) · n >= 0`.
pub(crate) fn clip_half(pts: &[Vec2], p: Vec2, n: Vec2) -> Vec<Vec2> {
    let m = pts.len();
    let mut out = Vec::with_capacity(m + 1);
    for i in 0..m {
        let a = pts[i];
        let b = pts[(i + 1) % m];
        let sa = (a - p).dot(n);
        let sb = (b - p).dot(n);
        if sa >= 0.0 {
            out.push(a);
        }
        if (sa >= 0.0) != (sb >= 0.0) {
            let t = sa / (sa - sb);
            out.push(a + (b - a) * t);
        }
    }
    dedupe(out, 1e-4)
}

/// Exact inset of a convex CCW ring by `d` (successive half-plane clips, so
/// short edges vanish gracefully instead of flipping).
pub(crate) fn inset_convex(pts: &[Vec2], d: f32) -> Vec<Vec2> {
    let m = pts.len();
    let mut out = pts.to_vec();
    for i in 0..m {
        let a = pts[i];
        let b = pts[(i + 1) % m];
        let dir = (b - a).normalize_or_zero();
        if dir == Vec2::ZERO {
            continue;
        }
        let inward = Vec2::new(-dir.y, dir.x);
        out = clip_half(&out, a + inward * d, inward);
        if out.len() < 3 {
            return Vec::new();
        }
    }
    out
}

/// Remove consecutive (and wrap-around) points closer than `eps`.
pub(crate) fn dedupe(mut pts: Vec<Vec2>, eps: f32) -> Vec<Vec2> {
    pts.dedup_by(|a, b| a.distance(*b) < eps);
    while pts.len() > 1 && pts[0].distance(pts[pts.len() - 1]) < eps {
        pts.pop();
    }
    pts
}

/// Minimum width of a convex ring (smallest extent over its edge normals).
pub(crate) fn min_width(pts: &[Vec2]) -> f32 {
    let n = pts.len();
    let mut best = f32::INFINITY;
    for i in 0..n {
        let a = pts[i];
        let dir = (pts[(i + 1) % n] - a).normalize_or_zero();
        if dir == Vec2::ZERO {
            continue;
        }
        let nrm = Vec2::new(-dir.y, dir.x);
        let w = pts.iter().map(|p| (*p - a).dot(nrm).abs()).fold(0.0f32, f32::max);
        best = best.min(w);
    }
    if best.is_finite() { best } else { 0.0 }
}

/// Smallest interior angle of a convex ring, degrees.
pub(crate) fn min_angle_deg(pts: &[Vec2]) -> f32 {
    let n = pts.len();
    let mut best = 180.0f32;
    for i in 0..n {
        let p = pts[(i + n - 1) % n];
        let c = pts[i];
        let q = pts[(i + 1) % n];
        let u = (p - c).normalize_or_zero();
        let v = (q - c).normalize_or_zero();
        if u == Vec2::ZERO || v == Vec2::ZERO {
            continue;
        }
        best = best.min(u.dot(v).clamp(-1.0, 1.0).acos().to_degrees());
    }
    best
}

/// Remove vertices that are (nearly) collinear with their neighbours or that
/// form edges shorter than `min_edge`. For convex rings this only shrinks the
/// polygon.
pub(crate) fn tidy(pts: Vec<Vec2>, min_edge: f32) -> Vec<Vec2> {
    let mut pts = dedupe(pts, min_edge);
    let mut changed = true;
    while changed && pts.len() > 3 {
        changed = false;
        let n = pts.len();
        for i in 0..n {
            let a = pts[(i + n - 1) % n];
            let b = pts[i];
            let c = pts[(i + 1) % n];
            let ab = b - a;
            let bc = c - b;
            let turn = cross(ab.normalize_or_zero(), bc.normalize_or_zero());
            if turn.abs() < 0.02 || ab.length() < min_edge {
                pts.remove(i);
                changed = true;
                break;
            }
        }
    }
    pts
}

/// Distance from `p` to a polyline.
pub(crate) fn polyline_dist(p: Vec2, line: &[Vec2]) -> f32 {
    if line.len() == 1 {
        return p.distance(line[0]);
    }
    line.windows(2).map(|w| point_seg_dist(p, w[0], w[1])).fold(f32::INFINITY, f32::min)
}

/// Unit outward normal of the CCW edge `a → b`.
#[inline]
pub(crate) fn outward(a: Vec2, b: Vec2) -> Vec2 {
    let d = (b - a).normalize_or_zero();
    Vec2::new(d.y, -d.x)
}

/// Oriented rectangle (CCW) centred at `c` with long axis `axis` (unit).
pub(crate) fn oriented_rect(c: Vec2, axis: Vec2, len: f32, wid: f32) -> Vec<Vec2> {
    let u = axis * (len * 0.5);
    let v = Vec2::new(-axis.y, axis.x) * (wid * 0.5);
    vec![c - u - v, c + u - v, c + u + v, c - u + v]
}

/// Uniform grid of item ids over a square region.
#[derive(Clone, Debug)]
pub(crate) struct Grid {
    min: Vec2,
    cell: f32,
    nx: usize,
    ny: usize,
    buckets: Vec<Vec<u32>>,
}

impl Grid {
    pub fn new(min: Vec2, max: Vec2, cell: f32) -> Self {
        let size = (max - min).max(Vec2::splat(cell));
        let nx = ((size.x / cell).ceil() as usize).clamp(1, 2048);
        let ny = ((size.y / cell).ceil() as usize).clamp(1, 2048);
        Self { min, cell, nx, ny, buckets: vec![Vec::new(); nx * ny] }
    }

    fn range(&self, lo: Vec2, hi: Vec2) -> (usize, usize, usize, usize) {
        let f = |v: f32, n: usize| (((v) / self.cell).floor().max(0.0) as usize).min(n - 1);
        (
            f(lo.x - self.min.x, self.nx),
            f(hi.x - self.min.x, self.nx),
            f(lo.y - self.min.y, self.ny),
            f(hi.y - self.min.y, self.ny),
        )
    }

    pub fn insert(&mut self, lo: Vec2, hi: Vec2, id: u32) {
        let (x0, x1, y0, y1) = self.range(lo, hi);
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.buckets[y * self.nx + x].push(id);
            }
        }
    }

    /// Ids whose boxes may overlap `[lo, hi]`, ascending and unique.
    pub fn query(&self, lo: Vec2, hi: Vec2, out: &mut Vec<u32>) {
        out.clear();
        let (x0, x1, y0, y1) = self.range(lo, hi);
        for y in y0..=y1 {
            for x in x0..=x1 {
                out.extend_from_slice(&self.buckets[y * self.nx + x]);
            }
        }
        out.sort_unstable();
        out.dedup();
    }
}

/// Something lots and trees must keep clear of.
#[derive(Clone, Debug)]
pub(crate) enum Shape {
    /// Segment with a half-width (street, wall, river, bridge deck).
    Seg { a: Vec2, b: Vec2, r: f32 },
    /// Disc (tower, tree).
    Circle { c: Vec2, r: f32 },
    /// Solid convex polygon (building, field, plaza).
    Poly { pts: Vec<Vec2> },
}

impl Shape {
    fn bounds(&self) -> (Vec2, Vec2) {
        match self {
            Shape::Seg { a, b, r } => (a.min(*b) - Vec2::splat(*r), a.max(*b) + Vec2::splat(*r)),
            Shape::Circle { c, r } => (*c - Vec2::splat(*r), *c + Vec2::splat(*r)),
            Shape::Poly { pts } => {
                let mut lo = Vec2::splat(f32::INFINITY);
                let mut hi = Vec2::splat(f32::NEG_INFINITY);
                for p in pts {
                    lo = lo.min(*p);
                    hi = hi.max(*p);
                }
                (lo, hi)
            }
        }
    }

    /// Clearance between this shape and a disc at `p` with radius `r`
    /// (negative = overlapping).
    pub fn disc_gap(&self, p: Vec2, r: f32) -> f32 {
        match self {
            Shape::Seg { a, b, r: w } => point_seg_dist(p, *a, *b) - w - r,
            Shape::Circle { c, r: rc } => p.distance(*c) - rc - r,
            Shape::Poly { pts } => {
                if contains(pts, p) {
                    -r
                } else {
                    boundary_dist(pts, p) - r
                }
            }
        }
    }
}

/// A set of [`Shape`]s with a grid for fast neighbourhood queries.
#[derive(Clone, Debug)]
pub(crate) struct Obstacles {
    pub items: Vec<Shape>,
    grid: Grid,
    scratch: Vec<u32>,
}

impl Obstacles {
    pub fn new(half: f32) -> Self {
        let h = half + 50.0;
        Self { items: Vec::new(), grid: Grid::new(Vec2::splat(-h), Vec2::splat(h), 24.0), scratch: Vec::new() }
    }

    pub fn add(&mut self, s: Shape) {
        let (lo, hi) = s.bounds();
        let id = self.items.len() as u32;
        self.grid.insert(lo, hi, id);
        self.items.push(s);
    }

    /// Ids of shapes whose bounds may overlap `[lo, hi]`.
    pub fn near(&mut self, lo: Vec2, hi: Vec2) -> Vec<u32> {
        let mut out = std::mem::take(&mut self.scratch);
        self.grid.query(lo, hi, &mut out);
        let res = out.clone();
        self.scratch = out;
        res
    }

    /// Is a disc at `p` of radius `r` at least `gap` away from every shape?
    pub fn disc_clear(&mut self, p: Vec2, r: f32, gap: f32) -> bool {
        let e = Vec2::splat(r + gap + 0.01);
        let ids = self.near(p - e, p + e);
        ids.iter().all(|&i| self.items[i as usize].disc_gap(p, r) >= gap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inset_and_width() {
        let sq = vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 4.0), Vec2::new(0.0, 4.0)];
        let ins = inset_convex(&sq, 1.0);
        assert!((signed_area(&ins) - 16.0).abs() < 1e-3);
        assert!((min_width(&sq) - 4.0).abs() < 1e-5);
        assert!(inset_convex(&sq, 2.5).is_empty());
    }

    #[test]
    fn overlap_sat() {
        let a = oriented_rect(Vec2::ZERO, Vec2::X, 4.0, 2.0);
        let b = oriented_rect(Vec2::new(3.0, 0.0), Vec2::X, 4.0, 2.0);
        let c = oriented_rect(Vec2::new(5.0, 0.0), Vec2::X, 4.0, 2.0);
        assert!(convex_overlap(&a, &b, 0.01));
        assert!(!convex_overlap(&a, &c, 0.01));
    }
}
