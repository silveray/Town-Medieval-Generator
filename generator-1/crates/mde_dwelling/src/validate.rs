//! Invariant checks on a finished [`Dwelling`], working only from the output
//! data (so they also apply to hand-edited dwellings).
//!
//! [`validate`] returns human-readable problems; an empty list means the
//! dwelling is consistent and walkable: one entrance, openings on walls,
//! windows on exterior walls, rooms inside the outline, aligned stairs,
//! furniture inside rooms and clear of doors, landings and each other, every
//! room reachable through doors and stairs, and a 0.25 m walk grid that
//! reaches every door and stair landing from the entrance.

use std::collections::BTreeMap;

use mde_core::{poly::cross, Dwelling, Floor, OpeningKind, Polygon, Prop, RoomKind, Stair, Vec2, SCHEMA_VERSION};

use crate::walk::{point_in_polygon, Obb, WalkGrid};

/// Depth of the keep-clear zone checked on both sides of every door.
pub const DOOR_CLEARANCE: f32 = 0.9;
/// Depth of the keep-clear landing checked beyond both ends of every flight.
pub const LANDING_DEPTH: f32 = 0.6;
/// Walk-grid resolution used by [`validate`].
pub const WALK_RESOLUTION: f32 = 0.25;

/// All problems found in `d` (empty = valid).
pub fn validate(d: &Dwelling) -> Vec<String> {
    let mut errs = Vec::new();
    structure(d, &mut errs);
    for f in &d.floors {
        floor_geometry(f, &mut errs);
        furniture(d, f, &mut errs);
    }
    stairs(d, &mut errs);
    connectivity(d, &mut errs);
    errs.extend(walkability(d, WALK_RESOLUTION, 0.0));
    errs
}

/// Rotation (radians) that makes the dwelling's walls axis-aligned: the
/// direction of its longest wall.
pub fn dominant_angle(d: &Dwelling) -> f32 {
    let mut best = (0.0f32, 0.0f32);
    for f in &d.floors {
        for w in &f.walls {
            let v = w.b - w.a;
            let l = v.length();
            if l > best.0 {
                best = (l, v.y.atan2(v.x));
            }
        }
    }
    best.1
}

fn structure(d: &Dwelling, errs: &mut Vec<String>) {
    if d.schema != SCHEMA_VERSION {
        errs.push(format!("schema {} != {}", d.schema, SCHEMA_VERSION));
    }
    if d.floors.is_empty() {
        errs.push("no floors".into());
        return;
    }
    for w in d.floors.windows(2) {
        if w[1].level != w[0].level + 1 {
            errs.push(format!("levels not consecutive: {} then {}", w[0].level, w[1].level));
        }
    }
    if !d.floors.iter().any(|f| f.level == 0) {
        errs.push("no ground floor".into());
    }
    let entrances: Vec<(i32, &mde_core::Opening)> =
        d.floors.iter().flat_map(|f| f.openings.iter().filter(|o| o.kind == OpeningKind::Entrance).map(move |o| (f.level, o))).collect();
    if entrances.len() != 1 {
        errs.push(format!("{} entrances (expected 1)", entrances.len()));
    }
    for (level, o) in &entrances {
        if *level != 0 {
            errs.push(format!("entrance on level {level}"));
        }
        if o.rooms.iter().filter(|r| r.is_some()).count() != 1 {
            errs.push("entrance must join one room to the outside".into());
        }
    }
    let mut ids = BTreeMap::new();
    for f in &d.floors {
        if f.rooms.is_empty() {
            errs.push(format!("level {}: no rooms", f.level));
        }
        for r in &f.rooms {
            if ids.insert(r.id, f.level).is_some() {
                errs.push(format!("duplicate room id {}", r.id));
            }
        }
    }
}

/// Distance from `p` to segment `a–b` and the parameter along it.
fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> (f32, f32) {
    let ab = b - a;
    let l2 = ab.length_squared().max(1e-12);
    let t = (p - a).dot(ab) / l2;
    let q = a + ab * t.clamp(0.0, 1.0);
    (p.distance(q), t)
}

