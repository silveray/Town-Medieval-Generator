//! Walls, doors, the entrance, windows and stair flights for laid-out levels,
//! all in the local frame.

use std::collections::BTreeMap;

use mde_core::{Appearance, OpeningKind, RoomKind, Rng, Vec2};

use crate::frame::Side;
use crate::grid::{Axis, Grid};
use crate::layout::{door_runs, LevelLayout, StairWell};
use crate::runs::{self, label_at, Run, JUNCTION_CLEARANCE, OUTSIDE};
use crate::style::WindowStyle;
use crate::{DOOR_HEIGHT, DOOR_MIN_WIDTH, EXTERIOR_WALL, INTERIOR_WALL};

/// Wall segment in the local frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LWall {
    pub a: Vec2,
    pub b: Vec2,
    pub thickness: f32,
    pub exterior: bool,
}

/// Door or window in the local frame. `rooms` are level-local room indices.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LOpening {
    pub kind: OpeningKind,
    pub a: Vec2,
    pub b: Vec2,
    pub sill: f32,
    pub height: f32,
    pub rooms: [Option<usize>; 2],
    /// Thickness of the wall the opening sits in.
    pub thickness: f32,
}

impl LOpening {
    /// Doors, doorways and the entrance (anything people walk through).
    pub fn is_passage(&self) -> bool {
        self.kind != OpeningKind::Window
    }

    pub fn mid(&self) -> Vec2 {
        (self.a + self.b) * 0.5
    }

    /// Unit normal of the wall line (CCW perpendicular of `a → b`).
    pub fn normal(&self) -> Vec2 {
        let d = (self.b - self.a).try_normalize().unwrap_or(Vec2::X);
        Vec2::new(-d.y, d.x)
    }
}

/// One straight flight in the local frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LStair {
    pub min: Vec2,
    pub max: Vec2,
    /// Climbing direction (unit, axis-aligned).
    pub up: Vec2,
    pub from: i32,
    pub to: i32,
}

impl LStair {
    /// Centre of the bottom (`top = false`) or top edge of the flight.
    pub fn end_center(&self, top: bool) -> Vec2 {
        let c = (self.min + self.max) * 0.5;
        let half = (self.max - self.min) * 0.5;
        let reach = self.up.abs().dot(half);
        if top { c + self.up * reach } else { c - self.up * reach }
    }

    /// Point on the landing beyond the bottom or top end, `depth` metres past
    /// the end wall.
    pub fn landing_point(&self, top: bool, depth: f32) -> Vec2 {
        let dir = if top { self.up } else { -self.up };
        self.end_center(top) + dir * (INTERIOR_WALL + depth)
    }
}

fn room_of(label: i32) -> Option<usize> {
    (label >= 0).then_some(label as usize)
}

/// Opening on `run` spanning metric `[t0, t1]` along it.
fn opening_on_run(run: &Run, g: &Grid, t0: f32, t1: f32, kind: OpeningKind, sill: f32, height: f32) -> LOpening {
    LOpening {
        kind,
        a: run.point(g, t0),
        b: run.point(g, t1),
        sill,
        height,
        rooms: [room_of(run.lo), room_of(run.hi)],
        thickness: run.thickness(),
    }
}

