//! Procedural windows (frames, mullions, glass, sills, shutters, flower boxes),
//! door surrounds and door leaves, shared by dwellings and lot houses.

use mde_core::{DoorKind, MaterialSlot as S, MeshData, Model, Rng, Vec2, Vec3, WindowKind};

use crate::geom::{blob, cuboid, faces, jitter, poly_facing, quad_facing, Basis, Mb};
use crate::palette::{fixed, mix, scale, Linear, WHITE};
use crate::walls::{Head, Hole, WallFrame};

/// Colours a building's openings use (linear) and the level of detail.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OpeningColors {
    pub frame: Linear,
    pub timber: Linear,
    pub stone: Linear,
    /// Cheaper geometry (single-sided glass, hidden faces skipped): lot houses.
    pub lite: bool,
}

/// Hole shape for a window of `kind` in the box `s0..s1 × y0..y0+h`.
pub(crate) fn window_hole(kind: WindowKind, custom: bool, s0: f32, s1: f32, y0: f32, h: f32) -> Hole {
    let w = s1 - s0;
    if custom {
        return Hole::rect(s0, s1, y0, y0 + h);
    }
    match kind {
        WindowKind::Casement | WindowKind::Mullioned | WindowKind::Lattice => Hole::rect(s0, s1, y0, y0 + h),
        WindowKind::Arched => {
            let rise = (w * 0.5).min(h * 0.45);
            Hole { s0, s1, y0, y1: y0 + h - rise, head: Head::Round, rise, coarse: false }
        }
        WindowKind::Gothic => {
            let rise = (w * 0.85).min(h * 0.5).max(w * 0.5);
            Hole { s0, s1, y0, y1: y0 + h - rise, head: Head::Pointed, rise, coarse: false }
        }
        WindowKind::Round => {
            let d = w.min(h);
            let yc = y0 + h * 0.5;
            Hole { s0, s1, y0: yc - d * 0.5, y1: yc + d * 0.5, head: Head::Circle, rise: 0.0, coarse: false }
        }
    }
}

/// Hole for a door of `kind`: the rectangle `s0..s1 × y0..y0+h` stays fully
/// clear (at least `min_clear` tall) and arched kinds add their arch on top, as
/// long as the result stays below `max_top`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn door_hole(kind: DoorKind, custom: bool, s0: f32, s1: f32, y0: f32, h: f32, min_clear: f32, max_top: f32) -> Hole {
    let w = s1 - s0;
    let room = (max_top - y0).max(0.5);
    let clear = h.max(min_clear).min(room);
    let arched = !custom && matches!(kind, DoorKind::Arched | DoorKind::Studded);
    if !arched {
        return Hole::rect(s0, s1, y0, y0 + clear);
    }
    let wanted = if kind == DoorKind::Arched { w * 0.5 } else { (w * 0.3).min(0.4) };
    // Keep the nominal top if the clear rectangle stays tall enough, else
    // spring the arch from the minimum clear height; fit it under `max_top`.
    let spring = (y0 + h - wanted).max(y0 + min_clear.min(room)).min(y0 + room);
    let rise = wanted.min(max_top - spring).max(0.0);
    if rise < 0.08 {
        return Hole::rect(s0, s1, y0, y0 + clear);
    }
    Hole { s0, s1, y0, y1: spring, head: Head::Round, rise, coarse: false }
}

/// Outline with collinear runs merged (so straight edges are single pieces).
fn simplify_outline(pts: &[(f32, f32)]) -> Vec<Vec2> {
    let mut out: Vec<Vec2> = Vec::with_capacity(pts.len());
    for &(s, y) in pts {
        let p = Vec2::new(s, y);
        if out.last().is_none_or(|q: &Vec2| q.distance(p) > 1e-4) {
            out.push(p);
        }
    }
    while out.len() > 2 && out[0].distance(out[out.len() - 1]) <= 1e-4 {
        out.pop();
    }
    let mut changed = true;
    while changed && out.len() > 3 {
        changed = false;
        let n = out.len();
        for i in 0..n {
            let a = out[(i + n - 1) % n];
            let b = out[i];
            let c = out[(i + 1) % n];
            let d1 = (b - a).normalize_or_zero();
            let d2 = (c - b).normalize_or_zero();
            if d1.dot(d2) > 0.9995 {
                out.remove(i);
                changed = true;
                break;
            }
        }
    }
    out
}

