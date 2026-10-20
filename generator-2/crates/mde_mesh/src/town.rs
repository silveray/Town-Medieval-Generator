//! Town → layered models: ground, streets, plaza, fields, walls, river, trees,
//! bridges and lot houses.
//!
//! Layers sit at small, consistent offsets above the ground so nothing
//! z-fights: ground 0, fields +0.04, streets +0.06…0.075 (by kind), plaza
//! +0.09. The ground is a height field with the river channel carved in; the
//! water ribbon lies 0.35 m below ground level.

use mde_core::poly::closest_point_on_segment;
use mde_core::{MaterialSlot as S, MeshData, Model, Polygon, Rng, StreetKind, Town, Vec2, Vec3, Wall};

use crate::geom::{blob, frustum, jitter, mix_seed, poly_facing, quad_facing, vert, w3, wdir, Mb};
use crate::palette::{lin, mix, scale, Linear, WHITE};
use crate::triangulate::triangulate_points;

/// What [`build_town`] includes besides the always-present layers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TownMeshOptions {
    /// Lot houses ([`crate::build_lot_house`] for every building).
    pub buildings: bool,
    pub trees: bool,
    pub bridges: bool,
}

impl Default for TownMeshOptions {
    fn default() -> Self {
        Self { buildings: true, trees: true, bridges: true }
    }
}

/// Water surface height relative to the ground.
const WATER_Y: f32 = -0.35;

/// River channel: polyline, half width and bank width.
struct Channel {
    pts: Vec<Vec2>,
    half: f32,
    bank: f32,
    depth: f32,
}

impl Channel {
    fn new(t: &Town) -> Option<Channel> {
        let r = t.river.as_ref()?;
        let pts: Vec<Vec2> = r.path.0.iter().copied().filter(|p| p.is_finite()).collect();
        if pts.len() < 2 || !r.width.is_finite() || r.width <= 0.1 {
            return None;
        }
        let width = r.width.min(200.0);
        // Banks are short so bridge landings (2 m past the water) sit on level ground.
        Some(Channel { pts, half: width * 0.5, bank: 1.8, depth: 1.0 + 0.04 * width.min(40.0) })
    }

    fn dist(&self, p: Vec2) -> f32 {
        self.pts
            .windows(2)
            .map(|w| p.distance(closest_point_on_segment(p, w[0], w[1])))
            .fold(f32::INFINITY, f32::min)
    }

    /// Ground height (≤ 0) at `p`.
    fn ground(&self, p: Vec2) -> f32 {
        let d = self.dist(p);
        let outer = self.half + self.bank;
        if d >= outer {
            return 0.0;
        }
        let inner = self.half * 0.55;
        let t = ((outer - d) / (outer - inner).max(1e-3)).clamp(0.0, 1.0);
        -self.depth * t * t * (3.0 - 2.0 * t)
    }

    /// Whether `p` is in the river channel (streets are cut there).
    fn wet(&self, p: Vec2) -> bool {
        self.dist(p) < self.half + self.bank * 0.9
    }
}

