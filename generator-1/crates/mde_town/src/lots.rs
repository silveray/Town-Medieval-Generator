//! Building lots: per-edge block insets, recursive subdivision, depth limits,
//! clearance against streets / walls / water, and fitted special buildings.

use std::collections::BTreeSet;

use mde_core::{BuildingKind, Rng, StreetKind, Vec2, WardKind};

use crate::geom::{
    Obstacles, Shape, clip_half, contains, convex_overlap, inset_convex, min_angle_deg, min_width, outward,
    poly_seg_closest, poly_seg_dist, signed_area, tidy,
};
use crate::pipeline::EdgeClass;
use crate::streets::{ALLEY_WIDTH, width_on};
use crate::topo::Topo;
use crate::wall::{CASTLE_THICKNESS, LANE_OFFSET, WALL_THICKNESS};

/// Tag of an edge created by a subdivision cut.
pub(crate) const CUT: i32 = -1;
/// Tag of an edge created by a corner cut at a street junction.
pub(crate) const CORNER: i32 = -2;
/// Smallest footprint kept (m²).
pub(crate) const MIN_LOT_AREA: f32 = 12.0;
/// Narrowest footprint kept (m).
pub(crate) const MIN_LOT_WIDTH: f32 = 3.2;

/// Convex polygon whose edges remember which cell side they came from
/// (`tags[i]` belongs to edge `i → i+1`; negative = internal cut).
#[derive(Clone, Debug, Default)]
pub(crate) struct Piece {
    pub pts: Vec<Vec2>,
    pub tags: Vec<i32>,
}

impl Piece {
    /// Piece whose edge `i` is tagged `i`.
    pub fn from_pts(pts: Vec<Vec2>) -> Self {
        let tags = (0..pts.len() as i32).collect();
        Self { pts, tags }
    }

    pub fn area(&self) -> f32 {
        signed_area(&self.pts)
    }

    pub fn edge(&self, i: usize) -> (Vec2, Vec2) {
        (self.pts[i], self.pts[(i + 1) % self.pts.len()])
    }

    /// Longest edge carrying `tag`.
    pub fn tagged_edge(&self, tag: i32) -> Option<(Vec2, Vec2)> {
        (0..self.pts.len())
            .filter(|&i| self.tags[i] == tag)
            .map(|i| self.edge(i))
            .max_by(|a, b| a.0.distance(a.1).total_cmp(&b.0.distance(b.1)))
    }

    /// Keep the half-plane `(q - p) · n >= 0`; the new boundary edge gets `tag`.
    pub fn clip(&self, p: Vec2, n: Vec2, tag: i32) -> Piece {
        let m = self.pts.len();
        let mut pts = Vec::with_capacity(m + 1);
        let mut tags = Vec::with_capacity(m + 1);
        for i in 0..m {
            let a = self.pts[i];
            let b = self.pts[(i + 1) % m];
            let t = self.tags[i];
            let sa = (a - p).dot(n);
            let sb = (b - p).dot(n);
            if sa >= 0.0 {
                pts.push(a);
                tags.push(t);
                if sb < 0.0 {
                    pts.push(a + (b - a) * (sa / (sa - sb)));
                    tags.push(tag);
                }
            } else if sb >= 0.0 {
                pts.push(a + (b - a) * (sa / (sa - sb)));
                tags.push(t);
            }
        }
        dedupe_tagged(Piece { pts, tags })
    }
}

/// Drop zero-length edges, keeping the tag of the edge that follows.
fn dedupe_tagged(mut p: Piece) -> Piece {
    loop {
        let m = p.pts.len();
        if m < 3 {
            return p;
        }
        let mut removed = false;
        for i in 0..m {
            let j = (i + 1) % m;
            if p.pts[i].distance(p.pts[j]) < 1e-3 {
                p.pts.remove(i);
                p.tags.remove(i);
                removed = true;
                break;
            }
        }
        if !removed {
            return p;
        }
    }
}

/// Lot geometry targets for a ward.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LotStyle {
    /// Target street frontage of a lot (m).
    pub frontage: f32,
    /// Maximum lot depth from its street frontage (m); deeper ground is left
    /// as yards and courtyards.
    pub depth: f32,
    /// Probability of leaving a lot empty (gardens, yards).
    pub gap: f32,
    /// Probability of an internal alley gap when splitting large pieces.
    pub alley: f32,
    /// Typical gap between neighbouring buildings (0 = terraced rows).
    pub side_gap: f32,
}

