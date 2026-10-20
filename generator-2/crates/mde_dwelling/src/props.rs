//! Furniture: per-room item lists placed against walls or free-standing, never
//! in door clearance zones, stair landings, the stairwell or in front of
//! windows (tall items), and only where every door and landing stays reachable.

use mde_core::{DwellingStyle, PropKind, RoomKind, Rng, Vec2};

use crate::build::{LOpening, LStair, LWall};
use crate::grid::{Axis, CellRect, Grid};
use crate::layout::LevelLayout;
use crate::runs::{self, Run};
use crate::walk::{Obb, WalkGrid};

/// Clearance kept from walls and stairs on the walk grid.
const WALL_CLEARANCE: f32 = 0.15;
/// Clearance kept from furniture on the walk grid: paths between pieces stay
/// at least ~0.7 m wide, whatever the sampling of a later check.
const PROP_CLEARANCE: f32 = 0.35;
/// Depth of the keep-clear zone on both sides of a door.
pub(crate) const DOOR_ZONE: f32 = 0.95;
/// Resolution of the walkability grid.
const WALK_RES: f32 = 0.25;

/// Furniture item in the local frame (axis-aligned box).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LProp {
    pub kind: PropKind,
    pub room: usize,
    pub min: Vec2,
    pub max: Vec2,
    /// Direction the item's front faces (unit, axis-aligned).
    pub facing: Vec2,
}

impl LProp {
    /// (width across the facing direction, depth along it).
    pub fn size(&self) -> Vec2 {
        let e = self.max - self.min;
        if self.facing.x.abs() > 0.5 { Vec2::new(e.y, e.x) } else { e }
    }

    pub fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }
}

/// Axis-aligned box in the local frame.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Aabb {
    min: Vec2,
    max: Vec2,
}

impl Aabb {
    fn of(o: &Obb) -> Aabb {
        let (min, max) = o.aabb();
        Aabb { min, max }
    }

    /// Overlaps the box `min..max` unless separated by at least `gap`
    /// (negative `gap` tolerates that much interpenetration).
    fn hits(&self, min: Vec2, max: Vec2, gap: f32) -> bool {
        !(self.max.x <= min.x - gap || max.x <= self.min.x - gap || self.max.y <= min.y - gap || max.y <= self.min.y - gap)
    }
}