/// Smooth deterministic value noise in `[0, 1]`.
fn noise(seed: u64, p: Vec2, scale_m: f32) -> f32 {
    let q = p / scale_m;
    let (x0, y0) = (q.x.floor(), q.y.floor());
    let (fx, fy) = (q.x - x0, q.y - y0);
    let h = |ix: f32, iy: f32| -> f32 {
        let k = (ix as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (iy as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        Rng::new(seed ^ k).next_f32()
    };
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy) = (s(fx), s(fy));
    let a = h(x0, y0) + (h(x0 + 1.0, y0) - h(x0, y0)) * sx;
    let b = h(x0, y0 + 1.0) + (h(x0 + 1.0, y0 + 1.0) - h(x0, y0 + 1.0)) * sx;
    a + (b - a) * sy
}

/// Build the town's models, always in this order and with these names:
/// `"ground"`, `"streets"`, `"plaza"`, `"fields"`, `"walls"`, `"river"`,
/// `"trees"`, `"bridges"`, `"buildings"` (empty when absent or disabled).
pub fn build_town(t: &Town, opts: &TownMeshOptions) -> Vec<Model> {
    let seed = t.params.seed;
    let channel = Channel::new(t);
    let ground_h = |p: Vec2| channel.as_ref().map_or(0.0, |c| c.ground(p));
    let center = if t.center.is_finite() { t.center } else { Vec2::ZERO };
    let radius = if t.radius.is_finite() && t.radius > 1.0 { t.radius.min(5000.0) } else { 100.0 };

    let mut out = Vec::with_capacity(9);
    out.push(ground(t, center, radius, channel.as_ref(), seed));
    out.push(streets(t, channel.as_ref(), &ground_h, seed));
    out.push(plaza(t, seed));
    out.push(fields(t, &ground_h, seed));
    out.push(walls(t, seed));
    out.push(river(channel.as_ref(), center, radius * 1.6, seed));
    out.push(if opts.trees { trees(t, &ground_h, seed) } else { Model::new("trees") });
    let mut bridges = Mb::new();
    if opts.bridges {
        for b in &t.bridges {
            bridges.append_model(&crate::build_bridge(b));
        }
    }
    out.push(bridges.into_model("bridges"));
    let mut houses = Mb::new();
    if opts.buildings {
        for b in &t.buildings {
            houses.append_model(&crate::build_lot_house(b, None));
        }
    }
    out.push(houses.into_model("buildings"));
    out
}

fn ground(t: &Town, center: Vec2, radius: f32, ch: Option<&Channel>, seed: u64) -> Model {
    let mut mb = Mb::new();
    let rd = radius * 1.6;
    let cell = (2.0 * rd / 170.0).clamp(1.5, 8.0);
    let n = ((2.0 * rd / cell).ceil() as usize).clamp(2, 600);
    let cell = 2.0 * rd / n as f32;
    let city: Vec<(&Polygon, mde_core::poly::Rect)> =
        t.patches.iter().filter(|p| p.within_city && p.shape.len() >= 3).map(|p| (&p.shape, p.shape.bounds())).collect();
    let parks: Vec<(&Polygon, mde_core::poly::Rect)> = t
        .patches
        .iter()
        .filter(|p| matches!(p.ward, mde_core::WardKind::Park | mde_core::WardKind::Farm | mde_core::WardKind::Empty))
        .map(|p| (&p.shape, p.shape.bounds()))
        .collect();
    let inside = |set: &[(&Polygon, mde_core::poly::Rect)], p: Vec2| set.iter().any(|(s, b)| b.contains(p) && s.contains(p));
    let dirt = lin(1.0, 0.84, 0.66);
    let mud = lin(0.78, 0.68, 0.52);
    let height = |p: Vec2| ch.map_or(0.0, |c| c.ground(p));
    // Vertex attributes from plan position only, so shared positions match.
    let attrs = |p: Vec2| -> (Vec3, Vec3, Linear) {
        let y = height(p);
        let nz = noise(seed ^ 0x6E01, p, 23.0) * 0.6 + noise(seed ^ 0x6E02, p, 7.0) * 0.4;
        let mut col = scale(WHITE, 0.86 + 0.26 * nz);
        if inside(&city, p) && !inside(&parks, p) {
            col = crate::palette::mul(col, mix(WHITE, dirt, 0.55 + 0.3 * nz));
        }
        if y < -0.05 {
            col = crate::palette::mul(col, mix(WHITE, mud, (-y / 0.6).clamp(0.0, 1.0)));
        }
        let n3 = match ch {
            Some(c) => {
                let e = cell * 0.5;
                let hx = c.ground(p + Vec2::new(e, 0.0)) - c.ground(p - Vec2::new(e, 0.0));
                let hy = c.ground(p + Vec2::new(0.0, e)) - c.ground(p - Vec2::new(0.0, e));
                Vec3::new(-hx / (2.0 * e), 1.0, hy / (2.0 * e)).normalize()
            }
            None => Vec3::Y,
        };
        (w3(p, y), n3, col)
    };
    // Disc boundary as a fine convex polygon for clipping rim cells.
    let rim_segs = 160;
    let rim: Vec<Vec2> = (0..rim_segs).map(|k| center + Vec2::from_angle(k as f32 / rim_segs as f32 * std::f32::consts::TAU) * rd).collect();
    let grid = |i: usize, j: usize| center + Vec2::new(-rd + i as f32 * cell, -rd + j as f32 * cell);
    let m = mb.m(S::Grass);
    let mut shared = vec![u32::MAX; (n + 1) * (n + 1)];
    for j in 0..n {
        for i in 0..n {
            let corners = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)];
            let pts: Vec<Vec2> = corners.iter().map(|&(a, b)| grid(a, b)).collect();
            let inside_n = pts.iter().filter(|p| p.distance(center) <= rd).count();
            if inside_n == 4 {
                let mut idx = [0u32; 4];
                for (k, &(a, b)) in corners.iter().enumerate() {
                    let key = b * (n + 1) + a;
                    if shared[key] == u32::MAX {
                        let (w, nn, c) = attrs(grid(a, b));
                        shared[key] = vert(m, w, nn, [w.x, w.z], c);
                    }
                    idx[k] = shared[key];
                }
                m.indices.extend([idx[0], idx[1], idx[2], idx[0], idx[2], idx[3]]);
                continue;
            }
            if pts.iter().all(|p| p.distance(center) > rd + cell * 1.5) {
                continue;
            }
            // Rim cell: clip the square to the disc and fan it.
            let clipped = clip_convex(&pts, &rim);
            if clipped.len() < 3 || crate::geom::signed_area(&clipped) < 1e-4 {
                continue;
            }
            let base = m.positions.len() as u32;
            for &p in &clipped {
                let (w, nn, c) = attrs(p);
                vert(m, w, nn, [w.x, w.z], c);
            }
            for k in 1..clipped.len() as u32 - 1 {
                m.indices.extend([base, base + k, base + k + 1]);
            }
        }
    }
    // Skirt around the disc edge so it reads as a slab from low angles.
    let edge_col = lin(0.55, 0.45, 0.34);
    for k in 0..rim_segs {
        let (a, b) = (rim[k], rim[(k + 1) % rim_segs]);
        let out = wdir((a + b) * 0.5 - center);
        quad_facing(mb.m(S::Ground), w3(a, -2.0), w3(b, -2.0), w3(b, height(b)), w3(a, height(a)), out, edge_col);
    }
    mb.into_model("ground")
}