/// The wall an opening sits on, if any.
fn host_wall(f: &Floor, a: Vec2, b: Vec2) -> Option<&mde_core::WallSegment> {
    f.walls.iter().find(|w| {
        let len = w.a.distance(w.b).max(1e-6);
        let tol = 1e-3 / len;
        let (da, ta) = seg_dist(a, w.a, w.b);
        let (db, tb) = seg_dist(b, w.a, w.b);
        da < 1e-3 && db < 1e-3 && ta >= -tol && ta <= 1.0 + tol && tb >= -tol && tb <= 1.0 + tol
    })
}

fn floor_geometry(f: &Floor, errs: &mut Vec<String>) {
    let lv = f.level;
    let room_ids: Vec<u32> = f.rooms.iter().map(|r| r.id).collect();
    for r in &f.rooms {
        if r.shape.len() < 4 || r.shape.signed_area() <= 0.0 {
            errs.push(format!("level {lv}: room {} shape not CCW / degenerate", r.id));
        }
        for &p in &r.shape.0 {
            if !f.outline.contains(p) && f.outline.distance_to_boundary(p) > 1e-3 {
                errs.push(format!("level {lv}: room {} vertex {p} outside outline", r.id));
                break;
            }
        }
    }
    for o in &f.openings {
        let Some(w) = host_wall(f, o.a, o.b) else {
            errs.push(format!("level {lv}: {:?} {}–{} not on a wall", o.kind, o.a, o.b));
            continue;
        };
        match o.kind {
            OpeningKind::Window | OpeningKind::Entrance if !w.exterior => {
                errs.push(format!("level {lv}: {:?} on an interior wall", o.kind));
            }
            OpeningKind::Door | OpeningKind::Doorway if w.exterior => {
                errs.push(format!("level {lv}: {:?} on an exterior wall", o.kind));
            }
            _ => {}
        }
        if o.kind != OpeningKind::Window && o.a.distance(o.b) < crate::DOOR_MIN_WIDTH - 1e-3 {
            errs.push(format!("level {lv}: {:?} only {:.2} m wide", o.kind, o.a.distance(o.b)));
        }
        for r in o.rooms.iter().flatten() {
            if !room_ids.contains(r) {
                errs.push(format!("level {lv}: opening references unknown room {r}"));
            }
        }
    }
}

fn prop_obb(p: &Prop) -> Obb {
    Obb::from_prop(p.pos, p.size, p.rotation)
}

/// Oriented box of a flight's footprint (aligned with `up`).
pub fn stair_obb(s: &Stair) -> Obb {
    let up = s.up.try_normalize().unwrap_or(Vec2::Y);
    let across = Vec2::new(-up.y, up.x);
    let (mut a0, mut a1, mut b0, mut b1) = (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY);
    for p in &s.footprint.0 {
        a0 = a0.min(p.dot(up));
        a1 = a1.max(p.dot(up));
        b0 = b0.min(p.dot(across));
        b1 = b1.max(p.dot(across));
    }
    Obb { center: up * ((a0 + a1) * 0.5) + across * ((b0 + b1) * 0.5), u: up, half: Vec2::new((a1 - a0) * 0.5, (b1 - b0) * 0.5) }
}

/// Landing boxes beyond the bottom and top of a flight.
pub fn landing_boxes(s: &Stair, depth: f32) -> (Obb, Obb) {
    let up = s.up.try_normalize().unwrap_or(Vec2::Y);
    let across = Vec2::new(-up.y, up.x);
    let (mut a0, mut a1, mut b0, mut b1) = (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY);
    for p in &s.footprint.0 {
        a0 = a0.min(p.dot(up));
        a1 = a1.max(p.dot(up));
        b0 = b0.min(p.dot(across));
        b1 = b1.max(p.dot(across));
    }
    let mid_b = (b0 + b1) * 0.5;
    let half = Vec2::new(depth * 0.5, (b1 - b0) * 0.5);
    let bottom = Obb { center: up * (a0 - crate::INTERIOR_WALL - depth * 0.5) + across * mid_b, u: up, half };
    let top = Obb { center: up * (a1 + crate::INTERIOR_WALL + depth * 0.5) + across * mid_b, u: up, half };
    (bottom, top)
}

