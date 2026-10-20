//! Low-level mesh building: flat-shaded polygons, boxes, cylinders and blobs
//! with planar world UVs and linear vertex colours, grouped per material slot.

use std::collections::BTreeMap;

use mde_core::{plan_to_world, MaterialSlot, MeshData, Model, Rng, Vec2, Vec3};

use crate::palette::{clamp01, scale, Linear};
use crate::triangulate::triangulate_points;

/// Geometry grouped per material slot; turned into a [`Model`] at the end.
#[derive(Default)]
pub(crate) struct Mb {
    parts: BTreeMap<MaterialSlot, MeshData>,
}

impl Mb {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mesh of `slot`, created on first use.
    pub fn m(&mut self, slot: MaterialSlot) -> &mut MeshData {
        self.parts.entry(slot).or_default()
    }

    /// Append every part of `model`.
    pub fn append_model(&mut self, model: &Model) {
        for p in &model.parts {
            self.m(p.material).append(&p.mesh);
        }
    }

    /// Append every part of `model`, transformed by `f` (positions) and `g` (normals).
    pub fn append_transformed(&mut self, model: &Model, f: impl Fn(Vec3) -> Vec3, g: impl Fn(Vec3) -> Vec3) {
        for p in &model.parts {
            let mut mesh = p.mesh.clone();
            for v in &mut mesh.positions {
                *v = f(Vec3::from_array(*v)).to_array();
            }
            for n in &mut mesh.normals {
                *n = g(Vec3::from_array(*n)).normalize_or(Vec3::Y).to_array();
            }
            self.m(p.material).append(&mesh);
        }
    }

    /// Finish: one part per non-empty slot, in slot order.
    pub fn into_model(self, name: impl Into<String>) -> Model {
        let mut model = Model::new(name);
        for (slot, mesh) in self.parts {
            if !mesh.is_empty() {
                model.parts.push(mde_core::MeshPart { material: slot, mesh });
            }
        }
        model
    }
}

/// Plan point + height → world.
#[inline]
pub(crate) fn w3(p: Vec2, h: f32) -> Vec3 {
    plan_to_world(p, h)
}

/// Plan direction → horizontal world direction.
#[inline]
pub(crate) fn wdir(d: Vec2) -> Vec3 {
    Vec3::new(d.x, 0.0, -d.y)
}

/// Rotate a world vector about +Y by `yaw` radians (right-handed, so +X turns
/// towards -Z for positive yaw).
#[inline]
pub(crate) fn rot_y(v: Vec3, yaw: f32) -> Vec3 {
    let (s, c) = yaw.sin_cos();
    Vec3::new(v.x * c + v.z * s, v.y, -v.x * s + v.z * c)
}

/// Yaw that maps local +X onto the horizontal world direction `d`.
#[inline]
pub(crate) fn yaw_of_x(d: Vec3) -> f32 {
    (-d.z).atan2(d.x)
}

/// Planar world UV in metres, projected along the dominant normal axis.
#[inline]
pub(crate) fn uv_for(p: Vec3, n: Vec3) -> [f32; 2] {
    let a = n.abs();
    if a.y >= a.x && a.y >= a.z {
        [p.x, p.z]
    } else if a.x >= a.z {
        [-p.z * n.x.signum(), -p.y]
    } else {
        [p.x * n.z.signum(), -p.y]
    }
}

/// Newell normal (unnormalised; length = 2 × area for planar polygons).
pub(crate) fn newell(pts: &[Vec3]) -> Vec3 {
    let mut n = Vec3::ZERO;
    for i in 0..pts.len() {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        n.x += (a.y - b.y) * (a.z + b.z);
        n.y += (a.z - b.z) * (a.x + b.x);
        n.z += (a.x - b.x) * (a.y + b.y);
    }
    n
}

