//! Occupancy grid for walkability checks: floor area minus walls, stairs and
//! furniture, sampled at a fixed resolution, with breadth-first reachability.

use mde_core::Vec2;

/// Oriented box: centre, unit axis `u` (second axis is `perp(u)`) and half extents.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obb {
    pub center: Vec2,
    pub u: Vec2,
    pub half: Vec2,
}

impl Obb {
    /// Axis-aligned box from min / max corners.
    pub fn from_aabb(min: Vec2, max: Vec2) -> Obb {
        Obb { center: (min + max) * 0.5, u: Vec2::X, half: ((max - min) * 0.5).abs() }
    }

    /// Box around segment `a–b` with half width `half_width` across it and the
    /// ends extended by `extend`.
    pub fn from_segment(a: Vec2, b: Vec2, half_width: f32, extend: f32) -> Obb {
        let d = b - a;
        let len = d.length();
        let u = if len > 1e-6 { d / len } else { Vec2::X };
        Obb { center: (a + b) * 0.5, u, half: Vec2::new(len * 0.5 + extend, half_width) }
    }

    /// Box of a prop: `size` is (width along local x, depth along local y) and
    /// `rotation` turns local axes CCW.
    pub fn from_prop(pos: Vec2, size: Vec2, rotation: f32) -> Obb {
        Obb { center: pos, u: Vec2::from_angle(rotation), half: size.abs() * 0.5 }
    }

    /// Grow (or shrink, for negative `d`) every side by `d`.
    pub fn inflate(&self, d: f32) -> Obb {
        Obb { half: (self.half + Vec2::splat(d)).max(Vec2::ZERO), ..*self }
    }

    fn v(&self) -> Vec2 {
        Vec2::new(-self.u.y, self.u.x)
    }

    /// Point containment (boundary counts as inside).
    pub fn contains(&self, p: Vec2) -> bool {
        let d = p - self.center;
        d.dot(self.u).abs() <= self.half.x && d.dot(self.v()).abs() <= self.half.y
    }

    /// The four corners, CCW.
    pub fn corners(&self) -> [Vec2; 4] {
        let ux = self.u * self.half.x;
        let vy = self.v() * self.half.y;
        [self.center - ux - vy, self.center + ux - vy, self.center + ux + vy, self.center - ux + vy]
    }

    /// Axis-aligned bounds.
    pub fn aabb(&self) -> (Vec2, Vec2) {
        let c = self.corners();
        let mut min = c[0];
        let mut max = c[0];
        for p in &c[1..] {
            min = min.min(*p);
            max = max.max(*p);
        }
        (min, max)
    }

    fn axis_aligned(&self) -> bool {
        self.u.x.abs() < 1e-6 || self.u.y.abs() < 1e-6
    }

    /// Bounds of an axis-aligned box without computing corners.
    fn aligned_bounds(&self) -> (Vec2, Vec2) {
        let h = if self.u.x.abs() > 0.5 { self.half } else { Vec2::new(self.half.y, self.half.x) };
        (self.center - h, self.center + h)
    }

    /// Separating-axis overlap test. Boxes overlap only if they interpenetrate
    /// by more than `eps` along every axis (negative `eps` demands a gap).
    pub fn overlaps(&self, o: &Obb, eps: f32) -> bool {
        if self.axis_aligned() && o.axis_aligned() {
            let (a0, a1) = self.aligned_bounds();
            let (b0, b1) = o.aligned_bounds();
            return !(a1.x <= b0.x + eps || b1.x <= a0.x + eps || a1.y <= b0.y + eps || b1.y <= a0.y + eps);
        }
        let axes = [self.u, self.v(), o.u, o.v()];
        let ca = self.corners();
        let cb = o.corners();
        for ax in axes {
            let (mut a0, mut a1) = (f32::INFINITY, f32::NEG_INFINITY);
            let (mut b0, mut b1) = (f32::INFINITY, f32::NEG_INFINITY);
            for p in &ca {
                let t = p.dot(ax);
                a0 = a0.min(t);
                a1 = a1.max(t);
            }
            for p in &cb {
                let t = p.dot(ax);
                b0 = b0.min(t);
                b1 = b1.max(t);
            }
            if a1 <= b0 + eps || b1 <= a0 + eps {
                return false;
            }
        }
        true
    }
}

/// Regular sample grid over a rectangle: each sample is free (walkable) or not.
#[derive(Clone, Debug)]
pub struct WalkGrid {
    /// Sample spacing, metres.
    pub res: f32,
    /// Corner of sample `(0, 0)`'s cell.
    pub min: Vec2,
    pub w: i32,
    pub h: i32,
    /// Row-major walkable flags.
    pub free: Vec<bool>,
}

