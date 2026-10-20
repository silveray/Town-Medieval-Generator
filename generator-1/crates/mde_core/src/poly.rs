//! 2D polygon helpers in plan space (metres, CCW, no repeated closing vertex).

use glam::Vec2;

use crate::model::Polygon;

const EPS: f32 = 1e-6;

/// 2D cross product (z component of the 3D cross).
#[inline]
pub fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

/// Axis-aligned bounding box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }
    pub fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }
    pub fn contains(&self, p: Vec2) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }
}

impl Polygon {
    pub fn new(points: Vec<Vec2>) -> Self {
        Polygon(points)
    }

    /// Axis-aligned rectangle centred on `center`, CCW.
    pub fn rect(center: Vec2, size: Vec2) -> Self {
        let h = size * 0.5;
        Polygon(vec![
            center + Vec2::new(-h.x, -h.y),
            center + Vec2::new(h.x, -h.y),
            center + Vec2::new(h.x, h.y),
            center + Vec2::new(-h.x, h.y),
        ])
    }

    /// Regular `n`-gon, CCW, first vertex at angle `phase` (radians).
    pub fn regular(center: Vec2, radius: f32, n: usize, phase: f32) -> Self {
        Polygon(
            (0..n)
                .map(|i| {
                    let a = phase + i as f32 / n as f32 * std::f32::consts::TAU;
                    center + Vec2::new(a.cos(), a.sin()) * radius
                })
                .collect(),
        )
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Vertex `i` with wrap-around (also for negative `i`).
    pub fn at(&self, i: isize) -> Vec2 {
        let n = self.0.len() as isize;
        self.0[i.rem_euclid(n) as usize]
    }

    /// Edges as `(start, end)` pairs, including the closing edge.
    pub fn edges(&self) -> impl Iterator<Item = (Vec2, Vec2)> + '_ {
        let n = self.0.len();
        (0..n).map(move |i| (self.0[i], self.0[(i + 1) % n]))
    }

    /// Signed area: positive for CCW.
    pub fn signed_area(&self) -> f32 {
        self.edges().map(|(a, b)| cross(a, b)).sum::<f32>() * 0.5
    }

    pub fn area(&self) -> f32 {
        self.signed_area().abs()
    }

    pub fn is_ccw(&self) -> bool {
        self.signed_area() > 0.0
    }

    /// Reverse winding in place if needed so the polygon is CCW.
    pub fn make_ccw(&mut self) {
        if self.signed_area() < 0.0 {
            self.0.reverse();
        }
    }

    pub fn perimeter(&self) -> f32 {
        self.edges().map(|(a, b)| a.distance(b)).sum()
    }

    /// Area centroid (falls back to the vertex average for degenerate polygons).
    pub fn centroid(&self) -> Vec2 {
        let a = self.signed_area();
        if a.abs() < EPS {
            return self.vertex_mean();
        }
        let mut c = Vec2::ZERO;
        for (p, q) in self.edges() {
            c += (p + q) * cross(p, q);
        }
        c / (6.0 * a)
    }

    pub fn vertex_mean(&self) -> Vec2 {
        if self.0.is_empty() {
            return Vec2::ZERO;
        }
        self.0.iter().copied().sum::<Vec2>() / self.0.len() as f32
    }

    pub fn bounds(&self) -> Rect {
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for &p in &self.0 {
            min = min.min(p);
            max = max.max(p);
        }
        Rect { min, max }
    }

    /// Even-odd point-in-polygon test.
    pub fn contains(&self, p: Vec2) -> bool {
        let mut inside = false;
        for (a, b) in self.edges() {
            if (a.y > p.y) != (b.y > p.y) {
                let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
                if p.x < x {
                    inside = !inside;
                }
            }
        }
        inside
    }

    pub fn is_convex(&self) -> bool {
        let n = self.0.len();
        if n < 4 {
            return n == 3;
        }
        let mut sign = 0.0f32;
        for i in 0..n {
            let a = self.at(i as isize);
            let b = self.at(i as isize + 1);
            let c = self.at(i as isize + 2);
            let z = cross(b - a, c - b);
            if z.abs() > EPS {
                if sign == 0.0 {
                    sign = z.signum();
                } else if z.signum() != sign {
                    return false;
                }
            }
        }
        true
    }