/// Sutherland–Hodgman: convex `poly` (CCW) clipped to convex `clip` (CCW).
fn clip_convex(poly: &[Vec2], clip: &[Vec2]) -> Vec<Vec2> {
    let mut out = poly.to_vec();
    let n = clip.len();
    for i in 0..n {
        if out.len() < 3 {
            break;
        }
        let a = clip[i];
        let b = clip[(i + 1) % n];
        let side = |p: Vec2| mde_core::poly::cross(b - a, p - a);
        let input = std::mem::take(&mut out);
        for k in 0..input.len() {
            let p = input[k];
            let q = input[(k + 1) % input.len()];
            let (sp, sq) = (side(p), side(q));
            if sp >= 0.0 {
                out.push(p);
            }
            if (sp >= 0.0) != (sq >= 0.0) {
                let t = sp / (sp - sq);
                out.push(p + (q - p) * t);
            }
        }
    }
    out
}

/// Strip along a polyline at `y(p) + lift` with round joins (an arc fan on
/// the outside of every bend, a mitre inside) and round caps at both ends.
/// Segments, fans and caps tile the outline without overlapping.
#[allow(clippy::too_many_arguments)]
fn strip(m: &mut MeshData, pts: &[Vec2], half: f32, lift: f32, y: &dyn Fn(Vec2) -> f32, col: Linear, rng: &mut Rng) {
    let mut p: Vec<Vec2> = Vec::with_capacity(pts.len());
    for &q in pts {
        if p.last().is_none_or(|l: &Vec2| l.distance(q) > 1e-3) {
            p.push(q);
        }
    }
    let n = p.len();
    if n < 2 {
        return;
    }
    let perp = |d: Vec2| Vec2::new(-d.y, d.x);
    let dir = |i: usize| (p[i + 1] - p[i]).normalize_or(Vec2::X);
    let h = |i: usize| y(p[i]) + lift;
    // Left / right corner points where segment `i` starts and ends.
    let mut seg_start: Vec<(Vec2, Vec2)> = Vec::with_capacity(n - 1);
    let mut seg_end: Vec<(Vec2, Vec2)> = Vec::with_capacity(n - 1);
    for i in 0..n - 1 {
        let nrm = perp(dir(i)) * half;
        seg_start.push((p[i] + nrm, p[i] - nrm));
        seg_end.push((p[i + 1] + nrm, p[i + 1] - nrm));
    }
    // Interior joints: move the inner corners to the mitre point and fill the
    // outer wedge with an arc fan.
    let mut fans: Vec<Vec<Vec3>> = Vec::new();
    for i in 1..n - 1 {
        let (d0, d1) = (dir(i - 1), dir(i));
        let turn = mde_core::poly::cross(d0, d1);
        if turn.abs() < 1e-4 {
            continue;
        }
        let bis = (perp(d0) + perp(d1)).normalize_or(perp(d0));
        let scale = 1.0 / bis.dot(perp(d0)).max(0.4);
        let left_turn = turn > 0.0;
        // Inner side is to the left for left turns.
        let inner = if left_turn { p[i] + bis * half * scale } else { p[i] - bis * half * scale };
        if left_turn {
            seg_end[i - 1].0 = inner;
            seg_start[i].0 = inner;
        } else {
            seg_end[i - 1].1 = inner;
            seg_start[i].1 = inner;
        }
        let (a, b) = if left_turn { (seg_end[i - 1].1, seg_start[i].1) } else { (seg_end[i - 1].0, seg_start[i].0) };
        let a0 = (a - p[i]).to_angle();
        let mut sweep = (b - p[i]).to_angle() - a0;
        while sweep > std::f32::consts::PI {
            sweep -= std::f32::consts::TAU;
        }
        while sweep < -std::f32::consts::PI {
            sweep += std::f32::consts::TAU;
        }
        let steps = ((sweep.abs() / 0.35).ceil() as usize).clamp(1, 12);
        let hh = h(i);
        let mut fan = vec![w3(inner, hh)];
        for k in 0..=steps {
            let ang = a0 + sweep * k as f32 / steps as f32;
            fan.push(w3(p[i] + Vec2::from_angle(ang) * half, hh));
        }
        fans.push(fan);
    }
    for i in 0..n - 1 {
        let c = jitter(col, rng, 0.025);
        let ((l0, r0), (l1, r1)) = (seg_start[i], seg_end[i]);
        let (h0, h1) = (h(i), h(i + 1));
        quad_facing(m, w3(r0, h0), w3(r1, h1), w3(l1, h1), w3(l0, h0), Vec3::Y, c);
    }
    for fan in fans {
        poly_facing(m, &fan, Vec3::Y, col);
    }
    let segs = if half > 2.0 { 8 } else { 5 };
    for (pc, d, hh) in [(p[0], -dir(0), h(0)), (p[n - 1], dir(n - 2), h(n - 1))] {
        let side = perp(d);
        let mut cap = vec![w3(pc, hh)];
        for k in 0..=segs {
            let a = std::f32::consts::PI * k as f32 / segs as f32;
            cap.push(w3(pc + (side * a.cos() + d * a.sin()) * half, hh));
        }
        poly_facing(m, &cap, Vec3::Y, col);
    }
}

