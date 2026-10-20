//! Integration tests: mesh validity, budgets, openings, colours and glTF export.

use mde_core::poly::closest_point_on_segment;
use mde_core::{
    world_to_plan, Bridge, BridgeKind, Building, BuildingKind, DoorKind, Dwelling, MaterialSlot, MeshData, Model,
    OpeningKind, Polygon, Rng, RoofKind, RoofMaterial, Vec2, Vec3, WindowKind,
};
use mde_mesh::{
    bridge_deck_height, build_bridge, build_dwelling, build_lot_house, build_town, door_leaves, fixtures, gltf,
    lot_house_openings, opening_placements, triangulate, DwellingMeshOptions, TownMeshOptions,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn check_mesh(name: &str, slot: MaterialSlot, m: &MeshData) {
    let n = m.positions.len();
    assert_eq!(m.indices.len() % 3, 0, "{name}/{slot:?}: indices not a multiple of 3");
    assert!(m.indices.iter().all(|&i| (i as usize) < n), "{name}/{slot:?}: index out of range");
    assert!(m.normals.is_empty() || m.normals.len() == n, "{name}/{slot:?}: normals length");
    assert!(m.uvs.is_empty() || m.uvs.len() == n, "{name}/{slot:?}: uvs length");
    assert!(m.colors.is_empty() || m.colors.len() == n, "{name}/{slot:?}: colors length");
    assert!(m.positions.iter().flatten().all(|v| v.is_finite()), "{name}/{slot:?}: non-finite position");
    assert!(m.uvs.iter().flatten().all(|v| v.is_finite()), "{name}/{slot:?}: non-finite uv");
    assert!(m.colors.iter().flatten().all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "{name}/{slot:?}: colour");
    for nrm in &m.normals {
        let l = Vec3::from_array(*nrm).length();
        assert!(l.is_finite() && (l - 1.0).abs() < 1e-3, "{name}/{slot:?}: normal length {l}");
    }
}

fn check_model(m: &Model) {
    for p in &m.parts {
        check_mesh(&m.name, p.material, &p.mesh);
        assert!(!p.mesh.is_empty(), "{}: empty part {:?}", m.name, p.material);
    }
    // Parts are unique per slot.
    let mut slots: Vec<_> = m.parts.iter().map(|p| p.material).collect();
    slots.sort();
    slots.dedup();
    assert_eq!(slots.len(), m.parts.len(), "{}: duplicate slot parts", m.name);
}

fn tris(m: &Model) -> impl Iterator<Item = (MaterialSlot, [Vec3; 3], Vec3)> + '_ {
    m.parts.iter().flat_map(|p| {
        p.mesh.indices.as_chunks::<3>().0.iter().map(move |t| {
            let v = |i: u32| Vec3::from_array(p.mesh.positions[i as usize]);
            let n = Vec3::from_array(p.mesh.normals[t[0] as usize]);
            (p.material, [v(t[0]), v(t[1]), v(t[2])], n)
        })
    })
}

fn part(m: &Model, slot: MaterialSlot) -> Option<&MeshData> {
    m.parts.iter().find(|p| p.material == slot).map(|p| &p.mesh)
}

/// Separating-axis test: triangle vs. axis-aligned box (centre, half extents).
fn tri_box_overlap(tri: [Vec3; 3], c: Vec3, h: Vec3) -> bool {
    let v = tri.map(|p| p - c);
    let e = [v[1] - v[0], v[2] - v[1], v[0] - v[2]];
    let axes_box = [Vec3::X, Vec3::Y, Vec3::Z];
    let sep = |axis: Vec3| -> bool {
        if axis.length_squared() < 1e-12 {
            return false;
        }
        let p = v.map(|q| q.dot(axis));
        let r = h.x * axis.x.abs() + h.y * axis.y.abs() + h.z * axis.z.abs();
        let (mn, mx) = (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]));
        mn > r || mx < -r
    };
    for a in axes_box {
        for ed in e {
            if sep(a.cross(ed)) {
                return false;
            }
        }
    }
    for a in axes_box {
        if sep(a) {
            return false;
        }
    }
    !sep(e[0].cross(e[1]))
}

fn rect_building(kind: BuildingKind, w: f32, d: f32, floors: u8, seed: u64, app_seed: u64) -> Building {
    let mut rng = Rng::new(app_seed);
    Building {
        id: seed as u32,
        patch: 0,
        kind,
        footprint: Polygon::rect(Vec2::new(10.0, 20.0), Vec2::new(w, d)),
        floors,
        front: Vec2::new(0.0, -1.0),
        seed,
        appearance: fixtures::town_appearance(seed as usize, &mut rng),
    }
}

// ---------------------------------------------------------------------------
// Triangulation
// ---------------------------------------------------------------------------

fn tri_area_sum(poly: &Polygon) -> f32 {
    triangulate(poly)
        .iter()
        .map(|t| {
            let (a, b, c) = (poly.0[t[0]], poly.0[t[1]], poly.0[t[2]]);
            let ar = mde_core::poly::cross(b - a, c - a) * 0.5;
            assert!(ar > -1e-6, "triangle not CCW: {t:?}");
            ar
        })
        .sum()
}

