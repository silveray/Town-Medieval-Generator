//! Wall solids with real opening cut-outs, wall-end joins (mitres, T-butts),
//! and the half-timber / masonry dressing of exterior faces.

use mde_core::poly::{closest_point_on_segment, cross, line_intersection};
use mde_core::{MeshData, Rng, Vec2, Vec3, WallSegment};

use crate::geom::{cuboid, faces, poly_facing, quad_facing, w3, wdir, Basis};
use crate::palette::Linear;

/// Wall-local frame: plan point = `origin + dir·s + out·w`, world height `y`.
/// For exterior walls `out` points outside; for interior walls it is the left
/// normal of the segment.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WallFrame {
    pub origin: Vec2,
    pub dir: Vec2,
    pub out: Vec2,
}

impl WallFrame {
    #[inline]
    pub fn plan(&self, s: f32, w: f32) -> Vec2 {
        self.origin + self.dir * s + self.out * w
    }

    #[inline]
    pub fn p(&self, s: f32, y: f32, w: f32) -> Vec3 {
        w3(self.plan(s, w), y)
    }

    #[inline]
    pub fn along3(&self) -> Vec3 {
        wdir(self.dir)
    }

    #[inline]
    pub fn out3(&self) -> Vec3 {
        wdir(self.out)
    }

    /// Basis whose local `(x, y, z)` are `(s, y, w)`.
    pub fn basis(&self) -> Basis {
        Basis { o: w3(self.origin, 0.0), x: self.along3(), y: Vec3::Y, z: self.out3() }
    }

    /// Box spanning `s`, `y`, `w` ranges in wall coordinates.
    #[allow(clippy::too_many_arguments)]
    pub fn block(&self, m: &mut MeshData, s0: f32, s1: f32, y0: f32, y1: f32, w0: f32, w1: f32, col: Linear, skip: u8) {
        if s1 - s0 < 1e-4 || y1 - y0 < 1e-4 || w1 - w0 < 1e-4 {
            return;
        }
        cuboid(m, &self.basis(), Vec3::new(s0, y0, w0), Vec3::new(s1, y1, w1), col, skip);
    }

    /// In-plane bar from `a` to `b` (`(s, y)` points), `width` wide in the
    /// wall plane, spanning depth `w0..w1`.
    #[allow(clippy::too_many_arguments)]
    pub fn bar(&self, m: &mut MeshData, a: (f32, f32), b: (f32, f32), width: f32, w0: f32, w1: f32, col: Linear) {
        self.bar_skip(m, a, b, width, w0, w1, col, 0);
    }

    /// [`WallFrame::bar`] omitting the faces in `skip` (`NX`/`PX` ends,
    /// `NY`/`PY` sides, `NZ` back, `PZ` front).
    #[allow(clippy::too_many_arguments)]
    pub fn bar_skip(&self, m: &mut MeshData, a: (f32, f32), b: (f32, f32), width: f32, w0: f32, w1: f32, col: Linear, skip: u8) {
        let d = Vec2::new(b.0 - a.0, b.1 - a.1);
        let len = d.length();
        if len < 1e-4 {
            return;
        }
        let x = self.along3() * d.x / len + Vec3::Y * d.y / len;
        let y = self.along3() * (-d.y / len) + Vec3::Y * (d.x / len);
        let basis = Basis { o: self.p(a.0, a.1, w0), x, y, z: self.out3() };
        cuboid(m, &basis, Vec3::new(0.0, -width * 0.5, 0.0), Vec3::new(len, width * 0.5, w1 - w0), col, skip);
    }
}

/// Shape of the top of an opening.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Head {
    Flat,
    /// Round (semi-elliptical) arch of height `rise` above `y1`.
    Round,
    /// Pointed (two-centred) arch of height `rise` above `y1`.
    Pointed,
    /// Circular oculus inscribed in the `s0..s1 × y0..y1` box.
    Circle,
}

/// An opening cut through a wall, in wall coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hole {
    pub s0: f32,
    pub s1: f32,
    /// Bottom (sill or floor).
    pub y0: f32,
    /// Top of the straight part (spring line of an arch).
    pub y1: f32,
    pub head: Head,
    pub rise: f32,
    /// Fewer segments on curved heads (lot houses).
    pub coarse: bool,
}

impl Hole {
    pub fn rect(s0: f32, s1: f32, y0: f32, y1: f32) -> Self {
        Hole { s0, s1, y0, y1, head: Head::Flat, rise: 0.0, coarse: false }
    }

    /// Same hole with fewer arch segments.
    pub fn coarse(mut self) -> Self {
        self.coarse = true;
        self
    }

    pub fn width(&self) -> f32 {
        self.s1 - self.s0
    }

    pub fn sc(&self) -> f32 {
        (self.s0 + self.s1) * 0.5
    }