/// Frame ring lining the inside of a hole outline: `width` wide in the wall
/// plane, spanning depth `w0..w1`. Built as one mitred ring so no faces overlap.
#[allow(clippy::too_many_arguments)]
fn frame_ring(m: &mut MeshData, f: &WallFrame, h: &Hole, width: f32, w0: f32, w1: f32, col: Linear, lite: bool) {
    let outer = simplify_outline(&h.outline());
    let n = outer.len();
    if n < 3 {
        return;
    }
    let inner: Vec<Vec2> = (0..n)
        .map(|i| {
            let a = outer[(i + n - 1) % n];
            let b = outer[i];
            let c = outer[(i + 1) % n];
            let e1 = (b - a).normalize_or(Vec2::X);
            let e2 = (c - b).normalize_or(Vec2::X);
            let n1 = Vec2::new(-e1.y, e1.x);
            let n2 = Vec2::new(-e2.y, e2.x);
            let mdir = (n1 + n2).normalize_or(n1);
            b + mdir * (width / mdir.dot(n1).max(0.35))
        })
        .collect();
    let centre = outer.iter().copied().sum::<Vec2>() / n as f32;
    let p = |q: Vec2, w: f32| f.p(q.x, q.y, w);
    for i in 0..n {
        let j = (i + 1) % n;
        let (oa, ob, ia, ib) = (outer[i], outer[j], inner[i], inner[j]);
        quad_facing(m, p(oa, w1), p(ob, w1), p(ib, w1), p(ia, w1), f.out3(), col);
        if !lite {
            quad_facing(m, p(oa, w0), p(ob, w0), p(ib, w0), p(ia, w0), -f.out3(), scale(col, 0.85));
        }
        if lite {
            continue;
        }
        let mid = (ia + ib) * 0.5;
        let toward = centre - mid;
        quad_facing(
            m,
            p(ia, w0),
            p(ib, w0),
            p(ib, w1),
            p(ia, w1),
            f.along3() * toward.x + Vec3::Y * toward.y,
            scale(col, 0.92),
        );
    }
}

/// Glass, frame and glazing bars for a window `kind` with its glass plane at depth `wg`.
pub(crate) fn window_insert(mb: &mut Mb, f: &WallFrame, h: &Hole, kind: WindowKind, wg: f32, cols: &OpeningColors, rng: &mut Rng) {
    // Two-sided glass (lot houses: the inside is never seen).
    let glass = jitter(WHITE, rng, 0.05);
    crate::walls::hole_pane(mb.m(S::Glass), f, h, wg, true, glass);
    if !cols.lite {
        crate::walls::hole_pane(mb.m(S::Glass), f, h, wg - 0.006, false, glass);
    }
    let kind = if cols.lite && kind == WindowKind::Lattice { WindowKind::Casement } else { kind };

    let frame = cols.frame;
    let fd = 0.035; // frame half depth
    let (slot, fw, bar_w, col) = match kind {
        WindowKind::Mullioned => (S::Stone, 0.09, 0.1, mix(cols.stone, fixed::dressed_stone(), 0.6)),
        _ => (S::WindowFrame, 0.06, 0.045, frame),
    };
    let depth = if kind == WindowKind::Mullioned { 0.06 } else { fd };
    frame_ring(mb.m(slot), f, h, fw, wg - depth, wg + depth, col, cols.lite);
    let bskip = if cols.lite { faces::NZ | faces::NX | faces::PX } else { 0 };
    let sc = h.sc();
    let top = h.top();
    let m = mb.m(slot);
    match kind {
        WindowKind::Casement => {
            f.bar_skip(m, (sc, h.y0), (sc, h.y1), bar_w, wg - fd * 0.8, wg + fd * 0.8, col, bskip);
            let yt = h.y0 + (h.y1 - h.y0) * 0.64;
            f.bar_skip(m, (h.s0, yt), (h.s1, yt), bar_w, wg - fd * 0.65, wg + fd * 0.65, col, bskip);
        }
        WindowKind::Mullioned => {
            f.bar_skip(m, (sc, h.y0), (sc, h.y1), bar_w, wg - 0.054, wg + 0.054, col, bskip);
            let yt = h.y0 + (h.y1 - h.y0) * 0.66;
            f.bar_skip(m, (h.s0, yt), (h.s1, yt), bar_w * 0.9, wg - 0.046, wg + 0.046, col, bskip);
        }
        WindowKind::Arched => {
            f.bar_skip(m, (sc, h.y0), (sc, top), bar_w, wg - fd * 0.8, wg + fd * 0.8, col, bskip);
            f.bar_skip(m, (h.s0, h.y1), (h.s1, h.y1), bar_w, wg - fd * 0.65, wg + fd * 0.65, col, bskip);
        }
        WindowKind::Gothic => {
            f.bar_skip(m, (sc, h.y0), (sc, h.y1), bar_w, wg - fd * 0.8, wg + fd * 0.8, col, bskip);
            // Y tracery from the mullion head to the arch.
            let q = h.width() * 0.3;
            for x in [sc - q, sc + q] {
                let y = h.top_at(x) - 0.03;
                f.bar_skip(m, (sc, h.y1), (x, y), bar_w * 0.85, wg - fd * 0.7, wg + fd * 0.7, col, bskip);
            }
        }
        WindowKind::Round => {
            let (c, yc, r) = h.circle();
            f.bar_skip(m, (c, yc - r), (c, yc + r), bar_w, wg - fd * 0.8, wg + fd * 0.8, col, bskip);
            f.bar_skip(m, (c - r, yc), (c + r, yc), bar_w, wg - fd * 0.65, wg + fd * 0.65, col, bskip);
        }
        WindowKind::Lattice => {
            lattice(mb.m(S::Iron), f, h, wg, scale(WHITE, 0.32));
            f.bar_skip(mb.m(slot), (sc, h.y0), (sc, h.y1), bar_w, wg - fd * 0.8, wg + fd * 0.8, col, bskip);
        }
    }
}

