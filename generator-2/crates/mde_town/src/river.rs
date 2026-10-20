//! River centreline generation, wetness classification of street edges and
//! bridge placement at crossings.

use std::f32::consts::{PI, TAU};

use mde_core::poly::cross;
use mde_core::{Bridge, BridgeKind, Rng, Vec2};

use crate::geom::{poly_seg_dist, polyline_dist, seg_cross_point, seg_seg_dist};

/// Clearance (beyond the water's edge) that street-graph vertices must keep.
pub(crate) const VERTEX_CLEARANCE: f32 = 3.0;
/// How far a bridge deck reaches onto each bank beyond the water.
pub(crate) const BRIDGE_LANDING: f32 = 2.0;

/// River centreline plus width, with distance helpers.
#[derive(Clone, Debug)]
pub(crate) struct RiverGeom {
    pub path: Vec<Vec2>,
    pub width: f32,
}

impl RiverGeom {
    /// Half the water width.
    pub fn half(&self) -> f32 {
        self.width * 0.5
    }

    /// Distance from `p` to the centreline.
    pub fn dist(&self, p: Vec2) -> f32 {
        let mut best = f32::INFINITY;
        for w in self.path.windows(2) {
            // Cheap reject: the segment's box is farther than the best so far.
            let gap = (w[0].min(w[1]) - p).max(p - w[0].max(w[1])).max(Vec2::ZERO);
            if gap.length_squared() >= best * best {
                continue;
            }
            best = best.min(polyline_dist(p, w));
        }
        best
    }

