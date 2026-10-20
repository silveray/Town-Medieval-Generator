//! House sizes: every house-like building keeps a normal-sized footprint at
//! the street end of its lot. What is left of a large lot becomes a yard
//! (garden, backyard) behind a wooden fence with a gate, or open ground.

use mde_core::{BuildingKind, Polygon, Polyline, Rng, Vec2, Yard};

use crate::geom::{clip_half, signed_area, tidy};
use crate::lots::lot_ok;

/// Share of the leftover ground that is fenced as a yard (the rest stays open).
const YARD_CHANCE: f32 = 0.85;
/// Smallest yard kept (m²).
const MIN_YARD: f32 = 25.0;
/// Gate opening width (m).
const GATE: f32 = 1.2;

/// Normal footprint of a house kind: (depth from the street, width along
/// it) ranges in metres; `None` for buildings whose size is not capped
/// (temple, keep, tower, barracks, barn, mill).
pub(crate) fn house_size(kind: BuildingKind) -> Option<((f32, f32), (f32, f32))> {
    use BuildingKind as B;
    match kind {
        B::House | B::Shop | B::Workshop | B::Tavern | B::Farmhouse => Some(((7.0, 10.0), (9.0, 13.0))),
        B::Mansion => Some(((10.0, 14.0), (13.0, 18.0))),
        _ => None,
    }
}

/// Fit a normal-sized house (see [`house_size`]) on a lot (`pts`, CCW,
/// convex, `front` towards the street): the house keeps the street end and,
/// on a wide lot, one side. Returns the house footprint and, when the rest
/// is big enough, a fenced yard. `None` keeps the whole lot (it is already
/// normal-sized, the kind is not capped, or no valid footprint fits).
pub(crate) fn fit_house(pts: &[Vec2], front: Vec2, kind: BuildingKind, rng: &mut Rng) -> Option<(Vec<Vec2>, Option<Yard>)> {
    let ((dmin, dmax), (wmin, wmax)) = house_size(kind)?;
    // Always draw, so the stream does not depend on the lot.
    let (roll, depth_t, width_t, left) = (rng.next_f32(), rng.next_f32(), rng.next_f32(), rng.chance(0.5));
    let front = front.try_normalize()?;
    let along = Vec2::new(-front.y, front.x);
    let span = |d: Vec2| pts.iter().map(|p| p.dot(d)).fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
    let ((f0, f1), (a0, a1)) = (span(front), span(along));
    let (deep, wide) = (f1 - f0 > dmax, a1 - a0 > wmax);
    if !deep && !wide {
        return None;
    }
    let depth = dmin + (dmax - dmin) * depth_t;
    let width = wmin + (wmax - wmin) * width_t;
    let clip = |left: bool| {
        let mut h = if deep { clip_half(pts, front * (f1 - depth), front) } else { pts.to_vec() };
        if wide && h.len() >= 3 {
            h = if left { clip_half(&h, along * (a1 - width), along) } else { clip_half(&h, along * (a0 + width), -along) };
        }
        tidy(h, 0.3)
    };
    let house = [clip(left), clip(!left)].into_iter().find(|h| lot_ok(h))?;
    let rest = signed_area(pts) - signed_area(&house);
    let yard = (rest >= MIN_YARD && roll <= YARD_CHANCE)
        .then(|| fence(pts, &house, front))
        .filter(|(fences, _)| !fences.is_empty())
        .map(|(fences, gates)| Yard { building: 0, lot: Polygon(pts.to_vec()), fences, gates });
    Some((house, yard))
}

