//! Curated, grounded medieval palettes for building [`Appearance`]s.
//!
//! Colours are sRGB in `[0, 1]` (what a colour picker shows): weathered lime
//! plaster, dark oak timber, terracotta / slate / thatch / shingle roofs and
//! muted door and shutter paints. Nothing saturated or cartoonish.

use mde_core::{Appearance, BuildingKind, DoorKind, Rgba, Rng, RoofMaterial, WardKind, WindowKind};

/// Regional building tradition, chosen once per town so its houses read as
/// one place rather than a patchwork.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Region {
    /// Lowland timber-framing: lime-white and cream infill, red clay tiles,
    /// some slate and thatch.
    #[default]
    Heartland,
    /// Cold coast: pale grey plaster and stone, tarred timber, slate and thatch.
    Northern,
    /// Warm south: ochre and sand plaster, terracotta, little exposed timber.
    Southern,
    /// Uplands: rubble stone and silvered wood, shingles and slate.
    Highland,
    /// The storybook look of style references 01–03: cream walls, dark timber,
    /// teal slate or terracotta tiles. Never picked at random; a town asks for
    /// it with [`crate::generate_with`].
    Storybook,
}

impl Region {
    /// The regions a town picks from at random, in a stable order.
    pub const ALL: [Region; 4] = [Region::Heartland, Region::Northern, Region::Southern, Region::Highland];

    /// Region a town generator picks for `rng` (Heartland most often).
    pub fn pick(rng: &mut Rng) -> Region {
        Region::ALL[rng.weighted(&[0.42, 0.24, 0.2, 0.14])]
    }
}

const fn c(r: f32, g: f32, b: f32) -> Rgba {
    [r, g, b, 1.0]
}

