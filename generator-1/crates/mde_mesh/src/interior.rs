//! Floor slabs, ceiling joists, stairs and furniture.

use mde_core::{MaterialSlot as S, MeshData, Polygon, Prop, PropKind, Rng, Stair, Vec2, Vec3};

use crate::geom::{cuboid, faces, frustum, jitter, poly_facing, quad_facing, w3, wdir, Basis, Mb};
use crate::palette::{fixed, lin, mix, scale, Linear, WHITE};
use crate::triangulate::triangulate_with_holes;

/// Floor slab: `outline` minus `holes`, top at `top`, `thickness` deep. Top in
/// `top_slot`, edges and underside in `under_slot`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn slab(
    mb: &mut Mb,
    outline: &[Vec2],
    holes: &[Vec<Vec2>],
    top: f32,
    thickness: f32,
    top_slot: S,
    top_col: Linear,
    under_slot: S,
    under_col: Linear,
    underside: bool,
) {
    let (pts, tris) = triangulate_with_holes(outline, holes);
    if tris.is_empty() {
        return;
    }
    let bottom = top - thickness;
    {
        let m = mb.m(top_slot);
        let base = m.positions.len() as u32;
        for &p in &pts {
            let w = w3(p, top);
            crate::geom::vert(m, w, Vec3::Y, [w.x, w.z], top_col);
        }
        for t in &tris {
            m.indices.extend([base + t[0] as u32, base + t[1] as u32, base + t[2] as u32]);
        }
    }
    let m = mb.m(under_slot);
    if underside {
        let base = m.positions.len() as u32;
        for &p in &pts {
            let w = w3(p, bottom);
            crate::geom::vert(m, w, -Vec3::Y, [w.x, w.z], under_col);
        }
        for t in &tris {
            m.indices.extend([base + t[0] as u32, base + t[2] as u32, base + t[1] as u32]);
        }
    }
    // Edges: outer ring faces out, hole rings face into the hole.
    let mut rings: Vec<(Vec<Vec2>, bool)> = vec![(outline.to_vec(), true)];
    for h in holes {
        rings.push((h.clone(), false));
    }
    for (ring, outer) in rings {
        let n = ring.len();
        if n < 3 {
            continue;
        }
        let ccw = crate::geom::signed_area(&ring) >= 0.0;
        for i in 0..n {
            let a = ring[i];
            let b = ring[(i + 1) % n];
            if a.distance(b) < 1e-4 {
                continue;
            }
            let d = (b - a).normalize();
            let right = Vec2::new(d.y, -d.x);
            // Outward of a CCW ring is to the right; flip for holes.
            let out = if ccw == outer { right } else { -right };
            quad_facing(m, w3(a, bottom), w3(b, bottom), w3(b, top), w3(a, top), wdir(out), under_col);
        }
    }
}

/// Intervals of the infinite line `p + d·t` inside polygon `poly` (even-odd).
pub(crate) fn line_inside(poly: &[Vec2], p: Vec2, d: Vec2) -> Vec<(f32, f32)> {
    let n = poly.len();
    let mut ts = Vec::new();
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        let e = b - a;
        let den = mde_core::poly::cross(d, e);
        if den.abs() < 1e-9 {
            continue;
        }
        let t = mde_core::poly::cross(a - p, e) / den;
        let u = mde_core::poly::cross(a - p, d) / den;
        if (0.0..1.0).contains(&u) {
            ts.push(t);
        }
    }
    ts.sort_by(f32::total_cmp);
    ts.as_chunks::<2>().0.iter().map(|c| (c[0], c[1])).filter(|c| c.1 - c.0 > 1e-3).collect()
}

