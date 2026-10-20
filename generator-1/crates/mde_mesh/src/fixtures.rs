//! Hand-made sample data for tests, examples and previews.
//!
//! These do not come from the generators (which are separate crates); they are
//! small, fully specified [`Dwelling`] / [`Town`] values that exercise every
//! feature of the mesh builders: several floors, interior walls, every
//! opening kind, stairs, every [`PropKind`], rectangular and L-shaped plans,
//! every window / door kind, walls with towers and gates, streets, a river
//! with both kinds of bridge, fields, trees and a mix of building kinds.

use mde_core::{
    Appearance, Bridge, BridgeKind, Building, BuildingKind, DoorKind, Dwelling, DwellingParams, DwellingStyle, Floor,
    Opening, OpeningKind, Patch, Polygon, Polyline, Prop, PropKind, River, Rng, Roof, RoofKind, RoofMaterial, Room,
    RoomKind, Stair, Street, StreetKind, Town, TownParams, Tree, Vec2, Wall, WallSegment, WardKind, WindowKind, Yard,
    SCHEMA_VERSION,
};

const EXT: f32 = 0.4;
const INT: f32 = 0.15;
const STOREY: f32 = 3.0;
const CEIL: f32 = 2.75;

fn v(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y)
}

fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Polygon {
    Polygon(vec![v(x0, y0), v(x1, y0), v(x1, y1), v(x0, y1)])
}

fn wall(a: (f32, f32), b: (f32, f32), exterior: bool) -> WallSegment {
    WallSegment { a: v(a.0, a.1), b: v(b.0, b.1), thickness: if exterior { EXT } else { INT }, exterior }
}

fn opening(kind: OpeningKind, a: (f32, f32), b: (f32, f32), rooms: [Option<u32>; 2]) -> Opening {
    let (sill, height) = match kind {
        OpeningKind::Window => (0.9, 1.25),
        OpeningKind::Entrance => (0.0, 2.2),
        _ => (0.0, 2.1),
    };
    Opening { kind, a: v(a.0, a.1), b: v(b.0, b.1), sill, height, rooms }
}

fn window(a: (f32, f32), b: (f32, f32), room: u32) -> Opening {
    opening(OpeningKind::Window, a, b, [Some(room), None])
}

fn prop(kind: PropKind, room: u32, pos: (f32, f32), size: (f32, f32), rotation: f32) -> Prop {
    Prop { kind, room, pos: v(pos.0, pos.1), size: v(size.0, size.1), rotation }
}

fn floor(level: i32, outline: &Polygon) -> Floor {
    Floor {
        level,
        elevation: level as f32 * STOREY,
        height: CEIL,
        outline: outline.clone(),
        rooms: Vec::new(),
        walls: Vec::new(),
        openings: Vec::new(),
        stairs: Vec::new(),
        props: Vec::new(),
    }
}

/// Appearance used by the fixture houses: lattice windows, arched door,
/// shutters and flower boxes, clay tiles.
pub fn showcase_appearance() -> Appearance {
    Appearance {
        wall: [0.91, 0.87, 0.78, 1.0],
        timber: [0.36, 0.25, 0.17, 1.0],
        roof: [0.66, 0.36, 0.30, 1.0],
        roof_material: RoofMaterial::Tiles,
        door: [0.38, 0.24, 0.15, 1.0],
        door_kind: DoorKind::Arched,
        window: [0.38, 0.27, 0.19, 1.0],
        window_kind: WindowKind::Lattice,
        shutters: true,
        flower_boxes: true,
        custom_window: None,
        custom_door: None,
    }
}

/// A fixture house restyled with a storybook preset: the L-shaped house for
/// reference 02 (the manor), the rectangular townhouse for 01 and 03. Build
/// it with [`crate::StyleProfile::storybook`] in the mesh options.
pub fn storybook_house(preset: crate::Storybook) -> Dwelling {
    let mut d = match preset {
        crate::Storybook::TealManor02 => l_house(),
        _ => rect_house(),
    };
    d.params.style = preset.dwelling_style();
    preset.apply_to_dwelling(&mut d);
    d
}