/// Diamond leading across a rectangular light.
fn lattice(m: &mut MeshData, f: &WallFrame, h: &Hole, wg: f32, col: Linear) {
    let (s0, s1, y0, y1) = (h.s0 + 0.05, h.s1 - 0.05, h.y0 + 0.05, h.y1 - 0.05);
    if s1 <= s0 || y1 <= y0 {
        return;
    }
    let dx = 0.17;
    let dy = 0.24;
    // Lines y = y0 + k*dy ± (s - s0) * dy/dx, clipped to the box.
    let slope = dy / dx;
    let span = (s1 - s0) * slope + (y1 - y0);
    let n = (span / dy).ceil() as i32 + 1;
    for sign in [1.0f32, -1.0] {
        for k in -n..=n {
            // y = c + sign*slope*(s - s0)
            let c = y0 + k as f32 * dy;
            let ys0 = c;
            let ys1 = c + sign * slope * (s1 - s0);
            // Clip segment (s0, ys0)-(s1, ys1) to y0..y1.
            let (mut a, mut b) = ((s0, ys0), (s1, ys1));
            let clip = |p: (f32, f32), q: (f32, f32), yv: f32| -> (f32, f32) {
                let t = (yv - p.1) / (q.1 - p.1);
                (p.0 + (q.0 - p.0) * t, yv)
            };
            if (a.1 < y0 && b.1 < y0) || (a.1 > y1 && b.1 > y1) {
                continue;
            }
            if a.1 < y0 {
                a = clip(a, b, y0);
            } else if a.1 > y1 {
                a = clip(a, b, y1);
            }
            if b.1 < y0 {
                b = clip(a, b, y0);
            } else if b.1 > y1 {
                b = clip(a, b, y1);
            }
            if (b.0 - a.0).abs() + (b.1 - a.1).abs() > 0.05 {
                // The two directions sit at different depths so crossings never
                // share a plane.
                let (back, front) = if sign > 0.0 { (wg + 0.004, wg + 0.012) } else { (wg + 0.002, wg + 0.009) };
                f.bar(m, a, b, 0.014, back, front, col);
            }
        }
    }
}