/// Ceiling joists under a slab whose underside is at `y`: beams across the
/// short axis of the outline, skipping holes.
pub(crate) fn joists(m: &mut MeshData, outline: &Polygon, holes: &[Vec<Vec2>], y: f32, col: Linear, rng: &mut Rng) {
    if outline.len() < 3 {
        return;
    }
    let i = outline.longest_edge();
    let along = (outline.at(i as isize + 1) - outline.at(i as isize)).normalize_or(Vec2::X);
    let across = Vec2::new(-along.y, along.x);
    let pts = &outline.0;
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut clo, mut chi) = (f32::INFINITY, f32::NEG_INFINITY);
    for &p in pts {
        lo = lo.min(p.dot(along));
        hi = hi.max(p.dot(along));
        clo = clo.min(p.dot(across));
        chi = chi.max(p.dot(across));
    }
    // Joists span the shorter direction.
    let (run, span_dir, rlo, rhi) = if hi - lo >= chi - clo { (along, across, lo, hi) } else { (across, along, clo, chi) };
    let spacing = 1.1;
    let count = ((rhi - rlo) / spacing).floor() as i32;
    for k in 1..=count {
        let s = rlo + (rhi - rlo) * k as f32 / (count + 1) as f32;
        let origin = run * s;
        let mut spans = line_inside(pts, origin, span_dir);
        for h in holes {
            let hs = line_inside(h, origin, span_dir);
            for (a, b) in hs {
                spans = crate::walls::cut_runs(&spans, a - 0.05, b + 0.05);
            }
        }
        for (a, b) in spans {
            let (a, b) = (a + 0.05, b - 0.05);
            if b - a < 0.4 {
                continue;
            }
            let p0 = w3(origin + span_dir * a, y - 0.1);
            let p1 = w3(origin + span_dir * b, y - 0.1);
            crate::geom::beam(m, p0, p1, Vec3::Y, 0.16, 0.2, jitter(col, rng, 0.06), faces::PY);
        }
    }
}

/// A straight flight of solid timber steps rising `rise` metres along `stair.up`.
pub(crate) fn stair_flight(mb: &mut Mb, stair: &Stair, y0: f32, rise: f32, col: Linear, rng: &mut Rng) {
    let up = stair.up.normalize_or(Vec2::Y);
    let side = Vec2::new(-up.y, up.x);
    let pts = &stair.footprint.0;
    if pts.len() < 3 || !rise.is_finite() || rise.abs() < 0.1 {
        return;
    }
    let (mut l0, mut l1, mut s0, mut s1) = (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY);
    for &p in pts {
        l0 = l0.min(p.dot(up));
        l1 = l1.max(p.dot(up));
        s0 = s0.min(p.dot(side));
        s1 = s1.max(p.dot(side));
    }
    let length = l1 - l0;
    if length < 0.3 || s1 - s0 < 0.3 {
        return;
    }
    let n = ((rise.abs() / 0.19).round() as usize).clamp(3, 40);
    let run = length / n as f32;
    let riser = rise / n as f32;
    let b = Basis { o: w3(Vec2::ZERO, 0.0), x: wdir(up), y: Vec3::Y, z: wdir(side) };
    let m = mb.m(S::Timber);
    for i in 0..n {
        let top = y0 + riser * (i + 1) as f32;
        let c = jitter(col, rng, 0.05);
        // Tread board (slightly proud) on a solid riser block.
        cuboid(m, &b, Vec3::new(l0 + run * i as f32, y0, s0 + 0.03), Vec3::new(l0 + run * (i + 1) as f32, top - 0.04, s1 - 0.03), scale(c, 0.8), faces::NY);
        cuboid(
            m,
            &b,
            Vec3::new(l0 + run * i as f32 - 0.03, top - 0.04, s0 + 0.02),
            Vec3::new(l0 + run * (i + 1) as f32, top, s1 - 0.02),
            c,
            faces::NY,
        );
    }
    // Handrail on the +side edge: posts at both ends and a sloped rail.
    let rail_h = 0.9;
    let rc = scale(col, 0.75);
    let sa = s1 - 0.06;
    let pa = b.at(l0 + run * 0.5, y0 + riser, sa);
    let pb = b.at(l1 - run * 0.5, y0 + rise, sa);
    for p in [pa, pb] {
        crate::geom::aabox(m, p - Vec3::new(0.04, 0.0, 0.04), p + Vec3::new(0.04, rail_h + 0.05, 0.04), rc);
    }
    crate::geom::beam(m, pa + Vec3::Y * rail_h, pb + Vec3::Y * rail_h, Vec3::Y, 0.06, 0.06, rc, 0);
}