/// Wall segments: every grid edge between different labels, merged into
/// maximal straight runs of equal thickness.
pub(crate) fn walls(g: &Grid, labels: &[i32]) -> Vec<LWall> {
    let mut out = Vec::new();
    for axis in [Axis::X, Axis::Y] {
        let across = axis.other();
        for line in 0..=g.n(across) {
            let mut cur: Option<(i32, i32, bool)> = None;
            let flush = |cur: &mut Option<(i32, i32, bool)>, out: &mut Vec<LWall>| {
                if let Some((s0, s1, ext)) = cur.take() {
                    let c = g.line(across, line);
                    let (a, b) = match axis {
                        Axis::X => (Vec2::new(g.line(axis, s0), c), Vec2::new(g.line(axis, s1), c)),
                        Axis::Y => (Vec2::new(c, g.line(axis, s0)), Vec2::new(c, g.line(axis, s1))),
                    };
                    out.push(LWall { a, b, thickness: if ext { EXTERIOR_WALL } else { INTERIOR_WALL }, exterior: ext });
                }
            };
            for s in 0..g.n(axis) {
                let lo = label_at(g, labels, axis.cell(s, line - 1));
                let hi = label_at(g, labels, axis.cell(s, line));
                if lo == hi {
                    flush(&mut cur, &mut out);
                    continue;
                }
                let ext = lo == OUTSIDE || hi == OUTSIDE;
                match cur.as_mut() {
                    Some(c) if c.2 == ext && c.1 == s => c.1 = s + 1,
                    _ => {
                        flush(&mut cur, &mut out);
                        cur = Some((s, s + 1, ext));
                    }
                }
            }
            flush(&mut cur, &mut out);
        }
    }
    out
}

/// Rooms that read as hubs of circulation.
fn is_hub(k: RoomKind) -> bool {
    matches!(k, RoomKind::Hall | RoomKind::Corridor | RoomKind::Taproom | RoomKind::Living)
}

/// Rooms people should not have to walk through.
fn is_private(k: RoomKind) -> bool {
    matches!(
        k,
        RoomKind::Bedroom
            | RoomKind::Bath
            | RoomKind::Privy
            | RoomKind::Pantry
            | RoomKind::Storage
            | RoomKind::Study
            | RoomKind::Library
            | RoomKind::Chapel
            | RoomKind::Nursery
            | RoomKind::Servants
            | RoomKind::Cellar
            | RoomKind::Attic
    )
}

/// Cost of connecting two rooms with a door (lower = preferred).
fn door_cost(a: RoomKind, b: RoomKind) -> f32 {
    use RoomKind as K;
    if a == K::Stairwell || b == K::Stairwell {
        return -10.0;
    }
    let mut c = 1.0;
    if is_hub(a) || is_hub(b) {
        c -= 0.6;
    }
    if is_private(a) && is_private(b) {
        c += 2.0;
    }
    let pair = |x: K, y: K| (a == x && b == y) || (a == y && b == x);
    let preferred = [
        (K::Kitchen, K::Pantry),
        (K::Kitchen, K::Dining),
        (K::Hall, K::Dining),
        (K::Bedroom, K::Bath),
        (K::Shop, K::Workshop),
        (K::Kitchen, K::Storage),
        (K::Taproom, K::Kitchen),
        (K::Bedroom, K::Nursery),
        (K::Kitchen, K::Servants),
        (K::Library, K::Study),
        (K::Workshop, K::Storage),
        (K::Cellar, K::Cellar),
    ];
    if preferred.iter().any(|&(x, y)| pair(x, y)) {
        c -= 0.7;
    }
    if (a == K::Privy || b == K::Privy) && !(is_hub(a) || is_hub(b) || a == K::Kitchen || b == K::Kitchen) {
        c += 1.0;
    }
    c
}

/// Kind of opening between two rooms.
fn passage_kind(a: RoomKind, b: RoomKind) -> OpeningKind {
    use RoomKind as K;
    if a == K::Stairwell || b == K::Stairwell {
        return OpeningKind::Doorway;
    }
    let open = |k: K| matches!(k, K::Hall | K::Corridor | K::Living | K::Dining | K::Taproom);
    if open(a) && open(b) { OpeningKind::Doorway } else { OpeningKind::Door }
}