/// Push one vertex with all attributes.
#[inline]
pub(crate) fn vert(m: &mut MeshData, p: Vec3, n: Vec3, uv: [f32; 2], c: Linear) -> u32 {
    let i = m.positions.len() as u32;
    m.positions.push(p.to_array());
    m.normals.push(n.to_array());
    m.uvs.push(uv);
    m.colors.push(clamp01(c));
    i
}

/// Planar polygon (convex or concave), CCW seen from the side it faces, flat
/// normal. Degenerate polygons are skipped.
pub(crate) fn poly(m: &mut MeshData, pts: &[Vec3], c: Linear) {
    if pts.len() < 3 || pts.iter().any(|p| !p.is_finite()) {
        return;
    }
    let nn = newell(pts);
    let len = nn.length();
    if len < 1e-9 {
        return;
    }
    let n = nn / len;
    let base = m.positions.len() as u32;
    for &p in pts {
        vert(m, p, n, uv_for(p, n), c);
    }
    if pts.len() == 3 {
        m.indices.extend([base, base + 1, base + 2]);
        return;
    }
    // Project onto the plane (basis t, n × t keeps CCW about n) and ear-clip.
    let mut t = Vec3::ZERO;
    for i in 1..pts.len() {
        let d = pts[i] - pts[0];
        if d.length_squared() > 1e-12 {
            t = d.normalize();
            break;
        }
    }
    let b = n.cross(t);
    let flat: Vec<Vec2> = pts.iter().map(|&p| Vec2::new(p.dot(t), p.dot(b))).collect();
    for tri in triangulate_points(&flat) {
        m.indices.extend([base + tri[0] as u32, base + tri[1] as u32, base + tri[2] as u32]);
    }
}

/// [`poly`], reversing the winding if its normal points away from `facing`.
pub(crate) fn poly_facing(m: &mut MeshData, pts: &[Vec3], facing: Vec3, c: Linear) {
    if newell(pts).dot(facing) < 0.0 {
        let rev: Vec<Vec3> = pts.iter().rev().copied().collect();
        poly(m, &rev, c);
    } else {
        poly(m, pts, c);
    }
}

/// Quad oriented towards `facing`.
#[inline]
pub(crate) fn quad_facing(m: &mut MeshData, a: Vec3, b: Vec3, c: Vec3, d: Vec3, facing: Vec3, col: Linear) {
    poly_facing(m, &[a, b, c, d], facing, col);
}

/// Face mask bits for [`cuboid`].
pub(crate) mod faces {
    pub const NX: u8 = 1;
    pub const PX: u8 = 2;
    pub const NY: u8 = 4;
    pub const PY: u8 = 8;
    pub const NZ: u8 = 16;
    pub const PZ: u8 = 32;
}

/// Affine frame: `o + x·u + y·v + z·w` (axes need not be unit or orthogonal).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Basis {
    pub o: Vec3,
    pub x: Vec3,
    pub y: Vec3,
    pub z: Vec3,
}

impl Basis {
    pub fn world() -> Self {
        Basis { o: Vec3::ZERO, x: Vec3::X, y: Vec3::Y, z: Vec3::Z }
    }

    #[inline]
    pub fn at(&self, u: f32, v: f32, w: f32) -> Vec3 {
        self.o + self.x * u + self.y * v + self.z * w
    }

}

/// Box spanning local `min..max` in basis `b`; faces whose bit is in `skip` are omitted.
pub(crate) fn cuboid(m: &mut MeshData, b: &Basis, min: Vec3, max: Vec3, col: Linear, skip: u8) {
    let p = |x: f32, y: f32, z: f32| b.at(x, y, z);
    let (x0, y0, z0, x1, y1, z1) = (min.x, min.y, min.z, max.x, max.y, max.z);
    let fs: [(u8, [Vec3; 4], Vec3); 6] = [
        (faces::NX, [p(x0, y0, z0), p(x0, y0, z1), p(x0, y1, z1), p(x0, y1, z0)], -b.x),
        (faces::PX, [p(x1, y0, z0), p(x1, y1, z0), p(x1, y1, z1), p(x1, y0, z1)], b.x),
        (faces::NY, [p(x0, y0, z0), p(x1, y0, z0), p(x1, y0, z1), p(x0, y0, z1)], -b.y),
        (faces::PY, [p(x0, y1, z0), p(x0, y1, z1), p(x1, y1, z1), p(x1, y1, z0)], b.y),
        (faces::NZ, [p(x0, y0, z0), p(x0, y1, z0), p(x1, y1, z0), p(x1, y0, z0)], -b.z),
        (faces::PZ, [p(x0, y0, z1), p(x1, y0, z1), p(x1, y1, z1), p(x0, y1, z1)], b.z),
    ];
    for (bit, q, facing) in fs {
        if skip & bit == 0 {
            poly_facing(m, &q, facing, col);
        }
    }
}

