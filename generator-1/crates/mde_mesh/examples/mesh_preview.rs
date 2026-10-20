//! Writes preview `.glb` files of the fixture data to `out/mde_mesh/`:
//!
//! * `fixture_house.glb`: rectangular and L-shaped houses with roofs.
//! * `fixture_house_cutaway.glb`: the townhouse cut at the ground and first floor.
//! * `window_door_kinds.glb`: a wall sample of every window and door kind.
//! * `roof_kinds.glb`: every roof kind on the rectangular and L-shaped houses
//!   (classic profile: the default storybook roof is always a gable).
//! * `bridges.glb`: wooden and stone bridges of several sizes over a river.
//! * `lot_houses.glb`: one massing house per building kind.
//! * `fixture_town.glb`: the fixture town.
//! * `storybook_ref01.glb` … `storybook_ref03.glb`: the storybook presets on a
//!   fixture house and on a lot house beside it (16 m to the east).
//! * `storybook_wide_lot01.glb` … `03`: the presets on a wide 12 × 7 m lot
//!   house, which gets a front cross gable.
//!
//! Run with `cargo run -p mde_mesh --example mesh_preview --release`.

use std::path::{Path, PathBuf};

use mde_core::{Bridge, BridgeKind, Building, BuildingKind, Model, Polygon, Polyline, River, RoofKind, RoofMaterial, Rng, Town, Vec2};
use mde_mesh::{
    build_dwelling, build_lot_house, build_lot_house_with, build_town, fixtures, gltf, DwellingMeshOptions, Storybook,
    StyleProfile, TownMeshOptions,
};

fn shifted(mut m: Model, dx: f32, dz: f32) -> Model {
    for p in &mut m.parts {
        for v in &mut p.mesh.positions {
            v[0] += dx;
            v[2] += dz;
        }
    }
    m
}

/// A typical sRGB colour for a roof covering.
fn roof_colour(m: RoofMaterial) -> [f32; 4] {
    match m {
        RoofMaterial::Tiles => [0.66, 0.34, 0.26, 1.0],
        RoofMaterial::Slate => [0.36, 0.38, 0.43, 1.0],
        RoofMaterial::Thatch => [0.62, 0.52, 0.34, 1.0],
        RoofMaterial::Shingles => [0.47, 0.37, 0.29, 1.0],
    }
}

fn write(dir: &Path, name: &str, models: &[Model]) {
    let bytes = gltf::write_glb(models);
    let path = dir.join(name);
    if let Err(e) = std::fs::write(&path, &bytes) {
        eprintln!("cannot write {}: {e}", path.display());
        return;
    }
    let tris: usize = models.iter().map(|m| m.triangle_count()).sum();
    println!("{:<28} {:>9} triangles {:>10} bytes", name, tris, bytes.len());
}