/// A two-storey 8 × 10 m town house restyled with a storybook preset, facing
/// south. Build it with [`crate::build_lot_house_with`].
pub fn storybook_building(preset: crate::Storybook, seed: u64) -> Building {
    let mut b = Building {
        id: seed as u32,
        patch: 0,
        kind: BuildingKind::House,
        footprint: rect(-4.0, -5.0, 4.0, 5.0),
        floors: 2,
        front: Vec2::new(0.0, -1.0),
        seed,
        appearance: Appearance::default(),
    };
    preset.apply_to_building(&mut b);
    b
}

/// Three-storey rectangular townhouse (10 × 7 m): hall, kitchen and stairwell
/// on the ground floor, two bedrooms above, a study under the roof; two-lane
/// stairwell; every [`PropKind`] somewhere; gable roof along x.
pub fn rect_house() -> Dwelling {
    let outline = rect(0.0, 0.0, 10.0, 7.0);
    let (x0, y0, x1, y1) = (0.2, 0.2, 9.8, 6.8);
    let ext = vec![
        wall((x0, y0), (x1, y0), true),
        wall((x0, y1), (x1, y1), true),
        wall((x0, y0), (x0, y1), true),
        wall((x1, y0), (x1, y1), true),
    ];
    let half = std::f32::consts::FRAC_PI_2;
    let pi = std::f32::consts::PI;
    let mut floors = Vec::new();

    // Ground floor.
    let mut f0 = floor(0, &outline);
    f0.rooms = vec![
        Room { id: 0, kind: RoomKind::Hall, shape: rect(x0, y0, 4.0, y1) },
        Room { id: 1, kind: RoomKind::Kitchen, shape: rect(4.0, y0, 6.6, y1) },
        Room { id: 2, kind: RoomKind::Stairwell, shape: rect(6.6, y0, x1, y1) },
    ];
    f0.walls = ext.clone();
    f0.walls.push(wall((4.0, y0), (4.0, y1), false));
    f0.walls.push(wall((6.6, y0), (6.6, y1), false));
    f0.openings = vec![
        opening(OpeningKind::Entrance, (1.5, y0), (2.6, y0), [Some(0), None]),
        window((4.8, y0), (5.8, y0), 1),
        window((7.6, y0), (8.5, y0), 2),
        window((1.2, y1), (2.2, y1), 0),
        window((4.8, y1), (5.8, y1), 1),
        window((x0, 3.0), (x0, 4.0), 0),
        window((x1, 3.0), (x1, 3.9), 2),
        opening(OpeningKind::Door, (4.0, 3.0), (4.0, 3.95), [Some(0), Some(1)]),
        opening(OpeningKind::Door, (6.6, 0.55), (6.6, 1.5), [Some(1), Some(2)]),
    ];
    f0.stairs = vec![Stair { footprint: rect(8.3, 1.6, 9.6, 5.6), up: v(0.0, 1.0), from_level: 0, to_level: 1 }];
    f0.props = vec![
        prop(PropKind::Hearth, 0, (0.75, 5.3), (1.5, 0.8), -half),
        prop(PropKind::Table, 0, (2.2, 3.4), (1.6, 0.9), 0.0),
        prop(PropKind::Chair, 0, (1.7, 2.65), (0.45, 0.45), 0.0),
        prop(PropKind::Chair, 0, (2.7, 2.65), (0.45, 0.45), 0.0),
        prop(PropKind::Chair, 0, (2.2, 4.15), (0.45, 0.45), pi),
        prop(PropKind::Counter, 1, (5.3, 6.3), (2.0, 0.6), pi),
        prop(PropKind::Stove, 1, (4.5, 4.6), (0.8, 0.7), -half),
        prop(PropKind::Barrel, 1, (6.2, 3.2), (0.6, 0.6), 0.0),
        prop(PropKind::Crate, 1, (6.15, 4.3), (0.6, 0.6), 0.0),
        prop(PropKind::Bench, 2, (7.2, 3.5), (1.6, 0.4), -half),
    ];
    floors.push(f0);

    // First floor.
    let mut f1 = floor(1, &outline);
    f1.rooms = vec![
        Room { id: 10, kind: RoomKind::Bedroom, shape: rect(x0, y0, 4.0, y1) },
        Room { id: 11, kind: RoomKind::Bedroom, shape: rect(4.0, y0, 6.6, y1) },
        Room { id: 12, kind: RoomKind::Stairwell, shape: rect(6.6, y0, x1, y1) },
    ];
    f1.walls = ext.clone();
    f1.walls.push(wall((4.0, y0), (4.0, y1), false));
    f1.walls.push(wall((6.6, y0), (6.6, y1), false));
    f1.openings = vec![
        window((1.5, y0), (2.5, y0), 10),
        window((4.8, y0), (5.8, y0), 11),
        window((7.6, y0), (8.5, y0), 12),
        window((1.2, y1), (2.2, y1), 10),
        window((4.8, y1), (5.8, y1), 11),
        window((x0, 3.0), (x0, 4.0), 10),
        window((x1, 3.0), (x1, 3.9), 12),
        opening(OpeningKind::Door, (4.0, 3.0), (4.0, 3.95), [Some(10), Some(11)]),
        opening(OpeningKind::Door, (6.6, 5.6), (6.6, 6.45), [Some(11), Some(12)]),
    ];
    f1.stairs = vec![Stair { footprint: rect(6.8, 1.6, 8.1, 5.6), up: v(0.0, -1.0), from_level: 1, to_level: 2 }];
    f1.props = vec![
        prop(PropKind::Bed, 10, (1.3, 5.6), (1.6, 2.0), pi),
        prop(PropKind::Chest, 10, (1.3, 4.25), (0.9, 0.5), pi),
        prop(PropKind::Wardrobe, 10, (3.6, 1.3), (1.0, 0.55), half),
        prop(PropKind::Cot, 11, (5.3, 5.6), (0.9, 1.9), pi),
        prop(PropKind::Bathtub, 11, (5.3, 1.0), (1.5, 0.8), 0.0),
        prop(PropKind::Bookshelf, 11, (6.35, 3.2), (1.2, 0.35), half),
    ];
    floors.push(f1);

    // Second floor (under the roof).
    let mut f2 = floor(2, &outline);
    f2.rooms = vec![
        Room { id: 20, kind: RoomKind::Study, shape: rect(x0, y0, 6.6, y1) },
        Room { id: 21, kind: RoomKind::Stairwell, shape: rect(6.6, y0, x1, y1) },
    ];
    f2.walls = ext;
    f2.walls.push(wall((6.6, y0), (6.6, y1), false));
    f2.openings = vec![
        window((2.0, y0), (3.0, y0), 20),
        window((4.6, y0), (5.6, y0), 20),
        window((2.0, y1), (3.0, y1), 20),
        window((x0, 3.0), (x0, 4.0), 20),
        window((x1, 3.0), (x1, 3.9), 21),
        opening(OpeningKind::Doorway, (6.6, 0.5), (6.6, 1.5), [Some(20), Some(21)]),
    ];
    f2.props = vec![
        prop(PropKind::Desk, 20, (2.5, 6.2), (1.4, 0.7), pi),
        prop(PropKind::Chair, 20, (2.5, 5.55), (0.45, 0.45), pi),
        prop(PropKind::Shelf, 20, (0.55, 2.0), (1.2, 0.35), -half),
        prop(PropKind::Altar, 20, (6.0, 4.0), (1.2, 0.7), half),
        prop(PropKind::Anvil, 20, (3.6, 2.6), (0.6, 0.4), 0.3),
        prop(PropKind::Table, 20, (4.8, 2.0), (1.2, 0.8), 0.0),
    ];
    floors.push(f2);

    let appearance = showcase_appearance();
    Dwelling {
        schema: SCHEMA_VERSION,
        params: DwellingParams {
            seed: 7,
            footprint: outline,
            floors: 3,
            front: v(0.0, -1.0),
            style: DwellingStyle::Townhouse,
            appearance: Some(appearance.clone()),
            ..Default::default()
        },
        floors,
        roof: Roof { kind: RoofKind::Gable, pitch_deg: 46.0, overhang: 0.45, ridge: v(1.0, 0.0) },
        appearance,
    }
}