/// Split a polyline into pieces of at most `step` metres.
fn densify(pts: &[Vec2], step: f32) -> Vec<Vec2> {
    let mut out = Vec::new();
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let n = ((a.distance(b) / step).ceil() as usize).max(1);
        for k in 0..n {
            out.push(a + (b - a) * (k as f32 / n as f32));
        }
    }
    if let Some(&l) = pts.last() {
        out.push(l);
    }
    out
}

fn streets(t: &Town, ch: Option<&Channel>, gh: &dyn Fn(Vec2) -> f32, seed: u64) -> Model {
    let mut mb = Mb::new();
    let mut rng = Rng::new(mix_seed(seed, 0x57));
    // Draw minor streets first so arteries end up on top.
    let mut order: Vec<&mde_core::Street> = t.streets.iter().collect();
    let rank = |k: StreetKind| match k {
        StreetKind::Road => 0,
        StreetKind::Alley => 1,
        StreetKind::Street => 2,
        StreetKind::Artery => 3,
    };
    order.sort_by_key(|s| rank(s.kind));
    for (si, s) in order.into_iter().enumerate() {
        let pts: Vec<Vec2> = s.path.0.iter().copied().filter(|p| p.is_finite()).collect();
        if pts.len() < 2 || !s.width.is_finite() || s.width <= 0.05 {
            continue;
        }
        let half = s.width.min(40.0) * 0.5;
        let (lift, col) = match s.kind {
            StreetKind::Artery => (0.08, lin(1.0, 0.98, 0.95)),
            StreetKind::Street => (0.07, lin(0.95, 0.93, 0.9)),
            StreetKind::Alley => (0.06, lin(0.86, 0.84, 0.8)),
            StreetKind::Road => (0.05, lin(0.95, 0.82, 0.64)),
        };
        // A distinct height per street so crossings never share a plane.
        let lift = lift + (si % 6) as f32 * 0.0015;
        // Cut where the street crosses water (bridges span the gap).
        let dense = densify(&pts, 2.5);
        let mut runs: Vec<Vec<Vec2>> = vec![Vec::new()];
        for &p in &dense {
            if ch.is_some_and(|c| c.wet(p)) {
                if runs.last().is_some_and(|r| !r.is_empty()) {
                    runs.push(Vec::new());
                }
            } else if let Some(r) = runs.last_mut() {
                r.push(p);
            }
        }
        for run in runs.into_iter().filter(|r| r.len() >= 2) {
            strip(mb.m(S::Street), &run, half, lift, gh, col, &mut rng);
        }
    }
    mb.into_model("streets")
}

fn plaza(t: &Town, seed: u64) -> Model {
    let mut mb = Mb::new();
    let Some(p) = &t.plaza else { return mb.into_model("plaza") };
    let pts: Vec<Vec2> = p.0.iter().copied().filter(|q| q.is_finite()).collect();
    if pts.len() < 3 {
        return mb.into_model("plaza");
    }
    let mut rng = Rng::new(mix_seed(seed, 0x9A2));
    let y = 0.09;
    let tris = triangulate_points(&pts);
    let col = lin(1.0, 0.97, 0.92);
    let m = mb.m(S::Street);
    for tri in tris {
        let q = [w3(pts[tri[0]], y), w3(pts[tri[1]], y), w3(pts[tri[2]], y)];
        poly_facing(m, &q, Vec3::Y, jitter(col, &mut rng, 0.04));
    }
    // A well in the middle of larger squares.
    let poly = Polygon(pts);
    let c = poly.centroid();
    if poly.area() > 150.0 && poly.contains(c) {
        let ms = mb.m(S::Stone);
        frustum(ms, w3(c, y), Vec3::Y, 1.1, 1.0, 0.85, 12, jitter(WHITE, &mut rng, 0.04), false, (false, true));
        let mt = mb.m(S::Timber);
        let tc = lin(0.4, 0.3, 0.22);
        for s in [-1.0f32, 1.0] {
            let b = w3(c + Vec2::new(s * 0.9, 0.0), y + 0.85);
            crate::geom::aabox(mt, b - Vec3::new(0.08, 0.0, 0.08), b + Vec3::new(0.08, 1.6, 0.08), tc);
        }
        crate::geom::beam(mt, w3(c + Vec2::new(-1.0, 0.0), y + 2.3), w3(c + Vec2::new(1.0, 0.0), y + 2.3), Vec3::Y, 0.1, 0.1, tc, 0);
        // Little roof.
        let mr = mb.m(S::RoofTile);
        let rc = lin(0.62, 0.32, 0.22);
        let (h0, h1) = (y + 2.35, y + 3.0);
        for s in [-1.0f32, 1.0] {
            quad_facing(
                mr,
                w3(c + Vec2::new(-1.3, s * 0.9), h0),
                w3(c + Vec2::new(1.3, s * 0.9), h0),
                w3(c + Vec2::new(1.3, 0.0), h1),
                w3(c + Vec2::new(-1.3, 0.0), h1),
                Vec3::new(0.0, 1.0, -s),
                rc,
            );
            quad_facing(
                mr,
                w3(c + Vec2::new(-1.3, s * 0.9), h0),
                w3(c + Vec2::new(1.3, s * 0.9), h0),
                w3(c + Vec2::new(1.3, 0.0), h1),
                w3(c + Vec2::new(-1.3, 0.0), h1),
                Vec3::new(0.0, -1.0, s),
                scale(rc, 0.6),
            );
        }
        let mw = mb.m(S::Water);
        let ring: Vec<Vec3> = (0..12).map(|k| w3(c + Vec2::from_angle(k as f32 / 12.0 * std::f32::consts::TAU) * 0.8, y + 0.6)).collect();
        poly_facing(mw, &ring, Vec3::Y, WHITE);
    }
    mb.into_model("plaza")
}

