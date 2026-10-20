//! Wooden trestle and stone arch bridges of any length, width and rise.
//!
//! The deck follows a cosine hump from `a` to `b`: it leaves the ground at both
//! ends with zero slope (so the ends are their own ramps, exactly at `y = 0`)
//! and peaks at mid-span, plus an 18 cm lift that keeps even a flat deck off the
//! ground. The rise is limited to 18 % of the span so short bridges stay
//! walkable. [`bridge_deck_height`] gives the same profile for
//! placing walkers on the deck.

use mde_core::{Bridge, BridgeKind, MaterialSlot as S, MeshData, Model, Rng, Vec2, Vec3};

use crate::geom::{beam, faces, jitter, mix_seed, poly_facing, quad_facing, w3, Basis, Mb};
use crate::palette::{fixed, lin, mix, scale, Linear, WHITE};

/// Depth of river beds / footings below ground that piers and trestles reach.
const BED: f32 = -2.6;
/// Height of the deck above the ground away from the ends.
const DECK_LIFT: f32 = 0.18;
/// Length over which the deck rises to [`DECK_LIFT`] at each end.
const DECK_RAMP: f32 = 1.6;

/// Sanitised bridge geometry.
struct Span {
    a: Vec2,
    d: Vec2,
    n: Vec2,
    len: f32,
    width: f32,
    rise: f32,
}

impl Span {
    fn new(b: &Bridge) -> Option<Span> {
        if !b.a.is_finite() || !b.b.is_finite() {
            return None;
        }
        let len = b.a.distance(b.b);
        if len < 0.5 {
            return None;
        }
        let d = (b.b - b.a) / len;
        let width = if b.width.is_finite() { b.width.clamp(0.8, 16.0) } else { 3.0 };
        let h = if b.height.is_finite() { b.height.max(0.0) } else { 0.0 };
        Some(Span { a: b.a, d, n: Vec2::new(-d.y, d.x), len, width, rise: h.min(0.18 * len).min(12.0) })
    }

    /// Deck top height at distance `s` from `a`: the cosine hump plus a small
    /// lift (reached within the first metres) that keeps a flat deck clear of
    /// the ground.
    fn y(&self, s: f32) -> f32 {
        let t = (s / self.len).clamp(0.0, 1.0);
        let hump = self.rise * (1.0 - (std::f32::consts::TAU * t).cos()) * 0.5;
        let r = (s.min(self.len - s) / DECK_RAMP).clamp(0.0, 1.0);
        hump + DECK_LIFT * r * r * (3.0 - 2.0 * r)
    }

    /// World point at distance `s` along, `c` across (left positive), height `y`.
    fn p(&self, s: f32, c: f32, y: f32) -> Vec3 {
        w3(self.a + self.d * s + self.n * c, y)
    }

    fn along3(&self) -> Vec3 {
        crate::geom::wdir(self.d)
    }

    fn side3(&self) -> Vec3 {
        crate::geom::wdir(self.n)
    }
}

/// Height of the deck surface above the ground at fraction `t` (0 at `a`, 1 at
/// `b`) along the bridge. 0 at both ends; peaks at `t = 0.5`.
pub fn bridge_deck_height(b: &Bridge, t: f32) -> f32 {
    match Span::new(b) {
        Some(s) if t.is_finite() => s.y(t.clamp(0.0, 1.0) * s.len),
        _ => 0.0,
    }
}

/// Build a bridge (world space). Wood: plank deck on stringers and trestle
/// bents every ~3 m, with posts and rails. Stone: masonry arches sized to the
/// span with piers and cutwaters, parapets and a cobbled deck. Both meet the
/// ground (`y = 0`) at `a` and `b`. Degenerate input gives an empty model.
pub fn build_bridge(b: &Bridge) -> Model {
    let mut mb = Mb::new();
    if let Some(sp) = Span::new(b) {
        let seed = mix_seed((sp.a.x.to_bits() as u64) << 32 | sp.a.y.to_bits() as u64, sp.len.to_bits() as u64);
        let mut rng = Rng::new(seed);
        match b.kind {
            BridgeKind::Wood => wood(&mut mb, &sp, &mut rng),
            BridgeKind::Stone => stone(&mut mb, &sp, &mut rng),
        }
    }
    mb.into_model("bridge")
}