fn furniture(d: &Dwelling, f: &Floor, errs: &mut Vec<String>) {
    let lv = f.level;
    let mut zones: Vec<Obb> = Vec::new();
    for o in f.openings.iter().filter(|o| o.kind != OpeningKind::Window) {
        let th = host_wall(f, o.a, o.b).map(|w| w.thickness).unwrap_or(0.0);
        zones.push(Obb::from_segment(o.a, o.b, th * 0.5 + DOOR_CLEARANCE, 0.0));
    }
    for fl in &d.floors {
        for s in &fl.stairs {
            let (bottom, top) = landing_boxes(s, LANDING_DEPTH);
            if s.from_level == lv {
                zones.push(bottom);
            }
            if s.to_level == lv {
                zones.push(top);
            }
            if s.from_level == lv || s.to_level == lv {
                zones.push(stair_obb(s).inflate(-0.01));
            }
        }
    }
    let boxes: Vec<Obb> = f.props.iter().map(prop_obb).collect();
    for (k, p) in f.props.iter().enumerate() {
        let b = boxes[k];
        match f.rooms.iter().find(|r| r.id == p.room) {
            None => errs.push(format!("level {lv}: prop {k} in unknown room {}", p.room)),
            Some(r) => {
                let inner = b.inflate(-1e-3);
                if !inner.corners().iter().all(|&c| point_in_polygon(&r.shape.0, c)) {
                    errs.push(format!("level {lv}: {:?} {k} outside room {}", p.kind, r.id));
                }
            }
        }
        for (j, other) in boxes.iter().enumerate().skip(k + 1) {
            if b.overlaps(other, 1e-3) {
                errs.push(format!("level {lv}: props {k} ({:?}) and {j} ({:?}) overlap", p.kind, f.props[j].kind));
            }
        }
        for z in &zones {
            if b.overlaps(z, 1e-3) {
                errs.push(format!("level {lv}: {:?} {k} blocks a door or stair landing", p.kind));
                break;
            }
        }
        for w in &f.walls {
            if b.overlaps(&Obb::from_segment(w.a, w.b, w.thickness * 0.5, 0.0), 1e-3) {
                errs.push(format!("level {lv}: {:?} {k} intersects a wall", p.kind));
                break;
            }
        }
    }
}

fn stairs(d: &Dwelling, errs: &mut Vec<String>) {
    let all: Vec<&Stair> = d.floors.iter().flat_map(|f| f.stairs.iter()).collect();
    for w in d.floors.windows(2) {
        let n = all.iter().filter(|s| s.from_level == w[0].level && s.to_level == w[1].level).count();
        if n != 1 {
            errs.push(format!("{} flights between levels {} and {}", n, w[0].level, w[1].level));
        }
    }
    if let Some(first) = all.first() {
        for s in &all {
            if cross(first.up, s.up).abs() > 1e-3 {
                errs.push("stair flights not aligned".into());
            }
        }
        // All flights inside one stairwell footprint: their union fits in the
        // stairwell room of every level.
        for f in &d.floors {
            let well = f.rooms.iter().find(|r| r.kind == RoomKind::Stairwell);
            match well {
                None => errs.push(format!("level {}: no stairwell", f.level)),
                Some(r) => {
                    for s in &all {
                        let c = s.footprint.centroid();
                        if !point_in_polygon(&r.shape.0, c) {
                            errs.push(format!("level {}: flight {}→{} not inside the stairwell", f.level, s.from_level, s.to_level));
                        }
                    }
                }
            }
        }
    }
}

/// Room graph through passages and stairs.
fn connectivity(d: &Dwelling, errs: &mut Vec<String>) {
    let mut adj: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut all_rooms = Vec::new();
    let mut start = None;
    for f in &d.floors {
        for r in &f.rooms {
            all_rooms.push(r.id);
            adj.entry(r.id).or_default();
        }
        for o in f.openings.iter().filter(|o| o.kind != OpeningKind::Window) {
            match o.rooms {
                [Some(a), Some(b)] => {
                    adj.entry(a).or_default().push(b);
                    adj.entry(b).or_default().push(a);
                }
                [Some(a), None] | [None, Some(a)] if o.kind == OpeningKind::Entrance => start = Some(a),
                _ => {}
            }
        }
    }
    // Stairs connect the stairwell rooms of their two levels.
    let well_of = |level: i32, p: Vec2| -> Option<u32> {
        d.floors.iter().find(|f| f.level == level)?.rooms.iter().find(|r| r.kind == RoomKind::Stairwell && point_in_polygon(&r.shape.0, p)).map(|r| r.id)
    };
    for f in &d.floors {
        for s in &f.stairs {
            let c = s.footprint.centroid();
            if let (Some(a), Some(b)) = (well_of(s.from_level, c), well_of(s.to_level, c)) {
                adj.entry(a).or_default().push(b);
                adj.entry(b).or_default().push(a);
            }
        }
    }
    let Some(start) = start else { return };
    let mut seen = vec![start];
    let mut stack = vec![start];
    while let Some(r) = stack.pop() {
        for &n in adj.get(&r).map(|v| v.as_slice()).unwrap_or(&[]) {
            if !seen.contains(&n) {
                seen.push(n);
                stack.push(n);
            }
        }
    }
    for r in all_rooms {
        if !seen.contains(&r) {
            errs.push(format!("room {r} not reachable from the entrance"));
        }
    }
}