    /// Highest point of the opening.
    pub fn top(&self) -> f32 {
        match self.head {
            Head::Flat | Head::Circle => self.y1,
            Head::Round | Head::Pointed => self.y1 + self.rise,
        }
    }

    /// Circle centre and radius of an oculus.
    pub fn circle(&self) -> (f32, f32, f32) {
        let r = (self.width().min(self.y1 - self.y0) * 0.5).max(0.01);
        (self.sc(), (self.y0 + self.y1) * 0.5, r)
    }

    /// Horizontal extent actually cut (an oculus can be narrower than `s0..s1`).
    pub fn span(&self) -> (f32, f32) {
        if self.head == Head::Circle {
            let (c, _, r) = self.circle();
            (c - r, c + r)
        } else {
            (self.s0, self.s1)
        }
    }

    /// Cross-section samples `(s, bottom, top)` from left to right.
    pub fn profile(&self) -> Vec<(f32, f32, f32)> {
        use std::f32::consts::PI;
        match self.head {
            Head::Flat => vec![(self.s0, self.y0, self.y1), (self.s1, self.y0, self.y1)],
            Head::Round => {
                let n = if self.coarse { 4 } else { 10 };
                let r = self.width() * 0.5;
                (0..=n)
                    .map(|k| {
                        let th = PI * (1.0 - k as f32 / n as f32);
                        let s = if k == 0 {
                            self.s0
                        } else if k == n {
                            self.s1
                        } else {
                            self.sc() + r * th.cos()
                        };
                        (s, self.y0, self.y1 + self.rise * th.sin().max(0.0))
                    })
                    .collect()
            }
            Head::Pointed => {
                let w = self.width();
                let rise = self.rise.max(w * 0.5);
                let c = (w * w * 0.25 + rise * rise) / w.max(1e-4);
                let phi_apex = ((w * 0.5 - c) / c).clamp(-1.0, 1.0).acos();
                let n = if self.coarse { 2 } else { 6 };
                let mut out = Vec::with_capacity(2 * n + 1);
                for k in 0..n {
                    let phi = PI + (phi_apex - PI) * k as f32 / n as f32;
                    let s = if k == 0 { self.s0 } else { self.s0 + c + c * phi.cos() };
                    out.push((s, self.y0, self.y1 + c * phi.sin().max(0.0)));
                }
                out.push((self.sc(), self.y0, self.y1 + rise));
                for k in (0..n).rev() {
                    let phi = PI + (phi_apex - PI) * k as f32 / n as f32;
                    let s = if k == 0 { self.s1 } else { self.s1 - c - c * phi.cos() };
                    out.push((s, self.y0, self.y1 + c * phi.sin().max(0.0)));
                }
                out
            }
            Head::Circle => {
                let (sc, yc, r) = self.circle();
                let n = if self.coarse { 4 } else { 8 };
                (0..=n)
                    .map(|k| {
                        let th = PI * (1.0 - k as f32 / n as f32);
                        let h = r * th.sin().max(0.0);
                        (sc + r * th.cos(), yc - h, yc + h)
                    })
                    .collect()
            }
        }
    }

    /// Outline in `(s, y)`, counter-clockwise (s right, y up), no repeats.
    pub fn outline(&self) -> Vec<(f32, f32)> {
        let prof = self.profile();
        let mut out: Vec<(f32, f32)> = prof.iter().map(|&(s, b, _)| (s, b)).collect();
        if self.head == Head::Circle {
            // Shared end points: bottom left→right, then top right→left without ends.
            for &(s, _, t) in prof.iter().rev().skip(1).take(prof.len().saturating_sub(2)) {
                out.push((s, t));
            }
        } else {
            for &(s, _, t) in prof.iter().rev() {
                out.push((s, t));
            }
        }
        out
    }

    /// Height of the opening's top at `s` (linear between profile samples).
    pub fn top_at(&self, s: f32) -> f32 {
        let prof = self.profile();
        for w in prof.windows(2) {
            if s >= w[0].0 && s <= w[1].0 {
                let t = if w[1].0 - w[0].0 > 1e-6 { (s - w[0].0) / (w[1].0 - w[0].0) } else { 0.0 };
                return w[0].2 + (w[1].2 - w[0].2) * t;
            }
        }
        self.y1
    }
}