/// Everything on the outside face around a window: sill, stone lintel or
/// voussoirs, shutters and a flower box.
#[allow(clippy::too_many_arguments)]
pub(crate) fn window_exterior(
    mb: &mut Mb,
    f: &WallFrame,
    h: &Hole,
    others: &[Hole],
    w_face: f32,
    wg: f32,
    stone_wall: bool,
    shutters: bool,
    flower_box: bool,
    cols: &OpeningColors,
    rng: &mut Rng,
) {
    let dressed = mix(cols.stone, fixed::dressed_stone(), 0.7);
    if h.head != Head::Circle {
        let (slot, c) = if stone_wall { (S::Stone, dressed) } else { (S::Timber, cols.timber) };
        let skip = if cols.lite { faces::NZ | faces::NY } else { 0 };
        f.block(mb.m(slot), h.s0 - 0.07, h.s1 + 0.07, h.y0 - 0.075, h.y0 + 0.015, wg + 0.04, w_face + 0.075, jitter(c, rng, 0.05), skip);
    }
    if stone_wall {
        match h.head {
            Head::Flat => {
                let c = jitter(dressed, rng, 0.05);
                f.block(mb.m(S::Stone), h.s0 - 0.12, h.s1 + 0.12, h.y1, h.y1 + 0.22, w_face, w_face + 0.025, c, faces::NZ);
            }
            _ if !cols.lite => arch_ring(mb.m(S::Stone), f, h, 0.2, w_face, 0.025, dressed, scale(dressed, 0.86)),
            _ => {}
        }
    }
    if shutters && h.head != Head::Circle {
        let lw = h.width() * 0.5 + 0.03;
        let top = if h.head == Head::Flat { h.y1 } else { h.y1 + h.rise * 0.6 };
        for left in [true, false] {
            let (a, b) = if left { (h.s0 - 0.04 - lw, h.s0 - 0.04) } else { (h.s1 + 0.04, h.s1 + 0.04 + lw) };
            let clash = others.iter().any(|o| {
                let (os0, os1) = o.span();
                (os1 - h.s1).abs() + (os0 - h.s0).abs() > 1e-3 && a < os1 + 0.05 && b > os0 - 0.05
            });
            if clash {
                continue;
            }
            let c = jitter(cols.frame, rng, 0.06);
            let m = mb.m(S::WindowFrame);
            if cols.lite {
                f.block(m, a, b, h.y0, top, w_face + 0.055, w_face + 0.09, c, faces::NZ | faces::NY | faces::PY);
                continue;
            }
            f.block(m, a, b, h.y0, top, w_face + 0.055, w_face + 0.09, c, faces::NZ);
            for frac in [0.18f32, 0.82] {
                let y = h.y0 + (top - h.y0) * frac;
                f.block(m, a + 0.03, b - 0.03, y - 0.04, y + 0.04, w_face + 0.09, w_face + 0.105, scale(c, 0.85), faces::NZ);
            }
        }
    }
    if flower_box && h.head != Head::Circle {
        let (b0, b1) = (h.y0 - 0.27, h.y0 - 0.08);
        let c = jitter(cols.timber, rng, 0.06);
        let skip = if cols.lite { faces::NZ | faces::NY } else { faces::NZ };
        f.block(mb.m(S::Timber), h.s0 - 0.03, h.s1 + 0.03, b0, b1, w_face, w_face + 0.24, c, skip);
        let n = if cols.lite { 1 } else { ((h.width() / 0.26).round() as usize).max(2) };
        let flowers = [
            crate::palette::lin(0.78, 0.18, 0.20),
            crate::palette::lin(0.88, 0.48, 0.62),
            crate::palette::lin(0.93, 0.80, 0.30),
            crate::palette::lin(0.92, 0.90, 0.86),
            crate::palette::lin(0.55, 0.38, 0.72),
        ];
        let pick = rng.index(flowers.len());
        for i in 0..n {
            let s = h.s0 + h.width() * (i as f32 + 0.5) / n as f32;
            let centre = f.p(s, b1 + 0.03, w_face + 0.12);
            let r = h.width() / n as f32 * 0.62;
            blob(mb.m(S::Foliage), centre, Vec3::new(r, 0.13, r.clamp(0.1, 0.13)), 0, WHITE, rng);
            let fc = flowers[(pick + i % 2) % flowers.len()];
            for k in 0..2 {
                let ds = (k as f32 - 0.5) * r * 0.8 + rng.range_f32(-0.03, 0.03);
                let p = f.p(s + ds, b1 + 0.12 + rng.range_f32(0.0, 0.05), w_face + 0.12 + rng.range_f32(-0.04, 0.06));
                let r = rng.range_f32(0.024, 0.036);
                crate::geom::aabox(mb.m(S::Plaster), p - Vec3::splat(r), p + Vec3::splat(r), fc);
            }
        }
    }
}

/// Ring of voussoir blocks around an arched hole on the wall face, alternating colours.
#[allow(clippy::too_many_arguments)]
pub(crate) fn arch_ring(m: &mut MeshData, f: &WallFrame, h: &Hole, width: f32, w_face: f32, proud: f32, ca: Linear, cb: Linear) {
    let prof = h.profile();
    let (cs, cy) = match h.head {
        Head::Circle => {
            let (c, yc, _) = h.circle();
            (c, yc)
        }
        _ => (h.sc(), h.y1),
    };
    let pts: Vec<(f32, f32)> = match h.head {
        Head::Circle => h.outline(),
        _ => prof.iter().map(|&(s, _, t)| (s, t)).collect(),
    };
    let closed = h.head == Head::Circle;
    let n = pts.len();
    let segs = if closed { n } else { n - 1 };
    let outer = |p: (f32, f32)| {
        let d = Vec2::new(p.0 - cs, p.1 - cy);
        let d = if d.length() < 1e-4 { Vec2::Y } else { d.normalize() };
        (p.0 + d.x * width, p.1 + d.y * width)
    };
    let wa = w_face;
    let wb = w_face + proud;
    for i in 0..segs {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        let (ao, bo) = (outer(a), outer(b));
        let c = if i % 2 == 0 { ca } else { cb };
        let out = f.out3();
        quad_facing(m, f.p(a.0, a.1, wb), f.p(b.0, b.1, wb), f.p(bo.0, bo.1, wb), f.p(ao.0, ao.1, wb), out, c);
        // Outer edge and soffit edge.
        let mid_o = ((ao.0 + bo.0) * 0.5 - cs, (ao.1 + bo.1) * 0.5 - cy);
        let fo = f.along3() * mid_o.0 + Vec3::Y * mid_o.1;
        quad_facing(m, f.p(ao.0, ao.1, wa), f.p(bo.0, bo.1, wa), f.p(bo.0, bo.1, wb), f.p(ao.0, ao.1, wb), fo, c);
        quad_facing(m, f.p(a.0, a.1, wa), f.p(b.0, b.1, wa), f.p(b.0, b.1, wb), f.p(a.0, a.1, wb), -fo, c);
    }
    if !closed {
        // Springer ends.
        for &(p, dir) in &[(pts[0], -1.0f32), (pts[n - 1], 1.0)] {
            let po = outer(p);
            let fo = Vec3::Y * -1.0;
            quad_facing(m, f.p(p.0, p.1, wa), f.p(po.0, po.1, wa), f.p(po.0, po.1, wb), f.p(p.0, p.1, wb), fo + f.along3() * dir * 0.01, ca);
        }
    }
}