/// Axis-aligned box from a centre, (width, depth) and facing.
fn boxed(center: Vec2, w: f32, d: f32, facing: Vec2) -> (Vec2, Vec2) {
    let half = if facing.x.abs() > 0.5 { Vec2::new(d, w) * 0.5 } else { Vec2::new(w, d) * 0.5 };
    (center - half, center + half)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Place {
    /// Back against any wall.
    Wall,
    /// Prefer an exterior wall (hearths: chimney).
    WallExt,
    /// Prefer an interior wall, away from doors (beds, altars).
    WallInt,
    /// Against a wall near a corner.
    Corner,
    /// Free-standing near the middle of the room.
    Centre,
    /// `n` chairs around the previous (table) item.
    Seats(u8),
    /// Benches along both long sides of the previous (table) item.
    Benches,
    /// A chair in front of the previous (desk) item.
    DeskChair,
    /// Rows of benches facing the previous (altar) item.
    Pews,
}

#[derive(Clone, Copy, Debug)]
struct Item {
    kind: PropKind,
    w: f32,
    d: f32,
    place: Place,
    tall: bool,
}

fn it(kind: PropKind, w: f32, d: f32, place: Place) -> Item {
    let tall = matches!(kind, PropKind::Wardrobe | PropKind::Shelf | PropKind::Bookshelf | PropKind::Hearth | PropKind::Stove);
    Item { kind, w, d, place, tall }
}

/// Item list for a room.
fn items(kind: RoomKind, style: DwellingStyle, area: f32, rng: &mut Rng) -> Vec<Item> {
    use Place as P;
    use PropKind as K;
    let mut v = Vec::new();
    let maybe = |v: &mut Vec<Item>, p: f32, item: Item, rng: &mut Rng| {
        if rng.chance(p) {
            v.push(item);
        }
    };
    match kind {
        RoomKind::Bedroom => {
            let (bw, bd) = match style {
                DwellingStyle::Manor => (1.6, 2.1),
                DwellingStyle::Tavern => (if rng.chance(0.5) { 0.9 } else { 1.4 }, 2.0),
                _ => (1.4, 2.0),
            };
            v.push(it(K::Bed, bw, bd, P::WallInt));
            if style == DwellingStyle::Tavern && bw < 1.0 && area > 11.0 {
                v.push(it(K::Bed, bw, bd, P::WallInt));
            }
            v.push(it(K::Chest, 0.9, 0.5, P::Wall));
            maybe(&mut v, if style == DwellingStyle::Manor { 1.0 } else { 0.5 }, it(K::Wardrobe, 1.1, 0.6, P::Wall), rng);
            if rng.chance(if style == DwellingStyle::Manor { 0.6 } else { 0.25 }) {
                v.push(it(K::Desk, 1.0, 0.55, P::Wall));
                v.push(it(K::Chair, 0.45, 0.45, P::DeskChair));
            }
        }
        RoomKind::Living => {
            v.push(it(K::Hearth, 1.6, 0.6, P::WallExt));
            v.push(it(K::Table, 1.4, 0.8, P::Centre));
            v.push(it(K::Chair, 0.45, 0.45, P::Seats(4)));
            v.push(it(K::Bench, 1.4, 0.4, P::Wall));
            maybe(&mut v, 0.7, it(K::Shelf, 1.0, 0.35, P::Wall), rng);
            maybe(&mut v, 0.5, it(K::Chest, 0.9, 0.5, P::Wall), rng);
        }
        RoomKind::Kitchen => {
            if matches!(style, DwellingStyle::Manor | DwellingStyle::Tavern) {
                v.push(it(K::Hearth, 2.0, 0.8, P::WallExt));
                v.push(it(K::Stove, 1.2, 0.7, P::Wall));
            } else {
                v.push(it(K::Hearth, 1.6, 0.7, P::WallExt));
            }
            v.push(it(K::Counter, 1.6, 0.6, P::Wall));
            if style == DwellingStyle::Farmhouse {
                v.push(it(K::Table, 1.9, 0.9, P::Centre));
                v.push(it(K::Bench, 0.0, 0.0, P::Benches));
            } else {
                v.push(it(K::Table, 1.2, 0.8, P::Centre));
                v.push(it(K::Chair, 0.45, 0.45, P::Seats(2)));
            }
            v.push(it(K::Shelf, 1.0, 0.35, P::Wall));
            v.push(it(K::Barrel, 0.6, 0.6, P::Corner));
            maybe(&mut v, 0.5, it(K::Crate, 0.6, 0.6, P::Corner), rng);
        }
        RoomKind::Dining => {
            if area > 16.0 {
                v.push(it(K::Table, 2.4, 1.0, P::Centre));
                v.push(it(K::Chair, 0.45, 0.45, P::Seats(6)));
            } else {
                v.push(it(K::Table, 1.8, 0.9, P::Centre));
                v.push(it(K::Chair, 0.45, 0.45, P::Seats(4)));
            }
            v.push(it(K::Counter, 1.6, 0.5, P::Wall));
            maybe(&mut v, 0.5, it(K::Hearth, 1.6, 0.6, P::WallExt), rng);
            maybe(&mut v, 0.4, it(K::Shelf, 1.0, 0.35, P::Wall), rng);
        }
        RoomKind::Hall => {
            v.push(it(K::Hearth, 2.0, 0.7, P::WallExt));
            v.push(it(K::Table, 2.8, 0.9, P::Centre));
            v.push(it(K::Bench, 0.0, 0.0, P::Benches));
            v.push(it(K::Chest, 1.0, 0.55, P::Wall));
            v.push(it(K::Bench, 1.6, 0.45, P::Wall));
        }
        RoomKind::Taproom => {
            v.push(it(K::Counter, 2.6, 0.7, P::Wall));
            v.push(it(K::Barrel, 0.6, 0.6, P::Corner));
            v.push(it(K::Barrel, 0.6, 0.6, P::Corner));
            v.push(it(K::Hearth, 1.8, 0.7, P::WallExt));
            let tables = ((area / 10.0) as usize).clamp(1, 6);
            for _ in 0..tables {
                v.push(it(K::Table, 1.4, 0.8, P::Centre));
                v.push(it(K::Bench, 0.0, 0.0, P::Benches));
            }
            v.push(it(K::Shelf, 1.2, 0.35, P::Wall));
        }
        RoomKind::Storage | RoomKind::Attic => {
            let n = rng.range_i32(2, 5);
            for _ in 0..n {
                v.push(it(K::Barrel, 0.6, 0.6, P::Corner));
            }
            for _ in 0..rng.range_i32(1, 4) {
                v.push(it(K::Crate, 0.7, 0.7, P::Corner));
            }
            v.push(it(K::Chest, 0.9, 0.5, P::Wall));
            if kind == RoomKind::Storage {
                v.push(it(K::Shelf, 1.0, 0.4, P::Wall));
            } else {
                maybe(&mut v, 0.3, it(K::Cot, 0.8, 1.9, P::Wall), rng);
            }
        }
        RoomKind::Pantry => {
            v.push(it(K::Shelf, 1.0, 0.4, P::Wall));
            v.push(it(K::Shelf, 1.0, 0.4, P::Wall));
            v.push(it(K::Barrel, 0.6, 0.6, P::Corner));
            v.push(it(K::Crate, 0.6, 0.6, P::Corner));
        }
        RoomKind::Cellar => {
            let n = ((area / 3.0) as i32).clamp(3, 10);
            for _ in 0..n {
                v.push(it(K::Barrel, 0.7, 0.7, P::Wall));
            }
            for _ in 0..rng.range_i32(2, 5) {
                v.push(it(K::Crate, 0.7, 0.7, P::Corner));
            }
            v.push(it(K::Shelf, 1.2, 0.4, P::Wall));
        }
        RoomKind::Workshop => {
            v.push(it(K::Table, 1.8, 0.7, P::Wall));
            maybe(&mut v, 0.5, it(K::Anvil, 0.7, 0.4, P::Centre), rng);
            v.push(it(K::Shelf, 1.2, 0.4, P::Wall));
            v.push(it(K::Crate, 0.7, 0.7, P::Corner));
            v.push(it(K::Crate, 0.7, 0.7, P::Corner));
            v.push(it(K::Barrel, 0.6, 0.6, P::Corner));
            maybe(&mut v, 0.5, it(K::Chest, 0.9, 0.5, P::Wall), rng);
        }
        RoomKind::Shop => {
            v.push(it(K::Counter, 2.0, 0.6, P::WallInt));
            v.push(it(K::Shelf, 1.2, 0.4, P::Wall));
            v.push(it(K::Shelf, 1.2, 0.4, P::Wall));
            v.push(it(K::Crate, 0.7, 0.7, P::Corner));
            v.push(it(K::Barrel, 0.6, 0.6, P::Corner));
            maybe(&mut v, 0.6, it(K::Crate, 0.7, 0.7, P::Corner), rng);
        }
        RoomKind::Study => {
            v.push(it(K::Desk, 1.2, 0.6, P::WallExt));
            v.push(it(K::Chair, 0.45, 0.45, P::DeskChair));
            v.push(it(K::Bookshelf, 1.2, 0.35, P::WallInt));
            v.push(it(K::Bookshelf, 1.2, 0.35, P::WallInt));
            maybe(&mut v, 0.5, it(K::Chest, 0.9, 0.5, P::Wall), rng);
        }
        RoomKind::Library => {
            let n = ((area / 4.0) as usize).clamp(3, 7);
            for _ in 0..n {
                v.push(it(K::Bookshelf, 1.4, 0.4, P::WallInt));
            }
            v.push(it(K::Table, 1.4, 0.8, P::Centre));
            v.push(it(K::Chair, 0.45, 0.45, P::Seats(2)));
            maybe(&mut v, 0.5, it(K::Desk, 1.2, 0.6, P::WallExt), rng);
        }
        RoomKind::Chapel => {
            v.push(it(K::Altar, 1.4, 0.7, P::WallInt));
            v.push(it(K::Bench, 0.0, 0.0, P::Pews));
        }
        RoomKind::Bath => {
            v.push(it(K::Bathtub, 1.6, 0.8, P::Wall));
            maybe(&mut v, 0.6, it(K::Shelf, 0.8, 0.35, P::Wall), rng);
            maybe(&mut v, 0.5, it(K::Chest, 0.8, 0.45, P::Wall), rng);
        }
        RoomKind::Privy => {
            v.push(it(K::Bench, 0.8, 0.5, P::Wall));
        }
        RoomKind::Nursery => {
            v.push(it(K::Cot, 0.6, 1.1, P::Wall));
            v.push(it(K::Cot, 0.6, 1.1, P::Wall));
            v.push(it(K::Chest, 0.8, 0.45, P::Wall));
            maybe(&mut v, 0.6, it(K::Chair, 0.45, 0.45, P::Corner), rng);
        }
        RoomKind::Servants => {
            for _ in 0..rng.range_i32(2, 5) {
                v.push(it(K::Cot, 0.8, 1.9, P::Wall));
            }
            v.push(it(K::Chest, 0.8, 0.45, P::Wall));
            maybe(&mut v, 0.6, it(K::Chest, 0.8, 0.45, P::Wall), rng);
        }
        RoomKind::Corridor | RoomKind::Stairwell => {}
    }
    v
}

/// One straight stretch of wall bounding a room.
#[derive(Clone, Copy, Debug)]
struct Face {
    axis: Axis,
    /// Coordinate of the wall centreline across `axis`.
    line: f32,
    a0: f32,
    a1: f32,
    /// Normal into the room.
    n: Vec2,
    /// Half thickness of the wall.
    half: f32,
    exterior: bool,
}

fn faces(g: &Grid, all_runs: &[Run], room: i32) -> Vec<Face> {
    let mut out: Vec<Face> = Vec::new();
    for r in all_runs.iter().filter(|r| r.has(room)) {
        let n = r.normal_into(room);
        let (a0, a1) = r.metric(g);
        let f = Face { axis: r.axis, line: r.line_coord(g), a0, a1, n, half: r.thickness() * 0.5, exterior: r.exterior() };
        if let Some(last) = out.last_mut()
            && last.axis == f.axis
            && (last.line - f.line).abs() < 1e-4
            && last.n == f.n
            && last.exterior == f.exterior
            && (last.a1 - f.a0).abs() < 1e-4
        {
            last.a1 = f.a1;
            continue;
        }
        out.push(f);
    }
    out
}

/// Everything prop placement needs for one level.
pub(crate) struct Furnisher<'a> {
    g: &'a Grid,
    lay: &'a LevelLayout,
    style: DwellingStyle,
    wall_boxes: Vec<Aabb>,
    /// Keep-clear zones for every prop.
    zones: Vec<Aabb>,
    /// Extra keep-clear zones for tall props (in front of windows).
    window_zones: Vec<Aabb>,
    passages: Vec<(Vec2, Vec2)>,
    all_runs: Vec<Run>,
    base: WalkGrid,
    starts: Vec<Vec2>,
    targets: Vec<Vec2>,
    /// Targets reachable before any furniture is placed.
    must_reach: Vec<bool>,
    props: Vec<LProp>,
}