    /// Is the polygon at least `gap` away from the centreline (and not
    /// crossing it)?
    pub fn poly_clear(&self, pts: &[Vec2], gap: f32) -> bool {
        let mut lo = Vec2::splat(f32::INFINITY);
        let mut hi = Vec2::splat(f32::NEG_INFINITY);
        for p in pts {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        self.path.windows(2).all(|w| {
            let wl = w[0].min(w[1]);
            let wh = w[0].max(w[1]);
            let sep = (wl - hi).max(lo - wh).max(Vec2::ZERO);
            sep.length() >= gap || poly_seg_dist(pts, w[0], w[1]) >= gap
        })
    }

    /// Distance from segment `a–b` to the centreline.
    pub fn seg_dist(&self, a: Vec2, b: Vec2) -> f32 {
        let (lo, hi) = (a.min(b), a.max(b));
        let mut best = f32::INFINITY;
        for w in self.path.windows(2) {
            // Cheap reject using the current best.
            let wl = w[0].min(w[1]);
            let wh = w[0].max(w[1]);
            if best.is_finite() {
                let gap = (wl - hi).max(lo - wh).max(Vec2::ZERO);
                if gap.length() > best {
                    continue;
                }
            }
            best = best.min(seg_seg_dist(a, b, w[0], w[1]));
        }
        best
    }

    /// Distance from a polygon to the centreline (0 if they intersect).
    pub fn poly_dist(&self, pts: &[Vec2]) -> f32 {
        self.path.windows(2).map(|w| poly_seg_dist(pts, w[0], w[1])).fold(f32::INFINITY, f32::min)
    }

    /// Points where segment `a–b` crosses the centreline, with the unit river
    /// direction at each crossing.
    pub fn crossings(&self, a: Vec2, b: Vec2) -> Vec<(Vec2, Vec2)> {
        self.path
            .windows(2)
            .filter_map(|w| seg_cross_point(a, b, w[0], w[1]).map(|x| (x, (w[1] - w[0]).normalize_or_zero())))
            .collect()
    }
}

/// Meandering river across the square map `[-half, half]²`, passing within
/// roughly `0.2–0.45 · r_city` of the centre and keeping clear of `avoid`
/// (the market cell).
pub(crate) fn make_river(rng: &mut Rng, half: f32, r_city: f32, avoid: &[Vec2]) -> RiverGeom {
    let width = rng.range_f32(8.0, 14.0);
    let theta = rng.range_f32(0.0, PI);
    let u = Vec2::from_angle(theta);
    let nrm = Vec2::new(-u.y, u.x);
    let amp = (0.13 * r_city).clamp(10.0, 70.0);
    let waves = [
        (rng.range_f32(1.3, 2.2) * r_city.max(80.0), amp, rng.range_f32(0.0, TAU)),
        (rng.range_f32(0.55, 0.9) * r_city.max(80.0), amp * 0.55, rng.range_f32(0.0, TAU)),
        (rng.range_f32(110.0, 190.0), amp * 0.18 + 2.0, rng.range_f32(0.0, TAU)),
    ];
    let sign = if rng.chance(0.5) { 1.0 } else { -1.0 };
    let mut offset = sign * rng.range_f32(0.2, 0.42) * r_city;
    let reach = half * 1.6;
    let mut best: Option<RiverGeom> = None;
    for _attempt in 0..14 {
        let mut pts = Vec::new();
        let step = 6.0;
        let mut s = -reach;
        while s <= reach {
            let m: f32 = waves.iter().map(|&(l, a, ph)| a * (TAU * s / l + ph).sin()).sum();
            pts.push(u * s + nrm * (offset + m));
            s += step;
        }
        let path = clip_to_box(&pts, half);
        let river = RiverGeom { path, width };
        if river.path.len() >= 2 {
            let clear = river.poly_dist(avoid) - river.half();
            if clear > 14.0 {
                return river;
            }
            best = Some(river);
        }
        // Push the river further from the centre and try again.
        offset += sign * 0.08 * r_city.max(40.0);
    }
    best.unwrap_or_else(|| RiverGeom { path: vec![Vec2::new(-half, half * 0.9), Vec2::new(half, half * 0.9)], width })
}

/// Keep the longest run of points inside the box, extended exactly to the box
/// sides at both ends.
fn clip_to_box(pts: &[Vec2], half: f32) -> Vec<Vec2> {
    let inside = |p: Vec2| p.x.abs() <= half && p.y.abs() <= half;
    let mut best: (usize, usize) = (0, 0);
    let mut i = 0;
    while i < pts.len() {
        if inside(pts[i]) {
            let s = i;
            while i < pts.len() && inside(pts[i]) {
                i += 1;
            }
            if i - s > best.1 - best.0 {
                best = (s, i);
            }
        } else {
            i += 1;
        }
    }
    let (s, e) = best;
    if e <= s {
        return Vec::new();
    }
    let edge_point = |out: Vec2, inn: Vec2| {
        // Bisect for the box crossing.
        let (mut lo, mut hi) = (0.0f32, 1.0f32); // lo = inside end
        for _ in 0..40 {
            let mid = (lo + hi) * 0.5;
            if inside(inn + (out - inn) * mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let p = inn + (out - inn) * lo;
        let snap = |v: f32| if (v.abs() - half).abs() < 0.05 { half * v.signum() } else { v };
        Vec2::new(snap(p.x), snap(p.y))
    };
    let mut out = Vec::with_capacity(e - s + 2);
    if s > 0 {
        out.push(edge_point(pts[s - 1], pts[s]));
    }
    out.extend_from_slice(&pts[s..e]);
    if e < pts.len() {
        out.push(edge_point(pts[e], pts[e - 1]));
    }
    crate::geom::dedupe(out, 0.01)
}

/// How a street-graph edge relates to the river.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Water {
    /// Clear of the water.
    Dry,
    /// Unusable for streets (runs along or through the water badly).
    Wet,
    /// Crosses the river once, cleanly: usable with a bridge `[ta, tb]`
    /// (distances along the edge from its first endpoint).
    Cross { ta: f32, tb: f32 },
}

/// Classify edge `a–b` against the river.
pub(crate) fn classify(river: &RiverGeom, a: Vec2, b: Vec2) -> Water {
    let h = river.half();
    if river.dist(a) < h + VERTEX_CLEARANCE || river.dist(b) < h + VERTEX_CLEARANCE {
        return Water::Wet;
    }
    let xs = river.crossings(a, b);
    match xs.len() {
        0 => {
            if river.seg_dist(a, b) < h + 1.5 {
                Water::Wet
            } else {
                Water::Dry
            }
        }
        1 => {
            let (x, along) = xs[0];
            match span_over(river, a, b, x, along) {
                Some((ta, tb)) => Water::Cross { ta, tb },
                None => Water::Wet,
            }
        }
        _ => Water::Wet,
    }
}

/// Bridge span along `a → b` (distances from `a`) over the crossing at `x`,
/// verified by sampling so no part of the segment outside it is in the water.
fn span_over(river: &RiverGeom, a: Vec2, b: Vec2, x: Vec2, along: Vec2) -> Option<(f32, f32)> {
    let len = a.distance(b);
    if len < 1e-3 {
        return None;
    }
    let u = (b - a) / len;
    let s = cross(u, along).abs();
    if s < 0.57 {
        // Shallower than ~35°: the bridge would run along the river.
        return None;
    }
    let half_span = river.half() / s + BRIDGE_LANDING;
    let t0 = (x - a).dot(u);
    let mut ta = (t0 - half_span).max(0.0);
    let mut tb = (t0 + half_span).min(len);
    // On a curving river, push the landings out until they really are on dry
    // ground (the edge's endpoints always are).
    let landing = river.half() + BRIDGE_LANDING * 0.75;
    while ta > 0.0 && river.dist(a + u * ta) < landing {
        ta = (ta - 0.25).max(0.0);
    }
    while tb < len && river.dist(a + u * tb) < landing {
        tb = (tb + 0.25).min(len);
    }
    // Every sampled point in the water must be on the deck.
    let n = (len / 0.5).ceil() as usize;
    for i in 0..=n {
        let t = len * i as f32 / n as f32;
        if (t < ta || t > tb) && river.dist(a + u * t) < river.half() + 0.5 {
            return None;
        }
    }
    Some((ta, tb))
}

/// Deck rise for a span (metres): flatter for short spans.
pub(crate) fn rise_for_span(span: f32) -> f32 {
    (span * 0.12).clamp(1.5, 3.0)
}

/// Bridge over the crossing of edge `a → b` with span `[ta, tb]`.
pub(crate) fn bridge_on_edge(a: Vec2, b: Vec2, ta: f32, tb: f32, street_width: f32, kind: BridgeKind) -> Bridge {
    let u = (b - a).normalize_or_zero();
    let pa = a + u * ta;
    let pb = a + u * tb;
    Bridge { kind, a: pa, b: pb, width: street_width + 1.0, height: rise_for_span(tb - ta) }
}

/// Public helper: bridge for a straight path `a → b` crossing a river polyline.
pub(crate) fn bridge_across_path(
    path: &[Vec2],
    width: f32,
    a: Vec2,
    b: Vec2,
    deck_width: f32,
    kind: BridgeKind,
) -> Option<Bridge> {
    let river = RiverGeom { path: path.to_vec(), width };
    let len = a.distance(b);
    if len < 1e-3 || path.len() < 2 {
        return None;
    }
    let u = (b - a) / len;
    let (x, along) = river.crossings(a, b).into_iter().min_by(|p, q| (p.0 - a).dot(u).total_cmp(&(q.0 - a).dot(u)))?;
    let s = cross(u, along).abs().max(0.2);
    let half_span = river.half() / s + BRIDGE_LANDING;
    let t0 = (x - a).dot(u);
    let pa = a + u * (t0 - half_span);
    let pb = a + u * (t0 + half_span);
    Some(Bridge { kind, a: pa, b: pb, width: deck_width.max(1.0), height: rise_for_span(2.0 * half_span) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_crossing() {
        let river = RiverGeom { path: vec![Vec2::new(-100.0, 0.0), Vec2::new(100.0, 0.0)], width: 10.0 };
        match classify(&river, Vec2::new(0.0, -20.0), Vec2::new(0.0, 20.0)) {
            Water::Cross { ta, tb } => {
                assert!((ta - 13.0).abs() < 1e-3 && (tb - 27.0).abs() < 1e-3, "{ta} {tb}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(classify(&river, Vec2::new(-20.0, 3.0), Vec2::new(20.0, 3.0)), Water::Wet);
        assert_eq!(classify(&river, Vec2::new(-20.0, 30.0), Vec2::new(20.0, 30.0)), Water::Dry);
        // Too shallow.
        assert_eq!(classify(&river, Vec2::new(-60.0, -20.0), Vec2::new(60.0, 20.0)), Water::Wet);
    }

    #[test]
    fn river_spans_map() {
        let mut rng = Rng::new(5);
        let r = make_river(&mut rng, 400.0, 150.0, &[Vec2::new(-5.0, -5.0), Vec2::new(5.0, -5.0), Vec2::new(0.0, 5.0)]);
        assert!(r.path.len() > 10);
        let first = r.path[0];
        let last = *r.path.last().unwrap();
        let on_box = |p: Vec2| (p.x.abs() - 400.0).abs() < 0.1 || (p.y.abs() - 400.0).abs() < 0.1;
        assert!(on_box(first) && on_box(last), "{first} {last}");
    }
}