/// Exterior dressing of an entrance: timber frame or stone / brick jambs and
/// arch, threshold step and (sometimes) a lantern.
#[allow(clippy::too_many_arguments)]
pub(crate) fn door_surround(
    mb: &mut Mb,
    f: &WallFrame,
    h: &Hole,
    w_face: f32,
    floor_y: f32,
    stone_wall: bool,
    framed: bool,
    lantern: bool,
    cols: &OpeningColors,
    rng: &mut Rng,
) {
    let dressed = mix(cols.stone, fixed::dressed_stone(), 0.7);
    let proud = 0.05;
    let arched = h.head != Head::Flat;
    let brick = rng.chance(0.5) && !stone_wall;
    if arched {
        let (ca, cb) = if brick {
            (crate::palette::mul(cols.stone, fixed::brick()), scale(crate::palette::mul(cols.stone, fixed::brick()), 0.82))
        } else {
            (dressed, scale(dressed, 0.85))
        };
        arch_ring(mb.m(S::Stone), f, h, 0.24, w_face, proud * 0.6, ca, cb);
        // Jamb blocks (half-timbered walls have posts there instead).
        let n = if framed { 0 } else { ((h.y1 - h.y0) / 0.32).floor().max(1.0) as usize };
        let ch = (h.y1 - h.y0) / n as f32;
        for k in 0..n {
            let l = if k % 2 == 0 { 0.3 } else { 0.2 };
            let ya = h.y0 + ch * k as f32 + 0.01;
            let yb = h.y0 + ch * (k + 1) as f32 - 0.01;
            let c = jitter(if k % 2 == 0 { ca } else { cb }, rng, 0.05);
            let m = mb.m(S::Stone);
            f.block(m, h.s0 - l, h.s0, ya, yb, w_face, w_face + proud * 0.6, c, faces::NZ);
            f.block(m, h.s1, h.s1 + l, ya, yb, w_face, w_face + proud * 0.6, c, faces::NZ);
        }
    } else if framed {
        // The framing posts and rail already frame the door.
    } else if stone_wall {
        let c = jitter(dressed, rng, 0.05);
        let m = mb.m(S::Stone);
        f.block(m, h.s0 - 0.2, h.s0, h.y0, h.y1, w_face, w_face + 0.03, c, faces::NZ);
        f.block(m, h.s1, h.s1 + 0.2, h.y0, h.y1, w_face, w_face + 0.03, c, faces::NZ);
        f.block(m, h.s0 - 0.25, h.s1 + 0.25, h.y1, h.y1 + 0.28, w_face, w_face + 0.035, scale(c, 0.95), faces::NZ);
    } else {
        let c = jitter(cols.timber, rng, 0.05);
        let m = mb.m(S::Timber);
        f.block(m, h.s0 - 0.14, h.s0, h.y0, h.y1, w_face, w_face + proud, c, faces::NZ);
        f.block(m, h.s1, h.s1 + 0.14, h.y0, h.y1, w_face, w_face + proud, c, faces::NZ);
        f.block(m, h.s0 - 0.2, h.s1 + 0.2, h.y1, h.y1 + 0.2, w_face, w_face + proud + 0.01, scale(c, 0.92), faces::NZ);
    }
    // Threshold step.
    let step = jitter(dressed, rng, 0.05);
    f.block(mb.m(S::Stone), h.s0 - 0.15, h.s1 + 0.15, floor_y - 0.2, floor_y + 0.015, w_face - 0.06, w_face + 0.38, step, 0);
    if lantern {
        let s = h.s1 + 0.45;
        let y = (h.top() + 0.1).max(h.y0 + 2.2);
        let iron = scale(WHITE, 0.9);
        let mi = mb.m(S::Iron);
        f.block(mi, s - 0.02, s + 0.02, y + 0.18, y + 0.22, w_face, w_face + 0.3, iron, 0);
        f.block(mi, s - 0.09, s + 0.09, y - 0.02, y + 0.02, w_face + 0.16, w_face + 0.34, iron, 0);
        f.block(mi, s - 0.1, s + 0.1, y + 0.2, y + 0.25, w_face + 0.15, w_face + 0.35, iron, 0);
        f.block(mb.m(S::Plaster), s - 0.07, s + 0.07, y + 0.02, y + 0.2, w_face + 0.18, w_face + 0.32, fixed::ember(), 0);
    }
}