/// Fence along the lot edges (`lot`, CCW) that the house does not stand on,
/// and one gate: on the street side if the yard reaches it, else on the side
/// fence close to the house.
fn fence(lot: &[Vec2], house: &[Vec2], front: Vec2) -> (Vec<Polyline>, Vec<(Vec2, Vec2)>) {
    let n = lot.len();
    // Fence segments in lot order, with the lot edge they lie on.
    let mut segs: Vec<(Vec2, Vec2, usize)> = Vec::new();
    for i in 0..n {
        let (a, b) = (lot[i], lot[(i + 1) % n]);
        let len = a.distance(b);
        if len < 1e-3 {
            continue;
        }
        let d = (b - a) / len;
        let on: Vec<f32> = house
            .iter()
            .filter(|&&q| (q - a).perp_dot(d).abs() < 0.05)
            .map(|&q| ((q - a).dot(d) / len).clamp(0.0, 1.0))
            .collect();
        let covered = (on.len() >= 2).then(|| on.iter().fold((1.0f32, 0.0f32), |(lo, hi), &t| (lo.min(t), hi.max(t))));
        let parts: Vec<(f32, f32)> = match covered {
            Some((lo, hi)) if hi - lo > 1e-3 => vec![(0.0, lo), (hi, 1.0)],
            _ => vec![(0.0, 1.0)],
        };
        for (t0, t1) in parts {
            if (t1 - t0) * len > 0.05 {
                segs.push((a + (b - a) * t0, a + (b - a) * t1, i));
            }
        }
    }
    // Gate: the street-side segment, else the side segment nearest the street.
    let facing = |s: &(Vec2, Vec2, usize)| {
        let d = (s.1 - s.0).normalize_or_zero();
        Vec2::new(d.y, -d.x).dot(front)
    };
    let street = segs.iter().position(|s| facing(s) > 0.7 && s.0.distance(s.1) >= GATE + 1.0);
    let side = || {
        segs.iter()
            .enumerate()
            .filter(|(_, s)| facing(s).abs() < 0.5 && s.0.distance(s.1) >= GATE + 1.0)
            .max_by(|x, y| (x.1.0 + x.1.1).dot(front).total_cmp(&(y.1.0 + y.1.1).dot(front)))
            .map(|(i, _)| i)
    };
    let mut gates = Vec::new();
    if let Some(k) = street.or_else(side) {
        let (a, b, e) = segs[k];
        let d = (b - a).normalize();
        let len = a.distance(b);
        // Centred on the street side; on a side fence 0.4 m behind its
        // street-side end.
        let s0 = if street.is_some() {
            (len - GATE) * 0.5
        } else if (b - a).dot(front) > 0.0 {
            len - GATE - 0.4
        } else {
            0.4
        };
        let (g0, g1) = (a + d * s0, a + d * (s0 + GATE));
        segs.splice(k..=k, [(a, g0, e), (g1, b, e)]);
        gates.push((g0, g1));
    }
    // Join touching segments into runs.
    let mut runs: Vec<Polyline> = Vec::new();
    for (a, b, _) in segs {
        if (b - a).length() < 0.05 {
            continue;
        }
        match runs.last_mut() {
            Some(r) if r.0.last().is_some_and(|l| l.distance(a) < 1e-3) => r.0.push(b),
            _ => runs.push(Polyline(vec![a, b])),
        }
    }
    // The ring may close over the start.
    if runs.len() > 1 {
        let (first, last) = (runs[0].0[0], *runs[runs.len() - 1].0.last().unwrap());
        if first.distance(last) < 1e-3 {
            let mut tail = runs.pop().unwrap();
            tail.0.extend(runs[0].0.iter().skip(1));
            runs[0] = tail;
        }
    }
    (runs, gates)
}

/// Drop fence stretches that run along an earlier yard's fence or gate
/// (neighbouring lots share a boundary): one fence per line, and no fence
/// across a neighbour's gate.
pub(crate) fn dedupe(yards: &mut [Yard]) {
    let mut kept: Vec<(Vec2, Vec2)> = Vec::new();
    for y in yards.iter_mut() {
        let mut runs = Vec::new();
        let mut added = Vec::new();
        for run in &y.fences {
            let mut cur: Vec<Vec2> = Vec::new();
            for w in run.0.windows(2) {
                let (a, b) = (w[0], w[1]);
                for (p, q) in uncovered(a, b, &kept) {
                    if cur.last().is_some_and(|l| l.distance(p) < 1e-3) {
                        cur.push(q);
                    } else {
                        if cur.len() >= 2 {
                            runs.push(Polyline(std::mem::take(&mut cur)));
                        }
                        cur = vec![p, q];
                    }
                }
                added.push((a, b));
            }
            if cur.len() >= 2 {
                runs.push(Polyline(cur));
            }
        }
        y.fences = runs;
        kept.extend(added);
        kept.extend(y.gates.iter().copied());
    }
}