#[test]
fn triangulate_is_area_exact() {
    let v = |x: f32, y: f32| Vec2::new(x, y);
    let star = Polygon(
        (0..10)
            .map(|k| {
                let r = if k % 2 == 0 { 5.0 } else { 2.0 };
                Vec2::from_angle(k as f32 / 10.0 * std::f32::consts::TAU) * r
            })
            .collect(),
    );
    let cases = vec![
        ("convex", Polygon::regular(v(3.0, -2.0), 4.0, 12, 0.3)),
        ("square", Polygon::rect(Vec2::ZERO, v(3.0, 2.0))),
        ("L", Polygon(vec![v(0.0, 0.0), v(10.0, 0.0), v(10.0, 4.0), v(4.0, 4.0), v(4.0, 9.0), v(0.0, 9.0)])),
        (
            "U",
            Polygon(vec![
                v(0.0, 0.0),
                v(9.0, 0.0),
                v(9.0, 8.0),
                v(6.0, 8.0),
                v(6.0, 3.0),
                v(3.0, 3.0),
                v(3.0, 8.0),
                v(0.0, 8.0),
            ]),
        ),
        ("star", star),
        ("collinear", Polygon(vec![v(0.0, 0.0), v(1.0, 0.0), v(2.0, 0.0), v(2.0, 2.0), v(1.0, 2.0), v(0.0, 2.0)])),
    ];
    for (name, poly) in cases {
        let expected = poly.area();
        let got = tri_area_sum(&poly);
        assert!((got - expected).abs() <= expected * 1e-4, "{name}: {got} vs {expected}");
        // Clockwise input gives the same area.
        let cw = Polygon(poly.0.iter().rev().copied().collect());
        let got = tri_area_sum(&cw);
        assert!((got - expected).abs() <= expected * 1e-4, "{name} (cw): {got} vs {expected}");
        if name != "collinear" {
            assert_eq!(triangulate(&poly).len(), poly.len() - 2, "{name}");
        }
    }
}

// ---------------------------------------------------------------------------
// Dwellings
// ---------------------------------------------------------------------------

#[test]
fn fixture_dwellings_are_valid_and_within_budget() {
    for d in [fixtures::rect_house(), fixtures::l_house()] {
        let m = build_dwelling(&d, &DwellingMeshOptions::default());
        check_model(&m);
        assert!(m.triangle_count() > 1000);
        assert!(m.triangle_count() <= 40_000, "{} triangles", m.triangle_count());
        for opts in [
            DwellingMeshOptions { max_level: Some(0), ..Default::default() },
            DwellingMeshOptions { interior: false, props: false, ..Default::default() },
            DwellingMeshOptions { roof: false, door_leaves: false, ..Default::default() },
        ] {
            check_model(&build_dwelling(&d, &opts));
        }
    }
    assert_eq!(fixtures::rect_house().floors.len(), 3);
}

#[test]
fn every_roof_kind_and_material_builds() {
    let kinds = [RoofKind::Gable, RoofKind::Hip, RoofKind::HalfHip, RoofKind::Gambrel, RoofKind::Tent, RoofKind::Flat];
    let mats = [RoofMaterial::Tiles, RoofMaterial::Slate, RoofMaterial::Thatch, RoofMaterial::Shingles];
    for base in [fixtures::rect_house(), fixtures::l_house()] {
        for kind in kinds {
            for mat in mats {
                let mut d = base.clone();
                d.roof.kind = kind;
                d.appearance.roof_material = mat;
                let opts = DwellingMeshOptions { interior: false, props: false, ..Default::default() };
                let m = build_dwelling(&d, &opts);
                check_model(&m);
                let slot = match mat {
                    RoofMaterial::Tiles => MaterialSlot::RoofTile,
                    RoofMaterial::Slate => MaterialSlot::Slate,
                    RoofMaterial::Thatch => MaterialSlot::Thatch,
                    RoofMaterial::Shingles => MaterialSlot::Planks,
                };
                assert!(part(&m, slot).is_some_and(|p| p.triangle_count() > 0), "{kind:?}/{mat:?} has no covering");
                // The roof covers the whole outline: its top is above the wall plate.
                let top = m.parts.iter().flat_map(|p| p.mesh.positions.iter()).map(|p| p[1]).fold(f32::MIN, f32::max);
                let plate = base.floors.last().map_or(0.0, |f| f.elevation + 3.0);
                assert!(top > plate, "{kind:?}: top {top} below plate {plate}");
            }
        }
    }
}

#[test]
fn every_window_and_door_kind_builds() {
    let samples = fixtures::kind_samples();
    assert_eq!(samples.len(), 12);
    for d in &samples {
        let m = build_dwelling(d, &DwellingMeshOptions::default());
        check_model(&m);
        assert!(part(&m, MaterialSlot::Glass).is_some() || d.floors[0].openings.iter().all(|o| o.kind != OpeningKind::Window));
    }
    // Lot houses too.
    for (i, wk) in [WindowKind::Casement, WindowKind::Mullioned, WindowKind::Arched, WindowKind::Gothic, WindowKind::Round, WindowKind::Lattice]
        .into_iter()
        .enumerate()
    {
        for dk in [DoorKind::Plank, DoorKind::Arched, DoorKind::Double, DoorKind::Dutch, DoorKind::Studded, DoorKind::Glazed] {
            let mut b = rect_building(BuildingKind::House, 8.0, 10.0, 2, i as u64, 3);
            b.appearance.window_kind = wk;
            b.appearance.door_kind = dk;
            let m = build_lot_house(&b, None);
            check_model(&m);
            assert!(m.triangle_count() <= 2000);
        }
    }
}

