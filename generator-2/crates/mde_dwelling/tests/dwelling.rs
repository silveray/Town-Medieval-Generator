//! End-to-end tests of the dwelling generator: determinism, a large sweep over
//! seeds / styles / storeys / cellars and over rotated town lots, plus the
//! public helpers.

use mde_core::{
    Appearance, Building, BuildingKind, DoorKind, Dwelling, DwellingParams, DwellingStyle, OpeningKind, Polygon, Rng, RoomKind,
    Vec2, SCHEMA_VERSION,
};
use mde_dwelling::{entrance, from_building, generate, render_svg, render_svg_all, validate, validate::walkability, EXTERIOR_WALL};

const STYLES: [DwellingStyle; 6] = [
    DwellingStyle::Cottage,
    DwellingStyle::Townhouse,
    DwellingStyle::Farmhouse,
    DwellingStyle::Manor,
    DwellingStyle::Tavern,
    DwellingStyle::Shop,
];

fn params(seed: u64, style: DwellingStyle, floors: u8, basement: bool) -> DwellingParams {
    DwellingParams { seed, style, floors, basement, ..Default::default() }
}

/// Random convex quad lot of 40–300 m², rotated and moved, with a street side.
fn random_lot(seed: u64) -> (Polygon, Vec2) {
    let mut r = Rng::new(seed ^ 0x10C0_FFEE);
    let area = r.range_f32(40.0, 300.0);
    let aspect = r.range_f32(0.5, 2.0);
    let w = (area * aspect).sqrt();
    let h = area / w;
    let jit = |r: &mut Rng| Vec2::new(r.range_f32(-0.12, 0.12) * w, r.range_f32(-0.12, 0.12) * h);
    let pts = vec![
        Vec2::new(-w / 2.0, -h / 2.0) + jit(&mut r),
        Vec2::new(w / 2.0, -h / 2.0) + jit(&mut r),
        Vec2::new(w / 2.0, h / 2.0) + jit(&mut r),
        Vec2::new(-w / 2.0, h / 2.0) + jit(&mut r),
    ];
    let angle = r.range_f32(0.0, std::f32::consts::TAU);
    let offset = Vec2::new(r.range_f32(-200.0, 200.0), r.range_f32(-200.0, 200.0));
    let lot = Polygon(pts).rotated(Vec2::ZERO, angle).translated(offset);
    let front = Vec2::from_angle(angle - std::f32::consts::FRAC_PI_2 + r.range_f32(-0.3, 0.3));
    (lot, front)
}

/// Checks on top of [`validate`]: storey count, room areas against the grid
/// interior and floor elevations.
fn check(d: &Dwelling, name: &str) -> Vec<String> {
    let mut errs: Vec<String> = validate(d).into_iter().filter(|e| !e.starts_with("walk:")).collect();
    let expected = d.params.floors as usize + d.params.basement as usize;
    if d.floors.len() != expected {
        errs.push(format!("{} floors, params say {expected}", d.floors.len()));
    }
    for f in &d.floors {
        // Rooms tile the interior: their total area matches the outline
        // inset by half an exterior wall (the wall centreline polygon).
        let rooms: f32 = f.rooms.iter().map(|r| r.shape.area()).sum();
        let interior = f.outline.inset(EXTERIOR_WALL * 0.5).map(|p| p.area()).unwrap_or(0.0);
        if interior <= 0.0 || (rooms - interior).abs() > interior * 0.15 {
            errs.push(format!("level {}: room area {rooms:.1} vs interior {interior:.1}", f.level));
        }
        if f.elevation != f.level as f32 * d.params.storey_height {
            errs.push(format!("level {}: elevation {}", f.level, f.elevation));
        }
    }
    // The 0.25 m walk grid (also part of `validate`), reported separately.
    errs.extend(walkability(d, 0.25, 0.0).into_iter().map(|e| format!("walk grid: {e}")));
    errs.into_iter().map(|e| format!("{name}: {e}")).collect()
}

/// Generate and check every job on all cores.
fn run_parallel(jobs: Vec<(String, DwellingParams)>) -> Vec<String> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 16);
    let chunk = jobs.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = jobs
            .chunks(chunk)
            .map(|part| s.spawn(move || part.iter().flat_map(|(name, p)| check(&generate(p), name)).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_else(|_| vec!["worker panicked".to_string()])).collect()
    })
}