impl<'a> Furnisher<'a> {
    /// Prepare zones and the walk grid for a level. `stairs_here` are flights
    /// starting or ending on this level.
    pub fn new(
        g: &'a Grid,
        lay: &'a LevelLayout,
        style: DwellingStyle,
        walls: &[LWall],
        openings: &[LOpening],
        stairs_here: &[LStair],
        stair_rect: Option<CellRect>,
    ) -> Self {
        let cell_box = |r: CellRect| Obb::from_aabb(g.vertex((r.i0, r.j0)), g.vertex((r.i1, r.j1)));
        let wall_boxes: Vec<Obb> = walls.iter().map(|w| Obb::from_segment(w.a, w.b, w.thickness * 0.5, w.thickness * 0.5)).collect();
        let mut zones = Vec::new();
        let mut window_zones = Vec::new();
        let mut passages = Vec::new();
        for o in openings {
            if o.is_passage() {
                zones.push(Obb::from_segment(o.a, o.b, o.thickness * 0.5 + DOOR_ZONE, 0.08));
                passages.push((o.a, o.b));
            } else {
                window_zones.push(Obb::from_segment(o.a, o.b, o.thickness * 0.5 + 0.5, 0.1));
            }
        }
        if let Some(l) = lay.landing {
            zones.push(cell_box(l));
        }
        if let Some(r) = stair_rect {
            zones.push(cell_box(r));
        }

        // Walk grid: rooms free, walls / stairs blocked (inflated), door gaps open.
        let lo = Vec2::new(g.x0, g.y0) - Vec2::splat(0.5);
        let hi = g.vertex((g.nx, g.ny)) + Vec2::splat(0.5);
        let mut base = WalkGrid::new(lo, hi, WALK_RES);
        for k in 0..lay.labels.len() {
            if lay.labels[k] >= 0 {
                let (i, j) = g.coords(k);
                base.fill_obb(&cell_box(CellRect::new(i, j, i + 1, j + 1)), true);
            }
        }
        for w in &wall_boxes {
            base.fill_obb(&w.inflate(WALL_CLEARANCE), false);
        }
        for o in openings.iter().filter(|o| o.is_passage()) {
            base.fill_obb(&Obb::from_segment(o.a, o.b, o.thickness * 0.5 + WALL_CLEARANCE + 0.02, -WALL_CLEARANCE), true);
        }
        for s in stairs_here {
            base.fill_obb(&Obb::from_aabb(s.min, s.max).inflate(WALL_CLEARANCE), false);
        }

        // One way in per level: the entrance on the ground floor, otherwise the
        // landing of the flight that connects the level towards the ground.
        let mut starts = Vec::new();
        let mut targets = Vec::new();
        for o in openings.iter().filter(|o| o.is_passage()) {
            targets.push(o.mid());
            if o.kind == mde_core::OpeningKind::Entrance {
                let n = o.normal();
                let into = if lay.labels_side(g, o.mid() + n * 0.3) >= 0 { n } else { -n };
                starts.push(o.mid() + into * (o.thickness * 0.5 + 0.35));
            }
        }
        for s in stairs_here {
            if s.from == lay.level {
                let p = s.landing_point(false, 0.4);
                targets.push(p);
                if lay.level < 0 && s.to == lay.level + 1 {
                    starts.push(p);
                }
            }
            if s.to == lay.level {
                let p = s.landing_point(true, 0.4);
                targets.push(p);
                if lay.level > 0 {
                    starts.push(p);
                }
            }
        }
        if starts.is_empty()
            && let Some(&t) = targets.first()
        {
            starts.push(t);
        }
        let mut f = Furnisher {
            g,
            lay,
            style,
            wall_boxes: wall_boxes.iter().map(Aabb::of).collect(),
            zones: zones.iter().map(Aabb::of).collect(),
            window_zones: window_zones.iter().map(Aabb::of).collect(),
            passages,
            all_runs: runs::runs(g, &lay.labels),
            base,
            starts,
            targets,
            must_reach: Vec::new(),
            props: Vec::new(),
        };
        f.must_reach = f.reach(None);
        f
    }