/// Two-storey L-shaped house (12 × 12 m, 6 m wings) with an interior wall that
/// continues an exterior wall at the inner corner; half-hip roof.
pub fn l_house() -> Dwelling {
    let outline = Polygon(vec![v(0.0, 0.0), v(12.0, 0.0), v(12.0, 6.0), v(6.0, 6.0), v(6.0, 12.0), v(0.0, 12.0)]);
    let ext = [
        wall((0.2, 0.2), (11.8, 0.2), true),
        wall((11.8, 0.2), (11.8, 5.8), true),
        wall((5.8, 5.8), (11.8, 5.8), true),
        wall((5.8, 5.8), (5.8, 11.8), true),
        wall((0.2, 11.8), (5.8, 11.8), true),
        wall((0.2, 0.2), (0.2, 11.8), true),
    ];
    let inner = [wall((0.2, 5.8), (5.8, 5.8), false), wall((6.0, 0.2), (6.0, 5.8), false)];
    let mut floors = Vec::new();
    for level in 0..2 {
        let base = level as u32 * 10;
        let mut f = floor(level, &outline);
        f.rooms = vec![
            Room { id: base, kind: if level == 0 { RoomKind::Hall } else { RoomKind::Bedroom }, shape: rect(0.2, 0.2, 6.0, 5.8) },
            Room { id: base + 1, kind: if level == 0 { RoomKind::Kitchen } else { RoomKind::Study }, shape: rect(6.0, 0.2, 11.8, 5.8) },
            Room { id: base + 2, kind: if level == 0 { RoomKind::Living } else { RoomKind::Bedroom }, shape: rect(0.2, 5.8, 5.8, 11.8) },
        ];
        f.walls = ext.iter().chain(inner.iter()).cloned().collect();
        f.openings = vec![
            window((7.5, 0.2), (8.5, 0.2), base + 1),
            window((9.8, 0.2), (10.8, 0.2), base + 1),
            window((11.8, 2.5), (11.8, 3.5), base + 1),
            window((8.0, 5.8), (9.0, 5.8), base + 1),
            window((5.8, 8.0), (5.8, 9.0), base + 2),
            window((2.0, 11.8), (3.0, 11.8), base + 2),
            window((0.2, 8.0), (0.2, 9.0), base + 2),
            window((0.2, 3.0), (0.2, 4.0), base),
            opening(OpeningKind::Door, (2.5, 5.8), (3.4, 5.8), [Some(base), Some(base + 2)]),
            opening(OpeningKind::Door, (6.0, 2.6), (6.0, 3.5), [Some(base), Some(base + 1)]),
        ];
        if level == 0 {
            f.openings.push(opening(OpeningKind::Entrance, (3.0, 0.2), (4.1, 0.2), [Some(base), None]));
            f.stairs = vec![Stair { footprint: rect(0.5, 1.0, 1.6, 4.6), up: v(0.0, 1.0), from_level: 0, to_level: 1 }];
            f.props = vec![
                prop(PropKind::Hearth, base + 2, (0.75, 10.0), (1.4, 0.8), -std::f32::consts::FRAC_PI_2),
                prop(PropKind::Table, base + 1, (9.0, 3.0), (1.8, 1.0), 0.0),
            ];
        } else {
            f.openings.push(window((3.0, 0.2), (4.0, 0.2), base));
            f.props = vec![prop(PropKind::Bed, base, (4.5, 4.6), (1.6, 2.0), std::f32::consts::PI)];
        }
        floors.push(f);
    }
    let appearance = Appearance {
        wall: [0.86, 0.80, 0.68, 1.0],
        roof_material: RoofMaterial::Thatch,
        roof: [0.72, 0.62, 0.42, 1.0],
        window_kind: WindowKind::Casement,
        door_kind: DoorKind::Plank,
        shutters: true,
        ..Appearance::default()
    };
    Dwelling {
        schema: SCHEMA_VERSION,
        params: DwellingParams {
            seed: 11,
            footprint: outline,
            floors: 2,
            front: v(0.0, -1.0),
            style: DwellingStyle::Farmhouse,
            appearance: Some(appearance.clone()),
            ..Default::default()
        },
        floors,
        roof: Roof { kind: RoofKind::HalfHip, pitch_deg: 48.0, overhang: 0.5, ridge: v(1.0, 0.0) },
        appearance,
    }
}

