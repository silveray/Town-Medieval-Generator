//! Invariant tests for the town generator.
//!
//! The quick suite covers seeds 0..100 × patches {6, 15, 30, 60} with the
//! wall / citadel / plaza / river / temple toggles cycling through every
//! combination. The exhaustive cross product (6400 towns) is `#[ignore]`d;
//! run it with `cargo test -p mde_town --release -- --ignored`.

use std::collections::{BTreeMap, BTreeSet};

use mde_core::poly::{closest_point_on_segment, cross, segment_intersection};
use mde_core::{BridgeKind, Polygon, Town, TownParams, Vec2};

fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    p.distance(closest_point_on_segment(p, a, b))
}

fn polyline_dist(p: Vec2, pts: &[Vec2]) -> f32 {
    pts.windows(2).map(|w| seg_dist(p, w[0], w[1])).fold(f32::INFINITY, f32::min)
}

fn ring_dist(p: Vec2, ring: &[Vec2]) -> f32 {
    let n = ring.len();
    (0..n).map(|i| seg_dist(p, ring[i], ring[(i + 1) % n])).fold(f32::INFINITY, f32::min)
}

/// Inside or on the boundary (within `tol`).
fn inside(poly: &Polygon, p: Vec2, tol: f32) -> bool {
    poly.contains(p) || ring_dist(p, &poly.0) <= tol
}

fn convex(poly: &Polygon) -> bool {
    let n = poly.len();
    (0..n).all(|i| {
        let a = poly.0[i];
        let b = poly.0[(i + 1) % n];
        let c = poly.0[(i + 2) % n];
        cross(b - a, c - b) >= -1e-3 * (b - a).length().max(1.0) * (c - b).length().max(1.0)
    })
}

/// Separating-axis overlap of convex polygons, by more than `tol` metres.
fn overlap(a: &Polygon, b: &Polygon, tol: f32) -> bool {
    for (p, q) in [(a, b), (b, a)] {
        let n = p.len();
        for i in 0..n {
            let e = p.0[(i + 1) % n] - p.0[i];
            let Some(axis) = Vec2::new(-e.y, e.x).try_normalize() else { continue };
            let (amin, amax) =
                p.0.iter()
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| (lo.min(v.dot(axis)), hi.max(v.dot(axis))));
            let (bmin, bmax) =
                q.0.iter()
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| (lo.min(v.dot(axis)), hi.max(v.dot(axis))));
            if amax <= bmin + tol || bmax <= amin + tol {
                return false;
            }
        }
    }
    true
}

/// Uniform grid over items with bounding boxes.
struct Grid {
    cell: f32,
    map: BTreeMap<(i32, i32), Vec<usize>>,
}

impl Grid {
    fn new(cell: f32) -> Self {
        Self { cell, map: BTreeMap::new() }
    }
    fn key(&self, v: f32) -> i32 {
        (v / self.cell).floor() as i32
    }
    fn insert(&mut self, lo: Vec2, hi: Vec2, id: usize) {
        for x in self.key(lo.x)..=self.key(hi.x) {
            for y in self.key(lo.y)..=self.key(hi.y) {
                self.map.entry((x, y)).or_default().push(id);
            }
        }
    }
    fn query(&self, lo: Vec2, hi: Vec2) -> BTreeSet<usize> {
        let mut out = BTreeSet::new();
        for x in self.key(lo.x)..=self.key(hi.x) {
            for y in self.key(lo.y)..=self.key(hi.y) {
                if let Some(v) = self.map.get(&(x, y)) {
                    out.extend(v.iter().copied());
                }
            }
        }
        out
    }
}

fn bounds(pts: &[Vec2]) -> (Vec2, Vec2) {
    pts.iter()
        .fold((Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)), |(lo, hi), p| (lo.min(*p), hi.max(*p)))
}

fn key(p: Vec2) -> (u32, u32) {
    (p.x.to_bits(), p.y.to_bits())
}