    /// Index of the longest edge (edge `i` goes from vertex `i` to `i + 1`).
    pub fn longest_edge(&self) -> usize {
        self.edges()
            .enumerate()
            .max_by(|(_, (a, b)), (_, (c, d))| a.distance_squared(*b).total_cmp(&c.distance_squared(*d)))
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    /// Distance from `p` to the polygon boundary.
    pub fn distance_to_boundary(&self, p: Vec2) -> f32 {
        self.edges()
            .map(|(a, b)| p.distance(closest_point_on_segment(p, a, b)))
            .fold(f32::INFINITY, f32::min)
    }

    /// Offset every edge inwards by `d` (negative grows the polygon) using mitred
    /// joins. Exact for convex polygons; for concave ones it is a good
    /// approximation as long as `d` is small relative to the features. Returns
    /// `None` if the result collapses or flips.
    pub fn inset(&self, d: f32) -> Option<Polygon> {
        self.inset_edges(&vec![d; self.0.len()])
    }

    /// Like [`Polygon::inset`] but with a separate distance per edge
    /// (`d[i]` applies to edge `i → i+1`).
    pub fn inset_edges(&self, d: &[f32]) -> Option<Polygon> {
        let n = self.0.len();
        if n < 3 || d.len() != n {
            return None;
        }
        let ccw = if self.is_ccw() { 1.0 } else { -1.0 };
        // Each edge shifted along its inward normal.
        let lines: Vec<(Vec2, Vec2)> = (0..n)
            .map(|i| {
                let a = self.0[i];
                let b = self.0[(i + 1) % n];
                let dir = (b - a).normalize_or_zero();
                let inward = Vec2::new(-dir.y, dir.x) * ccw;
                (a + inward * d[i], dir)
            })
            .collect();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let (p0, d0) = lines[(i + n - 1) % n];
            let (p1, d1) = lines[i];
            let denom = cross(d0, d1);
            if denom.abs() < 1e-4 {
                // Parallel neighbours: just take the shifted vertex.
                out.push(p1);
            } else {
                let t = cross(p1 - p0, d1) / denom;
                out.push(p0 + d0 * t);
            }
        }
        // An edge that reverses direction means the offset overshot (for a convex
        // polygon this is a 180° flip that keeps the winding, so the area test
        // below would not catch it).
        for i in 0..n {
            let src = self.0[(i + 1) % n] - self.0[i];
            let dst = out[(i + 1) % n] - out[i];
            if src.dot(dst) < 0.0 {
                return None;
            }
        }
        let result = Polygon(out);
        let orig = self.signed_area();
        let new = result.signed_area();
        if new.abs() < EPS || new.signum() != orig.signum() {
            return None;
        }
        // Inset must shrink and outset must grow; otherwise it self-intersected.
        let all_positive = d.iter().all(|&x| x >= 0.0);
        let all_negative = d.iter().all(|&x| x <= 0.0);
        if (all_positive && new.abs() > orig.abs() + EPS) || (all_negative && new.abs() < orig.abs() - EPS) {
            return None;
        }
        Some(result)
    }

    /// Split a polygon by the infinite line through `p` with direction `dir`.
    /// Returns `(left, right)` parts relative to `dir`; either may be empty.
    /// Exact for convex input; for concave input each side may come back as a
    /// single (possibly self-touching) ring.
    pub fn split(&self, p: Vec2, dir: Vec2) -> (Polygon, Polygon) {
        let side = |q: Vec2| cross(dir, q - p);
        let mut left = Vec::new();
        let mut right = Vec::new();
        let n = self.0.len();
        for i in 0..n {
            let a = self.0[i];
            let b = self.0[(i + 1) % n];
            let sa = side(a);
            let sb = side(b);
            if sa >= 0.0 {
                left.push(a);
            }
            if sa <= 0.0 {
                right.push(a);
            }
            if (sa > 0.0 && sb < 0.0) || (sa < 0.0 && sb > 0.0) {
                let t = sa / (sa - sb);
                let x = a + (b - a) * t;
                left.push(x);
                right.push(x);
            }
        }
        (Polygon(left), Polygon(right))
    }