/// Simple timber casing around an interior door hole, on one face.
pub(crate) fn door_casing(m: &mut MeshData, f: &WallFrame, h: &Hole, w_face: f32, outward: bool, col: Linear) {
    let (wa, wb) = if outward { (w_face, w_face + 0.025) } else { (w_face - 0.025, w_face) };
    // The back sits on the wall and the jambs' feet on the floor: both hidden.
    let back = if outward { faces::NZ } else { faces::PZ };
    let cw = 0.08;
    f.block(m, h.s0 - cw, h.s0, h.y0, h.y1, wa, wb, col, back | faces::NY);
    f.block(m, h.s1, h.s1 + cw, h.y0, h.y1, wa, wb, col, back | faces::NY);
    if h.head == Head::Flat {
        f.block(m, h.s0 - cw, h.s1 + cw, h.y1, h.y1 + cw, wa, wb, col, back);
    }
}

// ---------------------------------------------------------------------------
// Door leaves
// ---------------------------------------------------------------------------

/// Everything needed to build one door leaf in its local space.
pub(crate) struct LeafSpec {
    pub kind: DoorKind,
    pub width: f32,
    pub height: f32,
    /// Leaf top height at local `x` (for arched heads); `None` = flat.
    pub top: Option<Vec<(f32, f32)>>,
    pub thickness: f32,
    pub door: Linear,
    /// Decorated (street) face is local +Z.
    pub front_pos_z: bool,
    pub seed: u64,
    /// Front-only boards, no ledges (lot houses, where the back is never seen).
    pub lite: bool,
}

impl LeafSpec {
    fn top_at(&self, x: f32) -> f32 {
        match &self.top {
            None => self.height,
            Some(p) => {
                for w in p.windows(2) {
                    let (lo, hi) = if w[0].0 <= w[1].0 { (w[0], w[1]) } else { (w[1], w[0]) };
                    if x >= lo.0 - 1e-5 && x <= hi.0 + 1e-5 {
                        let t = if hi.0 - lo.0 > 1e-6 { (x - lo.0) / (hi.0 - lo.0) } else { 0.0 };
                        return lo.1 + (hi.1 - lo.1) * t;
                    }
                }
                self.height
            }
        }
    }
}