/// Rotate a plan point into the dwelling's axis-aligned frame.
fn rot(p: Vec2, angle: f32) -> Vec2 {
    Vec2::from_angle(-angle).rotate(p)
}

/// One floor rasterised for walking, in the dwelling's axis-aligned frame.
struct FloorWalk {
    level: i32,
    grid: WalkGrid,
    /// Passage midpoints that must be reachable.
    targets: Vec<(String, Vec2)>,
}

fn floor_walk(d: &Dwelling, f: &Floor, angle: f32, res: f32, clearance: f32) -> FloorWalk {
    let r = |p: Vec2| rot(p, angle);
    let outline: Vec<Vec2> = f.outline.0.iter().map(|&p| r(p)).collect();
    let bounds = Polygon(outline).bounds();
    let mut grid = WalkGrid::new(bounds.min - Vec2::splat(0.5), bounds.max + Vec2::splat(0.5), res);
    for room in &f.rooms {
        let pts: Vec<Vec2> = room.shape.0.iter().map(|&p| r(p)).collect();
        grid.fill_polygon(&pts, true);
    }
    for w in &f.walls {
        grid.block_obb(&Obb::from_segment(r(w.a), r(w.b), w.thickness * 0.5 + clearance, w.thickness * 0.5));
    }
    let mut targets = Vec::new();
    for o in f.openings.iter().filter(|o| o.kind != OpeningKind::Window) {
        let th = host_wall(f, o.a, o.b).map(|w| w.thickness).unwrap_or(0.4);
        // Re-open the door gap through the (conservatively grown) wall.
        grid.fill_obb(&Obb::from_segment(r(o.a), r(o.b), th * 0.5 + clearance + res * 0.5 + 0.02, -clearance - res * 0.5), true);
        targets.push((format!("level {}: {:?} at {}", f.level, o.kind, (o.a + o.b) * 0.5), r((o.a + o.b) * 0.5)));
    }
    for fl in &d.floors {
        for s in fl.stairs.iter().filter(|s| s.from_level == f.level || s.to_level == f.level) {
            let o = stair_obb(s);
            grid.block_obb(&Obb { center: r(o.center), u: rot(o.u, angle), half: o.half }.inflate(clearance));
        }
    }
    for p in &f.props {
        let o = prop_obb(p);
        grid.block_obb(&Obb { center: r(o.center), u: rot(o.u, angle), half: o.half }.inflate(clearance));
    }
    FloorWalk { level: f.level, grid, targets }
}

/// ASCII picture of one floor's walk grid (`#` blocked, `.` free, `D` door
/// midpoints, `L` stair landings), top row = north in the dwelling's frame.
/// Handy when a walkability check fails.
pub fn walk_map(d: &Dwelling, level: i32, res: f32, clearance: f32) -> String {
    let Some(f) = d.floors.iter().find(|f| f.level == level) else {
        return String::new();
    };
    let angle = dominant_angle(d);
    let w = floor_walk(d, f, angle, res, clearance);
    let mut marks: Vec<(Vec2, char)> = w.targets.iter().map(|(_, p)| (*p, 'D')).collect();
    for fl in &d.floors {
        for s in &fl.stairs {
            let (bottom, top) = landing_boxes(s, LANDING_DEPTH);
            if s.from_level == level {
                marks.push((rot(bottom.center, angle), 'L'));
            }
            if s.to_level == level {
                marks.push((rot(top.center, angle), 'L'));
            }
        }
    }
    let mut out = String::new();
    for j in (0..w.grid.h).rev() {
        for i in 0..w.grid.w {
            let c = w.grid.center(i, j);
            let mark = marks.iter().find(|(p, _)| (p.x - c.x).abs() <= res * 0.5 && (p.y - c.y).abs() <= res * 0.5);
            out.push(match mark {
                Some((_, m)) => *m,
                None if w.grid.free[(j * w.grid.w + i) as usize] => '.',
                None => '#',
            });
        }
        out.push('\n');
    }
    out
}