/// Wall-local frame of the wall segment an opening sits on.
fn opening_wall(d: &Dwelling, level: i32, index: usize) -> (Vec2, Vec2, f32, f32) {
    let f = d.floors.iter().find(|f| f.level == level).expect("level");
    let o = &f.openings[index];
    let w = f
        .walls
        .iter()
        .find(|w| {
            let da = o.a.distance(closest_point_on_segment(o.a, w.a, w.b));
            let db = o.b.distance(closest_point_on_segment(o.b, w.a, w.b));
            da < 0.06 && db < 0.06
        })
        .expect("opening on a wall");
    (w.a, w.b, w.thickness, f.elevation)
}

#[test]
fn placements_and_leaves_lie_on_wall_centres() {
    for d in [fixtures::rect_house(), fixtures::l_house()] {
        let total: usize = d.floors.iter().map(|f| f.openings.len()).sum();
        let placements = opening_placements(&d);
        assert_eq!(placements.len(), total);
        for p in &placements {
            let (a, b, t, _) = opening_wall(&d, p.level, p.opening);
            let q = world_to_plan(p.center);
            let dist = q.distance(closest_point_on_segment(q, a, b));
            assert!(dist < 1e-3, "placement {:?} off its wall by {dist}", p.kind);
            assert!((p.wall_thickness - t).abs() < 1e-5);
            assert!(p.width > 0.5 && p.height > 0.3);
            // Facing direction is perpendicular to the wall.
            let face = Vec3::new(p.yaw.sin(), 0.0, p.yaw.cos());
            let wall_dir = mde_mesh_dir(a, b);
            assert!(face.dot(wall_dir).abs() < 1e-3);
            if p.kind != OpeningKind::Door && p.kind != OpeningKind::Doorway {
                // Exterior openings face away from the house.
                let out = world_to_plan(p.center + face) - q;
                let centre = d.floors[0].outline.centroid();
                assert!(out.dot(q - centre) > 0.0, "{:?} faces inwards", p.kind);
            }
        }
        let leaves = door_leaves(&d);
        let doors = d.floors.iter().flat_map(|f| f.openings.iter()).filter(|o| matches!(o.kind, OpeningKind::Door | OpeningKind::Entrance)).count();
        assert!(leaves.len() >= doors);
        for l in &leaves {
            let (a, b, _, elev) = opening_wall(&d, l.level, l.opening);
            let q = world_to_plan(l.hinge);
            let dist = q.distance(closest_point_on_segment(q, a, b));
            assert!(dist < 1e-3, "hinge off the wall centre by {dist}");
            assert!((l.hinge.y - elev).abs() < 1e-4);
            let f = d.floors.iter().find(|f| f.level == l.level).expect("level");
            let o = &f.openings[l.opening];
            let near_end = q.distance(o.a).min(q.distance(o.b));
            let mid = q.distance((o.a + o.b) * 0.5);
            assert!(near_end < 0.02 || mid < 0.02, "hinge not at a jamb");
            assert!(l.open_dir == 1.0 || l.open_dir == -1.0);
            assert!(l.width > 0.3 && l.height >= 1.9);
            check_model(&l.model);
            // Local model: hinge at the origin, spanning +X.
            let xs: Vec<f32> = l.model.parts.iter().flat_map(|p| p.mesh.positions.iter().map(|v| v[0])).collect();
            assert!(xs.iter().all(|&x| x > -0.05 && x < l.width + 0.05));
        }
    }
}

fn mde_mesh_dir(a: Vec2, b: Vec2) -> Vec3 {
    let d = (b - a).normalize();
    Vec3::new(d.x, 0.0, -d.y)
}

#[test]
fn entrance_leaf_opens_inwards() {
    let d = fixtures::rect_house();
    let leaves = door_leaves(&d);
    let entrance = leaves
        .iter()
        .find(|l| d.floors[0].openings.get(l.opening).is_some_and(|o| o.kind == OpeningKind::Entrance) && l.level == 0)
        .expect("entrance leaf");
    // Rotate the free end by the opening direction: it must move into the house.
    let yaw = entrance.yaw + entrance.open_dir * 0.5;
    let free_closed = entrance.hinge + Vec3::new(entrance.yaw.cos(), 0.0, -entrance.yaw.sin()) * entrance.width;
    let free_open = entrance.hinge + Vec3::new(yaw.cos(), 0.0, -yaw.sin()) * entrance.width;
    let outline = &d.floors[0].outline;
    assert!(outline.contains(world_to_plan(free_open)), "door opens outwards");
    assert!(world_to_plan(free_open).distance(world_to_plan(free_closed)) > 0.1);
}

#[test]
fn door_holes_are_clear() {
    for d in [fixtures::rect_house(), fixtures::l_house()] {
        let opts = DwellingMeshOptions { door_leaves: false, props: false, ..Default::default() };
        let m = build_dwelling(&d, &opts);
        let all: Vec<(MaterialSlot, [Vec3; 3], Vec3)> = tris(&m).collect();
        let mut checked = 0;
        for f in &d.floors {
            for (i, o) in f.openings.iter().enumerate() {
                if o.kind == OpeningKind::Window {
                    continue;
                }
                let (a, b, t, elev) = opening_wall(&d, f.level, i);
                // Local frame of the wall: s along, w across.
                let dir = (b - a).normalize();
                let nrm = Vec2::new(-dir.y, dir.x);
                let (s0, s1) = {
                    let sa = (o.a - a).dot(dir);
                    let sb = (o.b - a).dot(dir);
                    (sa.min(sb), sa.max(sb))
                };
                let width = s1 - s0;
                // Box: full opening width × 2.0 m × wall thickness, shrunk by 2 cm.
                let to_local = |p: Vec3| {
                    let q = world_to_plan(p) - a;
                    Vec3::new(q.dot(dir), p.y - elev, q.dot(nrm))
                };
                let c = Vec3::new((s0 + s1) * 0.5, 1.0 + 0.01, 0.0);
                let h = Vec3::new(width * 0.5 - 0.02, 1.0 - 0.02, t * 0.5 - 0.01);
                for (slot, tri, _) in &all {
                    let local = tri.map(to_local);
                    assert!(
                        !tri_box_overlap(local, c, h),
                        "{:?} triangle blocks the {:?} on level {} (width {width})",
                        slot,
                        o.kind,
                        f.level
                    );
                }
                checked += 1;
            }
        }
        assert!(checked >= 4);
    }
}