/// Door leaf model in local space: hinge axis through the origin, closed pose
/// spanning `0..width` along +X, `0..height` up, centred on z = 0.
pub(crate) fn door_leaf_model(spec: &LeafSpec) -> Model {
    let mut mb = Mb::new();
    let mut rng = Rng::new(spec.seed);
    let w = spec.width.max(0.2);
    let th = spec.thickness.clamp(0.03, 0.12);
    let fz = if spec.front_pos_z { 1.0 } else { -1.0 };
    let b = Basis::world();
    let n_boards = if spec.lite { ((w / 0.24).round() as usize).clamp(2, 6) } else { ((w / 0.17).round() as usize).clamp(2, 12) };
    let gap = if spec.lite { 0.0 } else { 0.006 };
    let split = if spec.kind == DoorKind::Dutch { Some((spec.height * 0.5).min(1.05)) } else { None };
    let glazed = (spec.kind == DoorKind::Glazed).then(|| {
        let y1 = (spec.height * 0.84).min(spec.top_at(w * 0.5) - 0.2);
        (w * 0.3, w * 0.7, (y1 - 0.38).max(spec.height * 0.5), y1)
    });
    // Boards.
    for i in 0..n_boards {
        let x0 = w * i as f32 / n_boards as f32 + gap * 0.5;
        let x1 = w * (i + 1) as f32 / n_boards as f32 - gap * 0.5;
        let jz = rng.range_f32(-0.004, 0.004);
        let c = jitter(spec.door, &mut rng, 0.08);
        let mut ranges: Vec<(f32, f32)> = vec![(0.0, f32::INFINITY)];
        if let Some(ys) = split {
            ranges = vec![(0.0, ys - 0.006), (ys + 0.006, f32::INFINITY)];
        }
        if let Some((gx0, gx1, gy0, gy1)) = glazed
            && x1 > gx0
            && x0 < gx1
        {
            ranges = ranges
                .iter()
                .flat_map(|&(a, bb)| {
                    let mut v = Vec::new();
                    if gy0 > a {
                        v.push((a, gy0.min(bb)));
                    }
                    if gy1 < bb {
                        v.push((gy1.max(a), bb));
                    }
                    v
                })
                .collect();
        }
        for (ya, yb) in ranges {
            board(mb.m(S::Door), spec, x0, x1, ya, yb, th, jz, c);
        }
    }
    // Ledges on the back face (and a brace on plank doors).
    let back = -fz;
    let ledge_ys: Vec<f32> = {
        let top_min = spec.top_at(0.0).min(spec.top_at(w));
        let mut v = vec![0.18, (top_min - 0.32).max(0.5)];
        if top_min > 1.6 {
            v.insert(1, top_min * 0.5);
        }
        v
    };
    let lz0 = back * th * 0.5;
    let lz1 = back * (th * 0.5 + 0.03);
    let (lz0, lz1) = (lz0.min(lz1), lz0.max(lz1));
    for &y in ledge_ys.iter().filter(|_| !spec.lite) {
        let c = jitter(scale(spec.door, 0.9), &mut rng, 0.05);
        cuboid(mb.m(S::Door), &b, Vec3::new(0.05, y - 0.06, lz0), Vec3::new(w - 0.05, y + 0.06, lz1), c, 0);
    }
    if spec.kind == DoorKind::Plank && ledge_ys.len() >= 2 && !spec.lite {
        let (ya, yb) = (ledge_ys[0] + 0.06, ledge_ys[1] - 0.06);
        let a = Vec3::new(w - 0.12, ya, (lz0 + lz1) * 0.5);
        let bpt = Vec3::new(0.12, yb, (lz0 + lz1) * 0.5);
        // Slightly thinner than the ledges so their faces never coincide.
        crate::geom::beam(mb.m(S::Door), a, bpt, Vec3::Z, 0.1, (lz1 - lz0) * 0.8, scale(spec.door, 0.9), 0);
    }
    // Iron: strap hinges on the front, handle / ring, studs.
    let iron = scale(WHITE, 0.95);
    let fz0 = fz * th * 0.5;
    let fz1 = fz * (th * 0.5 + 0.012);
    let (fz0, fz1) = (fz0.min(fz1), fz0.max(fz1));
    // Fittings lie on the boards: their backs are never seen.
    let iron_skip = if fz > 0.0 { faces::NZ } else { faces::PZ };
    for (k, &y) in ledge_ys.iter().enumerate() {
        if spec.lite && k == 1 && ledge_ys.len() > 2 {
            continue;
        }
        let l = (w * 0.68).min(0.9);
        cuboid(mb.m(S::Iron), &b, Vec3::new(0.02, y - 0.025, fz0), Vec3::new(l, y + 0.025, fz1), iron, iron_skip);
        if !spec.lite {
            // End plate, a touch prouder than the strap.
            let (p0, p1) = if fz > 0.0 { (fz0, fz1 + 0.004) } else { (fz0 - 0.004, fz1) };
            cuboid(mb.m(S::Iron), &b, Vec3::new(l - 0.04, y - 0.04, p0), Vec3::new(l + 0.03, y + 0.04, p1), iron, iron_skip);
        }
    }
    let hx = w - 0.12;
    let hy = (spec.height * 0.48).clamp(0.9, 1.1);
    let pz = fz * (th * 0.5 + 0.03);
    cuboid(
        mb.m(S::Iron),
        &b,
        Vec3::new(hx - 0.04, hy - 0.06, fz0.min(pz)),
        Vec3::new(hx + 0.04, hy + 0.06, fz1.max(pz)),
        iron,
        0,
    );
    if matches!(spec.kind, DoorKind::Arched | DoorKind::Studded) && !spec.lite {
        // Ring knocker.
        let r = 0.07;
        let cy = hy - 0.1;
        let z = fz * (th * 0.5 + 0.035);
        let pts: Vec<Vec3> = (0..8)
            .map(|k| {
                let a = k as f32 / 8.0 * std::f32::consts::TAU;
                Vec3::new(hx + r * a.sin(), cy - r * a.cos(), z)
            })
            .collect();
        for k in 0..8 {
            crate::geom::beam(mb.m(S::Iron), pts[k], pts[(k + 1) % 8], Vec3::Z, 0.018, 0.018, iron, 0);
        }
    }
    if spec.kind == DoorKind::Studded {
        let nx = ((w / if spec.lite { 0.3 } else { 0.2 }).round() as usize).max(2);
        let ny = ((spec.height / if spec.lite { 0.5 } else { 0.32 }).round() as usize).max(3);
        for i in 0..nx {
            for j in 0..ny {
                let x = w * (i as f32 + 0.5) / nx as f32;
                let y = spec.height * (j as f32 + 0.5) / ny as f32;
                if y > spec.top_at(x) - 0.1 {
                    continue;
                }
                let z0 = fz * th * 0.5;
                let z1 = fz * (th * 0.5 + 0.022);
                cuboid(
                    mb.m(S::Iron),
                    &b,
                    Vec3::new(x - 0.018, y - 0.018, z0.min(z1)),
                    Vec3::new(x + 0.018, y + 0.018, z0.max(z1)),
                    iron,
                    if fz > 0.0 { faces::NZ } else { faces::PZ },
                );
            }
        }
    }
    if let Some((gx0, gx1, gy0, gy1)) = glazed {
        let glass = WHITE;
        let mg = mb.m(S::Glass);
        poly_facing(
            mg,
            &[Vec3::new(gx0, gy0, 0.0), Vec3::new(gx1, gy0, 0.0), Vec3::new(gx1, gy1, 0.0), Vec3::new(gx0, gy1, 0.0)],
            Vec3::Z,
            glass,
        );
        poly_facing(
            mg,
            &[Vec3::new(gx0, gy0, -0.004), Vec3::new(gx1, gy0, -0.004), Vec3::new(gx1, gy1, -0.004), Vec3::new(gx0, gy1, -0.004)],
            -Vec3::Z,
            glass,
        );
        let fc = scale(spec.door, 0.8);
        let md = mb.m(S::Door);
        let z = th * 0.5 + 0.015;
        for (a, bb) in [
            (Vec3::new(gx0, gy0, -z), Vec3::new(gx1, gy0 + 0.04, z)),
            (Vec3::new(gx0, gy1 - 0.04, -z), Vec3::new(gx1, gy1, z)),
            (Vec3::new((gx0 + gx1) * 0.5 - 0.02, gy0, -z), Vec3::new((gx0 + gx1) * 0.5 + 0.02, gy1, z)),
            (Vec3::new(gx0, (gy0 + gy1) * 0.5 - 0.02, -z), Vec3::new(gx1, (gy0 + gy1) * 0.5 + 0.02, z)),
        ] {
            cuboid(md, &b, a, bb, fc, 0);
        }
    }
    if let Some(ys) = split {
        // Shelf on the lower half.
        let z0 = fz * th * 0.5;
        let z1 = fz * (th * 0.5 + 0.09);
        cuboid(
            mb.m(S::Door),
            &b,
            Vec3::new(0.0, ys - 0.05, z0.min(z1)),
            Vec3::new(w, ys - 0.01, z0.max(z1)),
            scale(spec.door, 0.9),
            0,
        );
    }
    mb.into_model("door_leaf")
}