/// Walk-grid reachability: rasterise each floor (rooms minus walls, stairs and
/// furniture, grown by `clearance`) at `res` metres in the dwelling's own axes,
/// blocking every sample an obstacle touches; flood from the entrance,
/// crossing floors through stair landings, and report every door / doorway and
/// landing that cannot be reached.
pub fn walkability(d: &Dwelling, res: f32, clearance: f32) -> Vec<String> {
    let mut errs = Vec::new();
    let angle = dominant_angle(d);
    let r = |p: Vec2| rot(p, angle);
    let all_stairs: Vec<&Stair> = d.floors.iter().flat_map(|f| f.stairs.iter()).collect();
    let walks: Vec<FloorWalk> = d.floors.iter().map(|f| floor_walk(d, f, angle, res, clearance)).collect();

    // Seeds: entrance on the ground floor.
    let mut seeds: BTreeMap<i32, Vec<Vec2>> = BTreeMap::new();
    if let Some(e) = crate::entrance(d) {
        let mid = (e.a + e.b) * 0.5;
        let dir = (e.b - e.a).try_normalize().unwrap_or(Vec2::X);
        let n = Vec2::new(-dir.y, dir.x);
        let ground = d.floors.iter().find(|f| f.level == 0);
        let inside = |p: Vec2| ground.is_some_and(|f| f.rooms.iter().any(|room| point_in_polygon(&room.shape.0, p)));
        let step = 0.55;
        let p = if inside(mid + n * step) { mid + n * step } else { mid - n * step };
        seeds.entry(0).or_default().push(r(p));
    } else {
        errs.push("no entrance to start the walk from".into());
        return errs;
    }
    let landings: Vec<(i32, Vec2, i32, Vec2)> = all_stairs
        .iter()
        .map(|s| {
            let (bottom, top) = landing_boxes(s, LANDING_DEPTH);
            (s.from_level, r(bottom.center), s.to_level, r(top.center))
        })
        .collect();
    let mut reached_any: BTreeMap<i32, Vec<bool>> = BTreeMap::new();
    // Propagate across floors until nothing changes.
    for _ in 0..=(2 * d.floors.len()) {
        let mut changed = false;
        for w in &walks {
            let starts: Vec<usize> = seeds.get(&w.level).map(|v| v.iter().flat_map(|&p| w.grid.free_near(p, 0.35)).collect()).unwrap_or_default();
            let reached = w.grid.flood(&starts);
            for &(lf, pb, lt, pt) in &landings {
                if lf == w.level && w.grid.reached_near(&reached, pb, 0.35) {
                    let e = seeds.entry(lt).or_default();
                    if !e.iter().any(|q| q.distance(pt) < 1e-4) {
                        e.push(pt);
                        changed = true;
                    }
                }
                if lt == w.level && w.grid.reached_near(&reached, pt, 0.35) {
                    let e = seeds.entry(lf).or_default();
                    if !e.iter().any(|q| q.distance(pb) < 1e-4) {
                        e.push(pb);
                        changed = true;
                    }
                }
            }
            reached_any.insert(w.level, reached);
        }
        if !changed {
            break;
        }
    }
    for w in &walks {
        let reached = reached_any.get(&w.level).cloned().unwrap_or_default();
        if reached.is_empty() {
            continue;
        }
        for (name, p) in &w.targets {
            if !w.grid.reached_near(&reached, *p, 0.35) {
                errs.push(format!("walk: cannot reach {name}"));
            }
        }
        for &(lf, pb, lt, pt) in &landings {
            if lf == w.level && !w.grid.reached_near(&reached, pb, 0.35) {
                errs.push(format!("walk: level {}: bottom landing of flight {lf}→{lt} unreachable", w.level));
            }
            if lt == w.level && !w.grid.reached_near(&reached, pt, 0.35) {
                errs.push(format!("walk: level {}: top landing of flight {lf}→{lt} unreachable", w.level));
            }
        }
    }
    errs
}