fn fields(t: &Town, gh: &dyn Fn(Vec2) -> f32, seed: u64) -> Model {
    let mut mb = Mb::new();
    let mut rng = Rng::new(mix_seed(seed, 0xF1E));
    let crops = [lin(1.0, 1.0, 1.0), lin(0.78, 0.98, 0.68), lin(0.86, 0.72, 0.58), lin(1.0, 0.92, 0.7), lin(0.7, 0.88, 0.62)];
    for f in &t.fields {
        let pts: Vec<Vec2> = f.0.iter().copied().filter(|p| p.is_finite()).collect();
        if pts.len() < 3 {
            continue;
        }
        let mut poly = Polygon(pts).simplified(1e-3);
        if poly.len() < 3 || poly.area() < 1.0 {
            continue;
        }
        poly.make_ccw();
        let crop = crops[rng.index(crops.len())];
        let i = poly.longest_edge();
        let along = (poly.at(i as isize + 1) - poly.at(i as isize)).normalize_or(Vec2::X);
        let across = Vec2::new(-along.y, along.x);
        let (lo, hi) = poly.0.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(l, h), p| (l.min(p.dot(across)), h.max(p.dot(across))));
        let stripe = 3.5;
        let n = (((hi - lo) / stripe).ceil() as usize).clamp(1, 200);
        for k in 0..n {
            let a = lo + (hi - lo) * k as f32 / n as f32;
            let b = lo + (hi - lo) * (k + 1) as f32 / n as f32;
            // Keep the part between the two lines (left of the first, right of the second).
            let (upper, _) = poly.split(across * a, along);
            let (_, band) = upper.split(across * b, along);
            let band = band.simplified(1e-4);
            if band.len() < 3 {
                continue;
            }
            let col = scale(crop, if k % 2 == 0 { 1.0 } else { 0.86 });
            let col = jitter(col, &mut rng, 0.03);
            let tris = triangulate_points(&band.0);
            let m = mb.m(S::Field);
            for tri in tris {
                let q: Vec<Vec3> = tri.iter().map(|&j| w3(band.0[j], gh(band.0[j]) + 0.04)).collect();
                poly_facing(m, &q, Vec3::Y, col);
            }
        }
    }
    mb.into_model("fields")
}

fn walls(t: &Town, seed: u64) -> Model {
    let mut mb = Mb::new();
    let mut rng = Rng::new(mix_seed(seed, 0xA11));
    if let Some(w) = &t.wall {
        fortification(&mut mb, w, 4.6, &mut rng);
    }
    if let Some(w) = &t.citadel {
        fortification(&mut mb, w, 3.6, &mut rng);
    }
    mb.into_model("walls")
}

/// Offset a polyline sideways by `d` (left positive) with mitred joins.
fn offset_polyline(pts: &[Vec2], d: f32, closed: bool) -> Vec<Vec2> {
    let n = pts.len();
    let dir = |i: usize, j: usize| (pts[j] - pts[i]).normalize_or(Vec2::X);
    (0..n)
        .map(|i| {
            let prev = if i > 0 { Some(dir(i - 1, i)) } else if closed { Some(dir(n - 1, 0)) } else { None };
            let next = if i + 1 < n { Some(dir(i, i + 1)) } else if closed { Some(dir(n - 1, 0)) } else { None };
            let nl = |v: Vec2| Vec2::new(-v.y, v.x);
            match (prev, next) {
                (Some(a), Some(b)) => {
                    let m = (nl(a) + nl(b)).normalize_or(nl(a));
                    let cos = m.dot(nl(a)).max(0.35);
                    pts[i] + m * (d / cos)
                }
                (Some(a), None) => pts[i] + nl(a) * d,
                (None, Some(b)) => pts[i] + nl(b) * d,
                (None, None) => pts[i],
            }
        })
        .collect()
}