/// One face of a wall (`w` = depth of the face plane) with holes cut out,
/// spanning `s0..s1 × y0..y1`. Holes must be sorted by `s0`, disjoint and inside
/// the face. `outward` selects which way the face points (+out or -out).
#[allow(clippy::too_many_arguments)]
pub(crate) fn face_with_holes(
    m: &mut MeshData,
    f: &WallFrame,
    w: f32,
    s0: f32,
    s1: f32,
    y0: f32,
    y1: f32,
    holes: &[Hole],
    outward: bool,
    col: Linear,
) {
    if s1 - s0 < 1e-4 || y1 - y0 < 1e-4 {
        return;
    }
    let facing = f.out3() * if outward { 1.0 } else { -1.0 };
    let q = |m: &mut MeshData, a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)| {
        quad_facing(m, f.p(a.0, a.1, w), f.p(b.0, b.1, w), f.p(c.0, c.1, w), f.p(d.0, d.1, w), facing, col);
    };
    let mut cursor = s0;
    for h in holes {
        let (hs0, hs1) = h.span();
        if hs0 < cursor - 1e-4 || hs1 > s1 + 1e-4 {
            continue;
        }
        if hs0 > cursor + 1e-5 {
            q(m, (cursor, y0), (hs0, y0), (hs0, y1), (cursor, y1));
        }
        let prof = h.profile();
        for pair in prof.windows(2) {
            let (sa, ba, ta) = pair[0];
            let (sb, bb, tb) = pair[1];
            if sb - sa < 1e-6 {
                continue;
            }
            let (ta, tb) = (ta.min(y1), tb.min(y1));
            if ta < y1 - 1e-5 || tb < y1 - 1e-5 {
                q(m, (sa, ta), (sb, tb), (sb, y1), (sa, y1));
            }
            let (ba, bb) = (ba.max(y0), bb.max(y0));
            if ba > y0 + 1e-5 || bb > y0 + 1e-5 {
                q(m, (sa, y0), (sb, y0), (sb, bb), (sa, ba));
            }
        }
        cursor = hs1;
    }
    if s1 > cursor + 1e-5 {
        q(m, (cursor, y0), (s1, y0), (s1, y1), (cursor, y1));
    }
}

/// Reveal faces lining a hole through the wall between depths `wa..wb`.
pub(crate) fn hole_reveals(m: &mut MeshData, f: &WallFrame, h: &Hole, wa: f32, wb: f32, col: Linear) {
    if (wb - wa).abs() < 1e-4 {
        return;
    }
    let prof = h.profile();
    let centre = (h.sc(), (h.y0 + h.top()) * 0.5);
    let toward = |s: f32, y: f32| f.along3() * (centre.0 - s) + Vec3::Y * (centre.1 - y);
    for pair in prof.windows(2) {
        let (sa, ba, ta) = pair[0];
        let (sb, bb, tb) = pair[1];
        if sb - sa < 1e-6 {
            continue;
        }
        // Head (soffit).
        let mid_t = ((sa + sb) * 0.5, (ta + tb) * 0.5);
        quad_facing(m, f.p(sa, ta, wa), f.p(sb, tb, wa), f.p(sb, tb, wb), f.p(sa, ta, wb), toward(mid_t.0, mid_t.1), col);
        // Sill / bottom (doors: the floor covers it, so only above y0 + eps matters
        // little; still emit for windows).
        if ba > h.y0 - 1e-5 && (h.head == Head::Circle || h.y0 > 1e-3 || ba > 1e-3) {
            let mid_b = ((sa + sb) * 0.5, (ba + bb) * 0.5);
            quad_facing(
                m,
                f.p(sa, ba, wa),
                f.p(sb, bb, wa),
                f.p(sb, bb, wb),
                f.p(sa, ba, wb),
                toward(mid_b.0, mid_b.1),
                col,
            );
        }
    }
    // Jambs.
    if let (Some(&(s, b, t)), Some(&(s2, b2, t2))) = (prof.first(), prof.last()) {
        if t - b > 1e-4 {
            quad_facing(m, f.p(s, b, wa), f.p(s, t, wa), f.p(s, t, wb), f.p(s, b, wb), f.along3(), col);
        }
        if t2 - b2 > 1e-4 {
            quad_facing(m, f.p(s2, b2, wa), f.p(s2, t2, wa), f.p(s2, t2, wb), f.p(s2, b2, wb), -f.along3(), col);
        }
    }
}

/// Flat polygon filling a hole outline at depth `w`, facing ±out.
pub(crate) fn hole_pane(m: &mut MeshData, f: &WallFrame, h: &Hole, w: f32, outward: bool, col: Linear) {
    let pts: Vec<Vec3> = h.outline().iter().map(|&(s, y)| f.p(s, y, w)).collect();
    poly_facing(m, &pts, f.out3() * if outward { 1.0 } else { -1.0 }, col);
}

// ---------------------------------------------------------------------------
// Wall-end joins
// ---------------------------------------------------------------------------

/// How one end of a wall is finished.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EndJoin {
    /// End coordinate (along the segment from `a`) of the face on the `+n` side.
    pub s_pos: f32,
    /// End coordinate of the face on the `-n` side.
    pub s_neg: f32,
    /// Whether an end cap is visible.
    pub cap: bool,
    /// Wall this end is mitred with, if any.
    pub mitre: Option<usize>,
}

const JOIN_EPS: f32 = 0.03;