impl WalkGrid {
    /// All-blocked grid covering `min..max` at resolution `res`.
    pub fn new(min: Vec2, max: Vec2, res: f32) -> WalkGrid {
        let res = res.max(0.05);
        let size = (max - min).max(Vec2::ZERO);
        let w = ((size.x / res).ceil() as i32).clamp(1, 4096);
        let h = ((size.y / res).ceil() as i32).clamp(1, 4096);
        WalkGrid { res, min, w, h, free: vec![false; (w * h) as usize] }
    }

    pub fn center(&self, i: i32, j: i32) -> Vec2 {
        self.min + Vec2::new((i as f32 + 0.5) * self.res, (j as f32 + 0.5) * self.res)
    }

    fn range(&self, lo: Vec2, hi: Vec2) -> (i32, i32, i32, i32) {
        let i0 = (((lo.x - self.min.x) / self.res).floor() as i32).clamp(0, self.w);
        let j0 = (((lo.y - self.min.y) / self.res).floor() as i32).clamp(0, self.h);
        let i1 = (((hi.x - self.min.x) / self.res).ceil() as i32 + 1).clamp(0, self.w);
        let j1 = (((hi.y - self.min.y) / self.res).ceil() as i32 + 1).clamp(0, self.h);
        (i0, j0, i1, j1)
    }

    /// Set every sample inside `obb` to `free`.
    pub fn fill_obb(&mut self, obb: &Obb, free: bool) {
        let (lo, hi) = obb.aabb();
        let (i0, j0, i1, j1) = self.range(lo, hi);
        for j in j0..j1 {
            for i in i0..i1 {
                if obb.contains(self.center(i, j)) {
                    self.free[(j * self.w + i) as usize] = free;
                }
            }
        }
    }

    /// Block every sample whose cell the box touches (the box grown by half a
    /// sample), so even obstacles thinner than the sample spacing cut paths.
    pub fn block_obb(&mut self, obb: &Obb) {
        self.fill_obb(&obb.inflate(self.res * 0.5), false);
    }

    /// Set every sample inside the (simple) polygon to `free`.
    pub fn fill_polygon(&mut self, poly: &[Vec2], free: bool) {
        if poly.len() < 3 {
            return;
        }
        let mut lo = poly[0];
        let mut hi = poly[0];
        for p in poly {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let (i0, j0, i1, j1) = self.range(lo, hi);
        for j in j0..j1 {
            for i in i0..i1 {
                if point_in_polygon(poly, self.center(i, j)) {
                    self.free[(j * self.w + i) as usize] = free;
                }
            }
        }
    }

    /// Whether any sample inside `obb` is free.
    pub fn any_free_in(&self, obb: &Obb) -> bool {
        self.any_in(obb, &self.free)
    }

    /// Whether any sample inside `obb` is set in `flags`.
    pub fn any_in(&self, obb: &Obb, flags: &[bool]) -> bool {
        let (lo, hi) = obb.aabb();
        let (i0, j0, i1, j1) = self.range(lo, hi);
        for j in j0..j1 {
            for i in i0..i1 {
                let k = (j * self.w + i) as usize;
                if flags[k] && obb.contains(self.center(i, j)) {
                    return true;
                }
            }
        }
        false
    }

    /// Free samples within `radius` of `p`.
    pub fn free_near(&self, p: Vec2, radius: f32) -> Vec<usize> {
        let (i0, j0, i1, j1) = self.range(p - Vec2::splat(radius), p + Vec2::splat(radius));
        let mut out = Vec::new();
        for j in j0..j1 {
            for i in i0..i1 {
                let k = (j * self.w + i) as usize;
                if self.free[k] && self.center(i, j).distance(p) <= radius {
                    out.push(k);
                }
            }
        }
        out
    }

    /// 4-connected flood fill over free samples from the given sample indices.
    pub fn flood(&self, starts: &[usize]) -> Vec<bool> {
        let mut seen = vec![false; self.free.len()];
        let mut stack: Vec<usize> = Vec::new();
        for &s in starts {
            if s < self.free.len() && self.free[s] && !seen[s] {
                seen[s] = true;
                stack.push(s);
            }
        }
        while let Some(k) = stack.pop() {
            let i = k as i32 % self.w;
            let j = k as i32 / self.w;
            for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (ni, nj) = (i + di, j + dj);
                if ni < 0 || nj < 0 || ni >= self.w || nj >= self.h {
                    continue;
                }
                let nk = (nj * self.w + ni) as usize;
                if self.free[nk] && !seen[nk] {
                    seen[nk] = true;
                    stack.push(nk);
                }
            }
        }
        seen
    }