    /// Remove consecutive vertices closer than `eps` and collinear vertices.
    pub fn simplified(&self, eps: f32) -> Polygon {
        let mut pts: Vec<Vec2> = Vec::with_capacity(self.0.len());
        for &p in &self.0 {
            if pts.last().is_none_or(|q: &Vec2| q.distance(p) > eps) {
                pts.push(p);
            }
        }
        while pts.len() > 1 && pts[0].distance(*pts.last().unwrap()) <= eps {
            pts.pop();
        }
        let mut changed = true;
        while changed && pts.len() > 3 {
            changed = false;
            let n = pts.len();
            for i in 0..n {
                let a = pts[(i + n - 1) % n];
                let b = pts[i];
                let c = pts[(i + 1) % n];
                if cross(b - a, c - b).abs() <= eps * (c - a).length().max(EPS) {
                    pts.remove(i);
                    changed = true;
                    break;
                }
            }
        }
        Polygon(pts)
    }

    /// Translate every vertex by `offset`.
    pub fn translated(&self, offset: Vec2) -> Polygon {
        Polygon(self.0.iter().map(|&p| p + offset).collect())
    }

    /// Rotate every vertex by `angle` radians around `pivot`.
    pub fn rotated(&self, pivot: Vec2, angle: f32) -> Polygon {
        let r = Vec2::from_angle(angle);
        Polygon(self.0.iter().map(|&p| pivot + r.rotate(p - pivot)).collect())
    }
}

/// Closest point to `p` on segment `a–b`.
pub fn closest_point_on_segment(p: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < EPS {
        return a;
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    a + ab * t
}

/// Intersection of segments `a1–a2` and `b1–b2` (proper or touching).
pub fn segment_intersection(a1: Vec2, a2: Vec2, b1: Vec2, b2: Vec2) -> Option<Vec2> {
    let r = a2 - a1;
    let s = b2 - b1;
    let denom = cross(r, s);
    if denom.abs() < EPS {
        return None;
    }
    let t = cross(b1 - a1, s) / denom;
    let u = cross(b1 - a1, r) / denom;
    if (-EPS..=1.0 + EPS).contains(&t) && (-EPS..=1.0 + EPS).contains(&u) {
        Some(a1 + r * t)
    } else {
        None
    }
}

/// Intersection of the infinite lines through `p1` (direction `d1`) and `p2` (direction `d2`).
pub fn line_intersection(p1: Vec2, d1: Vec2, p2: Vec2, d2: Vec2) -> Option<Vec2> {
    let denom = cross(d1, d2);
    if denom.abs() < EPS {
        return None;
    }
    Some(p1 + d1 * (cross(p2 - p1, d2) / denom))
}

/// Total length of a polyline.
pub fn polyline_length(points: &[Vec2]) -> f32 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Polygon {
        Polygon::rect(Vec2::ZERO, Vec2::splat(2.0))
    }

    #[test]
    fn area_centroid_winding() {
        let s = square();
        assert!(s.is_ccw());
        assert!((s.area() - 4.0).abs() < 1e-5);
        assert!(s.centroid().length() < 1e-5);
        let mut r = Polygon(s.0.iter().rev().copied().collect());
        assert!(!r.is_ccw());
        r.make_ccw();
        assert!(r.is_ccw());
    }

    #[test]
    fn contains_and_distance() {
        let s = square();
        assert!(s.contains(Vec2::new(0.5, 0.5)));
        assert!(!s.contains(Vec2::new(1.5, 0.0)));
        assert!((s.distance_to_boundary(Vec2::ZERO) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn inset_square() {
        let s = square().inset(0.25).unwrap();
        assert!((s.area() - 2.25).abs() < 1e-4);
        assert!(square().inset(1.5).is_none());
        let grown = square().inset(-0.5).unwrap();
        assert!((grown.area() - 9.0).abs() < 1e-4);
    }

    #[test]
    fn split_square() {
        let (l, r) = square().split(Vec2::ZERO, Vec2::Y);
        assert!((l.area() - 2.0).abs() < 1e-5);
        assert!((r.area() - 2.0).abs() < 1e-5);
        assert!(l.centroid().x < 0.0 && r.centroid().x > 0.0);
    }

    #[test]
    fn simplify_removes_collinear() {
        let p = Polygon(vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(2.0, 2.0),
            Vec2::new(0.0, 2.0),
        ]);
        assert_eq!(p.simplified(1e-3).len(), 4);
    }

    #[test]
    fn segments() {
        let x = segment_intersection(Vec2::new(-1.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, -1.0), Vec2::new(0.0, 1.0));
        assert!(x.unwrap().length() < 1e-6);
        assert!(segment_intersection(Vec2::ZERO, Vec2::X, Vec2::Y, Vec2::new(1.0, 1.0)).is_none());
    }
}