/// Hexahedron from 8 corners: bottom quad `b0..b3` and top `t0..t3`, both in
/// the same order. Faces whose bit is in `skip` (`NY` bottom, `PY` top, `NX`
/// start, `PX` end, `NZ` / `PZ` sides) are omitted.
fn hexa(m: &mut MeshData, b: [Vec3; 4], t: [Vec3; 4], col: Linear, skip: u8) {
    let c = (b.iter().copied().sum::<Vec3>() + t.iter().copied().sum::<Vec3>()) / 8.0;
    let mut f = |bit: u8, q: [Vec3; 4], shade: f32| {
        if skip & bit == 0 {
            let mid = q.iter().copied().sum::<Vec3>() / 4.0;
            quad_facing(m, q[0], q[1], q[2], q[3], mid - c, scale(col, shade));
        }
    };
    f(faces::NY, [b[0], b[1], b[2], b[3]], 0.8);
    f(faces::PY, [t[0], t[1], t[2], t[3]], 1.0);
    f(faces::NX, [b[0], b[3], t[3], t[0]], 0.9);
    f(faces::PX, [b[1], b[2], t[2], t[1]], 0.9);
    f(faces::NZ, [b[0], b[1], t[1], t[0]], 0.92);
    f(faces::PZ, [b[3], b[2], t[2], t[3]], 0.92);
}