/// Axis-aligned world box.
pub(crate) fn aabox(m: &mut MeshData, min: Vec3, max: Vec3, col: Linear) {
    cuboid(m, &Basis::world(), min, max, col, 0);
}

#[allow(clippy::too_many_arguments)]
/// Beam of rectangular section from `p0` to `p1`: `width` across (horizontal,
/// perpendicular to the beam and to `up`), `height` along `up`, centred on the line.
pub(crate) fn beam(m: &mut MeshData, p0: Vec3, p1: Vec3, up: Vec3, width: f32, height: f32, col: Linear, skip: u8) {
    let d = p1 - p0;
    let len = d.length();
    if len < 1e-5 {
        return;
    }
    let x = d / len;
    let mut z = x.cross(up);
    if z.length_squared() < 1e-8 {
        z = x.cross(if x.y.abs() < 0.9 { Vec3::Y } else { Vec3::X });
    }
    let z = z.normalize();
    let y = z.cross(x).normalize();
    let b = Basis { o: p0, x, y, z };
    cuboid(m, &b, Vec3::new(0.0, -height * 0.5, -width * 0.5), Vec3::new(len, height * 0.5, width * 0.5), col, skip);
}

/// Rectangular-section sweep along a path (shared cross-sections at the
/// joints, so consecutive pieces never overlap): `width` horizontal across the
/// path, `height` perpendicular to it, centred on the path, capped at both ends.
pub(crate) fn sweep_box(m: &mut MeshData, path: &[Vec3], width: f32, height: f32, col: Linear) {
    let n = path.len();
    if n < 2 {
        return;
    }
    let mut rings: Vec<[Vec3; 4]> = Vec::with_capacity(n);
    for i in 0..n {
        let t = (path[(i + 1).min(n - 1)] - path[i.saturating_sub(1)]).normalize_or(Vec3::X);
        let side = t.cross(Vec3::Y).normalize_or(Vec3::Z);
        let up = side.cross(t).normalize_or(Vec3::Y);
        let (sw, uh) = (side * (width * 0.5), up * (height * 0.5));
        let p = path[i];
        rings.push([p - sw - uh, p + sw - uh, p + sw + uh, p - sw + uh]);
    }
    for i in 0..n - 1 {
        let (a, b) = (rings[i], rings[i + 1]);
        let centre = (path[i] + path[i + 1]) * 0.5;
        for k in 0..4 {
            let q = [a[k], b[k], b[(k + 1) % 4], a[(k + 1) % 4]];
            let mid = (q[0] + q[1] + q[2] + q[3]) * 0.25;
            quad_facing(m, q[0], q[1], q[2], q[3], mid - centre, scale(col, if k == 0 { 0.8 } else { 1.0 }));
        }
    }
    let t0 = path[0] - path[1];
    let t1 = path[n - 1] - path[n - 2];
    poly_facing(m, &rings[0], t0, scale(col, 0.9));
    poly_facing(m, &rings[n - 1], t1, scale(col, 0.9));
}

/// Two unit vectors perpendicular to `axis` (and to each other).
pub(crate) fn perp_pair(axis: Vec3) -> (Vec3, Vec3) {
    let a = axis.normalize_or(Vec3::Y);
    let helper = if a.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let u = a.cross(helper).normalize();
    let v = u.cross(a).normalize();
    (u, v)
}