#[test]
fn custom_parts_leave_bare_holes() {
    let mut d = fixtures::rect_house();
    d.appearance.custom_window = Some("window/gothic_01".into());
    d.appearance.custom_door = Some("door/oak_01".into());
    let m = build_dwelling(&d, &DwellingMeshOptions::default());
    check_model(&m);
    assert!(part(&m, MaterialSlot::Glass).is_none(), "custom windows must not get glass");
    // No frame geometry inside any window opening.
    let placements = opening_placements(&d);
    for p in placements.iter().filter(|p| p.kind == OpeningKind::Window || p.kind == OpeningKind::Entrance) {
        let face = Vec3::new(p.yaw.sin(), 0.0, p.yaw.cos());
        let along = Vec3::new(face.z, 0.0, -face.x);
        let to_local = |v: Vec3| {
            let q = v - p.center;
            Vec3::new(q.dot(along), q.y, q.dot(face))
        };
        let h = Vec3::new(p.width * 0.5 - 0.03, p.height * 0.5 - 0.03, p.wall_thickness * 0.5 - 0.01);
        for (slot, tri, _) in tris(&m) {
            assert!(!tri_box_overlap(tri.map(to_local), Vec3::ZERO, h), "{slot:?} inside a custom {:?} hole", p.kind);
        }
    }
    // The entrance gets no animated leaf; interior doors still do.
    let leaves = door_leaves(&d);
    assert!(leaves.iter().all(|l| d.floors.iter().find(|f| f.level == l.level).is_some_and(|f| f.openings[l.opening].kind == OpeningKind::Door)));
    assert!(!leaves.is_empty());

    // Lot houses: bare niches, no glass.
    let mut b = rect_building(BuildingKind::House, 8.0, 10.0, 2, 3, 3);
    b.appearance.custom_window = Some("w".into());
    b.appearance.custom_door = Some("d".into());
    let m = build_lot_house(&b, None);
    check_model(&m);
    assert!(part(&m, MaterialSlot::Glass).is_none());
    let ops = lot_house_openings(&b, None);
    assert_eq!(ops[0].kind, OpeningKind::Entrance);
    assert!(ops.len() > 2);
}

#[test]
fn exterior_wall_faces_point_outwards() {
    let d = fixtures::rect_house();
    let m = build_dwelling(&d, &DwellingMeshOptions::default());
    let outline = &d.floors[0].outline;
    let b = outline.bounds();
    let mut checked = 0;
    for (slot, tri, n) in tris(&m) {
        if !matches!(slot, MaterialSlot::Plaster | MaterialSlot::Stone) || n.y.abs() > 0.1 {
            continue;
        }
        let c = (tri[0] + tri[1] + tri[2]) / 3.0;
        let q = world_to_plan(c);
        let on_face = |v: f32, edge: f32| (v - edge).abs() < 1e-3;
        let (out, hit) = if on_face(q.x, b.min.x) {
            (Vec2::new(-1.0, 0.0), true)
        } else if on_face(q.x, b.max.x) {
            (Vec2::new(1.0, 0.0), true)
        } else if on_face(q.y, b.min.y) {
            (Vec2::new(0.0, -1.0), true)
        } else if on_face(q.y, b.max.y) {
            (Vec2::new(0.0, 1.0), true)
        } else {
            (Vec2::ZERO, false)
        };
        if !hit {
            continue;
        }
        let n2 = world_to_plan(c + n) - q;
        assert!(n2.dot(out) > 0.9, "{slot:?} face at {q:?} points inwards");
        checked += 1;
    }
    assert!(checked > 50, "only {checked} exterior faces checked");

    // Lot house shell.
    let b2 = rect_building(BuildingKind::House, 8.0, 10.0, 2, 5, 5);
    let m = build_lot_house(&b2, None);
    let bb = b2.footprint.bounds();
    let mut checked = 0;
    for (slot, tri, n) in tris(&m) {
        if !matches!(slot, MaterialSlot::Plaster | MaterialSlot::Stone) || n.y.abs() > 0.1 {
            continue;
        }
        let q = world_to_plan((tri[0] + tri[1] + tri[2]) / 3.0);
        let out = if (q.x - bb.min.x).abs() < 1e-3 {
            Vec2::new(-1.0, 0.0)
        } else if (q.x - bb.max.x).abs() < 1e-3 {
            Vec2::X
        } else if (q.y - bb.min.y).abs() < 1e-3 {
            Vec2::new(0.0, -1.0)
        } else if (q.y - bb.max.y).abs() < 1e-3 {
            Vec2::Y
        } else {
            continue;
        };
        assert!(Vec2::new(n.x, -n.z).dot(out) > 0.9, "lot face points inwards");
        checked += 1;
    }
    assert!(checked > 8);
}