struct Seg {
    a: Vec2,
    d: Vec2,
    n: Vec2,
    len: f32,
    t: f32,
}

impl Seg {
    fn new(w: &WallSegment) -> Option<Seg> {
        if !w.a.is_finite() || !w.b.is_finite() {
            return None;
        }
        let len = w.a.distance(w.b);
        if len < 1e-3 {
            return None;
        }
        let d = (w.b - w.a) / len;
        let t = if w.thickness.is_finite() { w.thickness.clamp(0.02, 3.0) } else { 0.2 };
        Some(Seg { a: w.a, d, n: Vec2::new(-d.y, d.x), len, t })
    }

    /// Coordinate along `self` where its face on `side` (±1) meets the line `(p, dir)`.
    fn s_meet(&self, side: f32, p: Vec2, dir: Vec2) -> Option<f32> {
        let q = self.a + self.n * side * self.t * 0.5;
        line_intersection(q, self.d, p, dir).map(|x| (x - self.a).dot(self.d))
    }

    fn face_line(&self, side: f32) -> (Vec2, Vec2) {
        (self.a + self.n * side * self.t * 0.5, self.d)
    }
}

/// Clamped thickness of a wall segment (as used by the joins and solids).
pub(crate) fn wall_thickness(w: &WallSegment) -> f32 {
    if w.thickness.is_finite() { w.thickness.clamp(0.02, 3.0) } else { 0.2 }
}

/// Work out how both ends of every wall meet their neighbours: equal walls at a
/// corner are mitred, thinner walls butt against the face of thicker or
/// continuing walls, a thicker wall meeting only thinner ones covers the
/// corner, and free ends get a cap. Index `[0]` is end `a`, `[1]` end `b`.
pub(crate) fn wall_joins(walls: &[WallSegment]) -> Vec<[EndJoin; 2]> {
    let segs: Vec<Option<Seg>> = walls.iter().map(Seg::new).collect();
    let mut out = Vec::with_capacity(walls.len());
    for (i, si) in segs.iter().enumerate() {
        let Some(si) = si else {
            let z = EndJoin { s_pos: 0.0, s_neg: 0.0, cap: false, mitre: None };
            out.push([z, z]);
            continue;
        };
        let mut ends = [EndJoin { s_pos: 0.0, s_neg: 0.0, cap: true, mitre: None }; 2];
        for (e, end) in ends.iter_mut().enumerate() {
            let p = if e == 0 { si.a } else { si.a + si.d * si.len };
            let end_s = if e == 0 { 0.0 } else { si.len };
            let inward = if e == 0 { si.d } else { -si.d };
            *end = join_end(i, si, &segs, p, end_s, inward);
        }
        out.push(ends);
    }
    out
}