fn fortification(mb: &mut Mb, w: &Wall, gate_w: f32, rng: &mut Rng) {
    let mut ring = Polygon(w.shape.0.iter().copied().filter(|p| p.is_finite()).collect()).simplified(1e-3);
    if ring.len() < 3 {
        return;
    }
    ring.make_ccw();
    let h = if w.height.is_finite() { w.height.clamp(3.0, 25.0) } else { 8.0 };
    let th = if w.thickness.is_finite() { w.thickness.clamp(0.8, 8.0) } else { 2.0 };
    let bottom = -1.6;
    let pts = &ring.0;
    let n = pts.len();
    // Arc-length parameter of every ring vertex.
    let mut acc = vec![0.0f32; n + 1];
    for i in 0..n {
        acc[i + 1] = acc[i] + pts[i].distance(pts[(i + 1) % n]);
    }
    let total = acc[n];
    let param_of = |p: Vec2| -> f32 {
        let mut best = (f32::INFINITY, 0.0);
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let q = closest_point_on_segment(p, a, b);
            let d = q.distance(p);
            if d < best.0 {
                best = (d, acc[i] + a.distance(q));
            }
        }
        best.1
    };
    let at = |s: f32| -> Vec2 {
        let s = s.rem_euclid(total.max(1e-6));
        for i in 0..n {
            if s <= acc[i + 1] {
                let l = acc[i + 1] - acc[i];
                let t = if l > 1e-6 { (s - acc[i]) / l } else { 0.0 };
                return pts[i] + (pts[(i + 1) % n] - pts[i]) * t;
            }
        }
        pts[0]
    };
    let gates: Vec<f32> = w.gates.iter().filter(|g| g.is_finite()).map(|&g| param_of(g)).collect();
    let stone = |rng: &mut Rng| jitter(WHITE, rng, 0.04);
    // Runs between gates.
    let mut cuts: Vec<(f32, f32)> = gates.iter().map(|&g| (g - gate_w * 0.5, g + gate_w * 0.5)).collect();
    cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut runs: Vec<Vec<Vec2>> = Vec::new();
    if cuts.is_empty() {
        let mut r = pts.clone();
        r.push(pts[0]);
        runs.push(r);
    } else {
        for k in 0..cuts.len() {
            let s0 = cuts[k].1;
            let s1 = if k + 1 < cuts.len() { cuts[k + 1].0 } else { cuts[0].0 + total };
            if s1 - s0 < 0.5 {
                continue;
            }
            let mut r = vec![at(s0)];
            for i in 0..2 * n {
                let s = acc[i % n] + if i >= n { total } else { 0.0 };
                if s > s0 + 1e-3 && s < s1 - 1e-3 {
                    r.push(pts[i % n]);
                }
            }
            r.push(at(s1));
            runs.push(r);
        }
    }
    for run in &runs {
        let closed = cuts.is_empty();
        let base: Vec<Vec2> = if closed { run[..run.len() - 1].to_vec() } else { run.clone() };
        let left = offset_polyline(&base, th * 0.5, closed);
        let right = offset_polyline(&base, -th * 0.5, closed);
        let lip = offset_polyline(&base, th * 0.5 - 0.25, closed);
        let m = mb.m(S::Stone);
        let segs = if closed { base.len() } else { base.len() - 1 };
        for i in 0..segs {
            let j = (i + 1) % base.len();
            let (l0, l1, r0, r1) = (left[i], left[j], right[i], right[j]);
            let d = (base[j] - base[i]).normalize_or(Vec2::X);
            let nl = wdir(Vec2::new(-d.y, d.x));
            let c = stone(rng);
            // Inner (left of a CCW ring = town side) and outer faces, walkway.
            quad_facing(m, w3(l0, bottom), w3(l1, bottom), w3(l1, h), w3(l0, h), nl, scale(c, 0.95));
            quad_facing(m, w3(r0, bottom), w3(r1, bottom), w3(r1, h), w3(r0, h), -nl, c);
            quad_facing(m, w3(l0, h), w3(l1, h), w3(r1, h), w3(r0, h), Vec3::Y, scale(c, 0.85));
            // Merlons on the outer edge.
            let len = base[i].distance(base[j]);
            let count = (len / 1.5).floor() as usize;
            for k in 0..count {
                let s = (k as f32 + 0.5) * len / count as f32;
                let p = base[i] + d * s;
                let f = crate::walls::WallFrame { origin: p, dir: d, out: -Vec2::new(-d.y, d.x) };
                f.block(m, -0.38, 0.38, h, h + 0.75, th * 0.5 - 0.5, th * 0.5, jitter(c, rng, 0.04), crate::geom::faces::NY);
            }
            // Low parapet on the town side of the walkway (mitred with its neighbours).
            let (a0, a1, b0, b1) = (left[i], left[j], lip[i], lip[j]);
            let lc = scale(c, 0.9);
            quad_facing(m, w3(a0, h), w3(a1, h), w3(a1, h + 0.35), w3(a0, h + 0.35), nl, lc);
            quad_facing(m, w3(b0, h), w3(b1, h), w3(b1, h + 0.35), w3(b0, h + 0.35), -nl, lc);
            quad_facing(m, w3(a0, h + 0.35), w3(a1, h + 0.35), w3(b1, h + 0.35), w3(b0, h + 0.35), Vec3::Y, scale(lc, 1.05));
        }
        // Run ends stop inside the gatehouse towers, so they need no caps.
    }
    // Round towers with conical caps.
    let slate = lin(0.36, 0.39, 0.44);
    for &tp in w.towers.iter().filter(|p| p.is_finite()) {
        if gates.iter().any(|&g| (param_of(tp) - g).abs() < gate_w * 1.5) {
            continue;
        }
        let r = (th * 1.15).max(2.3);
        let top = h + 2.8;
        let c = stone(rng);
        frustum(mb.m(S::Stone), w3(tp, bottom), Vec3::Y, r * 1.06, r, top - bottom, 14, c, false, (false, false));
        frustum(mb.m(S::Stone), w3(tp, top - 0.7), Vec3::Y, r + 0.25, r + 0.25, 0.7, 14, scale(c, 1.05), false, (true, true));
        frustum(mb.m(S::Slate), w3(tp, top), Vec3::Y, r + 0.55, 0.0, r * 1.9, 14, jitter(slate, rng, 0.05), false, (true, false));
        // Arrow slits.
        for k in 0..3 {
            let a = k as f32 * 2.1 + rng.range_f32(0.0, 0.5);
            let dir = Vec2::from_angle(a);
            let p = tp + dir * (r + 0.01);
            let f = crate::walls::WallFrame { origin: p, dir: Vec2::new(-dir.y, dir.x), out: dir };
            f.block(mb.m(S::Iron), -0.08, 0.08, h * 0.5, h * 0.5 + 1.0, -0.02, 0.01, scale(WHITE, 0.6), 0);
        }
    }
    // Gatehouses: two towers flanking a passable gap, a block over the passage.
    for &g in &gates {
        let p0 = at(g - gate_w * 0.5);
        let p1 = at(g + gate_w * 0.5);
        let d = (p1 - p0).normalize_or(Vec2::X);
        let nl = Vec2::new(-d.y, d.x);
        let depth = th + 2.4;
        let tw = 3.4;
        let top = h + 2.6;
        let c = stone(rng);
        let mid = (p0 + p1) * 0.5;
        for (p, sgn) in [(p0, -1.0f32), (p1, 1.0)] {
            let cen = p + d * (sgn * tw * 0.5);
            let fp: Vec<Vec2> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .iter()
                .map(|&(x, y)| cen + d * (x * tw * 0.5) + nl * (y * depth * 0.5))
                .collect();
            let m = mb.m(S::Stone);
            crate::geom::prism(m, &fp, bottom, top, c, true, false);
            // Pyramid roof.
            let apex = w3(cen, top + 3.2);
            let ov: Vec<Vec2> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .iter()
                .map(|&(x, y)| cen + d * (x * (tw * 0.5 + 0.35)) + nl * (y * (depth * 0.5 + 0.35)))
                .collect();
            let ms = mb.m(S::Slate);
            for k in 0..4 {
                let (a, b) = (ov[k], ov[(k + 1) % 4]);
                let out = wdir((a + b) * 0.5 - cen);
                poly_facing(ms, &[w3(a, top - 0.2), w3(b, top - 0.2), apex], out + Vec3::Y, jitter(slate, rng, 0.04));
                quad_facing(ms, w3(a, top - 0.2), w3(b, top - 0.2), w3(b, top - 0.35), w3(a, top - 0.35), out, scale(slate, 0.7));
            }
            let under: Vec<Vec3> = ov.iter().map(|&q| w3(q, top - 0.35)).collect();
            poly_facing(ms, &under, -Vec3::Y, scale(slate, 0.5));
        }
        // Block over the passage (clear height 4.6 m).
        let clear = 4.6f32.min(h - 0.5);
        let fp: Vec<Vec2> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .iter()
            .map(|&(x, y)| mid + d * (x * gate_w * 0.5) + nl * (y * (th * 0.5 + 0.4)))
            .collect();
        let m = mb.m(S::Stone);
        crate::geom::prism(m, &fp, clear, h + 1.2, scale(c, 0.95), true, true);
        // Portcullis hint: dark iron grille above the passage.
        let f = crate::walls::WallFrame { origin: mid - d * (gate_w * 0.5), dir: d, out: -nl };
        for k in 1..6 {
            let s = gate_w * k as f32 / 6.0;
            f.block(mb.m(S::Iron), s - 0.04, s + 0.04, clear - 0.6, clear, th * 0.5 + 0.3, th * 0.5 + 0.38, scale(WHITE, 0.7), 0);
        }
    }
}