/// Check every invariant; returns a list of violations.
fn check(town: &Town) -> Vec<String> {
    let mut errs = Vec::new();
    let tag = format!("seed {} patches {}", town.params.seed, town.params.patches);
    let patch_of: BTreeMap<u32, &mde_core::Patch> = town.patches.iter().map(|p| (p.id, p)).collect();

    assert_eq!(town.schema, mde_core::SCHEMA_VERSION);
    let inner = town.patches.iter().filter(|p| p.within_city).count();
    if inner != town.params.patches as usize {
        errs.push(format!("{tag}: {inner} within_city patches"));
    }
    for p in &town.patches {
        if !p.shape.is_ccw() {
            errs.push(format!("{tag}: patch {} not CCW", p.id));
        }
    }

    // Street segments grid.
    let mut segs: Vec<(Vec2, Vec2, f32)> = Vec::new();
    for s in &town.streets {
        if s.path.0.len() < 2 {
            errs.push(format!("{tag}: degenerate street"));
        }
        for w in s.path.0.windows(2) {
            segs.push((w[0], w[1], s.width));
        }
    }
    let mut sg = Grid::new(25.0);
    for (i, (a, b, w)) in segs.iter().enumerate() {
        sg.insert(a.min(*b) - Vec2::splat(*w), a.max(*b) + Vec2::splat(*w), i);
    }

    // Buildings.
    let mut bg = Grid::new(25.0);
    for (i, b) in town.buildings.iter().enumerate() {
        let fp = &b.footprint;
        if b.id as usize != i {
            errs.push(format!("{tag}: building ids not sequential"));
        }
        if fp.len() < 3 || !fp.is_ccw() {
            errs.push(format!("{tag}: building {} not CCW", b.id));
            continue;
        }
        if fp.area() < 10.0 {
            errs.push(format!("{tag}: building {} area {}", b.id, fp.area()));
        }
        if !convex(fp) {
            errs.push(format!("{tag}: building {} not convex", b.id));
        }
        if !(1..=4).contains(&b.floors) {
            errs.push(format!("{tag}: building {} floors {}", b.id, b.floors));
        }
        if (b.front.length() - 1.0).abs() > 1e-3 {
            errs.push(format!("{tag}: building {} front not unit", b.id));
        }
        match patch_of.get(&b.patch) {
            Some(p) => {
                if fp.0.iter().any(|&v| !inside(&p.shape, v, 1e-3)) {
                    errs.push(format!("{tag}: building {} outside patch {}", b.id, b.patch));
                }
                if p.within_walls
                    && let Some(w) = &town.wall
                    && fp.0.iter().any(|&v| !inside(&w.shape, v, 1e-3))
                {
                    errs.push(format!("{tag}: building {} outside the wall", b.id));
                }
            }
            None => errs.push(format!("{tag}: building {} bad patch", b.id)),
        }
        let (lo, hi) = bounds(&fp.0);
        for j in bg.query(lo, hi) {
            if overlap(fp, &town.buildings[j].footprint, 0.01) {
                errs.push(format!("{tag}: buildings {j} and {} overlap", b.id));
            }
        }
        bg.insert(lo, hi, i);
        for j in sg.query(lo - Vec2::splat(6.0), hi + Vec2::splat(6.0)) {
            let (a, c, w) = segs[j];
            for &v in &fp.0 {
                if seg_dist(v, a, c) < w * 0.5 {
                    errs.push(format!("{tag}: building {} vertex within street half-width", b.id));
                }
            }
        }
    }

    // Walls.
    for (name, wall) in [("wall", &town.wall), ("citadel", &town.citadel)] {
        let Some(w) = wall else { continue };
        if !w.shape.is_ccw() {
            errs.push(format!("{tag}: {name} ring not CCW"));
        }
        for g in &w.gates {
            if ring_dist(*g, &w.shape.0) > 1e-3 {
                errs.push(format!("{tag}: {name} gate off the wall"));
            }
        }
        for t in &w.towers {
            if ring_dist(*t, &w.shape.0) > 1e-2 {
                errs.push(format!("{tag}: {name} tower off the wall"));
            }
        }
    }
    if town.params.walls && town.wall.is_none() {
        errs.push(format!("{tag}: walls requested but missing"));
    }

    // Street network connectivity (polylines share exact vertex positions).
    let mut ids: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    let mut parent: Vec<usize> = Vec::new();
    fn find(p: &mut [usize], x: usize) -> usize {
        let mut r = x;
        while p[r] != r {
            r = p[r];
        }
        let mut c = x;
        while p[c] != r {
            let n = p[c];
            p[c] = r;
            c = n;
        }
        r
    }
    for s in &town.streets {
        let mut prev: Option<usize> = None;
        for &v in &s.path.0 {
            let id = *ids.entry(key(v)).or_insert_with(|| {
                parent.push(parent.len());
                parent.len() - 1
            });
            if let Some(p) = prev {
                let (ra, rb) = (find(&mut parent, p), find(&mut parent, id));
                parent[ra] = rb;
            }
            prev = Some(id);
        }
    }
    let roots: BTreeSet<usize> = (0..parent.len()).map(|i| find(&mut parent, i)).collect();
    if roots.len() > 1 {
        errs.push(format!("{tag}: street network has {} components", roots.len()));
    }
    if town.streets.is_empty() {
        errs.push(format!("{tag}: no streets"));
    }

    // River, bridges, water clearance.
    if let Some(r) = &town.river {
        let half = r.width * 0.5;
        let rp = &r.path.0;
        for (a, b, _) in &segs {
            for w in rp.windows(2) {
                if let Some(x) = segment_intersection(*a, *b, w[0], w[1]) {
                    let ok = town.bridges.iter().any(|br| seg_dist(x, br.a, br.b) < 0.05);
                    if !ok {
                        errs.push(format!("{tag}: street crosses the river without a bridge at {x}"));
                    }
                }
            }
            // Never in the water except on a bridge deck.
            let len = a.distance(*b);
            let n = (len / 1.0).ceil().max(1.0) as usize;
            for i in 0..=n {
                let p = *a + (*b - *a) * (i as f32 / n as f32);
                if polyline_dist(p, rp) < half - 0.05 && !town.bridges.iter().any(|br| seg_dist(p, br.a, br.b) < 0.05) {
                    errs.push(format!("{tag}: street in the water at {p}"));
                    break;
                }
            }
        }
        for br in &town.bridges {
            for end in [br.a, br.b] {
                if polyline_dist(end, rp) < half + 1.0 {
                    errs.push(format!("{tag}: bridge end in the water"));
                }
            }
            if br.kind != town.params.bridge_kind {
                errs.push(format!("{tag}: bridge kind"));
            }
        }
        for b in &town.buildings {
            let fp = &b.footprint;
            let wet = fp.0.iter().any(|&v| polyline_dist(v, rp) < half)
                || rp.iter().any(|&v| fp.contains(v))
                || rp.windows(2).any(|w| {
                    (0..fp.len()).any(|i| segment_intersection(fp.0[i], fp.0[(i + 1) % fp.len()], w[0], w[1]).is_some())
                });
            if wet {
                errs.push(format!("{tag}: building {} in the water", b.id));
            }
        }
        for t in &town.trees {
            if polyline_dist(t.pos, rp) < half + t.radius {
                errs.push(format!("{tag}: tree in the water"));
            }
        }
    } else if !town.bridges.is_empty() {
        errs.push(format!("{tag}: bridges without a river"));
    }

    // Trees off streets and buildings.
    for t in &town.trees {
        let r = t.radius;
        for j in sg.query(t.pos - Vec2::splat(r + 4.0), t.pos + Vec2::splat(r + 4.0)) {
            let (a, b, w) = segs[j];
            if seg_dist(t.pos, a, b) < w * 0.5 + r {
                errs.push(format!("{tag}: tree on a street"));
            }
        }
        for j in bg.query(t.pos - Vec2::splat(r), t.pos + Vec2::splat(r)) {
            let fp = &town.buildings[j].footprint;
            if fp.contains(t.pos) || ring_dist(t.pos, &fp.0) < r {
                errs.push(format!("{tag}: tree on a building"));
            }
        }
    }
    errs.truncate(20);
    errs
}