/// A one-room, one-storey "wall sample" box 3.2 × 2.4 m at `origin` with a
/// window of `window` kind or an entrance of `door` kind on its south wall.
pub fn wall_sample(origin: Vec2, window: Option<WindowKind>, door: Option<DoorKind>, seed: u64) -> Dwelling {
    let (w, d) = (3.2, 2.4);
    let o = origin;
    let outline = rect(o.x, o.y, o.x + w, o.y + d);
    let (x0, y0, x1, y1) = (o.x + 0.2, o.y + 0.2, o.x + w - 0.2, o.y + d - 0.2);
    let mut f = floor(0, &outline);
    f.height = 3.2;
    f.rooms = vec![Room { id: 0, kind: RoomKind::Living, shape: rect(x0, y0, x1, y1) }];
    f.walls = vec![
        wall((x0, y0), (x1, y0), true),
        wall((x0, y1), (x1, y1), true),
        wall((x0, y0), (x0, y1), true),
        wall((x1, y0), (x1, y1), true),
    ];
    let cx = o.x + w * 0.5;
    let mut app = showcase_appearance();
    if let Some(k) = window {
        app.window_kind = k;
        let (ww, h) = if k == WindowKind::Round { (0.8, 0.8) } else { (1.0, 1.5) };
        let mut op = opening(OpeningKind::Window, (cx - ww * 0.5, y0), (cx + ww * 0.5, y0), [Some(0), None]);
        op.height = h;
        f.openings.push(op);
        app.shutters = seed.is_multiple_of(2);
        app.flower_boxes = seed % 3 != 2;
    }
    if let Some(k) = door {
        app.door_kind = k;
        let dw = if k == DoorKind::Double { 1.7 } else { 1.1 };
        f.openings.push(opening(OpeningKind::Entrance, (cx - dw * 0.5, y0), (cx + dw * 0.5, y0), [Some(0), None]));
        app.shutters = false;
        app.flower_boxes = false;
    }
    Dwelling {
        schema: SCHEMA_VERSION,
        params: DwellingParams {
            seed,
            footprint: outline,
            floors: 1,
            style: DwellingStyle::Cottage,
            appearance: Some(app.clone()),
            ..Default::default()
        },
        floors: vec![f],
        roof: Roof { kind: RoofKind::Gable, pitch_deg: 40.0, overhang: 0.3, ridge: v(1.0, 0.0) },
        appearance: app,
    }
}