    /// Which targets are reachable, optionally with one more obstacle.
    fn reach(&self, extra: Option<&Obb>) -> Vec<bool> {
        let mut grid;
        let g = match extra {
            Some(o) => {
                grid = self.base.clone();
                grid.fill_obb(&o.inflate(PROP_CLEARANCE), false);
                &grid
            }
            None => &self.base,
        };
        let mut starts = Vec::new();
        for &s in &self.starts {
            starts.extend(g.free_near(s, 0.35));
        }
        let targets: Vec<Vec<usize>> = self.targets.iter().map(|&t| g.free_near(t, 0.35)).collect();
        g.flood_to(&starts, &targets)
    }

    fn walk_ok(&self, o: &Obb) -> bool {
        let now = self.reach(Some(o));
        self.must_reach.iter().zip(&now).all(|(&before, &after)| !before || after)
    }

    fn label_at(&self, p: Vec2) -> i32 {
        let i = ((p.x - self.g.x0) / self.g.cx).floor() as i32;
        let j = ((p.y - self.g.y0) / self.g.cy).floor() as i32;
        runs::label_at(self.g, &self.lay.labels, (i, j))
    }

    /// Box fits inside `room` without touching walls, zones or other props.
    fn fits(&self, room: usize, min: Vec2, max: Vec2, tall: bool) -> bool {
        let e = 0.01;
        let pts = [
            Vec2::new(min.x + e, min.y + e),
            Vec2::new(max.x - e, min.y + e),
            Vec2::new(max.x - e, max.y - e),
            Vec2::new(min.x + e, max.y - e),
            (min + max) * 0.5,
        ];
        if pts.iter().any(|&p| self.label_at(p) != room as i32) {
            return false;
        }
        if self.wall_boxes.iter().any(|w| w.hits(min, max, 0.01)) {
            return false;
        }
        if self.zones.iter().any(|z| z.hits(min, max, 0.005)) {
            return false;
        }
        if tall && self.window_zones.iter().any(|z| z.hits(min, max, 0.005)) {
            return false;
        }
        !self.props.iter().any(|p| Aabb { min: p.min, max: p.max }.hits(min, max, 0.03))
    }