/// Stylised furniture block for `prop` standing on a floor at `y`.
pub(crate) fn prop(mb: &mut Mb, p: &Prop, y: f32, timber: Linear, stone: Linear, rng: &mut Rng) {
    if !p.pos.is_finite() || !p.size.is_finite() || !p.rotation.is_finite() {
        return;
    }
    let w = p.size.x.clamp(0.2, 6.0);
    let d = p.size.y.clamp(0.2, 6.0);
    let ax = Vec2::from_angle(p.rotation);
    let ay = Vec2::new(-ax.y, ax.x);
    // Local frame: x across, y up, z towards the front (+y in plan).
    let b = Basis { o: w3(p.pos, y), x: wdir(ax), y: Vec3::Y, z: wdir(ay) };
    let (hw, hd) = (w * 0.5, d * 0.5);
    // Build into a scratch mesh first so the finished prop can be fitted to
    // its footprint (see `fit_to_footprint`).
    let outer = mb;
    let mut local = Mb::new();
    let mb = &mut local;
    let wood = jitter(timber, rng, 0.08);
    let dark = scale(wood, 0.75);
    let bx = |m: &mut MeshData, x0: f32, x1: f32, y0: f32, y1: f32, z0: f32, z1: f32, c: Linear| {
        cuboid(m, &b, Vec3::new(x0, y0, z0), Vec3::new(x1, y1, z1), c, faces::NY);
    };
    let legs = |m: &mut MeshData, h: f32, inset: f32, t: f32, c: Linear| {
        for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let x = sx * (hw - inset);
            let z = sz * (hd - inset);
            cuboid(m, &b, Vec3::new(x - t, 0.0, z - t), Vec3::new(x + t, h, z + t), c, faces::NY);
        }
    };
    match p.kind {
        PropKind::Bed | PropKind::Cot => {
            let low = p.kind == PropKind::Cot;
            let fh = if low { 0.22 } else { 0.34 };
            let m = mb.m(S::Timber);
            bx(m, -hw, hw, 0.0, fh, -hd, hd, wood);
            if !low {
                bx(m, -hw - 0.02, hw + 0.02, 0.0, 0.95, -hd - 0.02, -hd + 0.07, dark);
                bx(m, -hw - 0.02, hw + 0.02, 0.0, 0.55, hd - 0.06, hd + 0.02, dark);
            }
            let ml = mb.m(S::Plaster);
            let top = fh + 0.13;
            bx(ml, -hw + 0.05, hw - 0.05, fh, top, -hd + 0.08, hd - 0.07, fixed::linen());
            bx(ml, -hw + 0.04, hw - 0.04, fh + 0.02, top + 0.035, -hd + d * 0.38, hd - 0.05, jitter(fixed::wool(), rng, 0.15));
            bx(ml, -hw + 0.12, hw - 0.12, top, top + 0.1, -hd + 0.1, -hd + 0.38, scale(fixed::linen(), 1.05));
        }
        PropKind::Table | PropKind::Desk => {
            let m = mb.m(S::Timber);
            bx(m, -hw, hw, 0.7, 0.76, -hd, hd, wood);
            legs(m, 0.7, 0.08, 0.04, dark);
            if p.kind == PropKind::Desk {
                bx(m, hw - w * 0.35, hw - 0.05, 0.35, 0.7, -hd + 0.05, hd - 0.05, dark);
                let mp = mb.m(S::Plaster);
                bx(mp, -hw * 0.5, -hw * 0.1, 0.76, 0.78, -hd * 0.3, hd * 0.3, fixed::linen());
            } else {
                // A jug and a bowl.
                let mc = mb.m(S::Stone);
                let c = mix(stone, lin(0.6, 0.38, 0.28), 0.6);
                frustum(mc, b.at(-hw * 0.4, 0.76, 0.0), Vec3::Y, 0.07, 0.05, 0.2, 6, c, false, (false, true));
                frustum(mc, b.at(hw * 0.3, 0.76, hd * 0.2), Vec3::Y, 0.06, 0.11, 0.07, 6, scale(c, 1.1), false, (false, true));
            }
        }
        PropKind::Chair => {
            let m = mb.m(S::Timber);
            bx(m, -hw, hw, 0.42, 0.47, -hd, hd, wood);
            legs(m, 0.42, 0.05, 0.03, dark);
            bx(m, -hw, hw, 0.47, 1.0, -hd, -hd + 0.06, dark);
        }
        PropKind::Bench => {
            let m = mb.m(S::Timber);
            bx(m, -hw, hw, 0.4, 0.46, -hd, hd, wood);
            for sx in [-1.0, 1.0] {
                let x = sx * (hw - 0.15);
                bx(m, x - 0.04, x + 0.04, 0.0, 0.4, -hd + 0.04, hd - 0.04, dark);
            }
        }
        PropKind::Hearth => {
            let m = mb.m(S::Stone);
            let c = jitter(stone, rng, 0.05);
            // Cheeks, back and lintel; dark fire-back; hood up to ~2.3 m.
            bx(m, -hw, -hw + 0.25, 0.0, 1.1, -hd + 0.2, hd, c);
            bx(m, hw - 0.25, hw, 0.0, 1.1, -hd + 0.2, hd, c);
            bx(m, -hw, hw, 0.0, 1.1, -hd, -hd + 0.2, c);
            bx(m, -hw, hw, 1.1, 1.35, -hd, hd + 0.05, mix(c, fixed::dressed_stone(), 0.5));
            bx(m, -hw + 0.1, hw - 0.1, 1.35, 2.3, -hd, hd * 0.2, scale(c, 0.95));
            bx(m, -hw + 0.25, hw - 0.25, 0.0, 0.08, -hd + 0.2, hd, scale(c, 0.8));
            bx(m, -hw + 0.25, hw - 0.25, 0.08, 1.1, -hd + 0.2, -hd + 0.22, fixed::soot());
            let me = mb.m(S::Plaster);
            bx(me, -0.2, 0.2, 0.08, 0.2, -0.1, 0.1, fixed::ember());
            let mt = mb.m(S::Timber);
            bx(mt, -hw - 0.05, hw + 0.05, 1.35, 1.45, -hd + 0.1, hd + 0.1, dark);
        }
        PropKind::Stove => {
            let m = mb.m(S::Stone);
            let c = mix(stone, fixed::brick(), 0.5);
            bx(m, -hw, hw, 0.0, 0.85, -hd, hd, jitter(c, rng, 0.05));
            let mi = mb.m(S::Iron);
            bx(mi, -hw + 0.03, hw - 0.03, 0.85, 0.9, -hd + 0.03, hd - 0.03, WHITE);
            bx(mi, -0.18, 0.18, 0.2, 0.5, hd, hd + 0.02, WHITE);
            frustum(mi, b.at(0.0, 0.9, -hd * 0.5), Vec3::Y, 0.08, 0.08, 1.6, 6, WHITE, false, (false, false));
        }
        PropKind::Counter => {
            let m = mb.m(S::Timber);
            bx(m, -hw, hw, 0.0, 0.95, -hd + 0.05, hd - 0.05, dark);
            bx(m, -hw - 0.03, hw + 0.03, 0.95, 1.02, -hd, hd, wood);
            for k in 0..3 {
                let x = -hw + w * (k as f32 + 0.5) / 3.0;
                bx(m, x - 0.25, x + 0.25, 0.15, 0.8, hd - 0.05, hd - 0.02, scale(wood, 0.9));
            }
        }
        PropKind::Shelf | PropKind::Bookshelf => {
            let books = p.kind == PropKind::Bookshelf;
            let h = if books { 2.0 } else { 1.7 };
            let m = mb.m(S::Timber);
            bx(m, -hw, -hw + 0.04, 0.0, h, -hd, hd, dark);
            bx(m, hw - 0.04, hw, 0.0, h, -hd, hd, dark);
            bx(m, -hw + 0.04, hw - 0.04, 0.0, h, -hd, -hd + 0.03, scale(dark, 0.9));
            let rows = if books { 5 } else { 4 };
            for r in 0..=rows {
                let yy = h * r as f32 / rows as f32;
                bx(m, -hw + 0.04, hw - 0.04, yy.max(0.0) - 0.0, yy + 0.03, -hd + 0.03, hd, wood);
            }
            let palette = [lin(0.5, 0.16, 0.12), lin(0.2, 0.3, 0.2), lin(0.22, 0.25, 0.4), lin(0.5, 0.36, 0.2), lin(0.6, 0.55, 0.42)];
            for r in 0..rows {
                let yy = h * r as f32 / rows as f32 + 0.03;
                let mut x = -hw + 0.06;
                while x < hw - 0.12 {
                    let bw = rng.range_f32(0.08, 0.22);
                    let x1 = (x + bw).min(hw - 0.06);
                    if books {
                        let bh = (h / rows as f32 - 0.08) * rng.range_f32(0.7, 0.95);
                        let c = palette[rng.index(palette.len())];
                        bx(mb.m(S::Plaster), x, x1, yy, yy + bh, -hd + 0.06, hd - 0.04, c);
                    } else if rng.chance(0.5) {
                        let c = mix(stone, lin(0.62, 0.42, 0.3), 0.6);
                        frustum(mb.m(S::Stone), b.at((x + x1) * 0.5, yy, 0.0), Vec3::Y, 0.06, 0.045, 0.16, 6, c, false, (false, true));
                    }
                    x = x1 + 0.01;
                }
            }
        }
        PropKind::Wardrobe => {
            let m = mb.m(S::Timber);
            bx(m, -hw, hw, 0.0, 1.95, -hd, hd, wood);
            bx(m, -hw - 0.02, hw + 0.02, 1.95, 2.03, -hd - 0.02, hd + 0.03, dark);
            for sx in [-1.0f32, 1.0] {
                let x0 = if sx < 0.0 { -hw + 0.06 } else { 0.02 };
                let x1 = if sx < 0.0 { -0.02 } else { hw - 0.06 };
                bx(m, x0, x1, 0.12, 1.85, hd, hd + 0.02, scale(wood, 0.9));
            }
            let mi = mb.m(S::Iron);
            bx(mi, -0.06, -0.03, 1.0, 1.12, hd + 0.02, hd + 0.04, WHITE);
            bx(mi, 0.03, 0.06, 1.0, 1.12, hd + 0.02, hd + 0.04, WHITE);
        }
        PropKind::Chest => {
            let m = mb.m(S::Timber);
            bx(m, -hw, hw, 0.0, 0.42, -hd, hd, wood);
            bx(m, -hw - 0.02, hw + 0.02, 0.42, 0.52, -hd - 0.02, hd + 0.02, scale(wood, 1.05));
            let mi = mb.m(S::Iron);
            for sx in [-0.65f32, 0.65] {
                let x = sx * hw;
                bx(mi, x - 0.03, x + 0.03, 0.0, 0.53, -hd - 0.025, hd + 0.025, WHITE);
            }
            bx(mi, -0.05, 0.05, 0.3, 0.45, hd + 0.02, hd + 0.04, WHITE);
        }
        PropKind::Barrel => {
            let r = hw.min(hd).clamp(0.2, 0.6);
            let h = (r * 2.4).clamp(0.6, 1.2);
            let m = mb.m(S::Timber);
            let base = b.at(0.0, 0.0, 0.0);
            frustum(m, base, Vec3::Y, r * 0.85, r, h * 0.5, 10, wood, false, (false, false));
            frustum(m, base + Vec3::Y * (h * 0.5), Vec3::Y, r, r * 0.85, h * 0.5, 10, wood, false, (false, true));
            let mi = mb.m(S::Iron);
            for (yy, rr) in [(0.08, r * 0.87), (h * 0.5 - 0.04, r * 1.01), (h - 0.12, r * 0.88)] {
                frustum(mi, base + Vec3::Y * yy, Vec3::Y, rr + 0.012, rr + 0.012, 0.05, 10, WHITE, false, (false, false));
            }
        }
        PropKind::Crate => {
            let m = mb.m(S::Timber);
            let h = hw.min(hd).clamp(0.3, 0.8) * 2.0 * 0.8;
            bx(m, -hw, hw, 0.0, h, -hd, hd, wood);
            for (z0, z1) in [(-hd - 0.015, -hd), (hd, hd + 0.015)] {
                bx(m, -hw, -hw + 0.06, 0.0, h, z0, z1, dark);
                bx(m, hw - 0.06, hw, 0.0, h, z0, z1, dark);
                // Mid batten between the corner battens, a little thinner.
                let (m0, m1) = if z0 < 0.0 { (z0 + 0.003, z1) } else { (z0, z1 - 0.003) };
                bx(m, -hw + 0.06, hw - 0.06, h * 0.45, h * 0.55, m0, m1, dark);
            }
        }
        PropKind::Bathtub => {
            let m = mb.m(S::Timber);
            let h = 0.55;
            bx(m, -hw + 0.07, hw - 0.07, 0.0, 0.08, -hd + 0.07, hd - 0.07, dark);
            bx(m, -hw, -hw + 0.07, 0.0, h, -hd, hd, wood);
            bx(m, hw - 0.07, hw, 0.0, h, -hd, hd, wood);
            bx(m, -hw + 0.07, hw - 0.07, 0.0, h, -hd, -hd + 0.07, wood);
            bx(m, -hw + 0.07, hw - 0.07, 0.0, h, hd - 0.07, hd, wood);
            let mi = mb.m(S::Iron);
            for yy in [0.12, h - 0.1] {
                bx(mi, -hw - 0.01, hw + 0.01, yy, yy + 0.04, -hd - 0.01, hd + 0.01, WHITE);
            }
            let mw = mb.m(S::Water);
            let q = [b.at(-hw + 0.07, h - 0.12, -hd + 0.07), b.at(hw - 0.07, h - 0.12, -hd + 0.07), b.at(hw - 0.07, h - 0.12, hd - 0.07), b.at(-hw + 0.07, h - 0.12, hd - 0.07)];
            poly_facing(mw, &q, Vec3::Y, WHITE);
        }
        PropKind::Altar => {
            let m = mb.m(S::Stone);
            let c = mix(stone, fixed::dressed_stone(), 0.6);
            bx(m, -hw, hw, 0.0, 0.15, -hd - 0.05, hd + 0.05, scale(c, 0.9));
            bx(m, -hw + 0.08, hw - 0.08, 0.15, 0.95, -hd + 0.05, hd - 0.05, c);
            bx(m, -hw, hw, 0.95, 1.05, -hd, hd, scale(c, 1.05));
            let mc = mb.m(S::Plaster);
            bx(mc, -hw * 0.45, hw * 0.45, 0.4, 1.06, hd - 0.03, hd - 0.01, fixed::wool());
            bx(mc, -hw * 0.45, hw * 0.45, 1.05, 1.07, -hd + 0.02, hd - 0.01, fixed::wool());
            for sx in [-0.6f32, 0.6] {
                bx(mc, sx * hw - 0.03, sx * hw + 0.03, 1.05, 1.3, -0.03, 0.03, fixed::linen());
                bx(mc, sx * hw - 0.015, sx * hw + 0.015, 1.3, 1.36, -0.015, 0.015, fixed::ember());
            }
        }
        PropKind::Anvil => {
            let m = mb.m(S::Timber);
            frustum(m, b.at(0.0, 0.0, 0.0), Vec3::Y, 0.26, 0.24, 0.45, 8, dark, false, (false, true));
            let mi = mb.m(S::Iron);
            bx(mi, -0.12, 0.12, 0.45, 0.6, -0.1, 0.1, WHITE);
            bx(mi, -0.3, 0.25, 0.6, 0.75, -0.1, 0.1, WHITE);
            bx(mi, 0.25, 0.4, 0.66, 0.74, -0.05, 0.05, WHITE);
        }
    }
    fit_to_footprint(outer, &local.into_model("prop"), &b, hw, hd);
}