/// Every window kind followed by every door kind, on wall samples in a row along x.
pub fn kind_samples() -> Vec<Dwelling> {
    let windows = [
        WindowKind::Casement,
        WindowKind::Mullioned,
        WindowKind::Arched,
        WindowKind::Gothic,
        WindowKind::Round,
        WindowKind::Lattice,
    ];
    let doors = [DoorKind::Plank, DoorKind::Arched, DoorKind::Double, DoorKind::Dutch, DoorKind::Studded, DoorKind::Glazed];
    let mut out = Vec::new();
    for (i, &k) in windows.iter().enumerate() {
        out.push(wall_sample(v(i as f32 * 4.2, 0.0), Some(k), None, i as u64));
    }
    for (i, &k) in doors.iter().enumerate() {
        out.push(wall_sample(v((i + windows.len()) as f32 * 4.2, 0.0), None, Some(k), 100 + i as u64));
    }
    out
}

/// Pleasant varied appearance for building number `i`.
pub fn town_appearance(i: usize, rng: &mut Rng) -> Appearance {
    let walls = [[0.92, 0.88, 0.79], [0.88, 0.80, 0.66], [0.80, 0.76, 0.70], [0.93, 0.90, 0.84], [0.84, 0.72, 0.58], [0.76, 0.80, 0.78]];
    let timbers = [[0.36, 0.25, 0.17], [0.28, 0.20, 0.15], [0.45, 0.33, 0.23], [0.22, 0.19, 0.18]];
    let roofs = [
        ([0.66, 0.34, 0.26], RoofMaterial::Tiles),
        ([0.58, 0.30, 0.24], RoofMaterial::Tiles),
        ([0.36, 0.38, 0.42], RoofMaterial::Slate),
        ([0.72, 0.62, 0.42], RoofMaterial::Thatch),
        ([0.48, 0.38, 0.30], RoofMaterial::Shingles),
    ];
    let doors = [[0.38, 0.24, 0.15], [0.30, 0.35, 0.28], [0.45, 0.20, 0.16], [0.28, 0.30, 0.38]];
    let windows = [[0.30, 0.40, 0.33], [0.38, 0.27, 0.19], [0.55, 0.50, 0.42], [0.30, 0.33, 0.42]];
    let wk = [WindowKind::Casement, WindowKind::Lattice, WindowKind::Mullioned, WindowKind::Arched, WindowKind::Casement];
    let dk = [DoorKind::Plank, DoorKind::Arched, DoorKind::Plank, DoorKind::Dutch, DoorKind::Glazed, DoorKind::Studded];
    let pick3 = |a: [f32; 3]| [a[0], a[1], a[2], 1.0];
    let (roof, mat) = roofs[(i + rng.index(roofs.len())) % roofs.len()];
    Appearance {
        wall: pick3(walls[rng.index(walls.len())]),
        timber: pick3(timbers[rng.index(timbers.len())]),
        roof: pick3(roof),
        roof_material: mat,
        door: pick3(doors[rng.index(doors.len())]),
        door_kind: dk[rng.index(dk.len())],
        window: pick3(windows[rng.index(windows.len())]),
        window_kind: wk[rng.index(wk.len())],
        shutters: rng.chance(0.4),
        flower_boxes: rng.chance(0.35),
        custom_window: None,
        custom_door: None,
    }
}