/// Frustum / cylinder along `axis` from `base`: radii `r0` (bottom) and `r1`
/// (top), `h` long, `segs` sides. `smooth` shares normals around the side.
#[allow(clippy::too_many_arguments)]
pub(crate) fn frustum(
    m: &mut MeshData,
    base: Vec3,
    axis: Vec3,
    r0: f32,
    r1: f32,
    h: f32,
    segs: usize,
    col: Linear,
    smooth: bool,
    caps: (bool, bool),
) {
    let segs = segs.max(3);
    let a = axis.normalize_or(Vec3::Y);
    let (u, v) = perp_pair(a);
    let ring = |r: f32, k: usize| {
        let t = k as f32 / segs as f32 * std::f32::consts::TAU;
        (u * t.cos() + v * t.sin()) * r
    };
    let top = base + a * h;
    if smooth {
        let slope = (r0 - r1) / h.max(1e-6);
        let b0 = m.positions.len() as u32;
        for k in 0..=segs {
            let dir = ring(1.0, k);
            let n = (dir + a * slope).normalize_or(dir);
            let s = k as f32 / segs as f32 * (r0.max(r1) * std::f32::consts::TAU);
            vert(m, base + dir * r0, n, [s, 0.0], col);
            vert(m, top + dir * r1, n, [s, -h], col);
        }
        for k in 0..segs as u32 {
            let i = b0 + k * 2;
            // CCW seen from outside: bottom k, bottom k+1, top k+1, top k.
            let (b_k, t_k, b_n, t_n) = (i, i + 1, i + 2, i + 3);
            if r1 > 1e-6 {
                m.indices.extend([b_k, t_n, b_n, b_k, t_k, t_n]);
            } else {
                m.indices.extend([b_k, t_k, b_n]);
            }
        }
        // Fix winding: computed for u × v = -a or a; check one triangle.
        fix_last_winding(m, b0 as usize, base + a * (h * 0.5), true);
    } else {
        for k in 0..segs {
            let d0 = ring(1.0, k);
            let d1 = ring(1.0, k + 1);
            let mid = (d0 + d1).normalize_or(d0);
            if r1 > 1e-6 {
                quad_facing(m, base + d0 * r0, base + d1 * r0, top + d1 * r1, top + d0 * r1, mid, col);
            } else {
                poly_facing(m, &[base + d0 * r0, base + d1 * r0, top], mid, col);
            }
        }
    }
    if caps.0 && r0 > 1e-6 {
        let pts: Vec<Vec3> = (0..segs).map(|k| base + ring(r0, k)).collect();
        poly_facing(m, &pts, -a, col);
    }
    if caps.1 && r1 > 1e-6 {
        let pts: Vec<Vec3> = (0..segs).map(|k| top + ring(r1, k)).collect();
        poly_facing(m, &pts, a, col);
    }
}

/// Make the triangles added since vertex `from` face away from (`outward`) or
/// towards `center`, judging by their average orientation.
fn fix_last_winding(m: &mut MeshData, from: usize, center: Vec3, outward: bool) {
    let first_index = m.indices.iter().position(|&i| i as usize >= from).unwrap_or(m.indices.len());
    let tris = &m.indices[first_index..];
    let mut score = 0.0;
    for t in tris.as_chunks::<3>().0 {
        let a = Vec3::from_array(m.positions[t[0] as usize]);
        let b = Vec3::from_array(m.positions[t[1] as usize]);
        let c = Vec3::from_array(m.positions[t[2] as usize]);
        let n = (b - a).cross(c - a);
        score += n.dot((a + b + c) / 3.0 - center);
    }
    if (score < 0.0) == outward {
        for t in m.indices[first_index..].as_chunks_mut::<3>().0 {
            t.swap(1, 2);
        }
    }
}