/// Append `prop` (built in the frame `b`) to `mb`, squeezed along the local x
/// and z axes where it overhangs the footprint `±hw × ±hd`. Tops, boards and
/// battens overhang by a few centimetres; props stand against walls, so
/// without this they would poke into the wall and into its window holes.
fn fit_to_footprint(mb: &mut Mb, prop: &mde_core::Model, b: &Basis, hw: f32, hd: f32) {
    let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for v in prop.parts.iter().flat_map(|p| &p.mesh.positions) {
        let q = Vec3::from_array(*v) - b.o;
        let l = Vec2::new(q.dot(b.x), q.dot(b.z));
        lo = lo.min(l);
        hi = hi.max(l);
    }
    if !lo.is_finite() || !hi.is_finite() {
        return;
    }
    // Per axis: map [lo, hi] onto its overlap with the footprint.
    let fit = |lo: f32, hi: f32, h: f32| -> (f32, f32) {
        let (t0, t1) = (lo.max(-h), hi.min(h));
        if (lo >= -h && hi <= h) || t1 <= t0 {
            return (1.0, 0.0);
        }
        let s = (t1 - t0) / (hi - lo);
        (s, t0 - lo * s)
    };
    let (sx, ox) = fit(lo.x, hi.x, hw);
    let (sz, oz) = fit(lo.y, hi.y, hd);
    if sx == 1.0 && sz == 1.0 {
        mb.append_model(prop);
        return;
    }
    let (o, bx, bz) = (b.o, b.x, b.z);
    mb.append_transformed(
        prop,
        |p| {
            let q = p - o;
            let (x, z) = (q.dot(bx), q.dot(bz));
            p + bx * (x * sx + ox - x) + bz * (z * sz + oz - z)
        },
        // Normals take the inverse scale.
        |n| {
            let (x, z) = (n.dot(bx), n.dot(bz));
            n + bx * (x / sx - x) + bz * (z / sz - z)
        },
    );
}