/// Lot style for `ward`, or `None` for unbuilt wards. `lot` is the average
/// frontage from [`mde_core::TownParams::lot_size`].
pub(crate) fn lot_style(ward: WardKind, dense: bool, lot: f32) -> Option<LotStyle> {
    use WardKind as W;
    // (frontage × lot, depth × lot, gap probability, alley probability)
    let (f, d, g, al): (f32, f32, f32, f32) = match ward {
        W::Craftsmen => (1.0, 2.25, 0.03, 0.35),
        W::Merchant => (1.25, 2.4, 0.02, 0.3),
        W::Market => (1.1, 2.25, 0.0, 0.2),
        W::Administration => (2.0, 3.0, 0.1, 0.2),
        W::Patriciate => (2.2, 3.1, 0.22, 0.15),
        W::Military => (2.0, 2.7, 0.12, 0.2),
        W::Slum => (0.75, 1.9, 0.02, 0.5),
        W::Gate => (1.2, 1.8, 0.55, 0.0),
        W::Cathedral => (1.4, 2.0, 0.35, 0.0),
        W::Castle => (1.6, 0.95, 0.25, 0.0),
        _ => return None,
    };
    let (mut f, mut d, mut g) = (f, d, g);
    let mut side_gap: f32 = match ward {
        W::Patriciate => 1.6,
        W::Administration | W::Military => 0.8,
        W::Gate | W::Cathedral => 1.4,
        _ => 0.0,
    };
    if !dense && ward != W::Castle {
        f *= 1.45;
        d *= 0.62;
        g = (g + 0.12).min(0.75);
        side_gap = side_gap.max(2.2);
    }
    Some(LotStyle { frontage: f * lot, depth: d * lot, gap: g, alley: if dense { al } else { 0.0 }, side_gap })
}

/// Longest edge accepted by `pred`: `(index, start, end)`.
fn longest_where(piece: &Piece, pred: &dyn Fn(i32) -> bool) -> Option<(usize, Vec2, Vec2)> {
    (0..piece.pts.len())
        .filter(|&i| pred(piece.tags[i]))
        .map(|i| {
            let (a, b) = piece.edge(i);
            (i, a, b)
        })
        .max_by(|x, y| x.1.distance(x.2).total_cmp(&y.1.distance(y.2)))
}

