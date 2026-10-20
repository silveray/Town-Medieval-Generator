//! Ear-clipping triangulation of simple polygons (optionally with holes).

use mde_core::poly::cross;
use mde_core::{Polygon, Vec2};

/// Triangulate a simple polygon (convex or concave, either winding) by ear
/// clipping.
///
/// Returns index triples into `poly.0`, each counter-clockwise in plan space, so
/// the triangles' areas add up to the polygon's area. Collinear vertices are
/// skipped without producing slivers. Degenerate input (fewer than three points,
/// non-finite coordinates, zero area) gives an empty list; self-intersecting
/// input still terminates with a best-effort result.
pub fn triangulate(poly: &Polygon) -> Vec<[usize; 3]> {
    triangulate_points(&poly.0)
}

/// [`triangulate`] on a bare point slice.
pub(crate) fn triangulate_points(pts: &[Vec2]) -> Vec<[usize; 3]> {
    let n = pts.len();
    if n < 3 || pts.iter().any(|p| !p.is_finite()) {
        return Vec::new();
    }
    let area2: f32 = (0..n).map(|i| cross(pts[i], pts[(i + 1) % n])).sum();
    let (min, max) = bounds(pts);
    let scale = (max - min).length().max(1e-6);
    // Area tolerance relative to the polygon's size.
    let eps = scale * scale * 1e-9;
    if area2.abs() <= eps {
        return Vec::new();
    }
    let mut idx: Vec<usize> = if area2 > 0.0 { (0..n).collect() } else { (0..n).rev().collect() };
    let mut out = Vec::with_capacity(n.saturating_sub(2));
    let mut start = 0usize;
    while idx.len() > 3 {
        let m = idx.len();
        let mut clipped = false;
        for step in 0..m {
            let k = (start + step) % m;
            let i0 = idx[(k + m - 1) % m];
            let i1 = idx[k];
            let i2 = idx[(k + 1) % m];
            let (a, b, c) = (pts[i0], pts[i1], pts[i2]);
            if cross(b - a, c - b) <= eps {
                continue;
            }
            if idx.iter().any(|&j| {
                if j == i0 || j == i1 || j == i2 {
                    return false;
                }
                let p = pts[j];
                if p == a || p == b || p == c {
                    return false;
                }
                in_triangle(p, a, b, c, eps)
            }) {
                continue;
            }
            out.push([i0, i1, i2]);
            idx.remove(k);
            start = k % idx.len();
            clipped = true;
            break;
        }
        if clipped {
            continue;
        }
        // No proper ear (collinear runs or numerical trouble). Drop a
        // collinear vertex if there is one, else clip the most convex vertex.
        let m = idx.len();
        let mut best = None;
        let mut best_cross = f32::NEG_INFINITY;
        let mut collinear = None;
        for k in 0..m {
            let a = pts[idx[(k + m - 1) % m]];
            let b = pts[idx[k]];
            let c = pts[idx[(k + 1) % m]];
            let z = cross(b - a, c - b);
            if z.abs() <= eps {
                collinear = Some(k);
                break;
            }
            if z > best_cross {
                best_cross = z;
                best = Some(k);
            }
        }
        if let Some(k) = collinear {
            idx.remove(k);
        } else if let Some(k) = best {
            let m = idx.len();
            out.push([idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]]);
            idx.remove(k);
        } else {
            break;
        }
        start = 0;
    }
    if idx.len() == 3 {
        let (a, b, c) = (pts[idx[0]], pts[idx[1]], pts[idx[2]]);
        if cross(b - a, c - b) > eps {
            out.push([idx[0], idx[1], idx[2]]);
        }
    }
    out
}

/// Triangulate `outer` (any winding) minus the `holes` (any winding, each
/// strictly inside `outer` and not overlapping each other).
///
/// Holes are joined to the outer ring with bridge edges (Eberly's method) and
/// the result is ear-clipped. Returns the merged point list and CCW triangles
/// indexing it. Holes that cannot be bridged are ignored.
pub(crate) fn triangulate_with_holes(outer: &[Vec2], holes: &[Vec<Vec2>]) -> (Vec<Vec2>, Vec<[usize; 3]>) {
    let mut ring: Vec<Vec2> = outer.to_vec();
    if signed_area(&ring) < 0.0 {
        ring.reverse();
    }
    let mut hs: Vec<Vec<Vec2>> = holes
        .iter()
        .filter(|h| h.len() >= 3 && h.iter().all(|p| p.is_finite()) && signed_area(h).abs() > 1e-8)
        .map(|h| {
            let mut h = h.clone();
            // Holes run clockwise.
            if signed_area(&h) > 0.0 {
                h.reverse();
            }
            h
        })
        .collect();
    hs.sort_by(|a, b| max_x(b).total_cmp(&max_x(a)));
    for h in &hs {
        if let Some(merged) = bridge_hole(&ring, h) {
            ring = merged;
        }
    }
    let tris = triangulate_points(&ring);
    (ring, tris)
}