#[test]
fn cutaway_has_no_roof_and_stops_at_the_level() {
    let d = fixtures::rect_house();
    let m = build_dwelling(&d, &DwellingMeshOptions { max_level: Some(0), ..Default::default() });
    check_model(&m);
    assert!(part(&m, MaterialSlot::RoofTile).is_none());
    let top_of = |slot: MaterialSlot| part(&m, slot).map_or(f32::MIN, |p| p.positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max));
    // Walls stop at the ceiling; only the stair flight (and its rail) reaches up to the next floor.
    let ceiling = d.floors[0].elevation + d.floors[0].height;
    assert!(top_of(MaterialSlot::Plaster) <= ceiling + 1e-3, "walls reach {}", top_of(MaterialSlot::Plaster));
    let top = m.parts.iter().flat_map(|p| p.mesh.positions.iter()).map(|p| p[1]).fold(f32::MIN, f32::max);
    assert!(top <= d.floors[1].elevation + 1.0, "top {top}");
    let full = build_dwelling(&d, &DwellingMeshOptions::default());
    assert!(full.triangle_count() > m.triangle_count());
    let no_leaves = build_dwelling(&d, &DwellingMeshOptions { door_leaves: false, ..Default::default() });
    let doors = |m: &Model| part(m, MaterialSlot::Door).map_or(0, |p| p.triangle_count());
    assert!(doors(&no_leaves) < doors(&full));
}

#[test]
fn recolouring_changes_only_the_expected_parts() {
    let base = fixtures::rect_house();
    let m0 = build_dwelling(&base, &DwellingMeshOptions::default());
    type Edit = fn(&mut Dwelling);
    let cases: [(Edit, MaterialSlot); 5] = [
        (|d| d.appearance.wall = [0.5, 0.6, 0.7, 1.0], MaterialSlot::Plaster),
        (|d| d.appearance.door = [0.1, 0.5, 0.2, 1.0], MaterialSlot::Door),
        (|d| d.appearance.window = [0.8, 0.1, 0.1, 1.0], MaterialSlot::WindowFrame),
        (|d| d.appearance.timber = [0.6, 0.6, 0.6, 1.0], MaterialSlot::Timber),
        (|d| d.appearance.roof = [0.2, 0.3, 0.6, 1.0], MaterialSlot::RoofTile),
    ];
    for (edit, slot) in cases {
        let mut d = base.clone();
        edit(&mut d);
        let m1 = build_dwelling(&d, &DwellingMeshOptions::default());
        assert_eq!(m0.parts.len(), m1.parts.len());
        for (p0, p1) in m0.parts.iter().zip(&m1.parts) {
            assert_eq!(p0.material, p1.material);
            assert_eq!(p0.mesh.positions, p1.mesh.positions, "{:?} geometry changed", p0.material);
            assert_eq!(p0.mesh.indices, p1.mesh.indices);
            if p0.material == slot {
                assert_ne!(p0.mesh.colors, p1.mesh.colors, "{slot:?} colours did not change");
            } else {
                assert_eq!(p0.mesh.colors, p1.mesh.colors, "{:?} changed when editing {slot:?}", p0.material);
            }
        }
    }
}

#[test]
fn deterministic_output() {
    let d = fixtures::rect_house();
    assert_eq!(build_dwelling(&d, &DwellingMeshOptions::default()), build_dwelling(&d, &DwellingMeshOptions::default()));
    let t = fixtures::town();
    let opts = TownMeshOptions { buildings: false, ..Default::default() };
    assert_eq!(build_town(&t, &opts), build_town(&t, &opts));
}

// ---------------------------------------------------------------------------
// Lot houses
// ---------------------------------------------------------------------------

#[test]
fn lot_houses_stay_within_budget() {
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
    let mut n = 0;
    for (i, &kind) in kinds.iter().enumerate() {
        for floors in 1..=4u8 {
            for (w, d) in [(5.0, 6.0), (8.0, 11.0), (14.0, 9.0), (22.0, 30.0)] {
                let b = rect_building(kind, w, d, floors, (i * 31 + floors as usize) as u64, i as u64 + floors as u64);
                let m = build_lot_house(&b, None);
                check_model(&m);
                assert!(m.triangle_count() <= 2000, "{kind:?} {w}x{d}x{floors}: {}", m.triangle_count());
                assert!(m.triangle_count() > 50);
                n += 1;
            }
        }
    }
    for b in &fixtures::town().buildings {
        let m = build_lot_house(b, None);
        check_model(&m);
        assert!(m.triangle_count() <= 2000, "{:?} #{}: {}", b.kind, b.id, m.triangle_count());
        n += 1;
    }
    assert!(n > 200);
}

#[test]
fn lot_house_entrance_segment_is_used() {
    let b = rect_building(BuildingKind::House, 8.0, 10.0, 2, 9, 9);
    // Footprint spans x 6..14, y 15..25. Entrance on the east edge.
    let seg = (Vec2::new(14.0, 19.0), Vec2::new(14.0, 20.2));
    let ops = lot_house_openings(&b, Some(seg));
    let e = &ops[0];
    assert_eq!(e.kind, OpeningKind::Entrance);
    let q = world_to_plan(e.center);
    assert!((q.x - (14.0 - 0.15)).abs() < 1e-3, "entrance not on the east wall centre: {q:?}");
    assert!((q.y - 19.6).abs() < 1e-3);
    assert!((e.width - 1.2).abs() < 1e-3);
    // Without a segment the door is centred on the front (south) edge.
    let ops = lot_house_openings(&b, None);
    let q = world_to_plan(ops[0].center);
    assert!((q.x - 10.0).abs() < 1e-3 && (q.y - (15.0 + 0.15)).abs() < 1e-3, "{q:?}");
    check_model(&build_lot_house(&b, Some(seg)));
}

