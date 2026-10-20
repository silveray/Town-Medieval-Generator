//! Bowyer–Watson Delaunay triangulation and its dual, a Voronoi diagram clipped
//! exactly to an axis-aligned square.
//!
//! Everything runs in `f64` for robustness; the town generator converts to the
//! `f32` plan-space types of `mde_core` afterwards.
//!
//! Clipping uses the *mirror trick*: every site is reflected across the four
//! sides of the box. The bisector between a site and its reflection is the box
//! side itself, so the Voronoi cells of the real sites come out already clipped
//! to the box, and every cell vertex is a Delaunay circumcentre. Cells therefore
//! share vertices **by index**, which keeps adjacency exact (no float-equality
//! matching of edges).

use std::collections::BTreeMap;

/// A point / vector in `f64`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    fn sub(self, o: Pt) -> Pt {
        Pt::new(self.x - o.x, self.y - o.y)
    }

    fn len2(self) -> f64 {
        self.x * self.x + self.y * self.y
    }
}

/// Delaunay triangle: indices into the input points, counter-clockwise.
pub type Triangle = [usize; 3];

#[derive(Clone, Debug)]
struct Tri {
    v: [usize; 3],
    c: Pt,
    r2: f64,
}

/// Circumcentre of `a, b, c`, or `None` if they are (nearly) collinear.
pub fn circumcentre(a: Pt, b: Pt, c: Pt) -> Option<Pt> {
    let bx = b.x - a.x;
    let by = b.y - a.y;
    let cx = c.x - a.x;
    let cy = c.y - a.y;
    let d = 2.0 * (bx * cy - by * cx);
    let scale = (bx * bx + by * by).max(cx * cx + cy * cy);
    if d.abs() <= 1e-12 * scale.max(1e-300) {
        return None;
    }
    let b2 = bx * bx + by * by;
    let c2 = cx * cx + cy * cy;
    let ux = (cy * b2 - by * c2) / d;
    let uy = (bx * c2 - cx * b2) / d;
    Some(Pt::new(a.x + ux, a.y + uy))
}

fn make_tri(v: [usize; 3], pts: &[Pt]) -> Tri {
    let (a, b, c) = (pts[v[0]], pts[v[1]], pts[v[2]]);
    let orient = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let v = if orient < 0.0 { [v[0], v[2], v[1]] } else { v };
    match circumcentre(a, b, c) {
        Some(cc) => Tri { v, c: cc, r2: cc.sub(a).len2() },
        // Degenerate sliver: make it "contain" everything so the next insertion
        // nearby re-triangulates it.
        None => Tri { v, c: a, r2: f64::INFINITY },
    }
}

/// Delaunay triangulation of `points` (Bowyer–Watson with a super-triangle).
///
/// Duplicate points are skipped. Returns CCW triangles over the input indices.
pub fn delaunay(points: &[Pt]) -> Vec<Triangle> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    let (mut lo, mut hi) = (Pt::new(f64::INFINITY, f64::INFINITY), Pt::new(f64::NEG_INFINITY, f64::NEG_INFINITY));
    for p in points {
        lo = Pt::new(lo.x.min(p.x), lo.y.min(p.y));
        hi = Pt::new(hi.x.max(p.x), hi.y.max(p.y));
    }
    let d = (hi.x - lo.x).max(hi.y - lo.y).max(1.0);
    let mid = Pt::new((lo.x + hi.x) * 0.5, (lo.y + hi.y) * 0.5);
    let mut pts = points.to_vec();
    pts.push(Pt::new(mid.x - 60.0 * d, mid.y - 40.0 * d));
    pts.push(Pt::new(mid.x + 60.0 * d, mid.y - 40.0 * d));
    pts.push(Pt::new(mid.x, mid.y + 60.0 * d));

    let mut tris = vec![make_tri([n, n + 1, n + 2], &pts)];
    let mut bad: Vec<usize> = Vec::new();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for i in 0..n {
        let p = pts[i];
        bad.clear();
        for (ti, t) in tris.iter().enumerate() {
            let d2 = p.sub(t.c).len2();
            if d2 < t.r2 * (1.0 - 1e-12) {
                bad.push(ti);
            }
        }
        if bad.is_empty() {
            continue;
        }
        edges.clear();
        for &ti in &bad {
            let v = tris[ti].v;
            for k in 0..3 {
                edges.push((v[k], v[(k + 1) % 3]));
            }
        }
        let boundary: Vec<(usize, usize)> = edges.iter().filter(|&&(a, b)| !edges.contains(&(b, a))).copied().collect();
        // `bad` is ascending; removing from the back keeps indices valid.
        for &ti in bad.iter().rev() {
            tris.swap_remove(ti);
        }
        for (a, b) in boundary {
            if a == i || b == i {
                continue;
            }
            tris.push(make_tri([a, b, i], &pts));
        }
    }
    tris.into_iter().filter(|t| t.v.iter().all(|&v| v < n)).map(|t| t.v).collect()
}

/// Voronoi diagram clipped to the square `[-half, half]²`.
#[derive(Clone, Debug, Default)]
pub struct Diagram {
    /// Shared cell vertices.
    pub vertices: Vec<Pt>,
    /// One CCW ring of vertex indices per input site (same order as the sites).
    /// A site whose cell degenerated gets an empty ring.
    pub cells: Vec<Vec<usize>>,
}

