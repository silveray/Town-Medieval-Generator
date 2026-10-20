//! Procedural medieval town / village layout generator.
//!
//! [`generate`] turns [`TownParams`] into a [`Town`]: plain serde data (plan
//! space, metres, CCW polygons) that the Bevy viewer renders and other tools
//! read as JSON. [`render_svg`] draws it as an ink map.
//!
//! # Algorithm
//!
//! 1. Sites on a jittered spiral (fine in the centre, coarse in the
//!    countryside) → Bowyer–Watson Delaunay → Voronoi cells clipped to a
//!    square map (see [`voronoi`]); two Lloyd steps on the innermost sites.
//!    Cells share vertices by index, so adjacency is exact.
//! 2. The `patches` cells nearest the centre (grown as a connected,
//!    hole-free region) form the city.
//! 3. Optional river (meandering polyline near the centre); edges near the
//!    water are unusable, clean single crossings get bridges.
//! 4. Wall ring = outer boundary of the city; towers at its corners and on
//!    long stretches; gates at well-separated corners where an artery can
//!    enter (the outer cell at a gate is split so a road can leave). An edge
//!    cell becomes the citadel with its own wall and gate.
//! 5. Streets along cell edges: arteries gate → market, roads gate → map
//!    edge, frontage streets so every built cell touches the network, alleys
//!    between all dense blocks; the network is made connected.
//! 6. Wards by weighted rules (market at the centre, cathedral beside it,
//!    rich wards inward, slums and military by the walls, farms and empty
//!    land outside).
//! 7. Lots: each cell is inset per edge by half the street width (or wall
//!    clearance), corners are cut back from junctions, then recursively split
//!    perpendicular to the longest edge; lots keep a street frontage and a
//!    ward-dependent maximum depth (the rest are courtyards / gardens), and are
//!    clipped clear of streets, walls, towers and water. Houses keep a normal
//!    size (≤ 10 × 13 m, mansions 14 × 18 m) at the street end of their lot;
//!    the rest becomes a yard behind a wooden fence with a gate
//!    ([`mde_core::Yard`]) or open ground.
//! 8. Farms get a farmstead and long strip fields; trees fill parks, gardens,
//!    hedgerows, road verges and countryside groves.
//!
//! Randomness comes only from [`mde_core::Rng`], forked per stage and per
//! building, so a seed always yields the same town.
//!
//! The [`edit`] module holds pure helpers for the free-design editor
//! (drag-to-build, resize, cut and erase bridges).

pub mod edit;
mod farms;
mod geom;
mod lots;
mod pipeline;
mod river;
mod streets;
mod style;
mod svg;
mod topo;
mod trees;
pub mod voronoi;
mod wall;
mod wards;
mod yards;

pub use edit::{
    BridgeEnd, MIN_BRIDGE_SPAN, bridge_across, bridge_at, bridge_between, bridge_rise, building_at, cut_bridge,
    move_bridge_end, set_bridge_length,
};
pub use streets::{ALLEY_WIDTH, ARTERY_WIDTH, ROAD_WIDTH, STREET_WIDTH, street_width};
pub use style::{Region, default_appearance, regional_appearance};
pub use svg::render_svg;

use mde_core::{Town, TownParams};

/// Generate a town or village from `params`.
///
/// Deterministic: the same params always give the same [`Town`] (byte-identical
/// JSON). Never panics; out-of-range params are clamped (`patches` to
/// `1..=400`, `lot_size` to `3..=30` m). `Town::schema` is
/// [`mde_core::SCHEMA_VERSION`] and `Town::params` echoes `params` unchanged.
pub fn generate(params: &TownParams) -> Town {
    pipeline::generate(params, None)
}

/// Like [`generate`], but every building takes its palette from `region`
/// when it is `Some` (for example [`Region::Storybook`]). Footprints, kinds,
/// floors, yards and trees are the same as [`generate`] gives for `params`:
/// only the appearances change.
pub fn generate_with(params: &TownParams, region: Option<Region>) -> Town {
    pipeline::generate(params, region)
}