fn report(errs: Vec<String>) {
    if !errs.is_empty() {
        let shown: Vec<&String> = errs.iter().take(25).collect();
        panic!("{} problems, first ones:\n{}", errs.len(), shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"));
    }
}

#[test]
fn deterministic_and_reproducible() {
    for style in STYLES {
        for seed in [0u64, 7, 12345] {
            let p = params(seed, style, 3, seed % 2 == 0);
            let a = generate(&p);
            let b = generate(&p);
            assert_eq!(a, b, "{style:?} {seed}");
            assert_eq!(serde_json::to_string(&a).unwrap(), serde_json::to_string(&b).unwrap());
            // The resolved params reproduce the same house.
            assert_eq!(generate(&a.params), a, "{style:?} {seed} regenerate");
        }
    }
    // Different seeds give different houses.
    let a = generate(&params(1, DwellingStyle::Townhouse, 2, false));
    let b = generate(&params(2, DwellingStyle::Townhouse, 2, false));
    assert_ne!(a.floors, b.floors);
}

#[test]
fn json_round_trip() {
    for style in STYLES {
        let d = generate(&params(42, style, 2, true));
        let s = serde_json::to_string(&d).unwrap();
        let back: Dwelling = serde_json::from_str(&s).unwrap();
        assert_eq!(back, d, "{style:?}");
    }
}

#[test]
fn output_basics() {
    let d = generate(&params(3, DwellingStyle::Tavern, 3, true));
    assert_eq!(d.schema, SCHEMA_VERSION);
    assert_eq!(d.floors.iter().map(|f| f.level).collect::<Vec<_>>(), vec![-1, 0, 1, 2]);
    assert!(d.floors[0].elevation < 0.0);
    assert_eq!(Some(&d.appearance), d.params.appearance.as_ref());
    assert!((35.0..=55.0).contains(&d.roof.pitch_deg));
    assert!((0.3..=0.6).contains(&d.roof.overhang));
    assert!((d.roof.ridge.length() - 1.0).abs() < 1e-4);
    // Cellars hold cellars, the ground floor the taproom and the entrance.
    assert!(d.floors[0].rooms.iter().any(|r| r.kind == RoomKind::Cellar));
    assert!(d.floors[1].rooms.iter().any(|r| r.kind == RoomKind::Taproom));
    let e = entrance(&d).expect("entrance");
    assert_eq!(e.kind, OpeningKind::Entrance);
    let w = e.a.distance(e.b);
    assert!((1.1..=1.8 + 1e-3).contains(&w), "entrance width {w}");
    // Upper floors of a tavern are guest rooms off a corridor.
    for f in &d.floors[2..] {
        assert!(f.rooms.iter().any(|r| r.kind == RoomKind::Corridor), "level {} has no corridor", f.level);
        assert!(f.rooms.iter().filter(|r| r.kind == RoomKind::Bedroom).count() >= 3);
    }
    // Doors are at least 0.9 m wide; windows sit above the floor.
    for f in &d.floors {
        for o in &f.openings {
            match o.kind {
                OpeningKind::Door => assert!(o.a.distance(o.b) >= 0.9 - 1e-3 && (o.height - 2.1).abs() < 1e-4),
                OpeningKind::Window => assert!(o.sill > 0.5 && o.sill + o.height <= f.height),
                _ => {}
            }
        }
    }
}

/// Seeds 0..200 × every style × 1–4 storeys × with / without cellar, on
/// synthesised footprints.
#[test]
fn sweep_synthesised_footprints() {
    let mut jobs = Vec::new();
    for seed in 0..200u64 {
        for style in STYLES {
            for floors in 1..=4u8 {
                for basement in [false, true] {
                    jobs.push((format!("{style:?} seed {seed} floors {floors} cellar {basement}"), params(seed, style, floors, basement)));
                }
            }
        }
    }
    report(run_parallel(jobs));
}

/// Random rotated convex quads of 40–300 m² as town lots.
#[test]
fn sweep_rotated_lots() {
    let mut jobs = Vec::new();
    for seed in 0..200u64 {
        for (k, style) in STYLES.into_iter().enumerate() {
            let (lot, front) = random_lot(seed * 6 + k as u64);
            let mut r = Rng::new(seed ^ 0xABCD);
            let floors = r.range_i32(1, 5) as u8;
            let p = DwellingParams { seed, style, floors, basement: r.chance(0.5), footprint: lot, front, ..Default::default() };
            jobs.push((format!("lot {style:?} seed {seed} floors {floors}"), p));
        }
    }
    report(run_parallel(jobs));
}

#[test]
fn house_fills_its_lot_and_faces_the_street() {
    let mut facing = 0;
    let mut total = 0;
    for seed in 0..60u64 {
        // Rectangular lots: the house should cover the lot almost exactly.
        let mut r = Rng::new(seed);
        let size = Vec2::new(r.range_f32(6.0, 14.0), r.range_f32(6.0, 14.0));
        let angle = r.range_f32(-3.0, 3.0);
        let lot = Polygon::rect(Vec2::new(10.0, -4.0), size).rotated(Vec2::new(10.0, -4.0), angle);
        let front = Vec2::from_angle(angle - std::f32::consts::FRAC_PI_2);
        let p = DwellingParams { seed, style: STYLES[seed as usize % 6], floors: 2, footprint: lot.clone(), front, ..Default::default() };
        let d = generate(&p);
        let outline = &d.floors[0].outline;
        assert!((outline.area() - lot.area()).abs() < lot.area() * 0.02, "seed {seed}: outline {} lot {}", outline.area(), lot.area());
        let e = entrance(&d).expect("entrance");
        let dir = (e.b - e.a).normalize();
        let mut n = Vec2::new(dir.y, -dir.x);
        let mid = (e.a + e.b) * 0.5;
        if lot.contains(mid + n * 0.5) {
            n = -n;
        }
        total += 1;
        if n.dot(front) > 0.7 {
            facing += 1;
        }
    }
    assert!(facing * 10 >= total * 9, "only {facing}/{total} entrances face the street");
}

#[test]
fn awkward_inputs_do_not_panic() {
    let cases: Vec<(&str, DwellingParams)> = vec![
        ("degenerate", DwellingParams { footprint: Polygon(vec![Vec2::ZERO, Vec2::X]), ..Default::default() }),
        ("nan", DwellingParams { footprint: Polygon(vec![Vec2::NAN, Vec2::X, Vec2::Y]), storey_height: f32::NAN, cell_size: f32::NAN, ..Default::default() }),
        ("tiny", DwellingParams { footprint: Polygon::rect(Vec2::ZERO, Vec2::new(1.0, 1.0)), floors: 3, ..Default::default() }),
        ("small", DwellingParams { footprint: Polygon::rect(Vec2::ZERO, Vec2::new(3.2, 3.0)), floors: 3, basement: true, ..Default::default() }),
        ("thin", DwellingParams { footprint: Polygon::rect(Vec2::ZERO, Vec2::new(2.6, 30.0)), floors: 2, ..Default::default() }),
        ("huge", DwellingParams { footprint: Polygon::rect(Vec2::ZERO, Vec2::new(90.0, 70.0)), style: DwellingStyle::Manor, ..Default::default() }),
        ("clockwise", DwellingParams { footprint: Polygon(Polygon::rect(Vec2::ZERO, Vec2::new(8.0, 6.0)).0.into_iter().rev().collect()), ..Default::default() }),
        (
            "concave L",
            DwellingParams {
                footprint: Polygon(vec![Vec2::ZERO, Vec2::new(12.0, 0.0), Vec2::new(12.0, 5.0), Vec2::new(5.0, 5.0), Vec2::new(5.0, 11.0), Vec2::new(0.0, 11.0)]),
                floors: 3,
                ..Default::default()
            },
        ),
        ("many floors", DwellingParams { floors: 255, ..Default::default() }),
        ("zero floors", DwellingParams { floors: 0, ..Default::default() }),
        ("coarse cells", DwellingParams { cell_size: 2.0, style: DwellingStyle::Tavern, floors: 3, ..Default::default() }),
        ("fine cells", DwellingParams { cell_size: 0.6, style: DwellingStyle::Cottage, floors: 2, basement: true, ..Default::default() }),
        ("tall storeys", DwellingParams { storey_height: 6.0, style: DwellingStyle::Manor, floors: 3, ..Default::default() }),
        ("low storeys", DwellingParams { storey_height: 2.4, style: DwellingStyle::Townhouse, floors: 4, ..Default::default() }),
        ("odd front", DwellingParams { front: Vec2::new(3.0, 1e-3), footprint: Polygon::rect(Vec2::ZERO, Vec2::new(9.0, 7.0)), ..Default::default() }),
    ];
    let mut errs = Vec::new();
    for (name, p) in cases {
        let d = generate(&p);
        assert!(!d.floors.is_empty(), "{name}");
        assert!(d.floors.iter().all(|f| !f.rooms.is_empty()), "{name}");
        assert!(entrance(&d).is_some(), "{name}: no entrance");
        assert!(d.params.floors >= 1 && d.params.floors <= mde_dwelling::MAX_FLOORS);
        errs.extend(validate(&d).into_iter().map(|e| format!("{name}: {e}")));
        let _ = render_svg_all(&d);
    }
    report(errs);
}

#[test]
fn appearance_given_or_picked() {
    let custom = Appearance { door_kind: DoorKind::Double, roof: [0.1, 0.2, 0.3, 1.0], ..Default::default() };
    let d = generate(&DwellingParams { appearance: Some(custom.clone()), ..Default::default() });
    assert_eq!(d.appearance, custom);
    // Double doors get a wide entrance.
    let w = entrance(&d).map(|e| e.a.distance(e.b)).unwrap_or(0.0);
    assert!(w >= 1.2, "double door entrance {w}");
    // Picked looks vary with the seed but stay in a muted palette.
    let looks: Vec<Appearance> = (0..20).map(|s| generate(&params(s, DwellingStyle::Cottage, 1, false)).appearance).collect();
    assert!(looks.windows(2).any(|w| w[0] != w[1]));
    for a in &looks {
        for c in [a.wall, a.timber, a.roof, a.door, a.window] {
            assert!(c.iter().all(|v| (0.0..=1.0).contains(v)));
        }
    }
}

#[test]
fn from_building_maps_kinds() {
    let b = Building {
        id: 4,
        patch: 2,
        kind: BuildingKind::Tavern,
        footprint: Polygon::rect(Vec2::new(5.0, 5.0), Vec2::new(12.0, 10.0)),
        floors: 3,
        front: Vec2::new(0.0, -1.0),
        seed: 99,
        appearance: Appearance { shutters: true, ..Default::default() },
    };
    let p = from_building(&b);
    assert_eq!(p.style, DwellingStyle::Tavern);
    assert_eq!(p.floors, 3);
    assert!(p.basement);
    assert_eq!(p.seed, 99);
    assert_eq!(p.front, b.front);
    assert_eq!(p.footprint, b.footprint);
    assert_eq!(p.appearance, Some(b.appearance.clone()));
    let d = generate(&p);
    assert!(validate(&d).is_empty(), "{:?}", validate(&d));
    assert_eq!(d.appearance, b.appearance);
    let kinds = [
        (BuildingKind::House, Vec2::new(6.0, 5.0), DwellingStyle::Cottage),
        (BuildingKind::House, Vec2::new(7.0, 11.0), DwellingStyle::Townhouse),
        (BuildingKind::Shop, Vec2::new(7.0, 9.0), DwellingStyle::Shop),
        (BuildingKind::Workshop, Vec2::new(7.0, 9.0), DwellingStyle::Shop),
        (BuildingKind::Mansion, Vec2::new(16.0, 12.0), DwellingStyle::Manor),
        (BuildingKind::Farmhouse, Vec2::new(12.0, 7.0), DwellingStyle::Farmhouse),
        (BuildingKind::Barracks, Vec2::new(14.0, 9.0), DwellingStyle::Tavern),
    ];
    for (kind, size, style) in kinds {
        let b = Building { kind, footprint: Polygon::rect(Vec2::ZERO, size), floors: 0, ..b.clone() };
        let p = from_building(&b);
        assert_eq!(p.style, style, "{kind:?}");
        assert_eq!(p.floors, 1, "floors clamp to at least one");
    }
}

#[test]
fn svg_rendering() {
    let d = generate(&params(5, DwellingStyle::Manor, 2, true));
    for f in &d.floors {
        let svg = render_svg(&d, f.level);
        assert!(svg.starts_with("<svg") && svg.trim_end().ends_with("</svg>"));
        assert!(svg.contains(&mde_dwelling::level_name(f.level)));
        for r in f.rooms.iter().filter(|r| r.kind != RoomKind::Stairwell) {
            assert!(svg.contains(&mde_dwelling::room_label(r.kind).replace('\'', "&#39;")), "{:?} label missing", r.kind);
        }
    }
    let all = render_svg_all(&d);
    assert!(all.matches("class=\"floor\"").count() == d.floors.len());
    assert!(render_svg(&d, 9).contains("No level 9"));
}
