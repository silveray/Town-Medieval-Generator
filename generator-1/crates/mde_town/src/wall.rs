//! City wall ring, towers, gates and the citadel.

use std::collections::{BTreeMap, BTreeSet};
use std::f32::consts::{PI, TAU};

use mde_core::poly::cross;
use mde_core::{Rng, Vec2};

use crate::geom::signed_area;
use crate::topo::Topo;

/// City wall thickness (m).
pub(crate) const WALL_THICKNESS: f32 = 2.0;
/// Citadel wall thickness (m).
pub(crate) const CASTLE_THICKNESS: f32 = 2.5;
/// Footprint radius of a wall tower (m), used for clearances and drawing.
pub(crate) const TOWER_RADIUS: f32 = 3.2;
/// Distance from the wall line to the centreline of the intramural lane (m):
/// clear of the towers.
pub(crate) const LANE_OFFSET: f32 = TOWER_RADIUS + 1.0 + 0.5;

/// Outer boundary ring (vertex ids, CCW) of the union of `inside` cells.
/// Returns the ring with the largest area if the union is not a disc.
pub(crate) fn outer_ring(topo: &Topo, inside: &[bool]) -> Vec<usize> {
    let mut next: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (c, cell) in topo.cells.iter().enumerate() {
        if !inside[c] {
            continue;
        }
        let m = cell.len();
        for i in 0..m {
            let e = topo.cell_sides[c][i];
            let other = topo.edges[e].other_cell(c);
            if other.is_none_or(|o| !inside[o]) {
                next.entry(cell[i]).or_default().push(cell[(i + 1) % m]);
            }
        }
    }
    let mut used: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut best: (f32, Vec<usize>) = (0.0, Vec::new());
    for (&start, outs) in &next {
        for &first in outs {
            if used.contains(&(start, first)) {
                continue;
            }
            let mut ring = vec![start];
            let (mut cur, mut nxt) = (start, first);
            let mut closed = false;
            for _ in 0..100_000 {
                used.insert((cur, nxt));
                if nxt == start {
                    closed = true;
                    break;
                }
                ring.push(nxt);
                let cand = next.get(&nxt).and_then(|l| l.iter().copied().find(|&w| !used.contains(&(nxt, w))));
                match cand {
                    Some(w) => {
                        cur = nxt;
                        nxt = w;
                    }
                    None => break,
                }
            }
            if !closed || ring.len() < 3 {
                continue;
            }
            let pts: Vec<Vec2> = ring.iter().map(|&v| topo.verts[v]).collect();
            let a = signed_area(&pts);
            if a > best.0 {
                best = (a, ring);
            }
        }
    }
    best.1
}