/// Interior doors: a spanning tree over door-capable walls grown from the root
/// (hubs preferred, private rooms as leaves) plus a few loops in big houses.
pub(crate) fn place_doors(lay: &LevelLayout, g: &Grid, stair: Option<&StairWell>, rng: &mut Rng) -> Vec<LOpening> {
    let n = lay.room_count();
    // Longest usable run per room pair.
    let mut best: BTreeMap<(usize, usize), (f32, Run)> = BTreeMap::new();
    for r in door_runs(lay, g, stair) {
        let (a, b) = (r.lo.min(r.hi) as usize, r.lo.max(r.hi) as usize);
        let len = r.usable_len(g, &lay.labels, JUNCTION_CLEARANCE);
        if best.get(&(a, b)).is_none_or(|(l, _)| len > *l) {
            best.insert((a, b), (len, r));
        }
    }
    let pairs: Vec<((usize, usize), Run, f32)> = best
        .into_iter()
        .map(|(k, (_, r))| (k, r, door_cost(lay.kinds[k.0], lay.kinds[k.1]) + rng.next_f32() * 0.3))
        .collect();
    let mut in_tree = vec![false; n];
    if lay.root < n {
        in_tree[lay.root] = true;
    }
    let mut chosen = vec![false; pairs.len()];
    loop {
        let next = pairs
            .iter()
            .enumerate()
            .filter(|(k, (p, _, _))| !chosen[*k] && in_tree[p.0] != in_tree[p.1])
            .min_by(|a, b| a.1.2.total_cmp(&b.1.2))
            .map(|(k, _)| k);
        let Some(k) = next else { break };
        chosen[k] = true;
        let p = pairs[k].0;
        in_tree[p.0] = true;
        in_tree[p.1] = true;
    }
    // A few extra connections between public rooms in larger houses.
    let rooms = lay.kinds.iter().filter(|&&k| k != RoomKind::Stairwell).count();
    if rooms >= 6 {
        let social = |k: RoomKind| matches!(k, RoomKind::Hall | RoomKind::Corridor | RoomKind::Dining | RoomKind::Living | RoomKind::Taproom | RoomKind::Kitchen);
        let mut extra = 0;
        for (k, (p, _, _)) in pairs.iter().enumerate() {
            if extra < 2 && !chosen[k] && social(lay.kinds[p.0]) && social(lay.kinds[p.1]) && rng.chance(0.5) {
                chosen[k] = true;
                extra += 1;
            }
        }
    }
    let mut out = Vec::new();
    for (k, (p, run, _)) in pairs.iter().enumerate() {
        if !chosen[k] {
            continue;
        }
        let kind = passage_kind(lay.kinds[p.0], lay.kinds[p.1]);
        let (u0, u1) = run.usable(g, &lay.labels, JUNCTION_CLEARANCE);
        let len = u1 - u0;
        let stairwell = lay.kinds[p.0] == RoomKind::Stairwell || lay.kinds[p.1] == RoomKind::Stairwell;
        let (w, jitter) = if stairwell {
            (len.min(1.8), 0.0)
        } else if kind == OpeningKind::Doorway {
            (rng.range_f32(1.0, 1.4).min(len), 0.4)
        } else {
            (rng.range_f32(DOOR_MIN_WIDTH, 1.0).min(len), 0.4)
        };
        let slack = (len - w).max(0.0);
        let c = u0 + len * 0.5 + (rng.next_f32() * 2.0 - 1.0) * slack * 0.5 * jitter;
        let h = if kind == OpeningKind::Doorway { 2.2 } else { DOOR_HEIGHT };
        out.push(opening_on_run(run, g, c - w * 0.5, c + w * 0.5, kind, 0.0, h));
    }
    out
}

