//! Small plan-space geometry helpers shared by the grid, dwelling and town builders.

use mde_core::poly::{closest_point_on_segment, cross};
use mde_core::{Prop, Vec2, WallSegment};

/// Distance from `p` to segment `a–b`.
#[inline]
pub(crate) fn point_seg_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    p.distance(closest_point_on_segment(p, a, b))
}

/// `true` if segments `a1–a2` and `b1–b2` properly cross (endpoints strictly
/// on opposite sides of each other's lines).
fn proper_cross(a1: Vec2, a2: Vec2, b1: Vec2, b2: Vec2) -> bool {
    let d1 = cross(a2 - a1, b1 - a1);
    let d2 = cross(a2 - a1, b2 - a1);
    let d3 = cross(b2 - b1, a1 - b1);
    let d4 = cross(b2 - b1, a2 - b1);
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// Minimum distance between segments `a1–a2` and `b1–b2` (0 if they cross).
pub(crate) fn seg_seg_distance(a1: Vec2, a2: Vec2, b1: Vec2, b2: Vec2) -> f32 {
    if proper_cross(a1, a2, b1, b2) {
        return 0.0;
    }
    point_seg_distance(a1, b1, b2)
        .min(point_seg_distance(a2, b1, b2))
        .min(point_seg_distance(b1, a1, a2))
        .min(point_seg_distance(b2, a1, a2))
}

/// `true` if the segments cross or touch (within `1e-4` m).
#[inline]
pub(crate) fn segments_touch(a1: Vec2, a2: Vec2, b1: Vec2, b2: Vec2) -> bool {
    seg_seg_distance(a1, a2, b1, b2) < 1e-4
}

/// Unit CCW perpendicular.
#[inline]
pub(crate) fn perp(v: Vec2) -> Vec2 {
    Vec2::new(-v.y, v.x)
}

/// Oriented box in plan space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Obb {
    pub center: Vec2,
    /// Unit local x axis; local y is its CCW perpendicular.
    pub u: Vec2,
    pub half: Vec2,
}

impl Obb {
    /// Footprint of a prop (`rotation` radians CCW, local +y = front).
    pub fn from_prop(p: &Prop) -> Option<Obb> {
        let u = Vec2::from_angle(p.rotation);
        let half = p.size.abs() * 0.5;
        (p.pos.is_finite() && u.is_finite() && half.is_finite()).then_some(Obb { center: p.pos, u, half })
    }

    fn local(&self, p: Vec2) -> Vec2 {
        let d = p - self.center;
        Vec2::new(d.dot(self.u), d.dot(perp(self.u)))
    }

    /// `true` if `p` lies inside the box grown by `inflate` on every side.
    pub fn contains(&self, p: Vec2, inflate: f32) -> bool {
        let l = self.local(p);
        l.x.abs() <= self.half.x + inflate && l.y.abs() <= self.half.y + inflate
    }

    /// Bounding radius around the centre.
    pub fn radius(&self) -> f32 {
        self.half.length()
    }

    /// `true` if segment `a–b` passes through the box shrunk by `shrink`
    /// (Liang–Barsky clip in the box frame).
    pub fn hits_segment(&self, a: Vec2, b: Vec2, shrink: f32) -> bool {
        let h = (self.half - Vec2::splat(shrink)).max(Vec2::ZERO);
        if h.x <= 0.0 || h.y <= 0.0 {
            return false;
        }
        let p = self.local(a);
        let d = self.local(b) - p;
        let mut t0 = 0.0f32;
        let mut t1 = 1.0f32;
        for (pc, dc, hc) in [(p.x, d.x, h.x), (p.y, d.y, h.y)] {
            if dc.abs() < 1e-9 {
                if pc.abs() > hc {
                    return false;
                }
            } else {
                let mut ta = (-hc - pc) / dc;
                let mut tb = (hc - pc) / dc;
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
                if t0 > t1 {
                    return false;
                }
            }
        }
        true
    }
}

/// Dominant axis of a floor plan: the length-weighted mean wall direction
/// modulo 90°, falling back to the longest outline edge, then +X.
pub(crate) fn dominant_axis(walls: &[WallSegment], outline: &[Vec2]) -> Vec2 {
    let mut acc = Vec2::ZERO;
    for w in walls {
        let d = w.b - w.a;
        let len = d.length();
        if len.is_finite() && len > 1e-3 {
            let a = d.y.atan2(d.x) * 4.0;
            acc += Vec2::new(a.cos(), a.sin()) * len;
        }
    }
    if acc.length() < 1e-3 {
        let n = outline.len();
        let mut best = 0.0;
        for i in 0..n {
            let d = outline[(i + 1) % n] - outline[i];
            let len = d.length();
            if len.is_finite() && len > best {
                best = len;
                let a = d.y.atan2(d.x) * 4.0;
                acc = Vec2::new(a.cos(), a.sin());
            }
        }
    }
    if acc.length() < 1e-6 || !acc.is_finite() {
        return Vec2::X;
    }
    Vec2::from_angle(acc.y.atan2(acc.x) / 4.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_distances() {
        let d = seg_seg_distance(Vec2::ZERO, Vec2::X, Vec2::new(0.5, -1.0), Vec2::new(0.5, 1.0));
        assert_eq!(d, 0.0);
        let d = seg_seg_distance(Vec2::ZERO, Vec2::X, Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0));
        assert!((d - 1.0).abs() < 1e-6);
        assert!(segments_touch(Vec2::ZERO, Vec2::X, Vec2::X, Vec2::new(2.0, 0.0)));
    }

    #[test]
    fn obb_segment() {
        let o = Obb { center: Vec2::ZERO, u: Vec2::from_angle(0.5), half: Vec2::new(1.0, 0.5) };
        assert!(o.hits_segment(Vec2::new(-3.0, 0.0), Vec2::new(3.0, 0.0), 0.0));
        assert!(!o.hits_segment(Vec2::new(-3.0, 2.0), Vec2::new(3.0, 2.0), 0.0));
        assert!(o.contains(Vec2::new(0.1, 0.1), 0.0));
    }

    #[test]
    fn axis_of_rotated_walls() {
        let r = Vec2::from_angle(0.3);
        let walls = [
            WallSegment { a: Vec2::ZERO, b: r * 5.0, thickness: 0.2, exterior: true },
            WallSegment { a: Vec2::ZERO, b: perp(r) * 3.0, thickness: 0.2, exterior: true },
        ];
        let ax = dominant_axis(&walls, &[]);
        assert!(ax.dot(r).abs() > 0.999 || ax.dot(perp(r)).abs() > 0.999);
    }
}
