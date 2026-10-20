//! Farmsteads and long strip fields for farm cells.

use mde_core::{BuildingKind, Rng, Vec2};

use crate::geom::{clip_half, convex_overlap, min_width, oriented_rect, outward, signed_area};
use crate::lots::{CUT, Piece, inside_by};

/// Buildings and fields produced for one farm cell.
#[derive(Clone, Debug, Default)]
pub(crate) struct FarmOut {
    pub fields: Vec<Vec<Vec2>>,
    /// `(footprint, front, kind)`.
    pub buildings: Vec<(Vec<Vec2>, Vec2, BuildingKind)>,
}

/// Lay out a farm on `block`. If `front_tag` names a block edge (normally the
/// road side) and `farmstead` is set, a yard with a farmhouse, a barn and
/// optionally a mill is placed along it; the rest becomes strip fields.
pub(crate) fn farm(block: &Piece, front_tag: Option<i32>, farmstead: bool, mill: bool, rng: &mut Rng) -> FarmOut {
    let mut out = FarmOut::default();
    let mut region = block.pts.clone();
    if farmstead
        && let Some(tag) = front_tag
        && let Some((a, b)) = block.tagged_edge(tag)
        && a.distance(b) >= 14.0
    {
        let len = a.distance(b);
        let u = (b - a) / len;
        let out_n = outward(a, b);
        let inward = -out_n;
        let depth = rng.range_f32(26.0, 34.0);
        let yard = clip_half(&block.pts, a - out_n * depth, out_n);
        let rest = block.clip(a - out_n * (depth + 3.0), inward, CUT);
        let mut placed: Vec<Vec<Vec2>> = Vec::new();
        let mut try_place = |centre: Vec2, w: f32, d: f32, kind: BuildingKind, out: &mut FarmOut| {
            let r = oriented_rect(centre, u, w, d);
            if yard.len() >= 3
                && r.iter().all(|&p| inside_by(&yard, p, 0.3))
                && placed.iter().all(|o| !convex_overlap(&r, o, -2.0))
            {
                placed.push(r.clone());
                out.buildings.push((r, out_n, kind));
            }
        };
        let (fw, fd) = (rng.range_f32(9.0, 12.5), rng.range_f32(6.5, 8.5));
        let t1 = if len < 30.0 { 0.5 } else { rng.range_f32(0.25, 0.4) };
        try_place(a + u * (len * t1) + inward * (fd * 0.5 + 1.0), fw, fd, BuildingKind::Farmhouse, &mut out);
        if len >= 34.0 {
            let (bw, bd) = (rng.range_f32(12.0, 17.0), rng.range_f32(8.0, 10.5));
            let t2 = rng.range_f32(0.62, 0.78);
            try_place(a + u * (len * t2) + inward * (bd * 0.5 + 1.2), bw, bd, BuildingKind::Barn, &mut out);
            if rng.chance(0.35) {
                let (sw, sd) = (rng.range_f32(6.0, 8.0), rng.range_f32(4.5, 6.0));
                try_place(a + u * (len * t1) + inward * (fd + sd * 0.5 + 5.0), sw, sd, BuildingKind::Barn, &mut out);
            }
        }
        if mill && len >= 24.0 {
            // A mill on the road at the end of the frontage, else in the yard.
            let s = rng.range_f32(6.0, 7.5);
            let before = out.buildings.len();
            for t in [0.1f32, 0.9] {
                if out.buildings.len() == before {
                    try_place(a + u * (len * t) + inward * (s * 0.5 + 1.0), s, s, BuildingKind::Mill, &mut out);
                }
            }
            if out.buildings.len() == before {
                try_place(a + u * (len * 0.52) + inward * (depth - s * 0.5 - 2.0), s, s, BuildingKind::Mill, &mut out);
            }
        }
        region = rest.pts;
    }
    if region.len() < 3 || signed_area(&region) < 400.0 {
        return out;
    }
    // Furlongs, then strips.
    let mut furlongs = Vec::new();
    split_furlongs(region, rng.range_f32(7000.0, 15000.0), rng, &mut furlongs, 0);
    for f in furlongs {
        strips(&f, rng, &mut out.fields);
    }
    out
}

fn longest_dir(pts: &[Vec2]) -> Vec2 {
    let n = pts.len();
    let mut best = (Vec2::X, 0.0f32);
    for i in 0..n {
        let d = pts[(i + 1) % n] - pts[i];
        let l = d.length();
        if l > best.1 {
            best = (d / l, l);
        }
    }
    best.0
}

fn split_furlongs(pts: Vec<Vec2>, target: f32, rng: &mut Rng, out: &mut Vec<Vec<Vec2>>, depth: u32) {
    if pts.len() < 3 {
        return;
    }
    let area = signed_area(&pts);
    if area < target * 1.4 || depth > 6 {
        out.push(pts);
        return;
    }
    let n = pts.len();
    let mut best = (0usize, 0.0f32);
    for i in 0..n {
        let l = pts[i].distance(pts[(i + 1) % n]);
        if l > best.1 {
            best = (i, l);
        }
    }
    let a = pts[best.0];
    let b = pts[(best.0 + 1) % n];
    let dir = (b - a).normalize_or_zero();
    let x = a + (b - a) * rng.range_f32(0.35, 0.65);
    let nrm = Vec2::from_angle(rng.range_f32(-0.25, 0.25)).rotate(dir);
    let gap = rng.range_f32(1.5, 3.0);
    let l = clip_half(&pts, x - nrm * (gap * 0.5), -nrm);
    let r = clip_half(&pts, x + nrm * (gap * 0.5), nrm);
    split_furlongs(l, target, rng, out, depth + 1);
    split_furlongs(r, target, rng, out, depth + 1);
}

/// Parallel strips along the furlong's longest edge.
fn strips(f: &[Vec2], rng: &mut Rng, out: &mut Vec<Vec<Vec2>>) {
    let u = longest_dir(f);
    let v = Vec2::new(-u.y, u.x);
    let (mut vmin, mut vmax) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut umin, mut umax) = (f32::INFINITY, f32::NEG_INFINITY);
    for p in f {
        vmin = vmin.min(p.dot(v));
        vmax = vmax.max(p.dot(v));
        umin = umin.min(p.dot(u));
        umax = umax.max(p.dot(u));
    }
    let extent = vmax - vmin;
    if extent < 4.0 {
        return;
    }
    let target = rng.range_f32(9.0, 17.0);
    let count = (extent / target).round().max(1.0) as usize;
    let sw = extent / count as f32;
    let gap = rng.range_f32(0.8, 1.3);
    let long = umax - umin;
    let pieces_along = if long > 170.0 { 2 } else { 1 };
    for k in 0..count {
        let lo = vmin + sw * k as f32 + gap * 0.5;
        let hi = vmin + sw * (k + 1) as f32 - gap * 0.5;
        let mut s = clip_half(f, v * lo, v);
        s = clip_half(&s, v * hi, -v);
        if pieces_along == 2 {
            let mid = umin + long * rng.range_f32(0.4, 0.6);
            let a = clip_half(&s, u * (mid + gap * 0.5), u);
            let b = clip_half(&s, u * (mid - gap * 0.5), -u);
            for p in [a, b] {
                if p.len() >= 3 && signed_area(&p) >= 120.0 && min_width(&p) >= 3.0 {
                    out.push(p);
                }
            }
        } else if s.len() >= 3 && signed_area(&s) >= 120.0 && min_width(&s) >= 3.0 {
            out.push(s);
        }
    }
}
