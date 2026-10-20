//! Shared foundation for the Midearth town and dwelling generators.
//!
//! * [`model`] — the serialisable data contract every generator produces and every
//!   consumer (Bevy viewer, CLI, other tools) reads.
//! * [`rng`] — a tiny deterministic RNG so a seed gives the same result on every
//!   platform and can be re-implemented bit-for-bit in another language.
//! * [`poly`] — 2D polygon helpers in plan space.
//!
//! # Coordinate conventions
//!
//! * **Plan space** (all generator output): metres, `x` east, `y` north,
//!   polygons counter-clockwise, no repeated closing vertex.
//! * **World space** (meshes, Bevy, glTF): metres, right-handed, `Y` up.
//!   `world = (plan.x, height, -plan.y)`, see [`plan_to_world`].

pub mod model;
pub mod poly;
pub mod rng;

pub use glam::{Vec2, Vec3};
pub use model::*;
pub use rng::Rng;

/// Plan-space point plus height → world space (Y up, metres).
#[inline]
pub fn plan_to_world(p: Vec2, height: f32) -> Vec3 {
    Vec3::new(p.x, height, -p.y)
}

/// World space (Y up, metres) → plan space, dropping the height.
#[inline]
pub fn world_to_plan(w: Vec3) -> Vec2 {
    Vec2::new(w.x, -w.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_world_round_trip() {
        let p = Vec2::new(3.0, -7.5);
        assert_eq!(world_to_plan(plan_to_world(p, 2.0)), p);
    }
}