fn main() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/mde_mesh");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("cannot create {}: {e}", dir.display());
        return;
    }
    let full = DwellingMeshOptions::default();

    let house = fixtures::rect_house();
    let mut l = fixtures::l_house();
    l.roof.kind = RoofKind::HalfHip;
    write(&dir, "fixture_house.glb", &[build_dwelling(&house, &full), shifted(build_dwelling(&l, &full), 16.0, 0.0)]);

    let cut1 = DwellingMeshOptions { max_level: Some(1), door_leaves: false, ..Default::default() };
    let cut0 = DwellingMeshOptions { max_level: Some(0), ..Default::default() };
    write(
        &dir,
        "fixture_house_cutaway.glb",
        &[build_dwelling(&house, &cut1), shifted(build_dwelling(&house, &cut0), 14.0, 0.0), shifted(build_dwelling(&l, &cut0), 28.0, 0.0)],
    );

    // Every roof kind on the rectangular and the L-shaped house.
    let shell = DwellingMeshOptions { interior: false, props: false, profile: StyleProfile::classic(), ..Default::default() };
    let kinds = [RoofKind::Gable, RoofKind::Hip, RoofKind::HalfHip, RoofKind::Gambrel, RoofKind::Tent, RoofKind::Flat];
    let mats = [RoofMaterial::Tiles, RoofMaterial::Slate, RoofMaterial::Thatch, RoofMaterial::Shingles, RoofMaterial::Tiles, RoofMaterial::Slate];
    let mut roofs = Vec::new();
    for (i, (&kind, &mat)) in kinds.iter().zip(&mats).enumerate() {
        let mut h = fixtures::rect_house();
        h.roof.kind = kind;
        h.appearance.roof_material = mat;
        h.appearance.roof = roof_colour(mat);
        let mut m = shifted(build_dwelling(&h, &shell), i as f32 * 15.0, 0.0);
        m.name = format!("rect_{kind:?}");
        roofs.push(m);
        let mut l = fixtures::l_house();
        l.roof.kind = kind;
        l.appearance.roof_material = mats[(i + 2) % mats.len()];
        l.appearance.roof = roof_colour(l.appearance.roof_material);
        let mut m = shifted(build_dwelling(&l, &shell), i as f32 * 15.0, 18.0);
        m.name = format!("l_{kind:?}");
        roofs.push(m);
    }
    write(&dir, "roof_kinds.glb", &roofs);

    let no_roof = DwellingMeshOptions { roof: false, ..Default::default() };
    let samples: Vec<Model> = fixtures::kind_samples().iter().map(|d| build_dwelling(d, &no_roof)).collect();
    write(&dir, "window_door_kinds.glb", &samples);

    // Bridges of several sizes over a river running north–south through x = 0.
    let specs = [
        (BridgeKind::Wood, 30.0, 4.0, 2.0),
        (BridgeKind::Wood, 8.0, 2.5, 1.0),
        (BridgeKind::Wood, 60.0, 5.0, 0.0),
        (BridgeKind::Stone, 30.0, 5.0, 3.0),
        (BridgeKind::Stone, 12.0, 3.5, 2.0),
        (BridgeKind::Stone, 70.0, 7.0, 4.0),
    ];
    let mut scene = Town {
        schema: mde_core::SCHEMA_VERSION,
        params: Default::default(),
        center: Vec2::new(0.0, 30.0),
        radius: 30.0,
        patches: Vec::new(),
        wall: None,
        citadel: None,
        plaza: None,
        streets: Vec::new(),
        buildings: Vec::new(),
        fields: Vec::new(),
        trees: Vec::new(),
        river: Some(River { path: Polyline(vec![Vec2::new(-3.0, -40.0), Vec2::new(2.0, 30.0), Vec2::new(-2.0, 100.0)]), width: 7.0 }),
        bridges: Vec::new(),
        yards: Vec::new(),
    };
    for (i, &(kind, len, width, height)) in specs.iter().enumerate() {
        let y = i as f32 * 12.0;
        scene.bridges.push(Bridge { kind, a: Vec2::new(-len * 0.5, y), b: Vec2::new(len * 0.5, y), width, height });
    }
    let opts = TownMeshOptions { buildings: false, trees: false, ..Default::default() };
    let models: Vec<Model> = build_town(&scene, &opts)
        .into_iter()
        .filter(|m| matches!(m.name.as_str(), "ground" | "river" | "bridges"))
        .collect();
    write(&dir, "bridges.glb", &models);

    let kinds = [
        BuildingKind::House,
        BuildingKind::Shop,
        BuildingKind::Workshop,
        BuildingKind::Tavern,
        BuildingKind::Mansion,
        BuildingKind::Temple,
        BuildingKind::Keep,
        BuildingKind::Tower,
        BuildingKind::Barracks,
        BuildingKind::Farmhouse,
        BuildingKind::Barn,
        BuildingKind::Mill,
    ];
    let mut rng = Rng::new(5);
    let mut lots = Vec::new();
    for (i, &kind) in kinds.iter().enumerate() {
        let x = (i % 6) as f32 * 24.0;
        let y = (i / 6) as f32 * -26.0;
        let fp = match kind {
            BuildingKind::Temple => Polygon::rect(Vec2::new(x, y), Vec2::new(12.0, 20.0)),
            BuildingKind::Tower => Polygon::regular(Vec2::new(x, y), 4.0, 8, 0.0),
            BuildingKind::Keep => Polygon::rect(Vec2::new(x, y), Vec2::new(11.0, 11.0)),
            BuildingKind::House => Polygon(vec![
                Vec2::new(x - 4.0, y - 5.0),
                Vec2::new(x + 4.5, y - 4.4),
                Vec2::new(x + 4.0, y + 5.0),
                Vec2::new(x - 4.2, y + 5.0),
            ]),
            BuildingKind::Mansion => Polygon(vec![
                Vec2::new(x - 7.0, y - 6.0),
                Vec2::new(x + 7.0, y - 6.0),
                Vec2::new(x + 7.0, y),
                Vec2::new(x + 1.0, y),
                Vec2::new(x + 1.0, y + 6.0),
                Vec2::new(x - 7.0, y + 6.0),
            ]),
            // U-shaped workshop around a yard, T-shaped tavern.
            BuildingKind::Workshop => Polygon(
                [(-7.0, -5.0), (7.0, -5.0), (7.0, 6.0), (3.0, 6.0), (3.0, 0.0), (-3.0, 0.0), (-3.0, 6.0), (-7.0, 6.0)]
                    .iter()
                    .map(|&(dx, dy)| Vec2::new(x + dx, y + dy))
                    .collect(),
            ),
            BuildingKind::Tavern => Polygon(
                [(-7.0, -6.0), (7.0, -6.0), (7.0, 0.0), (2.5, 0.0), (2.5, 7.0), (-2.5, 7.0), (-2.5, 0.0), (-7.0, 0.0)]
                    .iter()
                    .map(|&(dx, dy)| Vec2::new(x + dx, y + dy))
                    .collect(),
            ),
            _ => Polygon::rect(Vec2::new(x, y), Vec2::new(9.0, 11.0)),
        };
        let b = Building {
            id: i as u32,
            patch: 0,
            kind,
            footprint: fp,
            floors: if matches!(kind, BuildingKind::Barn) { 1 } else { 2 + (i % 2) as u8 },
            front: Vec2::new(0.0, -1.0),
            seed: 40 + i as u64,
            appearance: fixtures::town_appearance(i, &mut rng),
        };
        lots.push(build_lot_house(&b, None));
    }
    write(&dir, "lot_houses.glb", &lots);

    let town = fixtures::town();
    write(&dir, "fixture_town.glb", &build_town(&town, &TownMeshOptions::default()));

    for (i, preset) in Storybook::ALL.into_iter().enumerate() {
        let profile = StyleProfile::storybook(preset);
        let opts = DwellingMeshOptions { profile, ..Default::default() };
        let house = build_dwelling(&fixtures::storybook_house(preset), &opts);
        let lot = build_lot_house_with(&fixtures::storybook_building(preset, 7 + i as u64), None, &profile);
        write(&dir, &format!("storybook_ref0{}.glb", i + 1), &[house, shifted(lot, 16.0, 0.0)]);
        let mut wide = fixtures::storybook_building(preset, 7 + i as u64);
        wide.footprint = Polygon::rect(Vec2::ZERO, Vec2::new(12.0, 7.0));
        write(&dir, &format!("storybook_wide_lot0{}.glb", i + 1), &[build_lot_house_with(&wide, None, &profile)]);
    }
}