fn join_end(i: usize, si: &Seg, segs: &[Option<Seg>], p: Vec2, end_s: f32, inward: Vec2) -> EndJoin {
    let plain = |cap: bool| EndJoin { s_pos: end_s, s_neg: end_s, cap, mitre: None };
    let mut through: Vec<usize> = Vec::new();
    let mut ends: Vec<(usize, Vec2)> = Vec::new();
    for (j, sj) in segs.iter().enumerate() {
        let Some(sj) = sj else { continue };
        if j == i {
            continue;
        }
        let b = sj.a + sj.d * sj.len;
        if p.distance(sj.a) < JOIN_EPS {
            ends.push((j, sj.d));
        } else if p.distance(b) < JOIN_EPS {
            ends.push((j, -sj.d));
        } else if p.distance(closest_point_on_segment(p, sj.a, b)) < JOIN_EPS {
            through.push(j);
        }
    }
    let seg = |j: usize| segs[j].as_ref();
    let parallel = |j: usize| seg(j).is_some_and(|s| cross(si.d, s.d).abs() < 0.05);
    let equal = |j: usize| seg(j).is_some_and(|s| (s.t - si.t).abs() < 1e-3);
    let max_shift = 2.5 * si.t + 0.5;
    let valid = |s: Option<f32>, extra: f32| s.filter(|v| (v - end_s).abs() <= max_shift + extra);

    // Trim this end back to the face of `j` that looks at it.
    let trim = |j: usize| -> EndJoin {
        let Some(sj) = seg(j) else { return plain(true) };
        let side = if sj.n.dot(inward) >= 0.0 { 1.0 } else { -1.0 };
        let (q, d) = sj.face_line(side);
        match (valid(si.s_meet(1.0, q, d), sj.t), valid(si.s_meet(-1.0, q, d), sj.t)) {
            (Some(a), Some(b)) => EndJoin { s_pos: a, s_neg: b, cap: false, mitre: None },
            _ => plain(true),
        }
    };

    // 1. Ends on the side of a continuing (through) wall.
    let thr: Vec<usize> = through.iter().copied().filter(|&j| !parallel(j)).collect();
    if let Some(&j) = thr.iter().max_by(|&&a, &&b| {
        let ta = seg(a).map_or(0.0, |s| s.t);
        let tb = seg(b).map_or(0.0, |s| s.t);
        ta.total_cmp(&tb)
    }) {
        return trim(j);
    }
    // 2. Continues straight into an equal wall.
    if ends.iter().any(|&(j, dj)| parallel(j) && equal(j) && dj.dot(inward) < 0.0) {
        return plain(false);
    }
    let cands: Vec<(usize, Vec2)> = ends.iter().copied().filter(|&(j, _)| !parallel(j)).collect();
    if cands.is_empty() {
        let covered = ends.iter().any(|&(j, dj)| {
            parallel(j) && dj.dot(inward) < 0.0 && seg(j).is_some_and(|s| s.t >= si.t - 1e-3)
        });
        return plain(!covered);
    }
    let tmax = cands.iter().filter_map(|&(j, _)| seg(j).map(|s| s.t)).fold(0.0f32, f32::max);
    let thickest = cands
        .iter()
        .copied()
        .max_by(|a, b| seg(a.0).map_or(0.0, |s| s.t).total_cmp(&seg(b.0).map_or(0.0, |s| s.t)));
    if si.t < tmax - 1e-3 {
        return thickest.map_or(plain(true), |(j, _)| trim(j));
    }
    let eq: Vec<(usize, Vec2)> = cands.iter().copied().filter(|&(j, _)| equal(j)).collect();
    match eq.len() {
        1 => {
            let (j, dj) = eq[0];
            let Some(sj) = seg(j) else { return plain(true) };
            if cross(si.d, sj.d).abs() < 0.2 {
                return plain(true);
            }
            let o_inner = if sj.n.dot(inward) >= 0.0 { 1.0 } else { -1.0 };
            let my_inner = if si.n.dot(dj) >= 0.0 { 1.0 } else { -1.0 };
            let s_for = |side: f32| {
                let o_side = if side == my_inner { o_inner } else { -o_inner };
                let (q, d) = sj.face_line(o_side);
                valid(si.s_meet(side, q, d), sj.t)
            };
            match (s_for(1.0), s_for(-1.0)) {
                (Some(a), Some(b)) => EndJoin { s_pos: a, s_neg: b, cap: false, mitre: Some(j) },
                _ => plain(true),
            }
        }
        0 => {
            // Thicker than every neighbour: extend over the corner.
            let Some((j, _)) = thickest else { return plain(true) };
            let Some(sj) = seg(j) else { return plain(true) };
            let sin = cross(si.d, sj.d).abs().max(0.2);
            let ext = sj.t * 0.5 / sin;
            let s = if end_s == 0.0 { -ext } else { end_s + ext };
            EndJoin { s_pos: s, s_neg: s, cap: true, mitre: None }
        }
        _ => {
            // Several equal walls end here: if two of them continue each other,
            // butt against that line, otherwise leave the end as is.
            for x in 0..eq.len() {
                for y in x + 1..eq.len() {
                    let (ja, da) = eq[x];
                    let (_, db) = eq[y];
                    if cross(da, db).abs() < 0.05 && da.dot(db) < 0.0 {
                        return trim(ja);
                    }
                }
            }
            plain(false)
        }
    }
}

// ---------------------------------------------------------------------------
// Half-timbering and masonry dressing
// ---------------------------------------------------------------------------

/// Parameters for [`half_timber`].
pub(crate) struct Framing<'a> {
    pub f: &'a WallFrame,
    /// Depth of the face the timbers sit on.
    pub w_face: f32,
    /// Face extent.
    pub s0: f32,
    pub s1: f32,
    /// Storey extent: `y0` = floor line, `y1` = next floor line / wall plate.
    pub y0: f32,
    pub y1: f32,
    pub holes: &'a [Hole],
    /// Extra length added to the corner posts and plates past `s0` / `s1`
    /// (closes the notch at convex corners on one of the two walls).
    pub ext: [f32; 2],
    /// Whether `s0` / `s1` is a convex building corner (corner posts).
    pub corner: [bool; 2],
    /// Shorten plates and end posts at `s0` / `s1` by this much (inner
    /// corners, so the two walls' timbers do not overlap).
    pub trim: [f32; 2],
    pub col: Linear,
    /// Typical window sill height above `y0` (mid rail).
    pub rail: f32,
    /// Wall ends here at a floor slab: draw the sill beam.
    pub sill_beam: bool,
    /// Fewer faces (hidden faces of beams are skipped): lot houses.
    pub lite: bool,
}