/// Parts of `a → b` not lying along any of `kept` (parallel within ~3°, at
/// most 0.4 m away), at least 0.3 m long.
fn uncovered(a: Vec2, b: Vec2, kept: &[(Vec2, Vec2)]) -> Vec<(Vec2, Vec2)> {
    let len = a.distance(b);
    if len < 1e-3 {
        return Vec::new();
    }
    let d = (b - a) / len;
    let mut cover: Vec<(f32, f32)> = kept
        .iter()
        .filter(|(p, q)| {
            let e = (*q - *p).normalize_or_zero();
            e.perp_dot(d).abs() < 0.05 && (*p - a).perp_dot(d).abs() < 0.4 && (*q - a).perp_dot(d).abs() < 0.4
        })
        .map(|(p, q)| {
            let (s, t) = ((*p - a).dot(d), (*q - a).dot(d));
            (s.min(t).max(0.0), s.max(t).min(len))
        })
        .filter(|(s, t)| t > s)
        .collect();
    cover.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut out = Vec::new();
    let mut s = 0.0f32;
    for (c0, c1) in cover {
        if c0 > s + 0.3 {
            out.push((a + d * s, a + d * c0));
        }
        s = s.max(c1);
    }
    if len > s + 0.3 {
        out.push((a + d * s, b));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(w: f32, d: f32) -> Vec<Vec2> {
        // Street to the south (front = -y), lot reaching north.
        vec![Vec2::new(0.0, 0.0), Vec2::new(w, 0.0), Vec2::new(w, d), Vec2::new(0.0, d)]
    }

    fn len(r: &Polyline) -> f32 {
        r.0.windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    #[test]
    fn deep_lot_gets_a_back_yard() {
        let lot = rect(10.0, 25.0);
        let mut hits = 0;
        for seed in 0..40 {
            let mut rng = Rng::new(seed);
            let Some((house, Some(yard))) = fit_house(&lot, -Vec2::Y, BuildingKind::House, &mut rng) else { continue };
            hits += 1;
            // House at the street, 7–10 m deep, full width.
            let ymax = house.iter().map(|p| p.y).fold(f32::MIN, f32::max);
            assert!(house.iter().any(|p| p.y.abs() < 1e-3));
            assert!((7.0 - 1e-3..=10.0 + 1e-3).contains(&ymax), "depth {ymax}");
            // Fence: both sides behind the house and the back: 2·(25 − d) + 10 − gate.
            let total: f32 = yard.fences.iter().map(len).sum();
            let expect = 2.0 * (25.0 - ymax) + 10.0 - GATE;
            assert!((total - expect).abs() < 0.05, "fence {total} vs {expect}");
            // Nothing on the street.
            assert!(yard.fences.iter().flat_map(|r| &r.0).all(|p| p.y > ymax - 1e-3));
            assert_eq!(yard.gates.len(), 1);
        }
        assert!(hits > 25, "{hits}");
    }

    #[test]
    fn wide_lot_gets_a_side_yard_with_a_street_gate() {
        let lot = rect(24.0, 12.0);
        for seed in 0..20 {
            let mut rng = Rng::new(seed);
            let Some((house, Some(yard))) = fit_house(&lot, -Vec2::Y, BuildingKind::House, &mut rng) else { continue };
            let (x0, x1) = house.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.x), hi.max(p.x)));
            assert!(x1 - x0 <= 13.0 + 1e-3);
            // The gate faces the street.
            let (g0, g1) = yard.gates[0];
            assert!(g0.y.abs() < 1e-3 && g1.y.abs() < 1e-3, "{g0} {g1}");
        }
    }

    #[test]
    fn normal_lots_and_landmarks_are_kept() {
        let mut rng = Rng::new(1);
        assert!(fit_house(&rect(8.0, 9.5), -Vec2::Y, BuildingKind::House, &mut rng).is_none());
        assert!(fit_house(&rect(30.0, 25.0), -Vec2::Y, BuildingKind::Temple, &mut rng).is_none());
    }

    #[test]
    fn every_house_kind_is_normal_sized() {
        use BuildingKind as B;
        for kind in [B::House, B::Shop, B::Workshop, B::Tavern, B::Farmhouse, B::Mansion] {
            let ((_, dmax), (_, wmax)) = house_size(kind).unwrap();
            for seed in 0..20 {
                let (house, _) = fit_house(&rect(26.0, 30.0), -Vec2::Y, kind, &mut Rng::new(seed)).unwrap();
                let (x0, x1, y1) = house.iter().fold((f32::MAX, f32::MIN, f32::MIN), |(a, b, c), p| (a.min(p.x), b.max(p.x), c.max(p.y)));
                assert!(x1 - x0 <= wmax + 1e-3 && y1 <= dmax + 1e-3, "{kind:?}: {}×{}", x1 - x0, y1);
            }
        }
    }

    #[test]
    fn shared_fences_are_drawn_once() {
        let mut a = fit_house(&rect(10.0, 25.0), -Vec2::Y, BuildingKind::House, &mut Rng::new(3)).unwrap().1.unwrap();
        let lot_b: Vec<Vec2> = rect(10.0, 25.0).iter().map(|p| *p + Vec2::new(10.0, 0.0)).collect();
        let b = fit_house(&lot_b, -Vec2::Y, BuildingKind::House, &mut Rng::new(3)).unwrap().1.unwrap();
        a.building = 0;
        let mut ys = vec![a, b];
        dedupe(&mut ys);
        // No point of b's fences lies on the shared line x = 10 except at its ends.
        for r in &ys[1].fences {
            for w in r.0.windows(2) {
                let mid = (w[0] + w[1]) * 0.5;
                assert!((mid.x - 10.0).abs() > 0.1, "shared stretch kept: {w:?}");
            }
        }
    }
}