    /// Accept a box if it fits and keeps paths open.
    fn try_add(&mut self, kind: PropKind, room: usize, min: Vec2, max: Vec2, facing: Vec2, tall: bool) -> bool {
        if !self.fits(room, min, max, tall) {
            return false;
        }
        let obb = Obb::from_aabb(min, max);
        if !self.walk_ok(&obb) {
            return false;
        }
        self.base.fill_obb(&obb.inflate(PROP_CLEARANCE), false);
        self.props.push(LProp { kind, room, min, max, facing });
        true
    }

    fn dist_to_passage(&self, p: Vec2) -> f32 {
        self.passages
            .iter()
            .map(|&(a, b)| mde_core::poly::closest_point_on_segment(p, a, b).distance(p))
            .fold(f32::INFINITY, f32::min)
    }

    /// Candidate wall placements, best first.
    fn wall_candidates(&self, room: usize, item: &Item, rng: &mut Rng) -> Vec<(f32, Vec2, Vec2, Vec2)> {
        let mut out = Vec::new();
        for f in faces(self.g, &self.all_runs, room as i32) {
            let len = f.a1 - f.a0;
            if len < item.w + 0.1 {
                continue;
            }
            let steps = ((len - item.w) / 0.2).floor().max(0.0) as i32;
            let mut ts: Vec<f32> = (0..=steps).map(|k| f.a0 + item.w * 0.5 + 0.05 + k as f32 * 0.2).collect();
            ts.push((f.a0 + f.a1) * 0.5);
            let surface = f.half + 0.02;
            for t in ts {
                if t - item.w * 0.5 < f.a0 || t + item.w * 0.5 > f.a1 {
                    continue;
                }
                let across = f.line + if f.n.x + f.n.y > 0.0 { surface + item.d * 0.5 } else { -(surface + item.d * 0.5) };
                let c = match f.axis {
                    Axis::X => Vec2::new(t, across),
                    Axis::Y => Vec2::new(across, t),
                };
                let (min, max) = boxed(c, item.w, item.d, f.n);
                let mut score = rng.next_f32() * 0.5;
                match item.place {
                    Place::WallExt => {
                        if !f.exterior {
                            score += 2.0;
                        }
                        score += (t - (f.a0 + f.a1) * 0.5).abs() * 0.3;
                    }
                    Place::WallInt => {
                        if f.exterior {
                            score += 1.5;
                        }
                        score += 2.0 / (self.dist_to_passage(c) + 0.5);
                        score += (t - (f.a0 + f.a1) * 0.5).abs() * 0.2;
                    }
                    Place::Corner => {
                        score += (t - f.a0).min(f.a1 - t) * 0.6;
                    }
                    _ => {
                        score += 0.6 / (self.dist_to_passage(c) + 0.3);
                    }
                }
                out.push((score, min, max, f.n));
            }
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    /// Candidate free-standing placements (tables), best first.
    fn centre_candidates(&self, room: usize, item: &Item, cells: &[(i32, i32)], rng: &mut Rng) -> Vec<(f32, Vec2, Vec2, Vec2)> {
        let mut out = Vec::new();
        let mut lo = Vec2::splat(f32::INFINITY);
        let mut hi = Vec2::splat(f32::NEG_INFINITY);
        let mut centroid = Vec2::ZERO;
        for &c in cells {
            let p = self.g.center(c);
            centroid += p;
            lo = lo.min(self.g.vertex(c));
            hi = hi.max(self.g.vertex((c.0 + 1, c.1 + 1)));
        }
        centroid /= cells.len().max(1) as f32;
        let ext = hi - lo;
        // Long side of the table follows the room's long side.
        let facings = if ext.x >= ext.y { [Vec2::Y, Vec2::X] } else { [Vec2::X, Vec2::Y] };
        let nx = (ext.x / 0.25).ceil() as i32;
        let ny = (ext.y / 0.25).ceil() as i32;
        for (fi, facing) in facings.into_iter().enumerate() {
            for j in 0..=ny {
                for i in 0..=nx {
                    let c = lo + Vec2::new(i as f32 * 0.25, j as f32 * 0.25);
                    let (min, max) = boxed(c, item.w, item.d, facing);
                    if !self.fits(room, min, max, item.tall) {
                        continue;
                    }
                    let mut score = c.distance(centroid) * 0.5 + fi as f32 * 0.8 + rng.next_f32() * 0.3;
                    // Room for chairs / benches around it.
                    let (rmin, rmax) = (min - Vec2::splat(0.6), max + Vec2::splat(0.6));
                    if self.wall_boxes.iter().any(|w| w.hits(rmin, rmax, 0.0)) {
                        score += 2.5;
                    }
                    if self.props.iter().any(|p| Aabb { min: p.min, max: p.max }.hits(rmin, rmax, 0.0)) || self.zones.iter().any(|z| z.hits(rmin, rmax, 0.0)) {
                        score += 2.5;
                    }
                    out.push((score, min, max, facing));
                }
            }
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    /// Furnish every room of the level.
    pub fn furnish(mut self, rng: &mut Rng) -> Vec<LProp> {
        for room in 0..self.lay.room_count() {
            let kind = self.lay.kinds[room];
            let cells = self.lay.cells_of(self.g, room);
            if cells.is_empty() {
                continue;
            }
            let area = cells.len() as f32 * self.g.cell_area();
            let list = items(kind, self.style, area, rng);
            let mut anchor: Option<LProp> = None;
            for item in list {
                match item.place {
                    Place::Seats(n) => {
                        if let Some(t) = anchor.clone() {
                            self.seats(room, &t, n);
                        }
                    }
                    Place::Benches => {
                        if let Some(t) = anchor.clone() {
                            self.benches(room, &t);
                        }
                    }
                    Place::DeskChair => {
                        if let Some(t) = anchor.clone() {
                            let c = t.center() + t.facing * (t.size().y * 0.5 + 0.3);
                            let (min, max) = boxed(c, item.w, item.d, -t.facing);
                            self.try_add(item.kind, room, min, max, -t.facing, false);
                        }
                    }
                    Place::Pews => {
                        if let Some(t) = anchor.clone() {
                            self.pews(room, &t);
                        }
                    }
                    Place::Centre => {
                        let cands = self.centre_candidates(room, &item, &cells, rng);
                        anchor = self.place_first(item, room, &cands);
                    }
                    _ => {
                        let cands = self.wall_candidates(room, &item, rng);
                        anchor = self.place_first(item, room, &cands);
                    }
                }
            }
        }
        self.props
    }

    fn place_first(&mut self, item: Item, room: usize, cands: &[(f32, Vec2, Vec2, Vec2)]) -> Option<LProp> {
        let mut tries = 0;
        for &(_, min, max, facing) in cands {
            if !self.fits(room, min, max, item.tall) {
                continue;
            }
            tries += 1;
            if self.try_add(item.kind, room, min, max, facing, item.tall) {
                return self.props.last().cloned();
            }
            if tries >= 6 {
                break;
            }
        }
        None
    }

    fn seats(&mut self, room: usize, table: &LProp, n: u8) {
        // Long axis of the table.
        let e = table.max - table.min;
        let (long, short) = if e.x >= e.y { (Vec2::X, Vec2::Y) } else { (Vec2::Y, Vec2::X) };
        let len = e.dot(long);
        let depth = e.dot(short);
        let per_side = (n as usize).div_ceil(2);
        for side in [1.0f32, -1.0] {
            for k in 0..per_side {
                let off = (k as f32 + 0.5) / per_side as f32 - 0.5;
                let c = table.center() + long * (off * len) + short * side * (depth * 0.5 + 0.06 + 0.225);
                let facing = -short * side;
                let (min, max) = boxed(c, 0.45, 0.45, facing);
                self.try_add(PropKind::Chair, room, min, max, facing, false);
            }
        }
    }

    fn benches(&mut self, room: usize, table: &LProp) {
        let e = table.max - table.min;
        let (long, short) = if e.x >= e.y { (Vec2::X, Vec2::Y) } else { (Vec2::Y, Vec2::X) };
        let len = e.dot(long) - 0.1;
        let depth = e.dot(short);
        for side in [1.0f32, -1.0] {
            let c = table.center() + short * side * (depth * 0.5 + 0.08 + 0.19);
            let facing = -short * side;
            let (min, max) = boxed(c, len, 0.38, facing);
            self.try_add(PropKind::Bench, room, min, max, facing, false);
        }
    }

    fn pews(&mut self, room: usize, altar: &LProp) {
        let f = altar.facing;
        let across = Vec2::new(-f.y, f.x).abs();
        let cells = self.lay.cells_of(self.g, room);
        let span = cells
            .iter()
            .map(|&c| self.g.center(c).dot(across))
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), t| (a.min(t), b.max(t)));
        let room_w = (span.1 - span.0) + self.g.cx.min(self.g.cy);
        let w = (room_w - 1.4).clamp(0.8, 2.4);
        let front = altar.center() + f * (altar.size().y * 0.5);
        for row in 0..6 {
            let c = front + f * (1.3 + row as f32 * 0.95);
            let (min, max) = boxed(c, w, 0.4, -f);
            if !self.try_add(PropKind::Bench, room, min, max, -f, false) {
                break;
            }
        }
    }
}