/// Timber framing proud of a wall face: sill beam and wall plate, corner
/// posts, posts flanking openings and between them, rails under / over windows
/// and diagonal braces in some closed panels.
pub(crate) fn half_timber(m: &mut MeshData, fr: &Framing, rng: &mut Rng) {
    let f = fr.f;
    let proud = 0.05;
    // Beams sit on the face (not embedded) so their tops never share a plane
    // with the wall top.
    let (w0, w1) = (fr.w_face, fr.w_face + proud);
    let bh = 0.2;
    let pw = 0.16;
    let cw = 0.22;
    let len = fr.s1 - fr.s0;
    if len < 0.3 || fr.y1 - fr.y0 < 0.6 {
        return;
    }
    let tint = |rng: &mut Rng| crate::geom::jitter(fr.col, rng, 0.07);
    let skip = if fr.lite { faces::NZ | faces::NY } else { faces::NZ };
    let post_skip = if fr.lite { faces::NZ | faces::NY | faces::PY } else { faces::NZ };
    let reaches_floor = |h: &Hole| h.y0 <= fr.y0 + bh + 0.02;

    // Plates.
    let (ps0, ps1) = (fr.s0 - fr.ext[0] + fr.trim[0], fr.s1 + fr.ext[1] - fr.trim[1]);
    let mut beam_runs: Vec<(f32, f32)> = vec![(ps0, ps1)];
    let mut sill_runs: Vec<(f32, f32)> = beam_runs.clone();
    for h in fr.holes {
        if reaches_floor(h) {
            sill_runs = cut_runs(&sill_runs, h.s0, h.s1);
        }
        if h.top() > fr.y1 - bh - 0.01 {
            beam_runs = cut_runs(&beam_runs, h.s0, h.s1);
        }
    }
    if fr.sill_beam {
        for &(a, b) in &sill_runs {
            let c = tint(rng);
            f.block(m, a, b, fr.y0, fr.y0 + bh, w0, w1, c, skip);
        }
    }
    let plate_y0 = fr.y1 - bh;
    for &(a, b) in &beam_runs {
        let c = tint(rng);
        f.block(m, a, b, plate_y0, fr.y1, w0, w1 + 0.01, c, skip);
    }

    // Posts: (s0, s1) intervals.
    let mut posts: Vec<(f32, f32)> = Vec::new();
    if fr.corner[0] {
        posts.push((fr.s0 - fr.ext[0], fr.s0 + cw));
    } else {
        posts.push((fr.s0 + fr.trim[0], fr.s0 + fr.trim[0] + pw * 0.8));
    }
    if fr.corner[1] {
        posts.push((fr.s1 - cw, fr.s1 + fr.ext[1]));
    } else {
        posts.push((fr.s1 - fr.trim[1] - pw * 0.8, fr.s1 - fr.trim[1]));
    }
    let mut holes: Vec<Hole> = fr.holes.to_vec();
    holes.sort_by(|a, b| a.s0.total_cmp(&b.s0));
    for h in &holes {
        let (hs0, hs1) = h.span();
        for (a, b) in [(hs0 - pw, hs0), (hs1, hs1 + pw)] {
            if a < fr.s0 + 0.05 || b > fr.s1 - 0.05 {
                continue;
            }
            if holes.iter().any(|o| {
                let (os0, os1) = o.span();
                a < os1 - 1e-3 && b > os0 + 1e-3
            }) {
                continue;
            }
            if posts.iter().any(|&(pa, pb)| a < pb + 0.02 && b > pa - 0.02) {
                continue;
            }
            posts.push((a, b));
        }
    }
    posts.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Fill wide panels with intermediate posts.
    let mut filled: Vec<(f32, f32)> = Vec::new();
    for k in 0..posts.len() {
        filled.push(posts[k]);
        if k + 1 < posts.len() {
            let gap0 = posts[k].1;
            let gap1 = posts[k + 1].0;
            let gap = gap1 - gap0;
            let inside_hole = holes.iter().any(|h| {
                let (a, b) = h.span();
                a >= gap0 - 0.01 && b <= gap1 + 0.01
            });
            let max_panel = if fr.lite { 1.9 } else { 1.3 };
            if gap > max_panel + 0.4 && !inside_hole {
                let n = (gap / max_panel).floor() as usize;
                for i in 1..=n {
                    let c = gap0 + gap * i as f32 / (n + 1) as f32;
                    filled.push((c - pw * 0.5, c + pw * 0.5));
                }
            }
        }
    }
    let posts = filled;
    let post_y0 = if fr.sill_beam { fr.y0 + bh } else { fr.y0 };
    for &(a, b) in &posts {
        let c = tint(rng);
        f.block(m, a, b, post_y0, plate_y0, w0, w1, c, post_skip);
    }

    // Panels between posts: rails and braces.
    let brace_lean = rng.chance(0.5);
    for k in 0..posts.len().saturating_sub(1) {
        let pa = posts[k].1;
        let pb = posts[k + 1].0;
        let width = pb - pa;
        if width < 0.25 {
            continue;
        }
        let hole = holes.iter().find(|h| {
            let (a, b) = h.span();
            a >= pa - 0.02 && b <= pb + 0.02
        });
        let w0r = w0 + 0.005;
        let w1r = w1 - 0.012;
        match hole {
            Some(h) => {
                if !reaches_floor(h) {
                    let y = h.y0;
                    if y - bh * 0.7 > post_y0 + 0.05 {
                        let c = tint(rng);
                        f.block(m, pa, pb, y - bh * 0.7, y, w0r, w1r, c, skip);
                    }
                }
                let top = h.top();
                if top + bh * 0.7 < plate_y0 - 0.05 {
                    let c = tint(rng);
                    f.block(m, pa, pb, top, top + bh * 0.7, w0r, w1r, c, skip);
                }
            }
            None => {
                let rail_y = fr.y0 + fr.rail;
                let has_rail = rail_y - bh * 0.7 > post_y0 + 0.35 && rail_y < plate_y0 - 0.5;
                if has_rail {
                    let c = tint(rng);
                    f.block(m, pa, pb, rail_y - bh * 0.7, rail_y, w0r, w1r, c, skip);
                }
                if width > 0.55 && rng.chance(if fr.lite { 0.5 } else { 0.7 }) {
                    // Lean braces towards the middle of the wall (or per panel).
                    let mid = (pa + pb) * 0.5;
                    let towards_centre = mid < (fr.s0 + fr.s1) * 0.5;
                    let lean_right = if brace_lean { towards_centre } else { k % 2 == 0 };
                    let (ylo, yhi) = if has_rail { (rail_y, plate_y0) } else { (post_y0, plate_y0) };
                    let bw = 0.13;
                    let (a, b) = if lean_right {
                        ((pa, ylo + 0.02), (pb, yhi - 0.02))
                    } else {
                        ((pb, ylo + 0.02), (pa, yhi - 0.02))
                    };
                    // Shorten so the brace ends inside the panel corners.
                    let dir = Vec2::new(b.0 - a.0, b.1 - a.1);
                    let l = dir.length();
                    if l > 0.3 {
                        let u = dir / l;
                        let inset = bw * 0.5 * (u.x.abs() / u.y.abs().max(0.2)).min(2.0);
                        let a2 = (a.0 + u.x * inset, a.1 + u.y * inset);
                        let b2 = (b.0 - u.x * inset, b.1 - u.y * inset);
                        let c = tint(rng);
                        f.bar_skip(m, a2, b2, bw, w0r, w1r - 0.006, c, if fr.lite { faces::NZ | faces::NX | faces::PX } else { 0 });
                    }
                }
            }
        }
    }
}