#[test]
fn degenerate_inputs_do_not_panic() {
    let mut b = rect_building(BuildingKind::House, 8.0, 10.0, 2, 1, 1);
    b.footprint = Polygon(vec![Vec2::ZERO, Vec2::X]);
    assert!(build_lot_house(&b, None).parts.is_empty());
    b.footprint = Polygon(vec![Vec2::ZERO, Vec2::new(f32::NAN, 1.0), Vec2::new(5.0, 5.0), Vec2::new(0.0, 5.0)]);
    check_model(&build_lot_house(&b, None));
    b.footprint = Polygon::rect(Vec2::ZERO, Vec2::new(6.0, 6.0));
    b.floors = 0;
    check_model(&build_lot_house(&b, Some((Vec2::new(f32::INFINITY, 0.0), Vec2::ZERO))));

    let mut d = fixtures::rect_house();
    d.floors.clear();
    assert!(build_dwelling(&d, &DwellingMeshOptions::default()).parts.is_empty() || d.floors.is_empty());
    let mut d = fixtures::rect_house();
    d.floors[0].walls[0].thickness = f32::NAN;
    d.floors[1].openings[0].height = f32::INFINITY;
    d.floors[0].outline = Polygon::default();
    d.roof.pitch_deg = f32::NAN;
    d.roof.ridge = Vec2::ZERO;
    check_model(&build_dwelling(&d, &DwellingMeshOptions::default()));

    for (a, bb, w) in [(Vec2::ZERO, Vec2::ZERO, 3.0), (Vec2::ZERO, Vec2::new(f32::NAN, 0.0), 3.0), (Vec2::ZERO, Vec2::X * 10.0, f32::NAN)] {
        for kind in [BridgeKind::Wood, BridgeKind::Stone] {
            check_model(&build_bridge(&Bridge { kind, a, b: bb, width: w, height: -5.0 }));
        }
    }
}

// ---------------------------------------------------------------------------
// Bridges
// ---------------------------------------------------------------------------