/// The main entrance on the ground floor: the best exterior run of the entry
/// room facing the street (or failing that, any public room's).
pub(crate) fn place_entrance(lay: &LevelLayout, g: &Grid, front: Side, app: &Appearance, rng: &mut Rng) -> Option<LOpening> {
    use RoomKind as K;
    let (want_w, height) = crate::style::entrance_size(app, rng);
    let mut best: Option<(f32, Run, f32, f32)> = None;
    for run in runs::runs(g, &lay.labels) {
        if !run.exterior() {
            continue;
        }
        let room = run.other(OUTSIDE);
        if room < 0 || Some(room as usize) == lay.stairwell {
            continue;
        }
        let (u0, u1) = run.usable(g, &lay.labels, 0.1);
        let len = u1 - u0;
        if len < DOOR_MIN_WIDTH + 0.05 {
            continue;
        }
        let outward = -run.normal_into(room);
        let kind = lay.kinds[room as usize];
        let mut score = outward.dot(front.normal()) * 10.0 + len.min(3.0) * 0.6;
        if room as usize == lay.root {
            score += 12.0;
        }
        score += match kind {
            K::Hall | K::Taproom | K::Shop | K::Living => 4.0,
            K::Kitchen | K::Workshop | K::Corridor => 2.0,
            K::Bedroom | K::Bath | K::Privy | K::Pantry | K::Storage | K::Cellar | K::Nursery => -6.0,
            _ => 0.0,
        };
        if len < want_w {
            score -= 4.0;
        }
        if best.as_ref().is_none_or(|b| score > b.0) {
            best = Some((score, run, u0, u1));
        }
    }
    // Fallback for cramped footprints: the longest exterior wall of any room.
    let best = best.or_else(|| {
        runs::runs(g, &lay.labels)
            .into_iter()
            .filter(|r| r.exterior() && r.other(OUTSIDE) >= 0)
            .map(|r| {
                let (u0, u1) = r.usable(g, &lay.labels, 0.0);
                (u1 - u0, r, u0, u1)
            })
            .filter(|c| c.0 > 0.3)
            .max_by(|a, b| a.0.total_cmp(&b.0))
    });
    let (_, run, u0, u1) = best?;
    let len = u1 - u0;
    let w = want_w.min(len);
    let slack = len - w;
    let c = u0 + len * 0.5 + (rng.next_f32() * 2.0 - 1.0) * slack * 0.15;
    Some(opening_on_run(&run, g, c - w * 0.5, c + w * 0.5, OpeningKind::Entrance, 0.0, height))
}

/// How a room kind is lit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lighting {
    None,
    Small,
    Normal,
}

fn lighting(k: RoomKind) -> Lighting {
    use RoomKind as K;
    match k {
        K::Cellar => Lighting::None,
        K::Storage | K::Pantry | K::Privy | K::Stairwell | K::Attic => Lighting::Small,
        _ => Lighting::Normal,
    }
}