/// Darken vertex colours near the ground (splash grime at the foot of walls):
/// vertices from index `from` on, by up to 18 % at `ground_y`, fading out
/// over 1.2 m.
pub(crate) fn ground_grime(m: &mut MeshData, from: usize, ground_y: f32) {
    for i in from..m.positions.len() {
        let y = m.positions[i][1];
        let t = ((y - ground_y) / 1.2).clamp(0.0, 1.0);
        let f = 0.82 + 0.18 * t * t * (3.0 - 2.0 * t);
        if let Some(c) = m.colors.get_mut(i) {
            c[0] *= f;
            c[1] *= f;
            c[2] *= f;
        }
    }
}

/// Remove `a..b` from a list of runs.
pub(crate) fn cut_runs(runs: &[(f32, f32)], a: f32, b: f32) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    for &(x, y) in runs {
        if b <= x || a >= y {
            out.push((x, y));
            continue;
        }
        if a > x + 1e-3 {
            out.push((x, a));
        }
        if b < y - 1e-3 {
            out.push((b, y));
        }
    }
    out
}

/// Alternating long / short dressed-stone quoins at a convex corner. `at_end`
/// picks the `s1` end (else `s0`); `parity` alternates between the two walls.
#[allow(clippy::too_many_arguments)]
pub(crate) fn quoins(
    m: &mut MeshData,
    f: &WallFrame,
    w_face: f32,
    s_corner: f32,
    at_end: bool,
    y0: f32,
    y1: f32,
    ext: f32,
    parity: bool,
    col: Linear,
    rng: &mut Rng,
    lite: bool,
) {
    let course = if lite { 0.6 } else { 0.34 };
    let skip = if lite { faces::NZ | faces::NY } else { faces::NZ };
    let n = ((y1 - y0) / course).floor().max(1.0) as usize;
    let ch = (y1 - y0) / n as f32;
    for k in 0..n {
        let long = (k % 2 == 0) == parity;
        let l = if long { 0.55 } else { 0.32 };
        let (a, b) = if at_end { (s_corner - l, s_corner + ext) } else { (s_corner - ext, s_corner + l) };
        let ya = y0 + ch * k as f32 + 0.012;
        let yb = y0 + ch * (k + 1) as f32 - 0.012;
        let c = crate::geom::jitter(col, rng, 0.08);
        f.block(m, a, b, ya, yb, w_face, w_face + 0.03, c, skip);
    }
}