/// A small walled town: plaza, arteries and a ring street, a river crossing
/// the south with a wooden bridge in town and a stone bridge outside, a
/// citadel, fields, trees and ~60 buildings of every kind.
pub fn town() -> Town {
    let mut rng = Rng::new(2024);
    let center = Vec2::ZERO;
    let radius = 110.0;
    // Wall ring: 14-gon with a little wobble.
    let n = 14;
    let ring: Vec<Vec2> = (0..n)
        .map(|k| {
            let a = k as f32 / n as f32 * std::f32::consts::TAU;
            Vec2::from_angle(a) * (85.0 + rng.range_f32(-4.0, 4.0))
        })
        .collect();
    let gates = vec![ring[0], ring[7], ring[11]];
    let towers: Vec<Vec2> = ring.iter().enumerate().filter(|(k, _)| ![0, 7, 11].contains(k)).map(|(_, p)| *p).collect();
    let wall = Wall { shape: Polygon(ring.clone()), towers, gates: gates.clone(), height: 8.0, thickness: 2.4 };
    let cit_c = v(48.0, 42.0);
    let cit_ring = Polygon::regular(cit_c, 16.0, 6, 0.3);
    let citadel = Wall {
        shape: cit_ring.clone(),
        towers: cit_ring.0[1..].to_vec(),
        gates: vec![cit_ring.0[3]],
        height: 10.0,
        thickness: 2.0,
    };
    // Patches: inner sectors and outer countryside.
    let mut patches = Vec::new();
    for k in 0..n {
        let a = ring[k];
        let b = ring[(k + 1) % n];
        patches.push(Patch {
            id: k as u32,
            shape: Polygon(vec![center, a, b]),
            ward: if k % 5 == 2 { WardKind::Park } else { WardKind::Craftsmen },
            within_walls: true,
            within_city: true,
        });
        patches.push(Patch {
            id: 100 + k as u32,
            shape: Polygon(vec![a, a * 2.0, b * 2.0, b]),
            ward: if k % 2 == 0 { WardKind::Farm } else { WardKind::Empty },
            within_walls: false,
            within_city: false,
        });
    }
    let plaza = Polygon::regular(center, 14.0, 8, 0.2);
    let line = |pts: &[(f32, f32)]| Polyline(pts.iter().map(|&(x, y)| v(x, y)).collect());
    let mut streets = vec![
        Street { kind: StreetKind::Artery, path: line(&[(13.0, 0.0), (45.0, 1.0), (ring[0].x, ring[0].y)]), width: 6.0 },
        Street { kind: StreetKind::Artery, path: line(&[(-13.0, 0.0), (-45.0, -1.0), (ring[7].x, ring[7].y)]), width: 6.0 },
        Street { kind: StreetKind::Artery, path: line(&[(0.0, -13.0), (2.0, -50.0), (ring[11].x, ring[11].y)]), width: 5.0 },
        Street { kind: StreetKind::Road, path: line(&[(ring[0].x, ring[0].y), (120.0, -2.0), (122.0, -40.0), (125.0, -90.0), (140.0, -170.0)]), width: 5.0 },
        Street { kind: StreetKind::Road, path: line(&[(ring[7].x, ring[7].y), (-130.0, 5.0), (-175.0, 20.0)]), width: 5.0 },
        Street { kind: StreetKind::Alley, path: line(&[(30.0, 1.0), (30.0, 30.0), (40.0, 38.0)]), width: 2.5 },
        Street { kind: StreetKind::Alley, path: line(&[(-30.0, -1.0), (-30.0, 30.0)]), width: 2.5 },
    ];
    let ring_street: Vec<Vec2> = (0..=16).map(|k| Vec2::from_angle(k as f32 / 16.0 * std::f32::consts::TAU + 0.1) * 45.0).collect();
    streets.push(Street { kind: StreetKind::Street, path: Polyline(ring_street), width: 4.0 });

    // River across the south.
    let river = River {
        path: line(&[(-190.0, -70.0), (-110.0, -52.0), (-50.0, -64.0), (0.0, -58.0), (60.0, -50.0), (122.0, -46.0), (190.0, -30.0)]),
        width: 12.0,
    };
    let bridges = vec![
        Bridge { kind: BridgeKind::Wood, a: v(1.6, -46.0), b: v(2.6, -70.0), width: 4.0, height: 1.6 },
        Bridge { kind: BridgeKind::Stone, a: v(122.3, -31.0), b: v(123.4, -61.0), width: 5.0, height: 2.4 },
    ];

    // Buildings along the arteries.
    let mut buildings = Vec::new();
    let mut id = 0u32;
    let kinds = [BuildingKind::House, BuildingKind::Shop, BuildingKind::House, BuildingKind::Tavern, BuildingKind::Workshop, BuildingKind::House, BuildingKind::Mansion];
    let mut push = |fp: Polygon, kind: BuildingKind, floors: u8, front: Vec2, rng: &mut Rng, buildings: &mut Vec<Building>| {
        let appearance = town_appearance(id as usize, rng);
        buildings.push(Building { id, patch: 0, kind, footprint: fp, floors, front, seed: 1000 + id as u64, appearance });
        id += 1;
    };
    for side in [-1.0f32, 1.0] {
        let mut x = 18.0;
        while x < 72.0 {
            let w = rng.range_f32(6.0, 9.0);
            for dir in [1.0f32, -1.0] {
                let depth = rng.range_f32(8.0, 11.0);
                let (ya, yb) = if dir > 0.0 { (4.0, 4.0 + depth) } else { (-4.0 - depth, -4.0) };
                let xs = side * x;
                let (xa, xb) = if side > 0.0 { (xs, xs + w - 0.3) } else { (xs - w + 0.3, xs) };
                let fp = rect(xa, ya, xb, yb);
                let kind = kinds[rng.index(kinds.len())];
                let floors = 1 + rng.index(3) as u8;
                push(fp, kind, floors, v(0.0, -dir), &mut rng, &mut buildings);
            }
            x += w;
        }
    }
    // Along the south artery (west side), irregular lots.
    let mut y = -17.0;
    while y > -40.0 {
        let h = rng.range_f32(6.0, 8.0);
        let fp = Polygon(vec![v(-12.5, y - h), v(-3.5, y - h + 0.6), v(-3.5, y), v(-12.0, y)]);
        push(fp, BuildingKind::House, 2, v(1.0, 0.0), &mut rng, &mut buildings);
        let fp = Polygon(vec![v(7.0, y - h), v(15.5, y - h - 0.5), v(16.0, y), v(7.0, y)]);
        push(fp, BuildingKind::Shop, 2 + rng.index(2) as u8, v(-1.0, 0.0), &mut rng, &mut buildings);
        y -= h;
    }
    // Landmarks.
    push(rect(-12.0, 18.0, 12.0, 32.0), BuildingKind::Temple, 2, v(0.0, -1.0), &mut rng, &mut buildings);
    push(rect(42.0, 36.0, 54.0, 48.0), BuildingKind::Keep, 4, v(0.0, -1.0), &mut rng, &mut buildings);
    push(Polygon::regular(v(-60.0, 40.0), 4.0, 8, 0.0), BuildingKind::Tower, 4, v(1.0, 0.0), &mut rng, &mut buildings);
    push(rect(-62.0, -30.0, -42.0, -20.0), BuildingKind::Barracks, 2, v(0.0, 1.0), &mut rng, &mut buildings);
    push(rect(100.0, 30.0, 110.0, 44.0), BuildingKind::Barn, 1, v(-1.0, 0.0), &mut rng, &mut buildings);
    push(rect(112.0, 30.0, 122.0, 38.0), BuildingKind::Farmhouse, 2, v(0.0, -1.0), &mut rng, &mut buildings);
    push(rect(-140.0, -30.0, -128.0, -22.0), BuildingKind::Mill, 2, v(0.0, -1.0), &mut rng, &mut buildings);
    push(
        Polygon(vec![v(20.0, 18.0), v(32.0, 18.0), v(32.0, 24.0), v(26.0, 24.0), v(26.0, 30.0), v(20.0, 30.0)]),
        BuildingKind::Mansion,
        2,
        v(0.0, -1.0),
        &mut rng,
        &mut buildings,
    );

    // Fields to the north-east and north-west outside the walls.
    let mut fields = Vec::new();
    for k in 0..6 {
        let a0 = 0.35 + k as f32 * 0.17;
        let a1 = a0 + 0.15;
        let (r0, r1) = (100.0, 160.0);
        fields.push(Polygon(vec![
            Vec2::from_angle(a0) * r0,
            Vec2::from_angle(a0) * r1,
            Vec2::from_angle(a1) * r1,
            Vec2::from_angle(a1) * r0,
        ]));
        let b0 = 2.1 + k as f32 * 0.17;
        fields.push(Polygon(vec![
            Vec2::from_angle(b0) * r0,
            Vec2::from_angle(b0 + 0.15) * r0,
            Vec2::from_angle(b0 + 0.15) * r1,
            Vec2::from_angle(b0) * r1,
        ]));
    }
    // Trees: countryside groves and a few in town.
    let mut trees = Vec::new();
    for _ in 0..180 {
        let a = rng.range_f32(3.6, 6.2);
        let r = rng.range_f32(95.0, 170.0);
        let pos = Vec2::from_angle(a) * r;
        let near_river = (pos.y + 55.0).abs() < 12.0;
        if !near_river {
            trees.push(Tree { pos, radius: rng.range_f32(1.6, 3.6) });
        }
    }
    for k in 0..12 {
        let pos = Vec2::from_angle(0.25 * k as f32 + 1.4) * 60.0;
        trees.push(Tree { pos, radius: rng.range_f32(1.8, 3.0) });
    }
    // A fenced backyard behind the farmhouse, gate in the back fence.
    let farm = buildings.iter().position(|b| b.kind == BuildingKind::Farmhouse).unwrap_or(0) as u32;
    let yards = vec![Yard {
        building: farm,
        lot: rect(112.0, 30.0, 122.0, 46.0),
        fences: vec![
            Polyline(vec![v(112.0, 38.0), v(112.0, 46.0), v(116.4, 46.0)]),
            Polyline(vec![v(117.6, 46.0), v(122.0, 46.0), v(122.0, 38.0)]),
        ],
        gates: vec![(v(116.4, 46.0), v(117.6, 46.0))],
    }];

    Town {
        schema: SCHEMA_VERSION,
        params: TownParams { seed: 2024, river: true, ..TownParams::default() },
        center,
        radius,
        patches,
        wall: Some(wall),
        citadel: Some(citadel),
        plaza: Some(plaza),
        streets,
        buildings,
        fields,
        trees,
        river: Some(river),
        bridges,
        yards,
    }
}
