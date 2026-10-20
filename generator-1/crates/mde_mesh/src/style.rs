//! Style profiles: named looks that override the per-house style rolls.
//!
//! A [`StyleProfile`] travels in [`crate::DwellingMeshOptions`] and
//! [`crate::TownMeshOptions`], or goes to [`crate::build_lot_house_with`]. The
//! default profile changes nothing. The storybook presets ([`Storybook`])
//! follow three storybook style references (01–03).
//!
//! A preset has two halves:
//!
//! * **Data:** [`Storybook::apply_to_dwelling`] / [`Storybook::apply_to_building`]
//!   set the appearance (and, for dwellings, a gable roof at the preset pitch
//!   and overhang) on the house data, so the mesh, [`crate::opening_placements`]
//!   and [`crate::door_leaves`] all agree.
//! * **Mesh:** the profile forces what the mesher otherwise rolls per seed: the
//!   wall treatment (stone / framed ground floor, framed upper floors), the
//!   entrance lantern and stone (not brick) chimneys, and, for lot houses,
//!   the roof shape.
//!   It also adds dark bargeboards on every verge (dropped from lot houses at
//!   fallback level ≥ 2), a steep cross gable on the street front of every
//!   house whose front is not a gable end already and, on dwellings, curved
//!   roofs (concave slopes that flare at the eaves) laid as individual,
//!   slightly uneven blocks.

use mde_core::{Appearance, Building, DoorKind, Dwelling, DwellingStyle, OpeningKind, Rgba, RoofKind, RoofMaterial, Rng, Vec2, WindowKind};

use crate::geom::mix_seed;

/// A named look for the mesh builders. The default keeps the per-house
/// colours and walls but gives every house the storybook roof
/// ([`Self::story_roof`]); [`Self::classic`] reproduces the original output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct StyleProfile {
    /// Storybook preset; `None` keeps the per-house rolls.
    pub storybook: Option<Storybook>,
    /// The original roofs: the per-house roof roll, planar, without trim or
    /// cross gable. Ignored with a preset.
    pub classic_roof: bool,
}

impl StyleProfile {
    pub fn storybook(preset: Storybook) -> Self {
        Self { storybook: Some(preset), ..Self::default() }
    }

    /// The original look, before the storybook style pass.
    pub fn classic() -> Self {
        Self { classic_roof: true, ..Self::default() }
    }

    /// Storybook roof: a steep gable (preset pitch, else [`default_pitch`])
    /// with dark bargeboards and a front cross gable and, on dwellings,
    /// curved slopes laid as uneven blocks.
    pub(crate) fn story_roof(&self) -> bool {
        self.storybook.is_some() || !self.classic_roof
    }

    /// Roof pitch and overhang of the storybook roof for the house with `seed`.
    pub(crate) fn roof_shape(&self, seed: u64) -> (f32, f32) {
        match self.storybook {
            Some(s) => (s.pitch(seed), s.overhang()),
            None => (default_pitch(seed), DEFAULT_OVERHANG),
        }
    }
}

/// Eave overhang of the default storybook roof, metres.
pub(crate) const DEFAULT_OVERHANG: f32 = 0.6;

/// Pitch of the default storybook roof (55–64°) for the house with `seed`.
pub(crate) fn default_pitch(seed: u64) -> f32 {
    Rng::new(mix_seed(seed, 0x5B_00C)).range_f32(55.0, 64.0)
}

/// Default-roof `pitch` capped so the roof rises at most 1.3 × the wall
/// height (`wall_h`) over a half-span `half`, but never below 45°: low
/// cottages keep a big but not overwhelming roof, tall houses stay steep.
pub(crate) fn cap_pitch(pitch: f32, wall_h: f32, half: f32) -> f32 {
    if !(wall_h > 0.5 && half > 0.5) {
        return pitch;
    }
    pitch.min((1.3 * wall_h / half).atan().to_degrees()).max(45.0)
}

/// Storybook-medieval presets, one per style reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Storybook {
    /// Reference 01: jettied townhouse, terracotta tiles, plaster ground floor
    /// with quoins and plinth, framed upper floor.
    Townhouse01,
    /// Reference 02: timber manor with teal slate roofs, framed on every floor.
    TealManor02,
    /// Reference 03: townhouse on a stone ground floor with very steep teal
    /// slate roofs, framed upper floor.
    TurretTownhouse03,
}

/// What a preset forces in the mesher.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StoryWalls {
    pub stone_ground: bool,
    pub frame_ground: bool,
    pub frame_upper: bool,
    pub lantern: bool,
}

/// sRGB `0xRRGGBB` → [`Rgba`].
fn hex(rgb: u32) -> Rgba {
    let c = |shift: u32| ((rgb >> shift) & 0xFF) as f32 / 255.0;
    [c(16), c(8), c(0), 1.0]
}

impl Storybook {
    pub const ALL: [Storybook; 3] = [Storybook::Townhouse01, Storybook::TealManor02, Storybook::TurretTownhouse03];

    /// Dwelling style to generate with before [`Self::apply_to_dwelling`].
    pub fn dwelling_style(self) -> DwellingStyle {
        match self {
            Storybook::TealManor02 => DwellingStyle::Manor,
            _ => DwellingStyle::Townhouse,
        }
    }

