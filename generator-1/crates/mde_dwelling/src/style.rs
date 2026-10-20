//! Style-dependent choices: a grounded medieval palette, window / door sizes
//! and the roof.

use mde_core::{Appearance, DoorKind, DwellingStyle, Rgba, Rng, Roof, RoofKind, RoofMaterial, Vec2, WindowKind};

/// Limewash, ochre and stone wall tones.
const WALLS: [Rgba; 7] = [
    [0.88, 0.85, 0.77, 1.0], // limewash
    [0.83, 0.78, 0.67, 1.0], // aged limewash
    [0.80, 0.70, 0.52, 1.0], // ochre plaster
    [0.74, 0.66, 0.56, 1.0], // warm sand
    [0.66, 0.64, 0.60, 1.0], // grey stone
    [0.76, 0.66, 0.60, 1.0], // faded rose plaster
    [0.58, 0.55, 0.50, 1.0], // dark rubble stone
];

/// Dark, weathered oak.
const TIMBERS: [Rgba; 4] = [
    [0.24, 0.17, 0.11, 1.0],
    [0.30, 0.21, 0.14, 1.0],
    [0.20, 0.16, 0.13, 1.0],
    [0.36, 0.26, 0.17, 1.0],
];

/// Door paint / stain.
const DOORS: [Rgba; 6] = [
    [0.40, 0.26, 0.15, 1.0], // oak
    [0.25, 0.17, 0.11, 1.0], // dark oak
    [0.24, 0.32, 0.26, 1.0], // muted green
    [0.42, 0.16, 0.12, 1.0], // oxblood
    [0.22, 0.28, 0.36, 1.0], // slate blue
    [0.33, 0.30, 0.24, 1.0], // weathered grey-brown
];

/// Window frames and shutters.
const FRAMES: [Rgba; 5] = [
    [0.26, 0.19, 0.13, 1.0],
    [0.27, 0.36, 0.30, 1.0],
    [0.30, 0.34, 0.40, 1.0],
    [0.55, 0.45, 0.30, 1.0],
    [0.40, 0.18, 0.14, 1.0],
];

/// Roof colour for a material, slightly varied.
fn roof_colour(m: RoofMaterial, rng: &mut Rng) -> Rgba {
    let base: Rgba = match m {
        RoofMaterial::Tiles => *rng.pick(&[[0.60, 0.31, 0.22, 1.0], [0.52, 0.30, 0.24, 1.0], [0.64, 0.38, 0.30, 1.0]]),
        RoofMaterial::Slate => *rng.pick(&[[0.27, 0.29, 0.32, 1.0], [0.33, 0.34, 0.37, 1.0], [0.24, 0.25, 0.27, 1.0]]),
        RoofMaterial::Thatch => *rng.pick(&[[0.60, 0.50, 0.32, 1.0], [0.52, 0.44, 0.30, 1.0]]),
        RoofMaterial::Shingles => *rng.pick(&[[0.38, 0.30, 0.24, 1.0], [0.44, 0.38, 0.32, 1.0]]),
    };
    let k = rng.range_f32(0.93, 1.05);
    [(base[0] * k).min(1.0), (base[1] * k).min(1.0), (base[2] * k).min(1.0), 1.0]
}

/// A style-appropriate look picked from the seed.
pub(crate) fn pick_appearance(style: DwellingStyle, rng: &mut Rng) -> Appearance {
    use DoorKind as D;
    use DwellingStyle as S;
    use RoofMaterial as M;
    use WindowKind as W;
    let (windows, doors, roofs, wall_w): (&[W], &[D], &[M], [f32; 7]) = match style {
        S::Cottage => (&[W::Casement, W::Lattice], &[D::Plank, D::Dutch, D::Arched], &[M::Thatch, M::Shingles, M::Tiles], [3.0, 2.0, 1.0, 1.0, 1.0, 0.6, 0.6]),
        S::Townhouse => (&[W::Casement, W::Lattice, W::Mullioned], &[D::Plank, D::Arched, D::Glazed], &[M::Tiles, M::Slate], [2.0, 2.0, 1.5, 1.5, 0.6, 1.0, 0.2]),
        S::Farmhouse => (&[W::Casement, W::Casement, W::Lattice], &[D::Dutch, D::Plank], &[M::Thatch, M::Tiles, M::Shingles], [2.0, 2.0, 1.0, 1.0, 1.5, 0.3, 1.0]),
        S::Manor => (&[W::Mullioned, W::Gothic, W::Arched, W::Mullioned], &[D::Studded, D::Arched, D::Double], &[M::Slate, M::Tiles], [0.6, 1.0, 0.5, 1.0, 3.0, 0.3, 1.5]),
        S::Tavern => (&[W::Lattice, W::Casement, W::Mullioned], &[D::Double, D::Arched, D::Plank], &[M::Tiles, M::Slate, M::Shingles], [1.5, 2.0, 1.5, 1.0, 1.0, 0.5, 0.6]),
        S::Shop => (&[W::Casement, W::Lattice, W::Mullioned], &[D::Double, D::Glazed, D::Plank], &[M::Tiles, M::Slate], [2.0, 2.0, 1.5, 1.0, 0.6, 1.0, 0.2]),
    };
    let wall = WALLS[rng.weighted(&wall_w)];
    let roof_material = *rng.pick(roofs);
    let roof = roof_colour(roof_material, rng);
    Appearance {
        wall,
        timber: *rng.pick(&TIMBERS),
        roof,
        roof_material,
        door: *rng.pick(&DOORS),
        door_kind: *rng.pick(doors),
        window: *rng.pick(&FRAMES),
        window_kind: *rng.pick(windows),
        shutters: rng.chance(match style {
            S::Cottage | S::Farmhouse => 0.6,
            S::Manor => 0.15,
            _ => 0.4,
        }),
        flower_boxes: rng.chance(match style {
            S::Cottage | S::Townhouse => 0.3,
            S::Manor | S::Farmhouse => 0.05,
            _ => 0.15,
        }),
        custom_window: None,
        custom_door: None,
    }
}