fn max_x(p: &[Vec2]) -> f32 {
    p.iter().map(|v| v.x).fold(f32::NEG_INFINITY, f32::max)
}

fn signed_area(p: &[Vec2]) -> f32 {
    let n = p.len();
    (0..n).map(|i| cross(p[i], p[(i + 1) % n])).sum::<f32>() * 0.5
}

fn bounds(pts: &[Vec2]) -> (Vec2, Vec2) {
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    for &p in pts {
        min = min.min(p);
        max = max.max(p);
    }
    (min, max)
}

/// Point in CCW triangle, boundary inclusive.
fn in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2, eps: f32) -> bool {
    cross(b - a, p - a) >= -eps && cross(c - b, p - b) >= -eps && cross(a - c, p - c) >= -eps
}

/// Splice a clockwise hole into a CCW ring through a mutually visible vertex pair.
fn bridge_hole(ring: &[Vec2], hole: &[Vec2]) -> Option<Vec<Vec2>> {
    let (mi, &m) = hole.iter().enumerate().max_by(|a, b| a.1.x.total_cmp(&b.1.x))?;
    let n = ring.len();
    // Closest intersection of the ray M → +x with the ring.
    let mut best: Option<(f32, usize)> = None;
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        // Edges going downwards-to-upwards on a CCW ring face the hole from the right.
        if (a.y > m.y) == (b.y > m.y) {
            continue;
        }
        let t = (m.y - a.y) / (b.y - a.y);
        let x = a.x + t * (b.x - a.x);
        if x >= m.x && best.is_none_or(|(bx, _)| x < bx) {
            best = Some((x, i));
        }
    }
    let (ix, ei) = best?;
    let i_pt = Vec2::new(ix, m.y);
    let a = ring[ei];
    let b = ring[(ei + 1) % n];
    let mut cand = if a.x > b.x { ei } else { (ei + 1) % n };
    if a.distance(i_pt) < 1e-6 {
        cand = ei;
    } else if b.distance(i_pt) < 1e-6 {
        cand = (ei + 1) % n;
    } else {
        // Reflex vertices inside triangle (M, I, P) may block the view; take the
        // one with the smallest angle to the ray.
        let p = ring[cand];
        let (t0, t1, t2) = if cross(i_pt - m, p - m) >= 0.0 { (m, i_pt, p) } else { (m, p, i_pt) };
        let mut best_angle = f32::INFINITY;
        let mut best_dist = f32::INFINITY;
        for j in 0..n {
            if j == cand {
                continue;
            }
            let q = ring[j];
            let prev = ring[(j + n - 1) % n];
            let next = ring[(j + 1) % n];
            let reflex = cross(q - prev, next - q) < 0.0;
            if !reflex || !in_triangle(q, t0, t1, t2, 0.0) {
                continue;
            }
            let d = q - m;
            let angle = d.y.abs().atan2(d.x);
            let dist = d.length();
            if angle < best_angle - 1e-6 || ((angle - best_angle).abs() <= 1e-6 && dist < best_dist) {
                best_angle = angle;
                best_dist = dist;
                cand = j;
            }
        }
    }
    let mut out = Vec::with_capacity(n + hole.len() + 2);
    out.extend_from_slice(&ring[..=cand]);
    for k in 0..hole.len() {
        out.push(hole[(mi + k) % hole.len()]);
    }
    out.push(m);
    out.push(ring[cand]);
    out.extend_from_slice(&ring[cand + 1..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri_area(p: &[Vec2], t: [usize; 3]) -> f32 {
        cross(p[t[1]] - p[t[0]], p[t[2]] - p[t[0]]) * 0.5
    }

    #[test]
    fn square_with_hole() {
        let outer = Polygon::rect(Vec2::ZERO, Vec2::splat(10.0)).0;
        let hole = Polygon::rect(Vec2::new(1.0, 1.0), Vec2::new(2.0, 3.0)).0;
        let (pts, tris) = triangulate_with_holes(&outer, &[hole]);
        let a: f32 = tris.iter().map(|&t| tri_area(&pts, t)).sum();
        assert!((a - 94.0).abs() < 1e-3, "{a}");
        assert!(tris.iter().all(|&t| tri_area(&pts, t) > 0.0));
    }

    #[test]
    fn degenerate_inputs() {
        assert!(triangulate(&Polygon(vec![])).is_empty());
        assert!(triangulate(&Polygon(vec![Vec2::ZERO, Vec2::X, Vec2::new(2.0, 0.0)])).is_empty());
        assert!(triangulate(&Polygon(vec![Vec2::ZERO, Vec2::X, Vec2::new(f32::NAN, 1.0)])).is_empty());
    }
}