fn params_for(seed: u64, patches: u32, bits: u32) -> TownParams {
    TownParams {
        seed,
        patches,
        walls: bits & 1 != 0,
        citadel: bits & 2 != 0,
        plaza: bits & 4 != 0,
        river: bits & 8 != 0,
        temple: bits & 16 == 0,
        bridge_kind: if bits & 32 != 0 { BridgeKind::Stone } else { BridgeKind::Wood },
        ..Default::default()
    }
}

#[test]
fn deterministic() {
    for (seed, n, bits) in [(1, 15, 0b1111), (99, 30, 0b1011), (5, 6, 0)] {
        let p = params_for(seed, n, bits);
        let a = serde_json::to_string(&mde_town::generate(&p)).expect("serialise");
        let b = serde_json::to_string(&mde_town::generate(&p)).expect("serialise");
        assert_eq!(a, b, "seed {seed}");
    }
}

#[test]
fn json_round_trip() {
    let town = mde_town::generate(&params_for(3, 30, 0b111111));
    let s = serde_json::to_string(&town).expect("serialise");
    let back: Town = serde_json::from_str(&s).expect("parse");
    assert_eq!(back, town);
    assert_eq!(back.params, params_for(3, 30, 0b111111));
}

#[test]
fn invariants_across_seeds_and_toggles() {
    let mut failures = Vec::new();
    for seed in 0..100u64 {
        for (k, &n) in [6u32, 15, 30, 60].iter().enumerate() {
            // Cycle the toggle bits so every combination appears ~6 times.
            let bits = (seed as u32 * 4 + k as u32).wrapping_mul(2654435761) >> 26;
            let town = mde_town::generate(&params_for(seed, n, bits));
            failures.extend(check(&town));
        }
    }
    assert!(failures.is_empty(), "{} violations:\n{}", failures.len(), failures[..failures.len().min(40)].join("\n"));
}