fn wood(mb: &mut Mb, sp: &Span, rng: &mut Rng) {
    let plank_col = lin(0.55, 0.45, 0.35);
    let timber_col = lin(0.42, 0.32, 0.24);
    let hw = sp.width * 0.5;
    let pt = 0.06;
    // Planks across the deck.
    let pw = (sp.len / 140.0).max(0.22);
    let n_planks = ((sp.len / pw).round() as usize).max(2);
    let step = sp.len / n_planks as f32;
    for i in 0..n_planks {
        let s0 = step * i as f32 + 0.006;
        let s1 = step * (i + 1) as f32 - 0.006;
        let (y0, y1) = (sp.y(s0), sp.y(s1));
        let el = hw + rng.range_f32(0.0, 0.06);
        let er = hw + rng.range_f32(0.0, 0.06);
        let c = jitter(plank_col, rng, 0.1);
        let t = [sp.p(s0, -er, y0), sp.p(s1, -er, y1), sp.p(s1, el, y1), sp.p(s0, el, y0)];
        let bt = [sp.p(s0, -er, y0 - pt), sp.p(s1, -er, y1 - pt), sp.p(s1, el, y1 - pt), sp.p(s0, el, y0 - pt)];
        hexa(mb.m(S::Planks), bt, t, c, faces::NZ | faces::PZ);
    }
    // Stringers.
    let n_str = if sp.width > 4.0 { 4 } else { 3 };
    let seg = ((sp.len / 2.5).ceil() as usize).max(1);
    let deck_under = |s: f32| sp.y(s) - pt;
    for k in 0..n_str {
        let c = -hw + 0.25 + (sp.width - 0.5) * k as f32 / (n_str - 1) as f32;
        let col = jitter(timber_col, rng, 0.06);
        let path: Vec<Vec3> = (0..=seg)
            .map(|j| {
                let s = sp.len * j as f32 / seg as f32;
                sp.p(s, c, deck_under(s) - 0.12)
            })
            .collect();
        crate::geom::sweep_box(mb.m(S::Timber), &path, 0.16, 0.24, col);
    }
    // Trestle bents.
    let n_bents = ((sp.len / 3.0).round() as i32 - 1).max(0) as usize;
    for k in 1..=n_bents {
        let s = sp.len * k as f32 / (n_bents + 1) as f32;
        let top = deck_under(s) - 0.24;
        let col = jitter(timber_col, rng, 0.07);
        let m = mb.m(S::Timber);
        // Cap beam.
        beam(m, sp.p(s, -hw - 0.15, top - 0.1), sp.p(s, hw + 0.15, top - 0.1), Vec3::Y, 0.22, 0.2, col, 0);
        let mut posts = vec![-hw + 0.15, hw - 0.15];
        if sp.width > 4.0 {
            posts.push(0.0);
        }
        for &c in &posts {
            let base = sp.p(s, c, BED);
            crate::geom::cuboid(
                m,
                &Basis { o: base, x: sp.along3(), y: Vec3::Y, z: sp.side3() },
                Vec3::new(-0.11, 0.0, -0.11),
                Vec3::new(0.11, top - 0.2 - BED, 0.11),
                scale(col, 0.95),
                faces::NY,
            );
        }
        // Cross bracing above the bed.
        let brace_lo = (BED + 0.8).max(top - 3.0);
        if top - 0.2 - brace_lo > 0.8 {
            let (l, r) = (-hw + 0.15, hw - 0.15);
            // The two diagonals sit side by side, never in one plane.
            for (c0, c1, ds) in [(l, r, 0.12), (r, l, 0.19)] {
                beam(m, sp.p(s + ds, c0, brace_lo), sp.p(s + ds, c1, top - 0.3), sp.along3(), 0.12, 0.06, scale(col, 0.9), 0);
            }
        }
    }
    // Railings.
    let n_posts = ((sp.len / 1.8).round() as usize).max(1);
    for side in [-1.0f32, 1.0] {
        let c = side * (hw - 0.06);
        let col = jitter(timber_col, rng, 0.05);
        let m = mb.m(S::Timber);
        let mut prev: Option<(f32, f32)> = None;
        for k in 0..=n_posts {
            let s = (sp.len * k as f32 / n_posts as f32).clamp(0.1, sp.len - 0.1);
            let y = sp.y(s);
            let end = k == 0 || k == n_posts;
            let (r, h) = if end { (0.11, 1.2) } else { (0.07, 1.0) };
            crate::geom::cuboid(
                m,
                &Basis { o: sp.p(s, c, y - pt), x: sp.along3(), y: Vec3::Y, z: sp.side3() },
                Vec3::new(-r, 0.0, -r),
                Vec3::new(r, h + pt, r),
                if end { scale(col, 0.9) } else { col },
                faces::NY,
            );
            if let Some((ps, py)) = prev {
                for (hgt, sz) in [(0.98, 0.09), (0.5, 0.06)] {
                    beam(m, sp.p(ps, c, py + hgt), sp.p(s, c, y + hgt), Vec3::Y, sz, sz, col, 0);
                }
            }
            prev = Some((s, y));
        }
    }
}