// Plaster / render.
const LIME_WHITE: Rgba = c(0.91, 0.89, 0.83);
const CREAM: Rgba = c(0.89, 0.84, 0.71);
const OCHRE: Rgba = c(0.80, 0.66, 0.44);
const SAND: Rgba = c(0.84, 0.76, 0.62);
const PALE_GREY: Rgba = c(0.79, 0.78, 0.75);
const DUSTY_ROSE: Rgba = c(0.80, 0.69, 0.63);
const DINGY: Rgba = c(0.72, 0.68, 0.60);
// Stone.
const GREY_STONE: Rgba = c(0.60, 0.59, 0.56);
const SANDSTONE: Rgba = c(0.72, 0.64, 0.52);
const LIMESTONE: Rgba = c(0.80, 0.77, 0.70);
const DARK_STONE: Rgba = c(0.47, 0.46, 0.44);
// Wood cladding (barns).
const WEATHERED_BOARD: Rgba = c(0.46, 0.39, 0.31);
const SILVER_BOARD: Rgba = c(0.55, 0.52, 0.47);
// Timber framing.
const DARK_OAK: Rgba = c(0.24, 0.17, 0.11);
const TARRED: Rgba = c(0.15, 0.13, 0.11);
const MID_OAK: Rgba = c(0.38, 0.27, 0.17);
const GREY_OAK: Rgba = c(0.34, 0.29, 0.24);
// Roofs.
const TILE_RED: Rgba = c(0.60, 0.30, 0.21);
const TILE_WEATHERED: Rgba = c(0.52, 0.32, 0.25);
const TILE_DARK: Rgba = c(0.43, 0.23, 0.18);
const TILE_ORANGE: Rgba = c(0.69, 0.39, 0.24);
const SLATE: Rgba = c(0.29, 0.31, 0.34);
const SLATE_BLUE: Rgba = c(0.33, 0.36, 0.41);
const SLATE_DARK: Rgba = c(0.24, 0.25, 0.27);
const THATCH: Rgba = c(0.62, 0.53, 0.37);
const THATCH_OLD: Rgba = c(0.50, 0.44, 0.32);
const THATCH_DARK: Rgba = c(0.43, 0.38, 0.29);
const SHINGLE: Rgba = c(0.42, 0.33, 0.25);
const SHINGLE_SILVER: Rgba = c(0.48, 0.45, 0.41);
const SHINGLE_DARK: Rgba = c(0.33, 0.26, 0.20);
// Doors.
const OXBLOOD: Rgba = c(0.42, 0.14, 0.11);
const DARK_GREEN: Rgba = c(0.17, 0.27, 0.19);
const NATURAL_OAK: Rgba = c(0.46, 0.32, 0.19);
const DOOR_DARK_OAK: Rgba = c(0.27, 0.19, 0.12);
const SLATE_BLUE_PAINT: Rgba = c(0.26, 0.32, 0.37);
// Window frames / shutters.
const FRAME_DARK: Rgba = c(0.25, 0.18, 0.12);
const FRAME_GREEN: Rgba = c(0.21, 0.31, 0.23);
const FRAME_GREY: Rgba = c(0.41, 0.41, 0.39);
const FRAME_NATURAL: Rgba = c(0.49, 0.37, 0.25);
const FRAME_OXBLOOD: Rgba = c(0.38, 0.15, 0.12);
// Storybook (style references 01–03, mid tones of the style notes).
const STORY_CREAM: Rgba = c(0.95, 0.89, 0.82);
const STORY_IVORY: Rgba = c(0.96, 0.94, 0.84);
const STORY_OAK_LIGHT: Rgba = c(0.42, 0.29, 0.20);
const STORY_OAK_MID: Rgba = c(0.35, 0.25, 0.20);
const STORY_OAK_DARK: Rgba = c(0.31, 0.23, 0.18);
const STORY_TEAL_SLATE: Rgba = c(0.37, 0.52, 0.54);
const STORY_AQUA_SLATE: Rgba = c(0.37, 0.66, 0.61);
const STORY_TERRACOTTA: Rgba = c(0.66, 0.38, 0.35);
const STORY_DOOR_OAK: Rgba = c(0.35, 0.23, 0.15);
const STORY_DOOR_TEAL: Rgba = c(0.09, 0.23, 0.26);
const STORY_DOOR_DARK: Rgba = c(0.29, 0.24, 0.20);
const STORY_WINDOW_TEAL: Rgba = c(0.23, 0.30, 0.29);
const STORY_WINDOW_CLAY: Rgba = c(0.64, 0.51, 0.44);

fn pick<T: Copy>(rng: &mut Rng, opts: &[(T, f32)]) -> T {
    let w: Vec<f32> = opts.iter().map(|o| o.1).collect();
    opts[rng.weighted(&w)].0
}

/// Subtle per-building variation so neighbours never match exactly.
fn vary(col: Rgba, rng: &mut Rng, amount: f32) -> Rgba {
    let k = 1.0 + rng.range_f32(-amount, amount);
    let mut out = col;
    for ch in out.iter_mut().take(3) {
        *ch = (*ch * k * (1.0 + rng.range_f32(-amount * 0.35, amount * 0.35))).clamp(0.0, 1.0);
    }
    out
}

/// Curated default look for a building of `kind` in a `ward`, in the default
/// ([`Region::Heartland`]) tradition. Deterministic for a given `rng` state.
pub fn default_appearance(kind: BuildingKind, ward: WardKind, rng: &mut Rng) -> Appearance {
    regional_appearance(kind, ward, Region::Heartland, rng)
}

/// Like [`default_appearance`] but for a specific regional tradition.
pub fn regional_appearance(kind: BuildingKind, ward: WardKind, region: Region, rng: &mut Rng) -> Appearance {
    appearance(kind, ward, region, 0.0, rng)
}