/// Recursively split `piece` into lots that each keep a street frontage
/// (per `front`). Large pieces are bisected across their longest edge;
/// frontage strips are then separated from deep interiors by a cut parallel to
/// the street, and sliced perpendicular to it into narrow, deep plots — the
/// burgage pattern of medieval towns. Pieces without frontage are dropped
/// (they become courtyards and gardens).
pub(crate) fn subdivide(
    piece: Piece,
    front: &dyn Fn(i32) -> bool,
    style: &LotStyle,
    rng: &mut Rng,
    out: &mut Vec<Piece>,
    depth: u32,
) {
    if piece.pts.len() < 3 {
        return;
    }
    let Some((_, fa, fb)) = longest_where(&piece, front) else { return };
    let area = piece.area();
    if area < 4.0 {
        return;
    }
    if depth > 28 {
        out.push(piece);
        return;
    }
    let flen = fa.distance(fb);
    let fdir = (fb - fa) / flen.max(1e-6);
    let inward = -outward(fa, fb);
    let fdepth = piece.pts.iter().map(|&p| (p - fa).dot(inward)).fold(0.0f32, f32::max);
    let lot_area = style.frontage * style.depth;

    if area > lot_area * 7.0 {
        // Big piece: bisect across its longest edge (frontage preferred).
        let m = piece.pts.len();
        let mut best = (0usize, 0.0f32);
        for i in 0..m {
            let (a, b) = piece.edge(i);
            let l = a.distance(b) * if front(piece.tags[i]) { 1.25 } else { 1.0 };
            if l > best.1 {
                best = (i, l);
            }
        }
        let (a, b) = piece.edge(best.0);
        let dir = (b - a).normalize_or_zero();
        if dir == Vec2::ZERO {
            return;
        }
        let x = a + (b - a) * rng.range_f32(0.38, 0.62);
        let n = Vec2::from_angle(rng.range_f32(-0.1, 0.1)).rotate(dir);
        let gap = if area > lot_area * 12.0 && rng.chance(style.alley) { rng.range_f32(1.2, 2.0) } else { 0.0 };
        let left = piece.clip(x - n * (gap * 0.5), -n, CUT);
        let right = piece.clip(x + n * (gap * 0.5), n, CUT);
        subdivide(left, front, style, rng, out, depth + 1);
        subdivide(right, front, style, rng, out, depth + 1);
    } else if fdepth > style.depth * 1.3 {
        // Separate the frontage strip from the interior.
        let cut = style.depth * rng.range_f32(0.95, 1.15);
        let p = fa + inward * cut;
        let near = piece.clip(p, -inward, CUT);
        let far = piece.clip(p, inward, CUT);
        subdivide(near, front, style, rng, out, depth + 1);
        subdivide(far, front, style, rng, out, depth + 1);
    } else if flen > style.frontage * rng.range_f32(1.35, 1.75) {
        // Slice the strip into plots perpendicular to the street.
        let w = style.frontage * rng.range_f32(0.8, 1.2);
        let t = if flen < style.frontage * 3.0 { 0.5 } else { (w / flen).clamp(0.2, 0.8) };
        let t = if rng.chance(0.5) { t } else { 1.0 - t };
        let x = fa + (fb - fa) * t;
        let n = Vec2::from_angle(rng.range_f32(-0.05, 0.05)).rotate(fdir);
        let left = piece.clip(x, -n, CUT);
        let right = piece.clip(x, n, CUT);
        subdivide(left, front, style, rng, out, depth + 1);
        subdivide(right, front, style, rng, out, depth + 1);
    } else {
        out.push(piece);
    }
}

/// A lot ready to become a building.
#[derive(Clone, Debug)]
pub(crate) struct Lot {
    pub pts: Vec<Vec2>,
    /// Unit vector towards the street (or courtyard for castle lean-tos).
    pub front: Vec2,
    /// Cell side the lot fronts onto (`-1` if none).
    pub side: i32,
}

/// Turn a subdivision piece into a lot: pick its frontage, clip its depth,
/// shrink it a little.
pub(crate) fn finish_lot(
    piece: &Piece,
    front_w: &dyn Fn(i32) -> f32,
    style: &LotStyle,
    invert_front: bool,
    rng: &mut Rng,
) -> Option<Lot> {
    let m = piece.pts.len();
    let mut best: Option<(usize, f32)> = None;
    for i in 0..m {
        let w = front_w(piece.tags[i]);
        if w <= 0.0 {
            continue;
        }
        let (a, b) = piece.edge(i);
        let l = a.distance(b);
        if l < 2.5 {
            continue;
        }
        let s = l * (1.0 + w / 5.0);
        if best.is_none_or(|bb| s > bb.1) {
            best = Some((i, s));
        }
    }
    let (fi, _) = best?;
    let (a, b) = piece.edge(fi);
    let out_n = outward(a, b);
    let depth = style.depth * rng.range_f32(1.15, 1.3);
    let pts = clip_half(&piece.pts, a - out_n * depth, out_n);
    let mut pts = tidy(pts, 0.4);
    if pts.len() < 3 {
        return None;
    }
    if style.side_gap > 0.0 {
        // Detached buildings: pull every edge except the frontage back.
        let g = style.side_gap * rng.range_f32(0.5, 1.2) * 0.5;
        let n = pts.len();
        let edges: Vec<(Vec2, Vec2)> = (0..n).map(|i| (pts[i], pts[(i + 1) % n])).collect();
        let on_front = |p: Vec2| (p - a).dot(out_n).abs() < 0.05;
        for (p, q) in edges {
            if on_front(p) && on_front(q) {
                continue;
            }
            let Some(along) = (q - p).try_normalize() else { continue };
            let inward = Vec2::new(-along.y, along.x);
            pts = clip_half(&pts, p + inward * g, inward);
            if pts.len() < 3 {
                return None;
            }
        }
    }
    let pts = inset_convex(&pts, rng.range_f32(0.3, 0.6));
    if pts.len() < 3 {
        return None;
    }
    let pts = tidy(pts, 0.3);
    Some(Lot { pts, front: if invert_front { -out_n } else { out_n }, side: piece.tags[fi] })
}