/// Prism: plan polygon `base` (any winding) extruded from `y0` to `y1`.
pub(crate) fn prism(m: &mut MeshData, base: &[Vec2], y0: f32, y1: f32, col: Linear, top: bool, bottom: bool) {
    let n = base.len();
    if n < 3 {
        return;
    }
    let ccw = signed_area(base) >= 0.0;
    for i in 0..n {
        let a = base[i];
        let b = base[(i + 1) % n];
        if a.distance_squared(b) < 1e-12 {
            continue;
        }
        let d = (b - a).normalize();
        let out = if ccw { Vec2::new(d.y, -d.x) } else { Vec2::new(-d.y, d.x) };
        quad_facing(m, w3(a, y0), w3(b, y0), w3(b, y1), w3(a, y1), wdir(out), col);
    }
    if top {
        let pts: Vec<Vec3> = base.iter().map(|&p| w3(p, y1)).collect();
        poly_facing(m, &pts, Vec3::Y, col);
    }
    if bottom {
        let pts: Vec<Vec3> = base.iter().map(|&p| w3(p, y0)).collect();
        poly_facing(m, &pts, -Vec3::Y, col);
    }
}

/// Mitred ring wall between the plan polygons `outer` and `inner` (same
/// vertex count, `inner` inset from `outer`), from `y0` to `y1`: outer and
/// inner faces, top, and optionally the bottom.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ring_wall(m: &mut MeshData, outer: &[Vec2], inner: &[Vec2], y0: f32, y1: f32, col: Linear, top: bool, bottom: bool) {
    let n = outer.len();
    if n < 3 || inner.len() != n {
        return;
    }
    let ccw = signed_area(outer) >= 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        let (oa, ob, ia, ib) = (outer[i], outer[j], inner[i], inner[j]);
        if oa.distance_squared(ob) < 1e-10 {
            continue;
        }
        let d = (ob - oa).normalize();
        let out = wdir(if ccw { Vec2::new(d.y, -d.x) } else { Vec2::new(-d.y, d.x) });
        quad_facing(m, w3(oa, y0), w3(ob, y0), w3(ob, y1), w3(oa, y1), out, col);
        quad_facing(m, w3(ia, y0), w3(ib, y0), w3(ib, y1), w3(ia, y1), -out, scale(col, 0.95));
        if top {
            quad_facing(m, w3(oa, y1), w3(ob, y1), w3(ib, y1), w3(ia, y1), Vec3::Y, scale(col, 1.03));
        }
        if bottom {
            quad_facing(m, w3(oa, y0), w3(ob, y0), w3(ib, y0), w3(ia, y0), -Vec3::Y, scale(col, 0.8));
        }
    }
}

/// Signed area of a plan polygon (positive = CCW).
pub(crate) fn signed_area(p: &[Vec2]) -> f32 {
    let n = p.len();
    (0..n).map(|i| mde_core::poly::cross(p[i], p[(i + 1) % n])).sum::<f32>() * 0.5
}

/// Rounded blob (subdivided icosahedron) with smooth normals and per-vertex
/// colour variation, darker underneath. `detail` 0 = 20 triangles, 1 = 80.
pub(crate) fn blob(m: &mut MeshData, center: Vec3, radii: Vec3, detail: u32, col: Linear, rng: &mut Rng) {
    let (verts, tris) = icosphere(detail.min(2));
    let base = m.positions.len() as u32;
    let mut pos: Vec<Vec3> = Vec::with_capacity(verts.len());
    for v in &verts {
        let bump = 1.0 + rng.range_f32(-0.13, 0.13);
        pos.push(center + *v * radii * bump);
    }
    let mut normals = vec![Vec3::ZERO; verts.len()];
    for t in &tris {
        let (a, b, c) = (pos[t[0]], pos[t[1]], pos[t[2]]);
        let n = (b - a).cross(c - a);
        for &i in t {
            normals[i] += n;
        }
    }
    for (i, &p) in pos.iter().enumerate() {
        let n = normals[i].normalize_or(verts[i]);
        let shade = 0.72 + 0.28 * (verts[i].y * 0.5 + 0.5);
        let jitter = 1.0 + rng.range_f32(-0.08, 0.08);
        vert(m, p, n, [p.x + p.y * 0.5, p.z - p.y * 0.5], scale(col, shade * jitter));
    }
    for t in &tris {
        m.indices.extend([base + t[0] as u32, base + t[1] as u32, base + t[2] as u32]);
    }
}