/// Voronoi cells of `sites`, clipped exactly to the square `[-half, half]²`.
///
/// Sites must lie strictly inside the box and be pairwise distinct (callers
/// dedupe near-coincident points first). Vertices closer than a tiny tolerance
/// are merged, so cells may have vertices of degree four or more.
pub fn voronoi_box(sites: &[Pt], half: f64) -> Diagram {
    let n = sites.len();
    if n == 0 {
        return Diagram::default();
    }
    let mut pts = Vec::with_capacity(n * 5);
    pts.extend_from_slice(sites);
    for s in sites {
        pts.push(Pt::new(2.0 * half - s.x, s.y));
        pts.push(Pt::new(-2.0 * half - s.x, s.y));
        pts.push(Pt::new(s.x, 2.0 * half - s.y));
        pts.push(Pt::new(s.x, -2.0 * half - s.y));
    }
    let tris = delaunay(&pts);

    let mut incident: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut used = vec![false; tris.len()];
    let mut centres = Vec::with_capacity(tris.len());
    for (ti, t) in tris.iter().enumerate() {
        let c = circumcentre(pts[t[0]], pts[t[1]], pts[t[2]]);
        centres.push(c);
        if c.is_none() {
            continue;
        }
        for &v in t {
            if v < n {
                incident[v].push(ti);
                used[ti] = true;
            }
        }
    }

    // Merge circumcentres that coincide (cocircular sites are guaranteed by the
    // mirror construction near the box sides).
    let eps = 1e-7 * half.max(1.0);
    let key = |p: Pt| ((p.x / eps).floor() as i64, (p.y / eps).floor() as i64);
    let mut buckets: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
    let mut vertices: Vec<Pt> = Vec::new();
    let mut vid = vec![usize::MAX; tris.len()];
    for ti in 0..tris.len() {
        if !used[ti] {
            continue;
        }
        let Some(mut p) = centres[ti] else { continue };
        // Snap onto the box sides so boundary vertices are exact.
        for coord in [&mut p.x, &mut p.y] {
            if (*coord - half).abs() < eps * 10.0 {
                *coord = half;
            } else if (*coord + half).abs() < eps * 10.0 {
                *coord = -half;
            }
            *coord = coord.clamp(-half, half);
        }
        let (kx, ky) = key(p);
        let mut found = None;
        'search: for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(list) = buckets.get(&(kx + dx, ky + dy)) {
                    for &v in list {
                        if vertices[v].sub(p).len2() <= eps * eps {
                            found = Some(v);
                            break 'search;
                        }
                    }
                }
            }
        }
        vid[ti] = match found {
            Some(v) => v,
            None => {
                vertices.push(p);
                buckets.entry((kx, ky)).or_default().push(vertices.len() - 1);
                vertices.len() - 1
            }
        };
    }

    let mut cells = Vec::with_capacity(n);
    for (s, inc) in incident.iter().enumerate() {
        let site = sites[s];
        let mut ring: Vec<(f64, usize)> = inc
            .iter()
            .filter(|&&ti| vid[ti] != usize::MAX)
            .map(|&ti| {
                let c = vertices[vid[ti]];
                ((c.y - site.y).atan2(c.x - site.x), vid[ti])
            })
            .collect();
        ring.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut cell: Vec<usize> = Vec::with_capacity(ring.len());
        for (_, v) in ring {
            if cell.last() != Some(&v) && !cell.contains(&v) {
                cell.push(v);
            }
        }
        if cell.len() < 3 {
            cell.clear();
        }
        cells.push(cell);
    }
    Diagram { vertices, cells }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell_area(d: &Diagram, c: usize) -> f64 {
        let ring = &d.cells[c];
        let m = ring.len();
        (0..m)
            .map(|i| {
                let a = d.vertices[ring[i]];
                let b = d.vertices[ring[(i + 1) % m]];
                a.x * b.y - a.y * b.x
            })
            .sum::<f64>()
            * 0.5
    }

    #[test]
    fn delaunay_square() {
        let pts = [Pt::new(0.0, 0.0), Pt::new(1.0, 0.0), Pt::new(1.0, 1.0), Pt::new(0.0, 1.1)];
        let t = delaunay(&pts);
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn cells_tile_the_box() {
        // Pseudo-random sites; the clipped cells must exactly tile the box.
        let mut state = 12345u64;
        let mut next = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 11) as f64 / (1u64 << 53) as f64) * 180.0 - 90.0
        };
        let sites: Vec<Pt> = (0..120).map(|_| Pt::new(next(), next())).collect();
        let d = voronoi_box(&sites, 100.0);
        let total: f64 = (0..sites.len()).map(|c| cell_area(&d, c)).sum();
        assert!((total - 40_000.0).abs() < 1e-3, "total {total}");
        for c in 0..sites.len() {
            assert!(cell_area(&d, c) > 0.0, "cell {c} not CCW");
        }
        // Every interior edge is shared by exactly two cells.
        let mut count: BTreeMap<(usize, usize), u32> = BTreeMap::new();
        for ring in &d.cells {
            for i in 0..ring.len() {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                *count.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        for (&(a, b), &k) in &count {
            let on_box = |p: Pt| p.x.abs() >= 100.0 - 1e-9 || p.y.abs() >= 100.0 - 1e-9;
            if k == 1 {
                assert!(on_box(d.vertices[a]) && on_box(d.vertices[b]));
            } else {
                assert_eq!(k, 2);
            }
        }
    }
}