    /// Colours and part choices (mid tones sampled from the reference).
    pub fn appearance(self) -> Appearance {
        let (wall, timber, roof, roof_material, door, door_kind, window, shutters, flower_boxes) = match self {
            Storybook::Townhouse01 => (0xE9E3D3, 0x6B4A32, 0xA8605A, RoofMaterial::Tiles, 0x5A3A26, DoorKind::Arched, 0x6B4A32, false, true),
            Storybook::TealManor02 => (0xF2E2D2, 0x5A4034, 0x5F8489, RoofMaterial::Slate, 0x173A42, DoorKind::Glazed, 0x3B4C49, true, false),
            Storybook::TurretTownhouse03 => (0xF5F0D6, 0x4E3A2E, 0x5FA99C, RoofMaterial::Slate, 0x4B3D32, DoorKind::Arched, 0xA4816F, false, false),
        };
        Appearance {
            wall: hex(wall),
            timber: hex(timber),
            roof: hex(roof),
            roof_material,
            door: hex(door),
            door_kind,
            window: hex(window),
            window_kind: WindowKind::Lattice,
            shutters,
            flower_boxes,
            custom_window: None,
            custom_door: None,
        }
    }

    /// Roof pitch range in degrees.
    pub fn pitch_range(self) -> (f32, f32) {
        match self {
            Storybook::Townhouse01 => (58.0, 62.0),
            Storybook::TealManor02 => (52.0, 58.0),
            Storybook::TurretTownhouse03 => (62.0, 68.0),
        }
    }

    /// Eave overhang in metres.
    pub fn overhang(self) -> f32 {
        match self {
            Storybook::Townhouse01 => 0.5,
            Storybook::TealManor02 => 0.9,
            Storybook::TurretTownhouse03 => 0.6,
        }
    }

    /// Roof pitch for the house with `seed`, inside [`Self::pitch_range`].
    pub fn pitch(self, seed: u64) -> f32 {
        let (lo, hi) = self.pitch_range();
        Rng::new(mix_seed(seed, 0x5B_00C)).range_f32(lo, hi)
    }

    /// Restyle a dwelling: the preset appearance (custom parts are kept) and a
    /// gable roof at the preset pitch and overhang. The ridge runs along the
    /// street when the front is wide (the mesher then adds a cross gable on
    /// it), else from front to back (a gable end faces the street). No part
    /// of the floor plan depends on the roof.
    pub fn apply_to_dwelling(self, d: &mut Dwelling) {
        d.appearance = self.restyled(&d.appearance);
        d.roof.kind = RoofKind::Gable;
        d.roof.pitch_deg = self.pitch(d.params.seed);
        d.roof.overhang = self.overhang();
        if let Some(ridge) = street_ridge(d) {
            d.roof.ridge = ridge;
        }
    }

    /// Give a town building the preset appearance (custom parts are kept). Its
    /// roof shape comes from the profile in [`crate::build_lot_house_with`].
    pub fn apply_to_building(self, b: &mut Building) {
        b.appearance = self.restyled(&b.appearance);
    }

    fn restyled(self, old: &Appearance) -> Appearance {
        Appearance { custom_window: old.custom_window.clone(), custom_door: old.custom_door.clone(), ..self.appearance() }
    }

    pub(crate) fn walls(self) -> StoryWalls {
        match self {
            Storybook::Townhouse01 => StoryWalls { stone_ground: false, frame_ground: false, frame_upper: true, lantern: true },
            Storybook::TealManor02 => StoryWalls { stone_ground: false, frame_ground: true, frame_upper: true, lantern: false },
            Storybook::TurretTownhouse03 => StoryWalls { stone_ground: true, frame_ground: false, frame_upper: true, lantern: true },
        }
    }
}

/// Plan direction from a dwelling towards its street: the outward normal of
/// the lowest floor's entrance, else `params.front` (`None` when neither).
pub(crate) fn dwelling_front(d: &Dwelling) -> Option<Vec2> {
    let entrance = d.floors.iter().flat_map(|f| f.openings.iter().map(move |o| (f, o))).find(|(_, o)| o.kind == OpeningKind::Entrance);
    if let Some((f, o)) = entrance
        && let Some(dir) = (o.b - o.a).try_normalize()
    {
        let n = Vec2::new(dir.y, -dir.x);
        let mid = (o.a + o.b) * 0.5;
        return Some(if f.outline.contains(mid + n * 0.6) { -n } else { n });
    }
    let front = d.params.front;
    (front.is_finite() && front.length_squared() > 1e-6).then(|| front.normalize())
}

/// Storybook ridge for a dwelling: along the street when the front is wide
/// (≥ 0.8 × the depth; a cross gable then faces the street), else from front
/// to back (a gable end faces it). `None` without a known front.
pub(crate) fn street_ridge(d: &Dwelling) -> Option<Vec2> {
    let front = dwelling_front(d)?;
    let top = d.floors.last().filter(|f| f.outline.len() >= 3)?;
    let along = Vec2::new(-front.y, front.x);
    let extent = |dir: Vec2| {
        let (lo, hi) = top.outline.0.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), q| (lo.min(q.dot(dir)), hi.max(q.dot(dir))));
        hi - lo
    };
    let ridge = if extent(along) >= extent(front) * 0.8 { along } else { front };
    // Canonical sign, as the generator writes it.
    Some(if ridge.x < -1e-6 || (ridge.x.abs() <= 1e-6 && ridge.y < 0.0) { -ridge } else { ridge })
}