/// Unit icosphere: vertices and CCW (outward) triangles.
pub(crate) fn icosphere(detail: u32) -> (Vec<Vec3>, Vec<[usize; 3]>) {
    let t = (1.0 + 5f32.sqrt()) / 2.0;
    let mut v: Vec<Vec3> = [
        (-1.0, t, 0.0),
        (1.0, t, 0.0),
        (-1.0, -t, 0.0),
        (1.0, -t, 0.0),
        (0.0, -1.0, t),
        (0.0, 1.0, t),
        (0.0, -1.0, -t),
        (0.0, 1.0, -t),
        (t, 0.0, -1.0),
        (t, 0.0, 1.0),
        (-t, 0.0, -1.0),
        (-t, 0.0, 1.0),
    ]
    .iter()
    .map(|&(x, y, z)| Vec3::new(x, y, z).normalize())
    .collect();
    let mut f: Vec<[usize; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..detail {
        let mut cache: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        let mut mid = |a: usize, b: usize, v: &mut Vec<Vec3>| -> usize {
            let key = (a.min(b), a.max(b));
            *cache.entry(key).or_insert_with(|| {
                v.push(((v[a] + v[b]) * 0.5).normalize());
                v.len() - 1
            })
        };
        let mut nf = Vec::with_capacity(f.len() * 4);
        for t in &f {
            let a = mid(t[0], t[1], &mut v);
            let b = mid(t[1], t[2], &mut v);
            let c = mid(t[2], t[0], &mut v);
            nf.push([t[0], a, c]);
            nf.push([t[1], b, a]);
            nf.push([t[2], c, b]);
            nf.push([a, b, c]);
        }
        f = nf;
    }
    (v, f)
}

/// Deterministic per-element colour jitter: multiply by `1 ± amount`.
#[inline]
pub(crate) fn jitter(col: Linear, rng: &mut Rng, amount: f32) -> Linear {
    scale(col, 1.0 + rng.range_f32(-amount, amount))
}

/// Seed mixer for derived RNG streams.
#[inline]
pub(crate) fn mix_seed(a: u64, b: u64) -> u64 {
    let mut r = Rng::new(a ^ b.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    r.next_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icosphere_is_outward() {
        let (v, f) = icosphere(1);
        assert_eq!(f.len(), 80);
        for t in f {
            let n = (v[t[1]] - v[t[0]]).cross(v[t[2]] - v[t[0]]);
            assert!(n.dot(v[t[0]] + v[t[1]] + v[t[2]]) > 0.0);
        }
    }

    #[test]
    fn box_faces_outward() {
        let mut m = MeshData::default();
        aabox(&mut m, Vec3::ZERO, Vec3::ONE, [1.0; 4]);
        assert_eq!(m.triangle_count(), 12);
        for t in m.indices.as_chunks::<3>().0 {
            let a = Vec3::from_array(m.positions[t[0] as usize]);
            let b = Vec3::from_array(m.positions[t[1] as usize]);
            let c = Vec3::from_array(m.positions[t[2] as usize]);
            let n = (b - a).cross(c - a);
            assert!(n.dot((a + b + c) / 3.0 - Vec3::splat(0.5)) > 0.0);
        }
    }

    #[test]
    fn rot_y_convention() {
        let x = rot_y(Vec3::X, std::f32::consts::FRAC_PI_2);
        assert!((x - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-6);
        let d = Vec3::new(0.3, 0.0, 0.8).normalize();
        assert!((rot_y(Vec3::X, yaw_of_x(d)) - d).length() < 1e-6);
    }
}