fn river(ch: Option<&Channel>, center: Vec2, rd: f32, seed: u64) -> Model {
    let mut mb = Mb::new();
    let Some(c) = ch else { return mb.into_model("river") };
    let mut rng = Rng::new(mix_seed(seed, 0x21E));
    // Keep the part over the ground disc, ending exactly at its rim.
    let pts = clip_to_disc(&densify(&c.pts, 4.0), center, rd - 0.05);
    if pts.len() < 2 {
        return mb.into_model("river");
    }
    let half = c.half + c.bank * 0.75;
    let left = offset_polyline(&pts, half, false);
    let right = offset_polyline(&pts, -half, false);
    let mid_l = offset_polyline(&pts, half * 0.4, false);
    let mid_r = offset_polyline(&pts, -half * 0.4, false);
    let deep = lin(0.82, 0.9, 0.92);
    let shallow = lin(1.1, 1.08, 1.0);
    let m = mb.m(S::Water);
    for i in 0..pts.len() - 1 {
        let j = i + 1;
        let jit = 1.0 + rng.range_f32(-0.02, 0.02);
        let strips = [(right[i], right[j], mid_r[i], mid_r[j], shallow), (mid_r[i], mid_r[j], mid_l[i], mid_l[j], deep), (mid_l[i], mid_l[j], left[i], left[j], shallow)];
        for (a0, a1, b0, b1, col) in strips {
            quad_facing(m, w3(a0, WATER_Y), w3(a1, WATER_Y), w3(b1, WATER_Y), w3(b0, WATER_Y), Vec3::Y, scale(col, jit));
        }
    }
    mb.into_model("river")
}