/// Full palette picker. `rustic` in `[0, 1]` pushes towards thatch and plainer
/// detailing (villages).
pub(crate) fn appearance(kind: BuildingKind, ward: WardKind, region: Region, rustic: f32, rng: &mut Rng) -> Appearance {
    use BuildingKind as B;
    use WardKind as W;
    let rich = matches!(ward, W::Patriciate | W::Administration | W::Merchant | W::Market | W::Cathedral)
        || matches!(kind, B::Mansion);
    let poor = matches!(ward, W::Slum);
    let rural = matches!(ward, W::Farm | W::Empty | W::Gate) || matches!(kind, B::Farmhouse | B::Barn | B::Mill);
    let stone = matches!(kind, B::Temple | B::Keep | B::Tower)
        || (kind == B::Barracks && rng.chance(0.7))
        || (kind == B::Mansion && region == Region::Highland && rng.chance(0.5));
    let rustic = if rural { rustic.max(0.5) } else { rustic };

    // Walls.
    let wall = if stone {
        match region {
            Region::Southern => pick(rng, &[(SANDSTONE, 0.6), (LIMESTONE, 0.4)]),
            Region::Northern => pick(rng, &[(GREY_STONE, 0.6), (DARK_STONE, 0.4)]),
            Region::Highland => pick(rng, &[(GREY_STONE, 0.5), (DARK_STONE, 0.3), (SANDSTONE, 0.2)]),
            Region::Heartland => pick(rng, &[(LIMESTONE, 0.4), (GREY_STONE, 0.4), (SANDSTONE, 0.2)]),
            Region::Storybook => pick(rng, &[(LIMESTONE, 0.5), (GREY_STONE, 0.3), (SANDSTONE, 0.2)]),
        }
    } else if kind == B::Barn {
        match region {
            Region::Highland | Region::Northern => {
                pick(rng, &[(SILVER_BOARD, 0.4), (WEATHERED_BOARD, 0.3), (GREY_STONE, 0.3)])
            }
            _ => pick(rng, &[(WEATHERED_BOARD, 0.55), (SILVER_BOARD, 0.25), (CREAM, 0.2)]),
        }
    } else {
        let base = match region {
            Region::Heartland => pick(
                rng,
                &[(LIME_WHITE, 0.34), (CREAM, 0.3), (SAND, 0.12), (PALE_GREY, 0.1), (OCHRE, 0.08), (DUSTY_ROSE, 0.06)],
            ),
            Region::Northern => {
                pick(rng, &[(PALE_GREY, 0.34), (LIME_WHITE, 0.3), (CREAM, 0.14), (GREY_STONE, 0.14), (SAND, 0.08)])
            }
            Region::Southern => {
                pick(rng, &[(OCHRE, 0.3), (SAND, 0.3), (CREAM, 0.2), (DUSTY_ROSE, 0.1), (LIME_WHITE, 0.1)])
            }
            Region::Highland => pick(rng, &[(LIME_WHITE, 0.3), (PALE_GREY, 0.25), (GREY_STONE, 0.25), (CREAM, 0.2)]),
            Region::Storybook => pick(rng, &[(LIME_WHITE, 0.35), (STORY_CREAM, 0.4), (STORY_IVORY, 0.25)]),
        };
        if poor && rng.chance(0.6) { DINGY } else { base }
    };
    let wall = vary(wall, rng, if poor { 0.07 } else { 0.04 });

    let timber = match region {
        Region::Heartland => pick(rng, &[(DARK_OAK, 0.65), (MID_OAK, 0.2), (TARRED, 0.15)]),
        Region::Northern => pick(rng, &[(TARRED, 0.55), (DARK_OAK, 0.35), (GREY_OAK, 0.1)]),
        Region::Southern => pick(rng, &[(MID_OAK, 0.5), (DARK_OAK, 0.4), (GREY_OAK, 0.1)]),
        Region::Highland => pick(rng, &[(GREY_OAK, 0.45), (DARK_OAK, 0.4), (TARRED, 0.15)]),
        Region::Storybook => pick(rng, &[(STORY_OAK_LIGHT, 0.4), (STORY_OAK_MID, 0.35), (STORY_OAK_DARK, 0.25)]),
    };
    let timber = vary(timber, rng, 0.05);

    // Roof material.
    let roof_material = if matches!(kind, B::Temple | B::Keep | B::Tower) {
        match region {
            Region::Southern => pick(rng, &[(RoofMaterial::Tiles, 0.8), (RoofMaterial::Slate, 0.2)]),
            Region::Heartland => pick(rng, &[(RoofMaterial::Slate, 0.6), (RoofMaterial::Tiles, 0.4)]),
            _ => RoofMaterial::Slate,
        }
    } else {
        let mut opts: Vec<(RoofMaterial, f32)> = match region {
            Region::Heartland => vec![
                (RoofMaterial::Tiles, 0.55),
                (RoofMaterial::Slate, 0.15),
                (RoofMaterial::Thatch, 0.2),
                (RoofMaterial::Shingles, 0.1),
            ],
            Region::Northern => vec![
                (RoofMaterial::Slate, 0.45),
                (RoofMaterial::Thatch, 0.35),
                (RoofMaterial::Tiles, 0.1),
                (RoofMaterial::Shingles, 0.1),
            ],
            Region::Southern => {
                vec![(RoofMaterial::Tiles, 0.9), (RoofMaterial::Shingles, 0.05), (RoofMaterial::Thatch, 0.05)]
            }
            Region::Highland => vec![
                (RoofMaterial::Shingles, 0.45),
                (RoofMaterial::Slate, 0.4),
                (RoofMaterial::Thatch, 0.1),
                (RoofMaterial::Tiles, 0.05),
            ],
            Region::Storybook => vec![(RoofMaterial::Slate, 0.5), (RoofMaterial::Tiles, 0.5)],
        };
        let thatchy = rustic + if poor { 0.6 } else { 0.0 } - if rich { 0.8 } else { 0.0 };
        for o in opts.iter_mut() {
            if o.0 == RoofMaterial::Thatch {
                o.1 *= (1.0 + 3.0 * thatchy).max(0.05);
            }
            if o.0 == RoofMaterial::Shingles && poor {
                o.1 *= 1.8;
            }
        }
        pick(rng, &opts)
    };
    let roof = match roof_material {
        RoofMaterial::Tiles => match region {
            Region::Southern => pick(rng, &[(TILE_ORANGE, 0.55), (TILE_RED, 0.3), (TILE_WEATHERED, 0.15)]),
            Region::Storybook => STORY_TERRACOTTA,
            _ => pick(rng, &[(TILE_RED, 0.4), (TILE_WEATHERED, 0.4), (TILE_DARK, 0.2)]),
        },
        RoofMaterial::Slate if region == Region::Storybook => pick(rng, &[(STORY_TEAL_SLATE, 0.5), (STORY_AQUA_SLATE, 0.5)]),
        RoofMaterial::Slate => pick(rng, &[(SLATE, 0.45), (SLATE_BLUE, 0.3), (SLATE_DARK, 0.25)]),
        RoofMaterial::Thatch => pick(rng, &[(THATCH, 0.4), (THATCH_OLD, 0.4), (THATCH_DARK, 0.2)]),
        RoofMaterial::Shingles => pick(rng, &[(SHINGLE, 0.4), (SHINGLE_SILVER, 0.35), (SHINGLE_DARK, 0.25)]),
    };
    let roof = vary(roof, rng, 0.06);

    // Door and window designs.
    let door_kind = match kind {
        B::Temple => pick(rng, &[(DoorKind::Arched, 0.6), (DoorKind::Studded, 0.4)]),
        B::Keep | B::Tower => pick(rng, &[(DoorKind::Studded, 0.8), (DoorKind::Arched, 0.2)]),
        B::Mansion => pick(rng, &[(DoorKind::Studded, 0.45), (DoorKind::Arched, 0.35), (DoorKind::Glazed, 0.2)]),
        B::Tavern => pick(rng, &[(DoorKind::Double, 0.55), (DoorKind::Dutch, 0.3), (DoorKind::Arched, 0.15)]),
        B::Shop => pick(rng, &[(DoorKind::Double, 0.6), (DoorKind::Dutch, 0.3), (DoorKind::Glazed, 0.1)]),
        B::Workshop => pick(rng, &[(DoorKind::Double, 0.45), (DoorKind::Plank, 0.4), (DoorKind::Dutch, 0.15)]),
        B::Barn => pick(rng, &[(DoorKind::Double, 0.9), (DoorKind::Plank, 0.1)]),
        B::Farmhouse => pick(rng, &[(DoorKind::Dutch, 0.5), (DoorKind::Plank, 0.4), (DoorKind::Arched, 0.1)]),
        B::Barracks => pick(rng, &[(DoorKind::Studded, 0.5), (DoorKind::Plank, 0.5)]),
        B::Mill => pick(rng, &[(DoorKind::Plank, 0.7), (DoorKind::Dutch, 0.3)]),
        B::House => {
            if rich {
                pick(
                    rng,
                    &[
                        (DoorKind::Plank, 0.35),
                        (DoorKind::Arched, 0.3),
                        (DoorKind::Glazed, 0.15),
                        (DoorKind::Studded, 0.2),
                    ],
                )
            } else {
                pick(
                    rng,
                    &[
                        (DoorKind::Plank, 0.55),
                        (DoorKind::Dutch, 0.25),
                        (DoorKind::Arched, 0.15),
                        (DoorKind::Glazed, 0.05),
                    ],
                )
            }
        }
    };
    let window_kind = match kind {
        B::Temple => pick(rng, &[(WindowKind::Gothic, 0.85), (WindowKind::Arched, 0.15)]),
        B::Keep => pick(rng, &[(WindowKind::Arched, 0.5), (WindowKind::Mullioned, 0.3), (WindowKind::Round, 0.2)]),
        B::Tower => pick(rng, &[(WindowKind::Arched, 0.5), (WindowKind::Round, 0.3), (WindowKind::Casement, 0.2)]),
        B::Mansion => pick(
            rng,
            &[
                (WindowKind::Mullioned, 0.45),
                (WindowKind::Lattice, 0.35),
                (WindowKind::Gothic, 0.1),
                (WindowKind::Arched, 0.1),
            ],
        ),
        B::Barracks => pick(rng, &[(WindowKind::Casement, 0.5), (WindowKind::Arched, 0.5)]),
        B::Barn => pick(rng, &[(WindowKind::Casement, 0.8), (WindowKind::Round, 0.2)]),
        _ if rich => {
            pick(rng, &[(WindowKind::Lattice, 0.35), (WindowKind::Mullioned, 0.3), (WindowKind::Casement, 0.35)])
        }
        _ => pick(
            rng,
            &[
                (WindowKind::Casement, 0.7),
                (WindowKind::Lattice, 0.2),
                (WindowKind::Round, 0.05),
                (WindowKind::Arched, 0.05),
            ],
        ),
    };

    let door = if matches!(door_kind, DoorKind::Studded) || stone {
        pick(rng, &[(DOOR_DARK_OAK, 0.6), (NATURAL_OAK, 0.25), (OXBLOOD, 0.15)])
    } else {
        match region {
            Region::Northern => {
                pick(rng, &[(DARK_GREEN, 0.3), (OXBLOOD, 0.25), (DOOR_DARK_OAK, 0.3), (SLATE_BLUE_PAINT, 0.15)])
            }
            Region::Southern => {
                pick(rng, &[(NATURAL_OAK, 0.35), (DARK_GREEN, 0.3), (OXBLOOD, 0.2), (SLATE_BLUE_PAINT, 0.15)])
            }
            Region::Storybook => pick(rng, &[(STORY_DOOR_OAK, 0.4), (STORY_DOOR_TEAL, 0.3), (STORY_DOOR_DARK, 0.3)]),
            _ => pick(rng, &[(OXBLOOD, 0.3), (DARK_GREEN, 0.25), (NATURAL_OAK, 0.25), (DOOR_DARK_OAK, 0.2)]),
        }
    };
    let door = vary(door, rng, 0.05);
    let window = if stone {
        pick(rng, &[(FRAME_DARK, 0.6), (FRAME_GREY, 0.4)])
    } else if region == Region::Storybook {
        pick(rng, &[(STORY_OAK_LIGHT, 0.4), (STORY_WINDOW_TEAL, 0.35), (STORY_WINDOW_CLAY, 0.25)])
    } else {
        pick(
            rng,
            &[(FRAME_DARK, 0.4), (FRAME_GREEN, 0.25), (FRAME_GREY, 0.15), (FRAME_NATURAL, 0.15), (FRAME_OXBLOOD, 0.05)],
        )
    };
    let window = vary(window, rng, 0.05);

    let domestic = matches!(kind, B::House | B::Shop | B::Tavern | B::Farmhouse | B::Workshop | B::Mansion);
    let shutter_p = if !domestic || stone {
        0.0
    } else if region == Region::Southern {
        0.7
    } else if poor {
        0.3
    } else {
        0.5
    };
    let shutters = rng.chance(shutter_p);
    let box_p = if !domestic || poor || stone || kind == B::Workshop {
        0.0
    } else if rich || region == Region::Storybook {
        0.3
    } else {
        0.12
    };
    let flower_boxes = rng.chance(box_p);

    Appearance {
        wall,
        timber,
        roof,
        roof_material,
        door,
        door_kind,
        window,
        window_kind,
        shutters,
        flower_boxes,
        custom_window: None,
        custom_door: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_are_valid_and_grounded() {
        let mut rng = Rng::new(3);
        let kinds = [
            BuildingKind::House,
            BuildingKind::Temple,
            BuildingKind::Keep,
            BuildingKind::Barn,
            BuildingKind::Tavern,
            BuildingKind::Mansion,
        ];
        for region in Region::ALL.into_iter().chain([Region::Storybook]) {
            for &k in &kinds {
                for _ in 0..50 {
                    let a = regional_appearance(k, WardKind::Craftsmen, region, &mut rng);
                    for col in [a.wall, a.timber, a.roof, a.door, a.window] {
                        assert!(col.iter().all(|v| (0.0..=1.0).contains(v)));
                        // Muted: no channel spread beyond ~0.5 (no neon).
                        let mx = col[0].max(col[1]).max(col[2]);
                        let mn = col[0].min(col[1]).min(col[2]);
                        assert!(mx - mn < 0.5);
                    }
                    if k == BuildingKind::Temple {
                        assert!(matches!(a.window_kind, WindowKind::Gothic | WindowKind::Arched));
                    }
                }
            }
        }
        let a = default_appearance(BuildingKind::Keep, WardKind::Castle, &mut rng);
        assert!(!a.shutters);
    }

    #[test]
    fn storybook_roofs_are_teal_slate_or_terracotta() {
        let mut rng = Rng::new(9);
        for k in [BuildingKind::House, BuildingKind::Tavern, BuildingKind::Keep] {
            for _ in 0..200 {
                let a = regional_appearance(k, WardKind::Craftsmen, Region::Storybook, &mut rng);
                let [r, g, b, _] = a.roof;
                match a.roof_material {
                    // Teal slate: blue-green. Terracotta tile: red.
                    RoofMaterial::Slate => assert!(b > r + 0.08 && g > r + 0.05, "{:?}", a.roof),
                    RoofMaterial::Tiles => assert!(r > g + 0.15 && r > b + 0.15, "{:?}", a.roof),
                    other => panic!("storybook roof is {other:?}"),
                }
                assert!(a.roof.iter().take(3).all(|v| (0.0..=1.0).contains(v)));
                let dark = |col: Rgba| col.iter().take(3).sum::<f32>() / 3.0 < 0.4;
                assert!(dark(a.timber), "timber {:?}", a.timber);
                if k == BuildingKind::House {
                    let light = a.wall.iter().take(3).sum::<f32>() / 3.0;
                    assert!(light > 0.8, "wall {:?}", a.wall);
                }
            }
        }
    }
}