    /// Flood from `starts` until every target (a set of sample indices) has a
    /// reached sample, or the free space is exhausted. Returns which targets
    /// were reached.
    pub fn flood_to(&self, starts: &[usize], targets: &[Vec<usize>]) -> Vec<bool> {
        if targets.len() > 64 {
            let reached = self.flood(starts);
            return targets.iter().map(|t| t.iter().any(|&k| reached[k])).collect();
        }
        // Bit `t` of `owner[k]` set = sample `k` belongs to target `t`.
        let mut owner = vec![0u64; self.free.len()];
        let mut want = 0u64;
        for (t, samples) in targets.iter().enumerate() {
            for &k in samples {
                if k < owner.len() {
                    owner[k] |= 1 << t;
                    want |= 1 << t;
                }
            }
        }
        let mut got = 0u64;
        let mut seen = vec![false; self.free.len()];
        let mut stack: Vec<usize> = Vec::new();
        for &k in starts {
            if k < self.free.len() && self.free[k] && !seen[k] {
                seen[k] = true;
                got |= owner[k];
                stack.push(k);
            }
        }
        while got != want {
            let Some(k) = stack.pop() else { break };
            let i = k as i32 % self.w;
            let j = k as i32 / self.w;
            for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (ni, nj) = (i + di, j + dj);
                if ni < 0 || nj < 0 || ni >= self.w || nj >= self.h {
                    continue;
                }
                let nk = (nj * self.w + ni) as usize;
                if self.free[nk] && !seen[nk] {
                    seen[nk] = true;
                    got |= owner[nk];
                    stack.push(nk);
                }
            }
        }
        (0..targets.len()).map(|t| got & (1 << t) != 0).collect()
    }

    /// Whether any reached sample lies within `radius` of `p`.
    pub fn reached_near(&self, reached: &[bool], p: Vec2, radius: f32) -> bool {
        self.free_near(p, radius).into_iter().any(|k| reached[k])
    }
}

/// Even-odd point-in-polygon test.
pub fn point_in_polygon(poly: &[Vec2], p: Vec2) -> bool {
    let n = poly.len();
    let mut inside = false;
    for k in 0..n {
        let a = poly[k];
        let b = poly[(k + 1) % n];
        if (a.y > p.y) != (b.y > p.y) {
            let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if p.x < x {
                inside = !inside;
            }
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obb_overlap_and_contains() {
        let a = Obb::from_aabb(Vec2::ZERO, Vec2::new(2.0, 1.0));
        let b = Obb::from_prop(Vec2::new(2.5, 0.5), Vec2::new(1.0, 1.0), 0.0);
        assert!(!a.overlaps(&b, 1e-4)); // touching
        let c = Obb::from_prop(Vec2::new(2.2, 0.5), Vec2::new(1.0, 1.0), 0.3);
        assert!(a.overlaps(&c, 1e-4));
        assert!(a.contains(Vec2::new(1.0, 0.5)));
        let s = Obb::from_segment(Vec2::ZERO, Vec2::new(0.0, 4.0), 0.1, 0.0);
        assert!(s.contains(Vec2::new(0.05, 3.9)) && !s.contains(Vec2::new(0.2, 2.0)));
    }

    #[test]
    fn flood_blocked_by_wall() {
        let mut g = WalkGrid::new(Vec2::ZERO, Vec2::new(4.0, 2.0), 0.25);
        g.fill_polygon(&[Vec2::ZERO, Vec2::new(4.0, 0.0), Vec2::new(4.0, 2.0), Vec2::new(0.0, 2.0)], true);
        // A 2 cm wall between sample centres still blocks.
        g.block_obb(&Obb::from_segment(Vec2::new(2.0, 0.0), Vec2::new(2.0, 2.0), 0.01, 0.0));
        let start = g.free_near(Vec2::new(0.5, 1.0), 0.2);
        let reached = g.flood(&start);
        assert!(!g.reached_near(&reached, Vec2::new(3.5, 1.0), 0.2));
        g.fill_obb(&Obb::from_aabb(Vec2::new(1.8, 0.5), Vec2::new(2.2, 1.5)), true);
        let reached = g.flood(&start);
        assert!(g.reached_near(&reached, Vec2::new(3.5, 1.0), 0.2));
    }
}