/// The longest piece of a polyline inside a disc, cut exactly at the rim.
fn clip_to_disc(pts: &[Vec2], c: Vec2, r: f32) -> Vec<Vec2> {
    let inside = |p: Vec2| p.distance(c) <= r;
    // Intersection of segment a→b with the circle (the root in 0..1).
    let cross_at = |a: Vec2, b: Vec2| -> Vec2 {
        let d = b - a;
        let f = a - c;
        let (qa, qb, qc) = (d.dot(d), 2.0 * f.dot(d), f.dot(f) - r * r);
        let disc = (qb * qb - 4.0 * qa * qc).max(0.0).sqrt();
        let t1 = (-qb - disc) / (2.0 * qa.max(1e-9));
        let t2 = (-qb + disc) / (2.0 * qa.max(1e-9));
        let t = if (0.0..=1.0).contains(&t1) { t1 } else { t2.clamp(0.0, 1.0) };
        a + d * t
    };
    let mut pieces: Vec<Vec<Vec2>> = Vec::new();
    let mut cur: Vec<Vec2> = Vec::new();
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        match (inside(a), inside(b)) {
            (true, true) => {
                if cur.is_empty() {
                    cur.push(a);
                }
                cur.push(b);
            }
            (true, false) => {
                if cur.is_empty() {
                    cur.push(a);
                }
                cur.push(cross_at(a, b));
                pieces.push(std::mem::take(&mut cur));
            }
            (false, true) => {
                cur = vec![cross_at(b, a), b];
            }
            (false, false) => {}
        }
    }
    if !cur.is_empty() {
        pieces.push(cur);
    }
    pieces.into_iter().max_by(|a, b| a.len().cmp(&b.len())).unwrap_or_default()
}

fn trees(t: &Town, gh: &dyn Fn(Vec2) -> f32, seed: u64) -> Model {
    let mut mb = Mb::new();
    let bark = WHITE;
    for tr in &t.trees {
        if !tr.pos.is_finite() || !tr.radius.is_finite() || tr.radius <= 0.1 {
            continue;
        }
        let r = tr.radius.min(15.0);
        let mut rng = Rng::new(mix_seed(seed, (tr.pos.x.to_bits() as u64) << 32 | tr.pos.y.to_bits() as u64));
        let y = gh(tr.pos);
        let base = w3(tr.pos, y - 0.2);
        let tint = match rng.index(5) {
            0 => lin(0.82, 0.9, 0.7),
            1 => lin(1.0, 1.02, 0.85),
            2 => lin(0.9, 0.95, 0.88),
            3 => lin(1.05, 0.98, 0.7),
            _ => WHITE,
        };
        let tint = jitter(tint, &mut rng, 0.06);
        if rng.chance(0.22) {
            // Conifer: trunk and stacked cones.
            let h = (r * 2.6).clamp(3.0, 18.0);
            frustum(mb.m(S::Bark), base, Vec3::Y, r * 0.12, r * 0.08, h * 0.35, 6, bark, false, (false, false));
            let fol = crate::palette::mul(tint, lin(0.7, 0.82, 0.78));
            let tiers = 3;
            for k in 0..tiers {
                let f = k as f32 / tiers as f32;
                let y0 = base.y + h * (0.22 + 0.24 * f);
                let rr = r * (1.0 - 0.28 * f);
                frustum(mb.m(S::Foliage), Vec3::new(base.x, y0, base.z), Vec3::Y, rr, 0.0, h * 0.45, 9, scale(fol, 0.9 + 0.08 * f), true, (true, false));
            }
        } else {
            let th = (r * 0.9).clamp(1.2, 5.0);
            frustum(mb.m(S::Bark), base, Vec3::Y, (r * 0.13).clamp(0.1, 0.6), (r * 0.09).clamp(0.07, 0.4), th + 0.4, 6, bark, false, (false, true));
            let blobs = 1 + rng.index(3);
            let mf = mb.m(S::Foliage);
            let main_y = base.y + th + r * 0.55;
            blob(mf, Vec3::new(base.x, main_y, base.z), Vec3::new(r * 0.92, r * 0.78, r * 0.92), 1, tint, &mut rng);
            for _ in 1..blobs {
                let a = rng.range_f32(0.0, std::f32::consts::TAU);
                let off = Vec2::from_angle(a) * r * rng.range_f32(0.35, 0.6);
                let rr = r * rng.range_f32(0.5, 0.7);
                let c = Vec3::new(base.x + off.x, main_y + rng.range_f32(-0.1, 0.45) * r, base.z - off.y);
                blob(mf, c, Vec3::new(rr, rr * 0.85, rr), 1, scale(tint, rng.range_f32(0.9, 1.06)), &mut rng);
            }
        }
    }
    mb.into_model("trees")
}