fn stone(mb: &mut Mb, sp: &Span, rng: &mut Rng) {
    let hw = sp.width * 0.5;
    let pt = 0.35; // parapet thickness
    let ph = 0.85; // parapet height above the deck
    let len = sp.len;
    let abut = (0.12 * len).clamp(0.4, 4.0);
    let inner = (len - 2.0 * abut).max(0.4);
    let n_arch = ((inner / 9.0).round() as usize).max(1);
    let pier = if n_arch > 1 { (0.14 * inner / n_arch as f32).clamp(0.8, 2.4) } else { 0.0 };
    let span = (inner - pier * (n_arch - 1) as f32) / n_arch as f32;
    // Arches: (start, end, centre_y, radius, springing_y).
    let mut arches: Vec<(f32, f32, f32, f32, f32)> = Vec::new();
    for k in 0..n_arch {
        let s0 = abut + k as f32 * (span + pier);
        let s1 = s0 + span;
        let mid = (s0 + s1) * 0.5;
        let crown = sp.y(mid) - (0.4 + 0.035 * span);
        let rise = (span * 0.5).min((crown - BED - 0.3).max(0.3)).max(0.2);
        let rho = (span * span * 0.25 + rise * rise) / (2.0 * rise);
        let cy = crown - rho;
        arches.push((s0, s1, cy, rho, crown - rise));
    }
    let intrados = |s: f32| -> Option<f32> {
        for &(s0, s1, cy, rho, _) in &arches {
            if s > s0 + 1e-4 && s < s1 - 1e-4 {
                let mid = (s0 + s1) * 0.5;
                return Some(cy + (rho * rho - (s - mid) * (s - mid)).max(0.0).sqrt());
            }
        }
        None
    };
    // Sample positions: a grid plus exact arch ends.
    let mut ss: Vec<f32> = Vec::new();
    let n = ((len / 0.6).ceil() as usize).max(2);
    for i in 0..=n {
        ss.push(len * i as f32 / n as f32);
    }
    for &(s0, s1, ..) in &arches {
        let k = 8;
        for j in 0..=k {
            ss.push(s0 + (s1 - s0) * j as f32 / k as f32);
        }
    }
    ss.sort_by(f32::total_cmp);
    ss.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
    let stone_col = jitter(WHITE, rng, 0.04);
    let deck_col = mix(WHITE, lin(0.62, 0.58, 0.52), 0.5);
    let dressed = mix(WHITE, fixed::dressed_stone(), 0.7);

    // Side (spandrel) walls up to the parapet top, with arch openings.
    for side in [-1.0f32, 1.0] {
        let c = side * hw;
        let out = sp.side3() * side;
        let m = mb.m(S::Stone);
        for w in ss.windows(2) {
            let (s0, s1) = (w[0], w[1]);
            let sm = (s0 + s1) * 0.5;
            let (b0, b1) = match intrados(sm) {
                Some(_) => (intrados(s0 + 1e-3).unwrap_or(BED), intrados(s1 - 1e-3).unwrap_or(BED)),
                None => (BED, BED),
            };
            let (t0, t1) = (sp.y(s0) + ph, sp.y(s1) + ph);
            let tone = jitter(stone_col, rng, 0.03);
            quad_facing(m, sp.p(s0, c, b0), sp.p(s1, c, b1), sp.p(s1, c, t1), sp.p(s0, c, t0), out, tone);
        }
        // Inner parapet face, coping.
        for w in ss.windows(2) {
            let (s0, s1) = (w[0], w[1]);
            let (y0, y1) = (sp.y(s0), sp.y(s1));
            let ci = side * (hw - pt);
            quad_facing(m, sp.p(s0, ci, y0), sp.p(s1, ci, y1), sp.p(s1, ci, y1 + ph), sp.p(s0, ci, y0 + ph), -out, scale(stone_col, 0.95));
            // Coping: proud on the outside (with a chamfer), flush inside.
            let co = side * (hw + 0.05);
            let (h0, h1) = (y0 + ph + 0.1, y1 + ph + 0.1);
            let cc = scale(dressed, 1.0);
            quad_facing(m, sp.p(s0, co, h0), sp.p(s1, co, h1), sp.p(s1, ci, h1), sp.p(s0, ci, h0), Vec3::Y, cc);
            quad_facing(m, sp.p(s0, co, h0 - 0.08), sp.p(s1, co, h1 - 0.08), sp.p(s1, co, h1), sp.p(s0, co, h0), out, scale(cc, 0.9));
            quad_facing(m, sp.p(s0, ci, y0 + ph), sp.p(s1, ci, y1 + ph), sp.p(s1, ci, h1), sp.p(s0, ci, h0), -out, scale(cc, 0.9));
            quad_facing(m, sp.p(s0, co, h0 - 0.08), sp.p(s1, co, h1 - 0.08), sp.p(s1, side * hw, y1 + ph), sp.p(s0, side * hw, y0 + ph), -Vec3::Y, scale(cc, 0.8));
        }
        // Parapet / coping end caps and newel blocks.
        for (s, dir) in [(0.0f32, -1.0f32), (len, 1.0)] {
            let ci = side * (hw - pt);
            let y = sp.y(s);
            quad_facing(m, sp.p(s, c, BED), sp.p(s, ci, BED), sp.p(s, ci, y + ph), sp.p(s, c, y + ph), sp.along3() * dir, stone_col);
            let co = side * (hw + 0.05);
            quad_facing(m, sp.p(s, co, y + ph - 0.02), sp.p(s, ci, y + ph - 0.02), sp.p(s, ci, y + ph + 0.1), sp.p(s, co, y + ph + 0.1), sp.along3() * dir, dressed);
            let sn = s - dir * 0.3;
            crate::geom::cuboid(
                m,
                &Basis { o: sp.p(sn, side * (hw - pt * 0.5), sp.y(sn)), x: sp.along3(), y: Vec3::Y, z: sp.side3() },
                Vec3::new(-0.32, 0.0, -0.3),
                Vec3::new(0.32, ph + 0.42, 0.3),
                jitter(dressed, rng, 0.04),
                faces::NY,
            );
        }
        // Voussoir ring around each arch on the outer face.
        for &(s0, s1, cy, _, _) in &arches {
            let mid = (s0 + s1) * 0.5;
            let k = 10;
            let ring_w = (0.3 + 0.03 * span).min(0.7);
            let pts: Vec<(f32, f32)> = (0..=k)
                .map(|j| {
                    let s = s0 + (s1 - s0) * j as f32 / k as f32;
                    let y = intrados(s.clamp(s0 + 2e-3, s1 - 2e-3)).unwrap_or(BED);
                    (s, y)
                })
                .collect();
            let off = |p: (f32, f32)| {
                let d = Vec2::new(p.0 - mid, p.1 - cy).normalize_or(Vec2::Y);
                (p.0 + d.x * ring_w, p.1 + d.y * ring_w)
            };
            let cf = c + side * 0.05;
            for j in 0..k {
                let (p0, p1) = (pts[j], pts[j + 1]);
                let (q0, q1) = (off(p0), off(p1));
                let col = if j % 2 == 0 { dressed } else { scale(dressed, 0.86) };
                quad_facing(m, sp.p(p0.0, cf, p0.1), sp.p(p1.0, cf, p1.1), sp.p(q1.0, cf, q1.1), sp.p(q0.0, cf, q0.1), out, col);
                quad_facing(m, sp.p(q0.0, c, q0.1), sp.p(q1.0, c, q1.1), sp.p(q1.0, cf, q1.1), sp.p(q0.0, cf, q0.1), Vec3::Y, col);
                quad_facing(m, sp.p(p0.0, c, p0.1), sp.p(p1.0, c, p1.1), sp.p(p1.0, cf, p1.1), sp.p(p0.0, cf, p0.1), -Vec3::Y, col);
            }
        }
    }
    // Vault soffits and pier faces.
    {
        let m = mb.m(S::Stone);
        for &(s0, s1, _, _, spring) in &arches {
            let k = 12;
            for j in 0..k {
                let a = s0 + (s1 - s0) * j as f32 / k as f32;
                let b = s0 + (s1 - s0) * (j + 1) as f32 / k as f32;
                let ya = intrados(a.clamp(s0 + 2e-3, s1 - 2e-3)).unwrap_or(spring);
                let yb = intrados(b.clamp(s0 + 2e-3, s1 - 2e-3)).unwrap_or(spring);
                let tone = scale(stone_col, if j % 2 == 0 { 0.82 } else { 0.78 });
                quad_facing(m, sp.p(a, -hw, ya), sp.p(b, -hw, yb), sp.p(b, hw, yb), sp.p(a, hw, ya), -Vec3::Y, tone);
            }
            // Pier / abutment faces below the springing.
            let ys0 = intrados(s0 + 2e-3).unwrap_or(spring);
            let ys1 = intrados(s1 - 2e-3).unwrap_or(spring);
            quad_facing(m, sp.p(s0, -hw, BED), sp.p(s0, hw, BED), sp.p(s0, hw, ys0), sp.p(s0, -hw, ys0), sp.along3(), scale(stone_col, 0.85));
            quad_facing(m, sp.p(s1, -hw, BED), sp.p(s1, hw, BED), sp.p(s1, hw, ys1), sp.p(s1, -hw, ys1), -sp.along3(), scale(stone_col, 0.85));
        }
        // Cutwaters on the piers, both sides.
        for k in 0..arches.len().saturating_sub(1) {
            let (s0, s1) = (arches[k].1, arches[k + 1].0);
            let top = arches[k].4.max(arches[k + 1].4) + 0.4;
            let mid = (s0 + s1) * 0.5;
            for side in [-1.0f32, 1.0] {
                let c = side * hw;
                let tip = side * (hw + (s1 - s0) * 0.6);
                let base = [sp.p(s0, c, BED), sp.p(mid, tip, BED), sp.p(s1, c, BED)];
                let tops = [sp.p(s0, c, top), sp.p(mid, tip, top), sp.p(s1, c, top)];
                let out = sp.side3() * side;
                for e in 0..2 {
                    let q = [base[e], base[e + 1], tops[e + 1], tops[e]];
                    let facing = (q[0] + q[1]) * 0.5 - sp.p(mid, c, BED) + out * 0.01;
                    quad_facing(m, q[0], q[1], q[2], q[3], facing, scale(stone_col, 0.92));
                }
                let apex = sp.p(mid, c, top + (s1 - s0) * 0.45);
                for e in 0..2 {
                    let tri = [tops[e], tops[e + 1], apex];
                    let facing = (tops[e] + tops[e + 1]) * 0.5 - sp.p(mid, c, top) + Vec3::Y * 0.3;
                    poly_facing(m, &tri, facing, scale(dressed, 0.9));
                }
            }
        }
    }
    // Cobbled deck: a grid of setts with colour variation.
    {
        let m = mb.m(S::Stone);
        let wd = sp.width - 2.0 * pt;
        let nc = ((wd / 0.5).round() as usize).clamp(1, 8);
        let rows = ((len / 0.65).ceil() as usize).max(2);
        let deck_s: Vec<f32> = (0..=rows).map(|i| len * i as f32 / rows as f32).collect();
        for w in deck_s.windows(2) {
            let (s0, s1) = (w[0], w[1]);
            let (y0, y1) = (sp.y(s0), sp.y(s1));
            for k in 0..nc {
                let c0 = -hw + pt + wd * k as f32 / nc as f32;
                let c1 = -hw + pt + wd * (k + 1) as f32 / nc as f32;
                let col = jitter(deck_col, rng, 0.1);
                quad_facing(m, sp.p(s0, c0, y0), sp.p(s1, c0, y1), sp.p(s1, c1, y1), sp.p(s0, c1, y0), Vec3::Y, col);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_ends_at_ground() {
        let b = Bridge { kind: BridgeKind::Wood, a: Vec2::ZERO, b: Vec2::new(20.0, 0.0), width: 3.0, height: 2.0 };
        assert_eq!(bridge_deck_height(&b, 0.0), 0.0);
        assert!(bridge_deck_height(&b, 1.0).abs() < 1e-5);
        assert!((bridge_deck_height(&b, 0.5) - 2.0 - DECK_LIFT).abs() < 1e-5);
    }
}