/// Windows on exterior walls. Window centres follow a regular bay rhythm per
/// facade so they line up between storeys; every habitable room with a long
/// enough exterior wall gets at least one.
pub(crate) fn place_windows(lay: &LevelLayout, g: &Grid, ws: &WindowStyle, front: Side, entrance: Option<&LOpening>, clear: f32) -> Vec<LOpening> {
    let mut out = Vec::new();
    if lay.level < 0 {
        return out;
    }
    // Bay centres per exterior wall line.
    let mut bays: Vec<(Axis, f32, Vec<f32>)> = Vec::new();
    for w in walls(g, &lay.labels).into_iter().filter(|w| w.exterior) {
        let axis = if (w.b.y - w.a.y).abs() < 1e-5 { Axis::X } else { Axis::Y };
        let (s0, s1, line) = match axis {
            Axis::X => (w.a.x.min(w.b.x), w.a.x.max(w.b.x), w.a.y),
            Axis::Y => (w.a.y.min(w.b.y), w.a.y.max(w.b.y), w.a.x),
        };
        let corner = 0.6;
        let usable = s1 - s0 - 2.0 * corner;
        if usable < ws.width {
            continue;
        }
        let mut nb = ((usable / ws.spacing).round() as i32).max(1);
        while nb > 1 && nb as f32 * ws.width + (nb - 1) as f32 * 0.6 > usable {
            nb -= 1;
        }
        let centres = (0..nb).map(|k| s0 + corner + (k as f32 + 0.5) * usable / nb as f32).collect();
        bays.push((axis, line, centres));
    }
    let max_h = (clear - 0.2).max(1.0);
    for run in runs::runs(g, &lay.labels) {
        if !run.exterior() {
            continue;
        }
        let room = run.other(OUTSIDE);
        if room < 0 {
            continue;
        }
        let kind = lay.kinds[room as usize];
        let light = lighting(kind);
        if light == Lighting::None {
            continue;
        }
        let outward = -run.normal_into(room);
        let facing_front = outward.dot(front.normal()) > 0.9;
        let (w, h, sill) = match light {
            Lighting::Small if kind == RoomKind::Stairwell => (0.6, 1.0, 1.2),
            Lighting::Small => (0.5, 0.6, 1.4),
            _ if lay.level == 0 && facing_front && kind == RoomKind::Shop => (1.7, 1.5, 0.7),
            _ => (ws.width, ws.height, ws.sill),
        };
        let h = h.min(max_h - sill).max(0.4);
        let (u0, u1) = run.usable(g, &lay.labels, 0.35);
        if u1 - u0 < w {
            continue;
        }
        let line = run.line_coord(g);
        let conflicts = |c: f32, placed: &[(f32, f32)]| -> bool {
            let (lo, hi) = (c - w * 0.5, c + w * 0.5);
            if placed.iter().any(|&(a, b)| lo < b + 0.4 && hi > a - 0.4) {
                return true;
            }
            if let Some(e) = entrance {
                let (ea, eb, el) = match run.axis {
                    Axis::X => (e.a.x.min(e.b.x), e.a.x.max(e.b.x), e.a.y),
                    Axis::Y => (e.a.y.min(e.b.y), e.a.y.max(e.b.y), e.a.x),
                };
                let same_line = (el - line).abs() < 1e-3 && ((e.b - e.a).abs().dot(run.axis.unit()) > 1e-3);
                if same_line && lo < eb + 0.3 && hi > ea - 0.3 {
                    return true;
                }
            }
            false
        };
        let mut placed: Vec<(f32, f32)> = Vec::new();
        let max_count = if light == Lighting::Small { 1 } else { usize::MAX };
        for (axis, l, centres) in &bays {
            if *axis != run.axis || (l - line).abs() > 1e-3 {
                continue;
            }
            for &c in centres {
                if placed.len() >= max_count {
                    break;
                }
                if c - w * 0.5 >= u0 && c + w * 0.5 <= u1 && !conflicts(c, &placed) {
                    placed.push((c - w * 0.5, c + w * 0.5));
                }
            }
        }
        if placed.is_empty() {
            for c in [(u0 + u1) * 0.5, u0 + w * 0.5, u1 - w * 0.5] {
                if !conflicts(c, &placed) {
                    placed.push((c - w * 0.5, c + w * 0.5));
                    break;
                }
            }
        }
        for (a, b) in placed {
            out.push(opening_on_run(&run, g, a, b, OpeningKind::Window, sill, h));
        }
    }
    out
}

/// One flight per consecutive pair of `levels` inside the stairwell.
pub(crate) fn flights(stair: &StairWell, g: &Grid, levels: &[i32]) -> Vec<LStair> {
    let axis = stair.axis;
    let across = axis.other();
    let side_half = |low: bool| -> f32 {
        let b = if low { stair.rect.lo(across) - 1 } else { stair.rect.hi(across) };
        let ext = (stair.rect.lo(axis)..stair.rect.hi(axis)).any(|a| !g.inside(axis.cell(a, b)));
        if ext { EXTERIOR_WALL * 0.5 } else { INTERIOR_WALL * 0.5 }
    };
    let mut out = Vec::new();
    for k in 0..levels.len().saturating_sub(1) {
        let lane = stair.lane(k % 2);
        let a0 = g.line(axis, stair.rect.lo(axis)) + INTERIOR_WALL * 0.5;
        let a1 = g.line(axis, stair.rect.hi(axis)) - INTERIOR_WALL * 0.5;
        let mut b0 = g.line(across, lane.lo(across));
        let mut b1 = g.line(across, lane.hi(across));
        if k % 2 == 0 {
            b0 += side_half(true);
            b1 -= 0.03;
        } else {
            b0 += 0.03;
            b1 -= side_half(false);
        }
        let (min, max) = match axis {
            Axis::X => (Vec2::new(a0, b0), Vec2::new(a1, b1)),
            Axis::Y => (Vec2::new(b0, a0), Vec2::new(b1, a1)),
        };
        let up = if stair.start_end(k) == 0 { axis.unit() } else { -axis.unit() };
        out.push(LStair { min, max, up, from: levels[k], to: levels[k + 1] });
    }
    out
}
