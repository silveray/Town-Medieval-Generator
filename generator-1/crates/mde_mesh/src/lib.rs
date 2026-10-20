//! Stylised, grounded-medieval meshes from Midearth data, plus binary glTF export.
//!
//! Turns [`mde_core`] data into low/mid-poly triangle meshes in **world space**
//! (metres, `Y` up, `world = (plan.x, h, -plan.y)`), counter-clockwise front
//! faces, flat-shaded "chunky" geometry (smooth normals only on tree foliage) and
//! planar world UVs in metres:
//!
//! * [`build_dwelling`]: a full house from a [`Dwelling`]: walls with real
//!   opening cut-outs (doors are open holes an NPC can walk through),
//!   half-timbering, procedural windows and doors per [`mde_core::WindowKind`] /
//!   [`mde_core::DoorKind`], floor slabs with stair holes, stairs, furniture and
//!   the roof. [`DwellingMeshOptions`] gives a dollhouse cut-away and leaves out
//!   door leaves so the viewer can animate the ones from [`door_leaves`].
//! * [`build_lot_house`]: a cheap massing house (≤ ~4k triangles) for the
//!   whole-town view.
//! * [`build_bridge`]: wooden trestle or stone arch bridges of any size.
//! * [`build_town`]: ground, streets, plaza, fields, walls, river, trees,
//!   bridges and lot houses as separate named [`Model`]s.
//! * [`gltf::write_glb`]: `.glb` export (one node per model, one material per slot).
//! * [`StyleProfile`]: named looks (the [`Storybook`] presets) that override
//!   the per-house style rolls; the default profile changes nothing.
//!
//! # Colours
//!
//! Per-building colours come from [`mde_core::Appearance`] (sRGB) and are written
//! as **linear vertex colours**. Recolourable slots ([`palette::is_recolourable`])
//! have a white base colour in [`palette::slot_color`], so the vertex colour
//! decides; other slots carry their real colour and the vertex colours only
//! vary it subtly. Recolouring a house is therefore a mesh rebuild, and a game engine
//! can use one material per slot multiplied by the vertex colour.
//!
//! # Custom parts
//!
//! When `Appearance::custom_window` / `custom_door` is set, the matching
//! openings are left as bare holes with reveals (no procedural frame, glass or
//! leaf); [`opening_placements`] / [`lot_house_openings`] say where to put the
//! part models. `custom_door` applies to street entrances; interior doors stay
//! procedural.
//!
//! Everything is deterministic for given input (randomness only from
//! [`mde_core::Rng`] seeded by the data).

mod bridge;
mod dwelling;
pub mod fixtures;
mod geom;
pub mod gltf;
mod interior;
mod lot;
mod openings;
pub mod palette;
mod roof;
mod style;
mod town;
mod triangulate;
mod walls;

pub use bridge::{bridge_deck_height, build_bridge};
pub use dwelling::{build_dwelling, door_leaves, opening_placements, DoorLeaf, DwellingMeshOptions, OpeningPlacement};
pub use lot::{
    build_lot_house, build_lot_house_with, lot_house_at_level, lot_house_at_level_with, lot_house_openings,
    lot_house_openings_with,
};
pub use style::{Storybook, StyleProfile};
pub use town::{build_town, TownMeshOptions};
pub use triangulate::triangulate;

#[doc(no_inline)]
pub use mde_core::{Bridge, Building, Dwelling, Model, Town};