#[test]
fn bridges_valid_for_all_sizes() {
    for kind in [BridgeKind::Wood, BridgeKind::Stone] {
        for len in [2.0f32, 5.0, 13.0, 30.0, 55.0, 80.0] {
            for width in [1.0f32, 3.0, 8.0] {
                for height in [0.0f32, 1.5, 6.0] {
                    let a = Vec2::new(3.0, -2.0);
                    let dir = Vec2::new(0.6, 0.8);
                    let b = Bridge { kind, a, b: a + dir * len, width, height };
                    let m = build_bridge(&b);
                    check_model(&m);
                    assert!(m.triangle_count() > 20);
                    assert_eq!(bridge_deck_height(&b, 0.0), 0.0);
                    assert!(bridge_deck_height(&b, 1.0).abs() < 1e-4);
                    // Deck surface reaches both ends at ground level.
                    for (end, sign) in [(b.a, 1.0f32), (b.b, -1.0)] {
                        let near: Vec<f32> = m
                            .parts
                            .iter()
                            .filter(|p| matches!(p.material, MaterialSlot::Planks | MaterialSlot::Stone))
                            .flat_map(|p| p.mesh.positions.iter())
                            .filter(|v| {
                                let q = world_to_plan(Vec3::from_array(**v)) - end;
                                let s = q.dot(dir) * sign;
                                (-0.01..0.05).contains(&s) && q.perp_dot(dir).abs() <= width * 0.5 + 0.1
                            })
                            .map(|v| v[1].abs())
                            .collect();
                        assert!(near.iter().any(|&y| y < 0.02), "{kind:?} {len} {width} {height}: no deck at ground level");
                    }
                    if len == 30.0 {
                        assert!(m.triangle_count() <= 5000, "{kind:?} 30 m: {}", m.triangle_count());
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Town and glTF
// ---------------------------------------------------------------------------

#[test]
fn town_builds_all_layers() {
    let t = fixtures::town();
    let models = build_town(&t, &TownMeshOptions::default());
    let names: Vec<&str> = models.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["ground", "streets", "plaza", "fields", "walls", "river", "trees", "bridges", "buildings"]);
    for m in &models {
        check_model(m);
        assert!(m.triangle_count() > 0, "{} is empty", m.name);
    }
    // Water lies below ground level; streets above it.
    let water = part(&models[5], MaterialSlot::Water).expect("water");
    assert!(water.positions.iter().all(|p| p[1] < -0.1));
    let streets = part(&models[1], MaterialSlot::Street).expect("streets");
    assert!(streets.positions.iter().all(|p| p[1] > 0.0), "streets dip into the river channel");
    let none = build_town(&t, &TownMeshOptions { buildings: false, trees: false, bridges: false });
    assert!(none[6].parts.is_empty() && none[7].parts.is_empty() && none[8].parts.is_empty());
}

fn check_glb(models: &[Model]) {
    let bytes = gltf::write_glb(models);
    assert_eq!(bytes.len() % 4, 0);
    let (json, bin) = gltf::read_glb(&bytes).expect("valid container");
    // Round-trips through serde_json as text too.
    let text = serde_json::to_string(&json).expect("json");
    let _: serde_json::Value = serde_json::from_str(&text).expect("parse");
    assert_eq!(json["asset"]["version"], "2.0");
    assert_eq!(json["asset"]["generator"], "midearth mde_mesh");
    let nodes = json["nodes"].as_array().expect("nodes");
    assert_eq!(nodes.len(), models.len());
    let accessors = json["accessors"].as_array().cloned().unwrap_or_default();
    let views = json["bufferViews"].as_array().cloned().unwrap_or_default();
    if !accessors.is_empty() {
        assert_eq!(json["buffers"][0]["byteLength"].as_u64().unwrap_or(0) as usize, bin.len());
    }
    for v in &views {
        let off = v["byteOffset"].as_u64().unwrap_or(0) as usize;
        let len = v["byteLength"].as_u64().unwrap_or(0) as usize;
        assert_eq!(off % 4, 0);
        assert!(off + len <= bin.len());
    }
    let materials = json["materials"].as_array().cloned().unwrap_or_default();
    for (ni, model) in models.iter().enumerate() {
        let node = &nodes[ni];
        assert_eq!(node["name"], model.name.as_str());
        let non_empty: Vec<_> = model.parts.iter().filter(|p| !p.mesh.is_empty()).collect();
        let Some(mesh_index) = node["mesh"].as_u64() else {
            assert!(non_empty.is_empty());
            continue;
        };
        let prims = json["meshes"][mesh_index as usize]["primitives"].as_array().cloned().unwrap_or_default();
        assert_eq!(prims.len(), non_empty.len());
        for (prim, p) in prims.iter().zip(non_empty) {
            let count = |name: &str| accessors[prim["attributes"][name].as_u64().expect("attr") as usize]["count"].as_u64();
            assert_eq!(count("POSITION"), Some(p.mesh.positions.len() as u64));
            assert_eq!(count("NORMAL"), Some(p.mesh.normals.len() as u64));
            assert_eq!(count("TEXCOORD_0"), Some(p.mesh.uvs.len() as u64));
            assert_eq!(count("COLOR_0"), Some(p.mesh.colors.len() as u64));
            let pos = &accessors[prim["attributes"]["POSITION"].as_u64().unwrap_or(0) as usize];
            assert!(pos["min"].is_array() && pos["max"].is_array());
            assert_eq!(accessors[prim["attributes"]["COLOR_0"].as_u64().unwrap_or(0) as usize]["type"], "VEC4");
            let idx = &accessors[prim["indices"].as_u64().expect("indices") as usize];
            assert_eq!(idx["count"].as_u64(), Some(p.mesh.indices.len() as u64));
            assert_eq!(idx["componentType"], 5125);
            assert!(idx["max"][0].as_u64().unwrap_or(u64::MAX) < p.mesh.positions.len() as u64);
            let mat = &materials[prim["material"].as_u64().expect("material") as usize];
            assert_eq!(mat["name"], mde_mesh::palette::slot_name(p.material));
            assert_eq!(mat["pbrMetallicRoughness"]["metallicFactor"], 0.0);
        }
    }
}

#[test]
fn glb_export_is_structurally_valid() {
    check_glb(&[build_dwelling(&fixtures::rect_house(), &DwellingMeshOptions::default()), Model::new("empty")]);
    check_glb(&build_town(&fixtures::town(), &TownMeshOptions { buildings: false, ..Default::default() }));
    check_glb(&[build_bridge(&Bridge { kind: BridgeKind::Stone, a: Vec2::ZERO, b: Vec2::X * 20.0, width: 4.0, height: 2.0 })]);
    check_glb(&[]);
}

// ---------------------------------------------------------------------------
// Randomised robustness
// ---------------------------------------------------------------------------

/// A random simple footprint: convex n-gon, rotated rectangle or rotated L / T.
fn random_footprint(rng: &mut Rng) -> Polygon {
    let c = Vec2::new(rng.range_f32(-200.0, 200.0), rng.range_f32(-200.0, 200.0));
    let rot = rng.range_f32(0.0, std::f32::consts::TAU);
    let local: Vec<Vec2> = match rng.index(4) {
        0 => {
            let n = 3 + rng.index(6);
            let r = rng.range_f32(3.0, 12.0);
            (0..n)
                .map(|k| {
                    let a = k as f32 / n as f32 * std::f32::consts::TAU + rng.range_f32(-0.2, 0.2);
                    Vec2::from_angle(a) * r * rng.range_f32(0.7, 1.0)
                })
                .collect()
        }
        1 => Polygon::rect(Vec2::ZERO, Vec2::new(rng.range_f32(3.0, 25.0), rng.range_f32(3.0, 25.0))).0,
        2 => {
            let (w, d, a, b) = (rng.range_f32(6.0, 20.0), rng.range_f32(6.0, 20.0), rng.range_f32(2.5, 5.0), rng.range_f32(2.5, 5.0));
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(w, 0.0),
                Vec2::new(w, a),
                Vec2::new(b, a),
                Vec2::new(b, d),
                Vec2::new(0.0, d),
            ]
        }
        _ => {
            let (w, d, s) = (rng.range_f32(8.0, 20.0), rng.range_f32(3.0, 7.0), rng.range_f32(2.5, 5.0));
            let x0 = (w - s) * rng.range_f32(0.1, 0.9);
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(w, 0.0),
                Vec2::new(w, d),
                Vec2::new(x0 + s, d),
                Vec2::new(x0 + s, d + rng.range_f32(3.0, 9.0)),
                Vec2::new(x0, d + 6.0),
                Vec2::new(x0, d),
                Vec2::new(0.0, d),
            ]
        }
    };
    let r = Vec2::from_angle(rot);
    let mut p = Polygon(local.iter().map(|&q| c + r.rotate(q)).collect());
    if rng.chance(0.5) {
        p.0.reverse();
    }
    p
}

#[test]
fn random_lot_houses_are_valid() {
    let mut rng = Rng::new(77);
    let kinds = [
        BuildingKind::House,
        BuildingKind::Shop,
        BuildingKind::Tavern,
        BuildingKind::Mansion,
        BuildingKind::Temple,
        BuildingKind::Keep,
        BuildingKind::Tower,
        BuildingKind::Barracks,
        BuildingKind::Barn,
        BuildingKind::Mill,
    ];
    for i in 0..1000 {
        let fp = random_footprint(&mut rng);
        let front = Vec2::from_angle(rng.range_f32(0.0, 6.3));
        let b = Building {
            id: i,
            patch: 0,
            kind: kinds[rng.index(kinds.len())],
            footprint: fp,
            floors: 1 + rng.index(5) as u8,
            front: if rng.chance(0.1) { Vec2::ZERO } else { front },
            seed: rng.next_u64(),
            appearance: fixtures::town_appearance(i as usize, &mut rng),
        };
        let m = build_lot_house(&b, None);
        check_model(&m);
        assert!(m.triangle_count() <= 2000, "{:?} #{i}: {}", b.kind, m.triangle_count());
        for o in lot_house_openings(&b, None) {
            assert!(o.center.is_finite() && o.yaw.is_finite() && o.width > 0.0 && o.height > 0.0);
        }
    }
}

#[test]
fn random_bridges_are_valid() {
    let mut rng = Rng::new(5);
    for _ in 0..200 {
        let a = Vec2::new(rng.range_f32(-100.0, 100.0), rng.range_f32(-100.0, 100.0));
        let b = a + Vec2::from_angle(rng.range_f32(0.0, 6.3)) * rng.range_f32(0.0, 90.0);
        let kind = if rng.chance(0.5) { BridgeKind::Wood } else { BridgeKind::Stone };
        let br = Bridge { kind, a, b, width: rng.range_f32(0.5, 10.0), height: rng.range_f32(-1.0, 8.0) };
        check_model(&build_bridge(&br));
        let h = bridge_deck_height(&br, rng.next_f32());
        assert!(h.is_finite() && h >= 0.0);
    }
}

#[test]
fn basement_and_odd_dwellings_build() {
    // Add a cellar under the townhouse.
    let mut d = fixtures::rect_house();
    let mut cellar = d.floors[0].clone();
    cellar.level = -1;
    cellar.elevation = -3.0;
    cellar.openings.retain(|o| o.kind != OpeningKind::Entrance && o.kind != OpeningKind::Window);
    cellar.props.clear();
    cellar.stairs = vec![mde_core::Stair {
        footprint: Polygon::rect(Vec2::new(7.45, 3.6), Vec2::new(1.3, 4.0)),
        up: Vec2::new(0.0, 1.0),
        from_level: -1,
        to_level: 0,
    }];
    d.floors.insert(0, cellar);
    let m = build_dwelling(&d, &DwellingMeshOptions::default());
    check_model(&m);
    let low = m.parts.iter().flat_map(|p| p.mesh.positions.iter()).map(|p| p[1]).fold(f32::MAX, f32::min);
    assert!(low < -3.0);
    check_model(&build_dwelling(&d, &DwellingMeshOptions { max_level: Some(-1), ..Default::default() }));
    assert!(!opening_placements(&d).is_empty());

    // Walls only, no outline, no openings; and a rotated house.
    let mut bare = fixtures::rect_house();
    for f in &mut bare.floors {
        f.outline = Polygon::default();
        f.openings.clear();
        f.props.clear();
    }
    check_model(&build_dwelling(&bare, &DwellingMeshOptions::default()));
    let mut rot = fixtures::l_house();
    let r = Vec2::from_angle(0.7);
    let tr = |p: Vec2| r.rotate(p) + Vec2::new(30.0, -12.0);
    for f in &mut rot.floors {
        f.outline = Polygon(f.outline.0.iter().map(|&p| tr(p)).collect());
        for w in &mut f.walls {
            w.a = tr(w.a);
            w.b = tr(w.b);
        }
        for o in &mut f.openings {
            o.a = tr(o.a);
            o.b = tr(o.b);
        }
        for room in &mut f.rooms {
            room.shape = Polygon(room.shape.0.iter().map(|&p| tr(p)).collect());
        }
        for s in &mut f.stairs {
            s.footprint = Polygon(s.footprint.0.iter().map(|&p| tr(p)).collect());
            s.up = r.rotate(s.up);
        }
        for p in &mut f.props {
            p.pos = tr(p.pos);
            p.rotation += 0.7;
        }
    }
    rot.roof.ridge = r.rotate(rot.roof.ridge);
    for kind in [RoofKind::Gable, RoofKind::Hip, RoofKind::Tent, RoofKind::Flat] {
        rot.roof.kind = kind;
        let m = build_dwelling(&rot, &DwellingMeshOptions::default());
        check_model(&m);
        assert!(m.triangle_count() < 40_000);
    }
    for p in opening_placements(&rot) {
        let (a, b, _, _) = opening_wall(&rot, p.level, p.opening);
        let q = world_to_plan(p.center);
        assert!(q.distance(closest_point_on_segment(q, a, b)) < 1e-3);
    }
}