/// Footprint sanity: CCW, big enough, not a sliver, and no small triangles
/// (awkward to fill with rooms).
pub(crate) fn lot_ok(pts: &[Vec2]) -> bool {
    let area = signed_area(pts);
    pts.len() >= 3
        && area >= MIN_LOT_AREA
        && (pts.len() > 3 || area >= 40.0)
        && min_width(pts) >= MIN_LOT_WIDTH
        && min_angle_deg(pts) >= 32.0
}

/// Clip a convex lot so it keeps clear of every obstacle (streets, walls,
/// towers). Returns `None` if the lot intersects one outright or vanishes.
/// Polygons in `obs` (other buildings, plaza) only reject, never clip.
pub(crate) fn clear_of(mut pts: Vec<Vec2>, obs: &mut Obstacles) -> Option<Vec<Vec2>> {
    for _ in 0..8 {
        if pts.len() < 3 {
            return None;
        }
        let mut lo = Vec2::splat(f32::INFINITY);
        let mut hi = Vec2::splat(f32::NEG_INFINITY);
        for p in &pts {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let ids = obs.near(lo - Vec2::splat(8.0), hi + Vec2::splat(8.0));
        let mut changed = false;
        for id in ids {
            match &obs.items[id as usize] {
                Shape::Seg { a, b, r } => {
                    if poly_seg_dist(&pts, *a, *b) <= 1e-4 {
                        return None;
                    }
                    let (p, q) = poly_seg_closest(&pts, *a, *b);
                    let d = p.distance(q);
                    if d < *r {
                        let n = (p - q) / d.max(1e-6);
                        pts = clip_half(&pts, q + n * *r, n);
                        changed = true;
                        if pts.len() < 3 {
                            return None;
                        }
                    }
                }
                Shape::Circle { c, r } => {
                    if contains(&pts, *c) {
                        return None;
                    }
                    let n = pts.len();
                    let p = (0..n)
                        .map(|i| mde_core::poly::closest_point_on_segment(*c, pts[i], pts[(i + 1) % n]))
                        .min_by(|x, y| x.distance_squared(*c).total_cmp(&y.distance_squared(*c)))
                        .unwrap_or(pts[0]);
                    let d = p.distance(*c);
                    if d < *r {
                        let nn = (p - *c) / d.max(1e-6);
                        pts = clip_half(&pts, *c + nn * *r, nn);
                        changed = true;
                        if pts.len() < 3 {
                            return None;
                        }
                    }
                }
                Shape::Poly { pts: o } => {
                    if convex_overlap(&pts, o, -0.3) {
                        return None;
                    }
                }
            }
        }
        if !changed {
            return Some(tidy(pts, 0.3));
        }
    }
    None
}

/// Inputs for building per-cell blocks.
pub(crate) struct BlockEnv<'a> {
    pub topo: &'a Topo,
    pub net: &'a [Option<StreetKind>],
    pub class: &'a [EdgeClass],
    pub walls: bool,
    /// Ring edges with an intramural lane running along their inner side.
    pub lane: &'a BTreeSet<usize>,
    /// Inner-city flag per cell (only inner cells front onto the lane).
    pub inner: &'a [bool],
}