/// One door board: x0..x1, y from `ya` to `min(yb, top(x))`.
#[allow(clippy::too_many_arguments)]
fn board(m: &mut MeshData, spec: &LeafSpec, x0: f32, x1: f32, ya: f32, yb: f32, th: f32, jz: f32, c: Linear) {
    let ta = spec.top_at(x0).min(yb);
    let tb = spec.top_at(x1).min(yb);
    let xm = (x0 + x1) * 0.5;
    let tm = spec.top_at(xm).min(yb);
    if ta - ya < 0.01 || tb - ya < 0.01 {
        return;
    }
    let outline = [Vec2::new(x0, ya), Vec2::new(x1, ya), Vec2::new(x1, tb), Vec2::new(xm, tm), Vec2::new(x0, ta)];
    let z0 = -th * 0.5 + jz;
    let z1 = th * 0.5 + jz;
    let front: Vec<Vec3> = outline.iter().map(|p| Vec3::new(p.x, p.y, z1)).collect();
    let back: Vec<Vec3> = outline.iter().map(|p| Vec3::new(p.x, p.y, z0)).collect();
    if spec.lite {
        // Only the street face is ever seen.
        if spec.front_pos_z {
            poly_facing(m, &front, Vec3::Z, c);
        } else {
            poly_facing(m, &back, -Vec3::Z, c);
        }
        return;
    }
    poly_facing(m, &front, Vec3::Z, c);
    poly_facing(m, &back, -Vec3::Z, c);
    let n = outline.len();
    let centre = Vec2::new(xm, (ya + tm) * 0.5);
    for i in 0..n {
        let a = outline[i];
        let b = outline[(i + 1) % n];
        if a.distance(b) < 1e-4 {
            continue;
        }
        let mid = (a + b) * 0.5 - centre;
        quad_facing(
            m,
            Vec3::new(a.x, a.y, z0),
            Vec3::new(b.x, b.y, z0),
            Vec3::new(b.x, b.y, z1),
            Vec3::new(a.x, a.y, z1),
            Vec3::new(mid.x, mid.y, 0.0),
            scale(c, 0.85),
        );
    }
}