/// Wall solid description for [`wall_solid`].
pub(crate) struct Solid<'a> {
    pub f: WallFrame,
    pub t: f32,
    pub y0: f32,
    pub y1: f32,
    /// Face extents along `s`: outer (`w = +t/2`) and inner (`w = -t/2`), per end.
    pub s_out: [f32; 2],
    pub s_in: [f32; 2],
    pub caps: [bool; 2],
    pub holes: &'a [Hole],
    /// Emit the top face (off when a floor or roof sits on the wall).
    pub top: bool,
}

/// Emit a wall solid with its holes: outer face into `outer`, inner face into
/// `inner`, top and caps into `inner`. Reveals are split at depth `split_w`
/// (outer part gets the outer colour).
pub(crate) fn wall_solid(
    outer: &mut MeshData,
    outer_col: Linear,
    inner: &mut MeshData,
    inner_col: Linear,
    w: &Solid,
    split_w: f32,
) {
    let f = &w.f;
    let ht = w.t * 0.5;
    face_with_holes(outer, f, ht, w.s_out[0], w.s_out[1], w.y0, w.y1, w.holes, true, outer_col);
    face_with_holes(inner, f, -ht, w.s_in[0], w.s_in[1], w.y0, w.y1, w.holes, false, inner_col);
    let top = [f.p(w.s_out[0], w.y1, ht), f.p(w.s_out[1], w.y1, ht), f.p(w.s_in[1], w.y1, -ht), f.p(w.s_in[0], w.y1, -ht)];
    if w.top {
        poly_facing(inner, &top, Vec3::Y, inner_col);
    }
    if w.caps[0] {
        quad_facing(
            inner,
            f.p(w.s_out[0], w.y0, ht),
            f.p(w.s_in[0], w.y0, -ht),
            f.p(w.s_in[0], w.y1, -ht),
            f.p(w.s_out[0], w.y1, ht),
            -f.along3(),
            inner_col,
        );
    }
    if w.caps[1] {
        quad_facing(
            inner,
            f.p(w.s_out[1], w.y0, ht),
            f.p(w.s_in[1], w.y0, -ht),
            f.p(w.s_in[1], w.y1, -ht),
            f.p(w.s_out[1], w.y1, ht),
            f.along3(),
            inner_col,
        );
    }
    let split = split_w.clamp(-ht, ht);
    for h in w.holes {
        hole_reveals(outer, f, h, split, ht, outer_col);
        hole_reveals(inner, f, h, -ht, split, inner_col);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(a: (f32, f32), b: (f32, f32), t: f32, exterior: bool) -> WallSegment {
        WallSegment { a: Vec2::new(a.0, a.1), b: Vec2::new(b.0, b.1), thickness: t, exterior }
    }

    #[test]
    fn square_corners_are_mitred() {
        let walls = vec![
            seg((0.0, 0.0), (4.0, 0.0), 0.4, true),
            seg((4.0, 0.0), (4.0, 3.0), 0.4, true),
            seg((0.0, 3.0), (4.0, 3.0), 0.4, true),
            seg((0.0, 0.0), (0.0, 3.0), 0.4, true),
            seg((2.0, 0.0), (2.0, 3.0), 0.15, false),
        ];
        let j = wall_joins(&walls);
        // Bottom wall: -n side (outside, y < 0) extends past the corner, +n side stops short.
        assert!((j[0][0].s_neg + 0.2).abs() < 1e-4 && (j[0][0].s_pos - 0.2).abs() < 1e-4, "{:?}", j[0]);
        assert!(j[0][0].mitre == Some(3) && !j[0][0].cap);
        // Interior wall butts against the inner faces.
        assert!((j[4][0].s_pos - 0.2).abs() < 1e-4 && (j[4][1].s_neg - 2.8).abs() < 1e-4, "{:?}", j[4]);
        assert!(!j[4][0].cap);
    }

    #[test]
    fn hole_outlines() {
        for head in [Head::Flat, Head::Round, Head::Pointed, Head::Circle] {
            let h = Hole { s0: 1.0, s1: 2.0, y0: 1.0, y1: 2.0, head, rise: 0.5, coarse: false };
            let o = h.outline();
            let n = o.len();
            let area: f32 = (0..n).map(|i| o[i].0 * o[(i + 1) % n].1 - o[(i + 1) % n].0 * o[i].1).sum::<f32>() * 0.5;
            assert!(area > 0.5, "{head:?} {area}");
            let prof = h.profile();
            assert!(prof.windows(2).all(|w| w[1].0 >= w[0].0));
        }
    }
}