impl BlockEnv<'_> {
    /// Street width on side `i` of cell `c` (0 if none).
    pub fn side_width(&self, c: usize, i: usize) -> f32 {
        let e = self.topo.cell_sides[c][i];
        if self.on_lane(c, e) { ALLEY_WIDTH } else { width_on(self.net, e) }
    }

    /// Does side edge `e` of cell `c` face the intramural lane?
    fn on_lane(&self, c: usize, e: usize) -> bool {
        self.inner[c] && self.lane.contains(&e)
    }

    /// Class of side `i` of cell `c`.
    pub fn side_class(&self, c: usize, i: usize) -> EdgeClass {
        self.class[self.topo.cell_sides[c][i]]
    }

    /// The buildable block of cell `c`: the cell inset per side by half the
    /// street width plus `front_margin` (or `back_margin` / wall clearance),
    /// with corners cut back from street junctions. Edge tags are cell sides.
    pub fn block(&self, c: usize, front_margin: f32, back_margin: f32, castle_inside: bool) -> Option<Piece> {
        let topo = self.topo;
        let cell = &topo.cells[c];
        let m = cell.len();
        let mut piece = Piece::from_pts(topo.pts(c));
        for i in 0..m {
            let w = self.side_width(c, i);
            let d = match self.side_class(c, i) {
                EdgeClass::Ring if self.walls => {
                    if self.on_lane(c, topo.cell_sides[c][i]) {
                        LANE_OFFSET + ALLEY_WIDTH * 0.5 + front_margin
                    } else {
                        WALL_THICKNESS * 0.5 + 1.5
                    }
                }
                EdgeClass::Castle => {
                    if castle_inside {
                        CASTLE_THICKNESS * 0.5 + 0.6
                    } else {
                        CASTLE_THICKNESS * 0.5 + 1.5
                    }
                }
                EdgeClass::Boundary => 1.0,
                _ => {
                    if w > 0.0 {
                        w * 0.5 + front_margin
                    } else {
                        back_margin
                    }
                }
            };
            let a = topo.verts[cell[i]];
            let b = topo.verts[cell[(i + 1) % m]];
            let dir = (b - a).normalize_or_zero();
            if dir == Vec2::ZERO {
                continue;
            }
            let inward = Vec2::new(-dir.y, dir.x);
            piece = piece.clip(a + inward * d, inward, i as i32);
            if piece.pts.len() < 3 {
                return None;
            }
        }
        // Corner cuts at junctions where streets meet.
        for i in 0..m {
            let v = cell[i];
            let h = topo.vert_edges[v].iter().map(|&e| width_on(self.net, e) * 0.5).fold(0.0f32, f32::max);
            if h <= 0.0 {
                continue;
            }
            let pv = topo.verts[v];
            let dp = (topo.verts[cell[(i + m - 1) % m]] - pv).normalize_or_zero();
            let dn = (topo.verts[cell[(i + 1) % m]] - pv).normalize_or_zero();
            let bis = (dp + dn).normalize_or_zero();
            if bis == Vec2::ZERO {
                continue;
            }
            piece = piece.clip(pv + bis * (h + front_margin * 0.8), bis, CORNER);
            if piece.pts.len() < 3 {
                return None;
            }
        }
        (piece.area() > 1.0).then_some(piece)
    }
}

/// Is `p` inside the convex CCW ring `poly` by at least `margin`?
pub(crate) fn inside_by(poly: &[Vec2], p: Vec2, margin: f32) -> bool {
    let n = poly.len();
    (0..n).all(|i| {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        let dir = (b - a).normalize_or_zero();
        let inward = Vec2::new(-dir.y, dir.x);
        (p - a).dot(inward) >= margin
    })
}