/// Tower positions along a closed ring: at corners flagged in `corner_ok`
/// (keeping `min_spacing` between towers) plus evenly spaced intermediate
/// towers on stretches longer than `max_gap`.
pub(crate) fn towers_along(ring: &[Vec2], corner_ok: &[bool], min_spacing: f32, max_gap: f32) -> Vec<Vec2> {
    let n = ring.len();
    let mut corners: Vec<usize> = Vec::new();
    for i in 0..n {
        if !corner_ok[i] {
            continue;
        }
        if let Some(&last) = corners.last()
            && ring[last].distance(ring[i]) < min_spacing
        {
            continue;
        }
        corners.push(i);
    }
    while corners.len() > 1 {
        let (f, l) = (corners[0], corners[corners.len() - 1]);
        if ring[f].distance(ring[l]) < min_spacing {
            corners.pop();
        } else {
            break;
        }
    }
    let mut out: Vec<Vec2> = corners.iter().map(|&i| ring[i]).collect();
    // Intermediate towers on long stretches between consecutive anchors
    // (corner towers or skipped corners treated as path points).
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        let len = a.distance(b);
        if len > max_gap {
            let k = (len / max_gap).floor() as usize;
            for j in 1..=k {
                let p = a + (b - a) * (j as f32 / (k + 1) as f32);
                if out.iter().all(|q| q.distance(p) >= min_spacing * 0.8) {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// Does `v` have an edge whose both cells satisfy `pred`?
pub(crate) fn has_edge_between(topo: &Topo, v: usize, pred: impl Fn(usize) -> bool) -> bool {
    topo.vert_edges[v].iter().any(|&e| {
        let ed = &topo.edges[e];
        matches!((ed.left, ed.right), (Some(l), Some(r)) if pred(l) && pred(r))
    })
}

fn angle_of(p: Vec2) -> f32 {
    p.y.atan2(p.x)
}

fn angle_dist(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(TAU);
    d.min(TAU - d)
}

/// Choose up to `count` gate vertices on the ring, far apart in angle.
#[allow(clippy::too_many_arguments)]
pub(crate) fn choose_gates(
    topo: &Topo,
    ring: &[usize],
    inner: &[bool],
    castle: Option<usize>,
    centre_cell: usize,
    centre: Vec2,
    wet: &dyn Fn(Vec2) -> bool,
    count: usize,
    rng: &mut Rng,
) -> Vec<usize> {
    let castle_verts: BTreeSet<usize> = castle.map(|c| topo.cells[c].iter().copied().collect()).unwrap_or_default();
    let centre_verts: BTreeSet<usize> = topo.cells[centre_cell].iter().copied().collect();
    let civic = |c: usize| inner[c] && Some(c) != castle;
    let mut cands: Vec<(usize, f32)> = Vec::new();
    for &v in ring {
        if castle_verts.contains(&v) || wet(topo.verts[v]) {
            continue;
        }
        let inward = has_edge_between(topo, v, civic) || centre_verts.contains(&v);
        if !inward {
            continue;
        }
        // At least one outer cell must touch the vertex so a road can leave.
        if !topo.cells_at(v).iter().any(|&c| !inner[c]) {
            continue;
        }
        let open = has_edge_between(topo, v, |c| !inner[c]);
        cands.push((v, if open { 1.35 } else { 1.0 }));
    }
    if cands.is_empty() {
        return Vec::new();
    }
    let mut chosen: Vec<usize> = Vec::new();
    let first = {
        let w: Vec<f32> = cands.iter().map(|c| c.1).collect();
        cands[rng.weighted(&w)].0
    };
    chosen.push(first);
    let min_sep = TAU / (count as f32 * 1.9);
    while chosen.len() < count {
        let mut best: Option<(usize, f32)> = None;
        for &(v, bonus) in &cands {
            if chosen.contains(&v) {
                continue;
            }
            let a = angle_of(topo.verts[v] - centre);
            let sep = chosen.iter().map(|&g| angle_dist(a, angle_of(topo.verts[g] - centre))).fold(PI, f32::min);
            if sep < min_sep {
                continue;
            }
            let score = sep * bonus * rng.range_f32(0.85, 1.15);
            if best.is_none_or(|b| score > b.1) {
                best = Some((v, score));
            }
        }
        match best {
            Some((v, _)) => chosen.push(v),
            None => break,
        }
    }
    chosen
}

/// Make sure a road can leave gate `v` outwards: if no edge between two outer
/// cells starts at `v`, split the outer cell cornered at `v` with a chord from
/// `v`. Returns the id of the new cell, if one was created.
pub(crate) fn open_gate(topo: &mut Topo, inner: &[bool], v: usize, centre: Vec2) -> Option<usize> {
    if has_edge_between(topo, v, |c| c < inner.len() && !inner[c]) {
        return None;
    }
    let o = topo.cells_at(v).into_iter().find(|&c| c < inner.len() && !inner[c])?;
    let cell = topo.cells[o].clone();
    let m = cell.len();
    let i = cell.iter().position(|&x| x == v)?;
    let pv = topo.verts[v];
    let prev = topo.verts[cell[(i + m - 1) % m]];
    let next = topo.verts[cell[(i + 1) % m]];
    let dp = (prev - pv).normalize_or_zero();
    let dn = (next - pv).normalize_or_zero();
    let mut dir = (dp + dn).normalize_or_zero();
    if dir == Vec2::ZERO {
        return None;
    }
    let radial = (pv - centre).normalize_or_zero();
    // Inside the convex corner wedge (between dn and dp, CCW)?
    if cross(dn, radial) > 0.0 && cross(radial, dp) > 0.0 {
        dir = (dir + radial).normalize_or_zero();
    }
    // Ray cast against the far edges.
    let mut hit: Option<(usize, f32, f32)> = None; // (edge k, t along ray, s along edge)
    for k in 0..m {
        if k == i || (k + 1) % m == i {
            continue;
        }
        let a = topo.verts[cell[k]];
        let b = topo.verts[cell[(k + 1) % m]];
        let e = b - a;
        let den = cross(dir, e);
        if den.abs() < 1e-9 {
            continue;
        }
        let t = cross(a - pv, e) / den;
        let s = cross(a - pv, dir) / den;
        if t > 0.0 && (-1e-4..=1.0 + 1e-4).contains(&s) && hit.is_none_or(|h| t < h.1) {
            hit = Some((k, t, s.clamp(0.0, 1.0)));
        }
    }
    let (k, _, s) = hit?;
    let ka = (k + m) % m;
    let kb = (k + 1) % m;
    let a = topo.verts[cell[ka]];
    let b = topo.verts[cell[kb]];
    let elen = a.distance(b);
    // Snap to an existing vertex when close (and not adjacent to `v`).
    let snap_ok = |idx: usize| idx != (i + 1) % m && idx != (i + m - 1) % m && idx != i;
    let target_local = if (s * elen < 6.0 || s < 0.15) && snap_ok(ka) {
        Some(ka)
    } else if ((1.0 - s) * elen < 6.0 || s > 0.85) && snap_ok(kb) {
        Some(kb)
    } else {
        None
    };
    // Check both halves are reasonable before editing.
    let total = signed_area(&topo.pts(o));
    let half_area = |j: usize, w: Vec2| {
        let mut pts = vec![pv];
        let mut idx = (i + 1) % m;
        loop {
            pts.push(topo.verts[cell[idx]]);
            if idx == j {
                break;
            }
            idx = (idx + 1) % m;
        }
        pts.push(w);
        signed_area(&crate::geom::dedupe(pts, 1e-3))
    };
    match target_local {
        Some(j) => {
            let a1 = half_area((j + m - 1) % m, topo.verts[cell[j]]);
            if a1 < 0.15 * total || total - a1 < 0.15 * total {
                return None;
            }
            let new = topo.split_cell(o, i, j);
            topo.rebuild();
            Some(new)
        }
        None => {
            let w = a + (b - a) * s;
            let a1 = half_area(ka, w);
            if a1 < 0.15 * total || total - a1 < 0.15 * total {
                return None;
            }
            let wid = topo.insert_on_edge(cell[ka], cell[kb], w);
            let cell2 = &topo.cells[o];
            let ii = cell2.iter().position(|&x| x == v)?;
            let jj = cell2.iter().position(|&x| x == wid)?;
            let new = topo.split_cell(o, ii, jj);
            topo.rebuild();
            Some(new)
        }
    }
}

/// Pick the citadel cell: an inner cell on the city edge, compact and dry.
#[allow(clippy::too_many_arguments)]
pub(crate) fn choose_citadel(
    topo: &Topo,
    inner: &[bool],
    centre_cell: usize,
    river_clear: &dyn Fn(&[Vec2]) -> bool,
    rng: &mut Rng,
) -> Option<usize> {
    let inner_cells: Vec<usize> = (0..topo.cells.len()).filter(|&c| inner[c]).collect();
    if inner_cells.len() < 3 {
        return None;
    }
    let mut areas: Vec<f32> = inner_cells.iter().map(|&c| signed_area(&topo.pts(c))).collect();
    areas.sort_by(f32::total_cmp);
    let median = areas[areas.len() / 2];
    let centre_nb = topo.neighbours(centre_cell);
    let mut best: Option<(usize, f32)> = None;
    for &c in &inner_cells {
        if c == centre_cell || (inner_cells.len() > 6 && centre_nb.contains(&c)) {
            continue;
        }
        let on_edge = topo.cell_sides[c].iter().any(|&e| topo.edges[e].other_cell(c).is_none_or(|o| !inner[o]));
        if !on_edge {
            continue;
        }
        let pts = topo.pts(c);
        let a = signed_area(&pts);
        if a < 0.4 * median || a > 2.6 * median || !river_clear(&pts) {
            continue;
        }
        let per: f32 = (0..pts.len()).map(|i| pts[i].distance(pts[(i + 1) % pts.len()])).sum();
        let compact = 4.0 * PI * a / (per * per);
        let score = compact * rng.range_f32(0.7, 1.3);
        if best.is_none_or(|b| score > b.1) {
            best = Some((c, score));
        }
    }
    best.map(|b| b.0)
}

/// Citadel gate: a vertex of the castle cell facing the city (not on the
/// outer ring when walled) with a street edge leading into town; closest to
/// the centre.
pub(crate) fn castle_gate(
    topo: &Topo,
    inner: &[bool],
    castle: usize,
    ring: &BTreeSet<usize>,
    walls: bool,
    centre: Vec2,
) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for &v in &topo.cells[castle] {
        if walls && ring.contains(&v) {
            continue;
        }
        let ok = has_edge_between(topo, v, |c| c != castle && (!walls || inner[c]));
        if !ok {
            continue;
        }
        let d = topo.verts[v].distance(centre);
        if best.is_none_or(|b| d < b.1) {
            best = Some((v, d));
        }
    }
    best.map(|b| b.0)
}