#[test]
#[ignore = "slow: 6400 towns; run in release"]
fn invariants_exhaustive() {
    let mut failures = Vec::new();
    for seed in 0..100u64 {
        for &n in &[6u32, 15, 30, 60] {
            for bits in 0..16u32 {
                let town = mde_town::generate(&params_for(seed, n, bits | ((seed as u32 & 1) << 5)));
                failures.extend(check(&town));
            }
        }
    }
    assert!(failures.is_empty(), "{} violations:\n{}", failures.len(), failures[..failures.len().min(60)].join("\n"));
}

/// NPCs enter through the front door, so the front must face a street.
#[test]
fn fronts_face_streets() {
    use mde_core::{BuildingKind, WardKind};
    let (mut total, mut ok) = (0usize, 0usize);
    for seed in 0..30u64 {
        for (k, &n) in [6u32, 15, 30, 60].iter().enumerate() {
            let bits = (seed as u32 * 4 + k as u32).wrapping_mul(2654435761) >> 26;
            let town = mde_town::generate(&params_for(seed, n, bits));
            let ward: BTreeMap<u32, WardKind> = town.patches.iter().map(|p| (p.id, p.ward)).collect();
            for b in &town.buildings {
                // Castle lean-tos face the courtyard; farm outbuildings face the yard.
                if matches!(ward[&b.patch], WardKind::Castle | WardKind::Farm) && b.kind != BuildingKind::Keep {
                    continue;
                }
                let fp = &b.footprint.0;
                let m = fp.len();
                let i = (0..m)
                    .max_by(|&i, &j| {
                        let o = |i: usize| {
                            let d = (fp[(i + 1) % m] - fp[i]).normalize_or_zero();
                            Vec2::new(d.y, -d.x).dot(b.front)
                        };
                        o(i).total_cmp(&o(j))
                    })
                    .unwrap_or(0);
                let mid = (fp[i] + fp[(i + 1) % m]) * 0.5;
                let mut best = (f32::INFINITY, false);
                for s in &town.streets {
                    for w in s.path.0.windows(2) {
                        let q = closest_point_on_segment(mid, w[0], w[1]);
                        let d = q.distance(mid) - s.width * 0.5;
                        if d < best.0 {
                            best = (d, (q - mid).dot(b.front) > 0.0);
                        }
                    }
                }
                let reachable = best.0 < 4.5 && best.1;
                assert!(
                    reachable || !matches!(b.kind, BuildingKind::Temple | BuildingKind::Keep),
                    "seed {seed} n {n}: {:?} {} door not on a street",
                    b.kind,
                    b.id
                );
                total += 1;
                ok += reachable as usize;
            }
        }
    }
    assert!(ok as f32 >= total as f32 * 0.995, "{ok}/{total} fronts face a street");
}

#[test]
fn villages_and_cities_have_substance() {
    let village =
        mde_town::generate(&TownParams { seed: 7, patches: 12, walls: false, citadel: false, ..Default::default() });
    assert!(village.wall.is_none());
    assert!(village.buildings.len() >= 30, "village buildings {}", village.buildings.len());
    assert!(!village.fields.is_empty() && village.trees.len() > 100);
    let city = mde_town::generate(&TownParams { seed: 1234, patches: 60, river: true, ..Default::default() });
    assert!(city.buildings.len() > 500);
    assert!(city.wall.as_ref().is_some_and(|w| w.gates.len() >= 2 && w.towers.len() > 8));
    assert!(city.citadel.is_some() && city.plaza.is_some());
    assert!(city.buildings.iter().any(|b| b.kind == mde_core::BuildingKind::Temple));
    assert!(city.buildings.iter().any(|b| b.kind == mde_core::BuildingKind::Keep));
    assert!(!city.bridges.is_empty());
}

#[test]
fn odd_params_do_not_panic() {
    for patches in [0u32, 1, 2, 3, 4, 200] {
        for lot in [0.0f32, -3.0, 1.0, 4.0, 25.0, 1000.0, f32::NAN] {
            let p = TownParams { seed: patches as u64, patches, lot_size: lot, river: true, ..Default::default() };
            let t = mde_town::generate(&p);
            assert!(t.radius > 0.0);
        }
    }
}