/// Size of the normal windows of one house.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WindowStyle {
    pub width: f32,
    pub height: f32,
    pub sill: f32,
    /// Target spacing between window centres along a facade.
    pub spacing: f32,
}

/// Window proportions from the style and the window kind, fitted under the
/// ceiling of a storey with clear height `clear`.
pub(crate) fn window_style(style: DwellingStyle, kind: WindowKind, clear: f32, rng: &mut Rng) -> WindowStyle {
    let (w0, w1, spacing) = match style {
        DwellingStyle::Cottage => (0.7, 0.9, 2.8),
        DwellingStyle::Townhouse => (0.8, 1.0, 2.3),
        DwellingStyle::Farmhouse => (0.8, 1.0, 2.8),
        DwellingStyle::Manor => (1.0, 1.2, 2.4),
        DwellingStyle::Tavern => (0.9, 1.1, 2.5),
        DwellingStyle::Shop => (0.9, 1.1, 2.4),
    };
    let mut width = rng.range_f32(w0, w1);
    let mut height = rng.range_f32(1.2, 1.5);
    let mut sill = 0.9;
    match kind {
        WindowKind::Gothic | WindowKind::Arched => height += 0.35,
        WindowKind::Round => {
            width = rng.range_f32(0.7, 0.9);
            height = width;
            sill = 1.2;
        }
        _ => {}
    }
    let max_h = (clear - 0.25 - sill).max(0.5);
    height = height.min(max_h);
    WindowStyle { width, height, sill, spacing }
}

/// Entrance width and height for the door kind.
pub(crate) fn entrance_size(app: &Appearance, rng: &mut Rng) -> (f32, f32) {
    match app.door_kind {
        DoorKind::Double => (rng.range_f32(1.5, 1.8), 2.4),
        DoorKind::Arched | DoorKind::Studded => (rng.range_f32(1.1, 1.4), 2.4),
        _ => (rng.range_f32(1.1, 1.3), 2.2),
    }
}

/// Roof for the style. `long_axis` is the plan direction of the house's long
/// axis, `front` the plan direction towards the street.
pub(crate) fn pick_roof(style: DwellingStyle, long_axis: Vec2, front: Vec2, rng: &mut Rng) -> Roof {
    use DwellingStyle as S;
    let kind = match style {
        S::Cottage | S::Farmhouse => *rng.pick(&[RoofKind::Gable, RoofKind::HalfHip, RoofKind::Gable]),
        S::Townhouse | S::Shop => RoofKind::Gable,
        S::Manor => *rng.pick(&[RoofKind::Hip, RoofKind::Gambrel, RoofKind::Hip]),
        S::Tavern => *rng.pick(&[RoofKind::Gable, RoofKind::Hip]),
    };
    let pitch_deg = match style {
        S::Cottage => rng.range_f32(45.0, 55.0),
        S::Townhouse | S::Shop => rng.range_f32(45.0, 55.0),
        S::Farmhouse => rng.range_f32(40.0, 50.0),
        S::Manor => rng.range_f32(35.0, 45.0),
        S::Tavern => rng.range_f32(40.0, 50.0),
    };
    let overhang = rng.range_f32(0.3, 0.6);
    // Townhouses turn their gable to the street: ridge runs front to back.
    let ridge = if matches!(style, S::Townhouse | S::Shop) && front.length_squared() > 0.5 {
        front
    } else {
        long_axis
    };
    let ridge = ridge.try_normalize().unwrap_or(Vec2::X);
    // Canonical sign: ridge points to +x (or +y when vertical).
    let ridge = if ridge.x < -1e-6 || (ridge.x.abs() <= 1e-6 && ridge.y < 0.0) { -ridge } else { ridge };
    Roof { kind, pitch_deg, overhang, ridge }
}