/// Largest scaled copy (scale in `[min_scale, 1]`) of a `len × wid` rectangle
/// centred at `c` along `axis` that fits inside `poly` with `margin`.
pub(crate) fn fit_rect(
    poly: &[Vec2],
    c: Vec2,
    axis: Vec2,
    len: f32,
    wid: f32,
    margin: f32,
    min_scale: f32,
) -> Option<(Vec<Vec2>, f32)> {
    let fits = |s: f32| {
        let r = crate::geom::oriented_rect(c, axis, len * s, wid * s);
        r.iter().all(|&p| inside_by(poly, p, margin)).then_some(r)
    };
    fits(min_scale)?;
    if let Some(r) = fits(1.0) {
        return Some((r, 1.0));
    }
    let (mut lo, mut hi) = (min_scale, 1.0f32);
    for _ in 0..18 {
        let mid = (lo + hi) * 0.5;
        if fits(mid).is_some() {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    fits(lo).map(|r| (r, lo))
}

/// Pick the building kind for an ordinary lot.
pub(crate) fn pick_kind(ward: WardKind, faces_market: bool, dense: bool, rng: &mut Rng) -> BuildingKind {
    use BuildingKind as B;
    use WardKind as W;
    let opts: &[(B, f32)] = if faces_market || ward == W::Market {
        &[(B::Shop, 0.55), (B::Tavern, 0.18), (B::House, 0.27)]
    } else if !dense && !matches!(ward, W::Castle | W::Cathedral | W::Patriciate) {
        &[(B::House, 0.72), (B::Workshop, 0.14), (B::Farmhouse, 0.1), (B::Tavern, 0.04)]
    } else {
        match ward {
            W::Merchant => &[(B::Shop, 0.4), (B::House, 0.45), (B::Tavern, 0.1), (B::Workshop, 0.05)],
            W::Craftsmen => &[(B::Workshop, 0.4), (B::House, 0.52), (B::Tavern, 0.05), (B::Shop, 0.03)],
            W::Patriciate => &[(B::Mansion, 0.65), (B::House, 0.35)],
            W::Administration => &[(B::Mansion, 0.5), (B::House, 0.5)],
            W::Military => &[(B::Barracks, 0.75), (B::House, 0.25)],
            W::Slum => &[(B::House, 0.92), (B::Tavern, 0.04), (B::Workshop, 0.04)],
            W::Gate => &[(B::House, 0.5), (B::Tavern, 0.3), (B::Workshop, 0.2)],
            W::Castle => &[(B::Barracks, 0.55), (B::Tower, 0.15), (B::House, 0.3)],
            _ => &[(B::House, 1.0)],
        }
    };
    let w: Vec<f32> = opts.iter().map(|o| o.1).collect();
    opts[rng.weighted(&w)].0
}

/// Storeys for a building; `d` is the normalised distance from the centre.
/// Houses, shops, workshops and taverns: 1 floor (60 %), 2 (30 %) or 3 (10 %).
pub(crate) fn pick_floors(kind: BuildingKind, d: f32, rng: &mut Rng) -> u8 {
    use BuildingKind as B;
    match kind {
        B::Temple => 3,
        B::Keep => 4,
        B::Tower => 4,
        B::Barn => 1,
        B::Mill => 2,
        B::Barracks => 2,
        B::Farmhouse => 1 + rng.chance(0.4) as u8,
        B::Mansion => 2 + rng.chance(if d < 0.5 { 0.6 } else { 0.3 }) as u8,
        _ => 1 + rng.weighted(&[0.6, 0.3, 0.1]) as u8,
    }
}

/// Is the footprint clear of `others` (special buildings) by `gap`?
pub(crate) fn clear_of_specials(pts: &[Vec2], others: &[Vec<Vec2>], gap: f32) -> bool {
    others.iter().all(|o| !convex_overlap(pts, o, -gap))
}

/// Shape helper for obstacle sets.
pub(crate) fn seg(a: Vec2, b: Vec2, r: f32) -> Shape {
    Shape::Seg { a, b, r }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_keeps_tags() {
        let sq = Piece::from_pts(vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ]);
        let half = sq.clip(Vec2::new(5.0, 0.0), Vec2::new(-1.0, 0.0), CUT);
        assert!((half.area() - 50.0).abs() < 1e-3);
        assert!(half.tags.contains(&0) && half.tags.contains(&2) && half.tags.contains(&3));
        assert!(half.tags.contains(&CUT) && !half.tags.contains(&1));
    }

    #[test]
    fn subdivision_covers_frontage() {
        let sq = Piece::from_pts(vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(60.0, 0.0),
            Vec2::new(60.0, 50.0),
            Vec2::new(0.0, 50.0),
        ]);
        let style = LotStyle { frontage: 8.0, depth: 18.0, gap: 0.0, alley: 0.0, side_gap: 0.0 };
        let mut rng = Rng::new(1);
        let mut out = Vec::new();
        subdivide(sq, &|t| t >= 0, &style, &mut rng, &mut out, 0);
        assert!(out.len() > 10);
        let total: f32 = out.iter().map(|p| p.area()).sum();
        assert!(total < 3000.0);
        for p in &out {
            let lot = finish_lot(p, &|t| if t >= 0 { 3.5 } else { 0.0 }, &style, false, &mut rng);
            if let Some(l) = lot {
                assert!((l.front.length() - 1.0).abs() < 1e-4);
            }
        }
    }
}
