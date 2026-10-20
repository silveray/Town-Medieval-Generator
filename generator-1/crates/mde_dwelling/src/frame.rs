//! Local planning frame: an orthonormal 2D basis aligned with the house.
//!
//! All layout work happens in this frame, where every wall is axis-aligned.
//! Results are mapped back to plan space at the very end.

use mde_core::Vec2;

/// Orthonormal frame: local `x` runs along `u`, local `y` along `v = perp(u)`
/// (`u` rotated 90° CCW), so the frame keeps plan-space handedness and CCW
/// polygons stay CCW.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Frame {
    /// Plan-space position of the local origin.
    pub origin: Vec2,
    /// Plan-space direction of local `+x`.
    pub u: Vec2,
    /// Plan-space direction of local `+y`.
    pub v: Vec2,
}

impl Frame {
    /// Frame at `origin` whose local `+x` points along `u` (normalised; falls back
    /// to plan `+x` for a degenerate vector).
    pub fn new(origin: Vec2, u: Vec2) -> Self {
        let u = u.try_normalize().unwrap_or(Vec2::X);
        Frame { origin, u, v: Vec2::new(-u.y, u.x) }
    }

    /// Plan point → local point.
    pub fn to_local(self, p: Vec2) -> Vec2 {
        let d = p - self.origin;
        Vec2::new(d.dot(self.u), d.dot(self.v))
    }

    /// Local point → plan point.
    pub fn to_plan(self, l: Vec2) -> Vec2 {
        self.origin + self.u * l.x + self.v * l.y
    }

    /// Local direction → plan direction.
    pub fn dir_to_plan(self, d: Vec2) -> Vec2 {
        self.u * d.x + self.v * d.y
    }

    /// Plan direction → local direction.
    pub fn dir_to_local(self, d: Vec2) -> Vec2 {
        Vec2::new(d.dot(self.u), d.dot(self.v))
    }
}

/// One of the four axis-aligned sides of the house in the local frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Side {
    /// Local `-y`.
    South,
    /// Local `+x`.
    East,
    /// Local `+y`.
    North,
    /// Local `-x`.
    West,
}

impl Side {
    /// Outward unit normal in the local frame.
    pub fn normal(self) -> Vec2 {
        match self {
            Side::South => Vec2::new(0.0, -1.0),
            Side::East => Vec2::new(1.0, 0.0),
            Side::North => Vec2::new(0.0, 1.0),
            Side::West => Vec2::new(-1.0, 0.0),
        }
    }

    /// The side whose outward normal is closest to `d` (local frame).
    pub fn from_dir(d: Vec2) -> Side {
        if d.x.abs() > d.y.abs() {
            if d.x > 0.0 { Side::East } else { Side::West }
        } else if d.y > 0.0 {
            Side::North
        } else {
            Side::South
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let f = Frame::new(Vec2::new(3.0, -2.0), Vec2::new(1.0, 1.0));
        let p = Vec2::new(-4.0, 7.5);
        assert!(f.to_plan(f.to_local(p)).distance(p) < 1e-5);
        assert!((f.v.dot(f.u)).abs() < 1e-6);
        // CCW basis: v is u rotated +90°.
        assert!(mde_core::poly::cross(f.u, f.v) > 0.0);
    }

    #[test]
    fn side_from_dir() {
        assert_eq!(Side::from_dir(Vec2::new(0.1, -1.0)), Side::South);
        assert_eq!(Side::from_dir(Vec2::new(2.0, 1.0)), Side::East);
        assert_eq!(Side::from_dir(Side::West.normal()), Side::West);
    }
}
