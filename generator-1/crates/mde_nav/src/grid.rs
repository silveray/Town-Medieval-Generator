//! Coarse walkable grid per dwelling floor: floor minus walls, furniture and
//! stairs, grown by the agent radius, with a guaranteed channel through every
//! door. The grid is aligned with the plan's dominant wall direction so rotated
//! houses rasterise as cleanly as axis-aligned ones.

use mde_core::{Dwelling, Floor, OpeningKind, Polygon, Stair, Vec2};
use serde::{Deserialize, Serialize};

use crate::geom::{Obb, dominant_axis, perp, point_seg_distance};

/// Radius of a walking NPC, metres. Cells closer than this to a wall or a
/// piece of furniture are not walkable.
pub const AGENT_RADIUS: f32 = 0.2;

/// Default grid cell size in metres.
pub const DEFAULT_CELL: f32 = 0.5;

/// How far a door's walkable channel reaches into the rooms beyond the wall
/// faces, metres.
const CARVE_DEPTH: f32 = 0.65;

/// Extra margin around stair flights and stair holes, metres.
const STAIR_MARGIN: f32 = 0.1;

/// Upper bound on cells per grid; bigger floors get coarser cells.
const MAX_CELLS: usize = 4_000_000;

/// Room label of cells outside every room.
pub const NO_ROOM: u32 = u32::MAX;

/// Classification of one grid cell (by its centre point).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CellState {
    /// Not inside any room.
    Outside,
    /// Walkable.
    Free,
    /// Too close to a wall.
    Wall,
    /// Covered by (or too close to) furniture.
    Prop,
    /// On a stair flight or over a stair hole.
    Stair,
}

/// Walkable grid of one floor in plan space. Cell `(i, j)` covers local
/// `[i, i + 1) × [j, j + 1)` cells from `origin`, where local x runs along
/// `axis` and local y along its CCW perpendicular.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WalkGrid {
    /// [`Floor::level`] this grid was built for.
    pub level: i32,
    /// [`Floor::elevation`], for drawing in world space.
    pub elevation: f32,
    /// Plan position of the grid's local origin (corner of cell `(0, 0)`).
    pub origin: Vec2,
    /// Unit plan direction of local +x.
    pub axis: Vec2,
    /// Cell size in metres.
    pub cell: f32,
    pub width: u32,
    pub height: u32,
    /// Row-major cell states (`j * width + i`).
    pub cells: Vec<CellState>,
    /// Row-major [`mde_core::Room::id`] per cell ([`NO_ROOM`] outside rooms).
    pub rooms: Vec<u32>,
}

impl WalkGrid {
    fn empty(level: i32, elevation: f32, cell: f32) -> Self {
        WalkGrid {
            level,
            elevation,
            origin: Vec2::ZERO,
            axis: Vec2::X,
            cell,
            width: 0,
            height: 0,
            cells: Vec::new(),
            rooms: Vec::new(),
        }
    }

    /// Plan point → grid-local coordinates (metres).
    pub fn to_local(&self, p: Vec2) -> Vec2 {
        let d = p - self.origin;
        Vec2::new(d.dot(self.axis), d.dot(perp(self.axis)))
    }

    /// Grid-local coordinates (metres) → plan point.
    pub fn to_plan(&self, l: Vec2) -> Vec2 {
        self.origin + self.axis * l.x + perp(self.axis) * l.y
    }

    /// Cell containing plan point `p`, if inside the grid.
    pub fn cell_of(&self, p: Vec2) -> Option<(u32, u32)> {
        let l = self.to_local(p) / self.cell;
        if !l.is_finite() || l.x < 0.0 || l.y < 0.0 {
            return None;
        }
        let (i, j) = (l.x as u32, l.y as u32);
        (i < self.width && j < self.height).then_some((i, j))
    }

    /// Row-major index of cell `(i, j)`.
    #[inline]
    pub fn index(&self, i: u32, j: u32) -> usize {
        j as usize * self.width as usize + i as usize
    }

    /// Index of the cell containing `p`.
    pub fn index_of(&self, p: Vec2) -> Option<usize> {
        self.cell_of(p).map(|(i, j)| self.index(i, j))
    }

    /// `(i, j)` of a row-major index.
    #[inline]
    pub fn coords(&self, idx: usize) -> (u32, u32) {
        ((idx % self.width as usize) as u32, (idx / self.width as usize) as u32)
    }

    /// Plan-space centre of cell `(i, j)`.
    pub fn center(&self, i: u32, j: u32) -> Vec2 {
        self.to_plan(Vec2::new((i as f32 + 0.5) * self.cell, (j as f32 + 0.5) * self.cell))
    }

    /// Plan-space centre of the cell with row-major index `idx`.
    pub fn center_of(&self, idx: usize) -> Vec2 {
        let (i, j) = self.coords(idx);
        self.center(i, j)
    }

    /// State of the cell containing `p` ([`CellState::Outside`] beyond the grid).
    pub fn state_at(&self, p: Vec2) -> CellState {
        self.index_of(p).map_or(CellState::Outside, |i| self.cells[i])
    }

    /// `true` if an NPC may stand at plan point `p`.
    pub fn is_walkable(&self, p: Vec2) -> bool {
        self.state_at(p) == CellState::Free
    }

    /// Room id of the cell containing `p`.
    pub fn room_at(&self, p: Vec2) -> Option<u32> {
        self.index_of(p).map(|i| self.rooms[i]).filter(|&r| r != NO_ROOM)
    }

    /// `true` if every point of segment `a–b` (sampled at a quarter cell) is walkable.
    pub fn segment_clear(&self, a: Vec2, b: Vec2) -> bool {
        self.segment_clear_in(a, b, None)
    }

    /// Like [`WalkGrid::segment_clear`], additionally requiring every sample to
    /// lie in room `room` when given.
    pub fn segment_clear_in(&self, a: Vec2, b: Vec2, room: Option<u32>) -> bool {
        let len = a.distance(b);
        if !len.is_finite() {
            return false;
        }
        let steps = ((len / (self.cell * 0.25)).ceil() as usize).max(1);
        (0..=steps).all(|k| {
            let p = a.lerp(b, k as f32 / steps as f32);
            match self.index_of(p) {
                Some(i) => self.cells[i] == CellState::Free && room.is_none_or(|r| self.rooms[i] == r),
                None => false,
            }
        })
    }

    /// Number of walkable cells.
    pub fn free_count(&self) -> usize {
        self.cells.iter().filter(|&&c| c == CellState::Free).count()
    }

    /// Range of cells covering a plan-space disc (clamped to the grid).
    fn cells_around(&self, p: Vec2, r: f32) -> Option<(u32, u32, u32, u32)> {
        self.cells_in_local_box(self.to_local(p) - Vec2::splat(r), self.to_local(p) + Vec2::splat(r))
    }

    fn cells_in_local_box(&self, lo: Vec2, hi: Vec2) -> Option<(u32, u32, u32, u32)> {
        if self.width == 0 || self.height == 0 || !lo.is_finite() || !hi.is_finite() {
            return None;
        }
        let i0 = (lo.x / self.cell).floor().max(0.0);
        let j0 = (lo.y / self.cell).floor().max(0.0);
        let i1 = (hi.x / self.cell).floor().min(self.width as f32 - 1.0);
        let j1 = (hi.y / self.cell).floor().min(self.height as f32 - 1.0);
        (i0 <= i1 && j0 <= j1).then_some((i0 as u32, j0 as u32, i1 as u32, j1 as u32))
    }

    /// Row-major indices of cells whose centres are within `r` of `p`.
    pub(crate) fn cells_near(&self, p: Vec2, r: f32) -> Vec<usize> {
        let mut out = Vec::new();
        if let Some((i0, j0, i1, j1)) = self.cells_around(p, r) {
            for j in j0..=j1 {
                for i in i0..=i1 {
                    if self.center(i, j).distance(p) <= r {
                        out.push(self.index(i, j));
                    }
                }
            }
        }
        out
    }

    /// Up to 8 walkable neighbours of `idx` restricted to `room`; diagonal
    /// steps need both orthogonal neighbours walkable (no corner cutting).
    pub(crate) fn neighbours_in(&self, idx: usize, room: u32, out: &mut Vec<(usize, f32)>) {
        out.clear();
        let (i, j) = self.coords(idx);
        let ok = |di: i32, dj: i32| -> Option<usize> {
            let (ni, nj) = (i as i32 + di, j as i32 + dj);
            if ni < 0 || nj < 0 || ni >= self.width as i32 || nj >= self.height as i32 {
                return None;
            }
            let k = self.index(ni as u32, nj as u32);
            (self.cells[k] == CellState::Free && self.rooms[k] == room).then_some(k)
        };
        for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            if let Some(k) = ok(di, dj) {
                out.push((k, self.cell));
            }
        }
        for (di, dj) in [(1, 1), (-1, 1), (1, -1), (-1, -1)] {
            if ok(di, 0).is_some() && ok(0, dj).is_some()
                && let Some(k) = ok(di, dj)
            {
                out.push((k, self.cell * std::f32::consts::SQRT_2));
            }
        }
    }
}

/// A wall piece that cannot be walked through (wall minus its door openings).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Solid {
    pub a: Vec2,
    pub b: Vec2,
    /// Half the wall thickness.
    pub half: f32,
}

/// A passable opening (door, doorway or entrance).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Passage {
    /// Index into [`Floor::openings`].
    pub opening: u32,
    pub kind: OpeningKind,
    pub mid: Vec2,
    /// Unit normal of the wall at the opening.
    pub n: Vec2,
    /// Thickness of the host wall (or a default when no host is found).
    pub thickness: f32,
}

/// Obstacles of one floor, derived once and shared by the grid and the
/// line-of-sight checks.
#[derive(Clone, Debug, Default)]
pub(crate) struct FloorGeom {
    pub solids: Vec<Solid>,
    pub passages: Vec<Passage>,
    pub props: Vec<Obb>,
}

fn passable(k: OpeningKind) -> bool {
    matches!(k, OpeningKind::Door | OpeningKind::Doorway | OpeningKind::Entrance)
}

impl FloorGeom {
    pub fn new(floor: &Floor) -> Self {
        // Passable openings and the wall intervals they cut.
        let mut cuts: Vec<Vec<(f32, f32)>> = vec![Vec::new(); floor.walls.len()];
        let mut passages = Vec::new();
        for (oi, o) in floor.openings.iter().enumerate() {
            if !passable(o.kind) || !o.a.is_finite() || !o.b.is_finite() {
                continue;
            }
            let Some(od) = (o.b - o.a).try_normalize() else { continue };
            let mut thickness: Option<f32> = None;
            for (wi, w) in floor.walls.iter().enumerate() {
                let Some(wd) = (w.b - w.a).try_normalize() else { continue };
                if od.perp_dot(wd).abs() > 0.05 {
                    continue;
                }
                let tol = (w.thickness * 0.5).max(0.05) + 0.02;
                let off_a = (o.a - w.a).perp_dot(wd).abs();
                let off_b = (o.b - w.a).perp_dot(wd).abs();
                if off_a > tol || off_b > tol {
                    continue;
                }
                let len = w.a.distance(w.b);
                let ta = (o.a - w.a).dot(wd);
                let tb = (o.b - w.a).dot(wd);
                let (lo, hi) = (ta.min(tb).max(0.0), ta.max(tb).min(len));
                if hi - lo > 1e-3 {
                    cuts[wi].push((lo, hi));
                    let th = if w.thickness.is_finite() { w.thickness.max(0.0) } else { 0.0 };
                    thickness = Some(thickness.map_or(th, |t: f32| t.max(th)));
                }
            }
            let default_th = if o.kind == OpeningKind::Entrance { 0.4 } else { 0.15 };
            passages.push(Passage {
                opening: oi as u32,
                kind: o.kind,
                mid: (o.a + o.b) * 0.5,
                n: perp(od),
                thickness: thickness.unwrap_or(default_th),
            });
        }
        // Solid wall pieces between the cuts.
        let mut solids = Vec::new();
        for (wi, w) in floor.walls.iter().enumerate() {
            if !w.a.is_finite() || !w.b.is_finite() {
                continue;
            }
            let half = if w.thickness.is_finite() { w.thickness.max(0.0) * 0.5 } else { 0.0 };
            let len = w.a.distance(w.b);
            let Some(wd) = (w.b - w.a).try_normalize() else { continue };
            let c = &mut cuts[wi];
            c.sort_by(|x, y| x.0.total_cmp(&y.0));
            let mut t = 0.0;
            for &(lo, hi) in c.iter() {
                if lo - t > 1e-3 {
                    solids.push(Solid { a: w.a + wd * t, b: w.a + wd * lo, half });
                }
                t = t.max(hi);
            }
            if len - t > 1e-3 {
                solids.push(Solid { a: w.a + wd * t, b: w.b, half });
            }
        }
        let props = floor.props.iter().filter_map(Obb::from_prop).collect();
        FloorGeom { solids, passages, props }
    }

    /// `true` if segment `a–b` does not cross any solid wall piece or furniture.
    pub fn segment_clear(&self, a: Vec2, b: Vec2) -> bool {
        !self.solids.iter().any(|s| crate::geom::segments_touch(a, b, s.a, s.b))
            && !self.props.iter().any(|o| o.hits_segment(a, b, 0.01))
    }
}

/// Walkable grid of one floor with cells of `cell` metres (≈0.5 is a good
/// default). Blocks walls (except at door openings), furniture and the stair
/// flights starting on this floor, all grown by [`AGENT_RADIUS`]. Stair holes
/// of flights arriving from below are listed on the lower floor; use
/// [`dwelling_walk_grids`] to block those too.
pub fn walkable_grid(floor: &Floor, cell: f32) -> WalkGrid {
    let geom = FloorGeom::new(floor);
    let flights: Vec<&Stair> = floor.stairs.iter().collect();
    build_grid(floor, &geom, cell, AGENT_RADIUS, &flights)
}

/// One walkable grid per floor of `d` (same order as [`Dwelling::floors`]),
/// also blocking the holes of stair flights arriving at each floor.
pub fn dwelling_walk_grids(d: &Dwelling, cell: f32) -> Vec<WalkGrid> {
    d.floors
        .iter()
        .map(|f| {
            let geom = FloorGeom::new(f);
            build_grid(f, &geom, cell, AGENT_RADIUS, &floor_flights(d, f.level))
        })
        .collect()
}

/// Stair flights occupying floor `level`: those starting on it and those arriving at it.
pub(crate) fn floor_flights(d: &Dwelling, level: i32) -> Vec<&Stair> {
    d.floors
        .iter()
        .flat_map(|f| f.stairs.iter())
        .filter(|s| s.from_level == level || s.to_level == level)
        .collect()
}

/// Sanitised cell size.
fn sane_cell(cell: f32) -> f32 {
    if cell.is_finite() && cell > 0.0 { cell.clamp(0.05, 10.0) } else { DEFAULT_CELL }
}

pub(crate) fn build_grid(floor: &Floor, geom: &FloorGeom, cell: f32, radius: f32, flights: &[&Stair]) -> WalkGrid {
    let mut cell = sane_cell(cell);
    let elevation = if floor.elevation.is_finite() { floor.elevation } else { 0.0 };
    let axis = dominant_axis(&floor.walls, &floor.outline.0);
    let side = perp(axis);
    let local = |p: Vec2| Vec2::new(p.dot(axis), p.dot(side));
    // Bounds of all rooms in grid-local axes.
    let mut lo = Vec2::splat(f32::INFINITY);
    let mut hi = Vec2::splat(f32::NEG_INFINITY);
    for r in &floor.rooms {
        for &p in r.shape.0.iter().filter(|p| p.is_finite()) {
            lo = lo.min(local(p));
            hi = hi.max(local(p));
        }
    }
    if !(lo.is_finite() && hi.is_finite()) {
        return WalkGrid::empty(floor.level, elevation, cell);
    }
    let size = hi - lo;
    let count = |c: f32| ((size.x / c).ceil() as usize + 2) * ((size.y / c).ceil() as usize + 2);
    if count(cell) > MAX_CELLS {
        cell *= (count(cell) as f32 / MAX_CELLS as f32).sqrt() * 1.01;
        if count(cell) > MAX_CELLS {
            return WalkGrid::empty(floor.level, elevation, cell);
        }
    }
    let lo = lo - Vec2::splat(cell);
    let width = (size.x / cell).ceil() as u32 + 2;
    let height = (size.y / cell).ceil() as u32 + 2;
    let n = width as usize * height as usize;
    let mut g = WalkGrid {
        level: floor.level,
        elevation,
        origin: axis * lo.x + side * lo.y,
        axis,
        cell,
        width,
        height,
        cells: vec![CellState::Outside; n],
        rooms: vec![NO_ROOM; n],
    };

    // Rooms (first room containing a cell centre wins).
    for r in &floor.rooms {
        let pts: Vec<Vec2> = r.shape.0.iter().copied().filter(|p| p.is_finite()).collect();
        if pts.len() < 3 {
            continue;
        }
        let shape = Polygon(pts);
        let (mut rlo, mut rhi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for &p in &shape.0 {
            rlo = rlo.min(g.to_local(p));
            rhi = rhi.max(g.to_local(p));
        }
        let Some((i0, j0, i1, j1)) = g.cells_in_local_box(rlo, rhi) else { continue };
        for j in j0..=j1 {
            for i in i0..=i1 {
                let k = g.index(i, j);
                if g.rooms[k] == NO_ROOM && shape.contains(g.center(i, j)) {
                    g.rooms[k] = r.id;
                    g.cells[k] = CellState::Free;
                }
            }
        }
    }

    // Walls.
    for s in &geom.solids {
        let reach = s.half + radius;
        let (la, lb) = (g.to_local(s.a), g.to_local(s.b));
        let Some((i0, j0, i1, j1)) = g.cells_in_local_box(la.min(lb) - Vec2::splat(reach), la.max(lb) + Vec2::splat(reach))
        else {
            continue;
        };
        for j in j0..=j1 {
            for i in i0..=i1 {
                let k = g.index(i, j);
                if g.cells[k] != CellState::Outside && point_seg_distance(g.center(i, j), s.a, s.b) < reach {
                    g.cells[k] = CellState::Wall;
                }
            }
        }
    }

    // Furniture.
    for o in &geom.props {
        let Some((i0, j0, i1, j1)) = g.cells_around(o.center, o.radius() + radius) else { continue };
        for j in j0..=j1 {
            for i in i0..=i1 {
                let k = g.index(i, j);
                if g.cells[k] != CellState::Outside && o.contains(g.center(i, j), radius) {
                    g.cells[k] = CellState::Prop;
                }
            }
        }
    }

    // Stair flights and holes.
    for s in flights {
        let pts: Vec<Vec2> = s.footprint.0.iter().copied().filter(|p| p.is_finite()).collect();
        if pts.len() < 3 {
            continue;
        }
        let fp = Polygon(pts);
        let (mut slo, mut shi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for &p in &fp.0 {
            slo = slo.min(g.to_local(p));
            shi = shi.max(g.to_local(p));
        }
        let m = Vec2::splat(STAIR_MARGIN);
        let Some((i0, j0, i1, j1)) = g.cells_in_local_box(slo - m, shi + m) else { continue };
        for j in j0..=j1 {
            for i in i0..=i1 {
                let k = g.index(i, j);
                let c = g.center(i, j);
                if g.cells[k] != CellState::Outside && (fp.contains(c) || fp.distance_to_boundary(c) < STAIR_MARGIN) {
                    g.cells[k] = CellState::Stair;
                }
            }
        }
    }

    // A walkable channel straight through every door, whatever the alignment.
    for p in &geom.passages {
        let reach = p.thickness * 0.5 + CARVE_DEPTH;
        let steps = ((2.0 * reach) / (cell * 0.25)).ceil().max(1.0) as i32;
        for k in 0..=steps {
            let s = -reach + 2.0 * reach * k as f32 / steps as f32;
            if let Some(idx) = g.index_of(p.mid + p.n * s)
                && g.cells[idx] == CellState::Wall
            {
                g.cells[idx] = CellState::Free;
            }
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use mde_core::{Opening, Room, RoomKind, WallSegment};

    /// 4 × 3 m room (wall centrelines) with a door in the south wall.
    fn floor(angle: f32) -> Floor {
        let r = |p: Vec2| Vec2::from_angle(angle).rotate(p);
        let pts = [Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0), Vec2::new(4.0, 3.0), Vec2::new(0.0, 3.0)];
        let walls = (0..4)
            .map(|i| WallSegment { a: r(pts[i]), b: r(pts[(i + 1) % 4]), thickness: 0.4, exterior: true })
            .collect();
        Floor {
            level: 0,
            elevation: 0.0,
            height: 2.8,
            outline: Polygon(pts.iter().map(|&p| r(p)).collect()),
            rooms: vec![Room { id: 7, kind: RoomKind::Hall, shape: Polygon(pts.iter().map(|&p| r(p)).collect()) }],
            walls,
            openings: vec![Opening {
                kind: OpeningKind::Entrance,
                a: r(Vec2::new(1.55, 0.0)),
                b: r(Vec2::new(2.45, 0.0)),
                sill: 0.0,
                height: 2.1,
                rooms: [Some(7), None],
            }],
            stairs: vec![],
            props: vec![],
        }
    }

    #[test]
    fn grid_blocks_walls_and_opens_doors() {
        for angle in [0.0, 0.4, 1.0, -2.2] {
            let f = floor(angle);
            let g = walkable_grid(&f, 0.5);
            let r = |p: Vec2| Vec2::from_angle(angle).rotate(p);
            assert!(g.is_walkable(r(Vec2::new(2.0, 1.5))), "centre walkable at {angle}");
            assert!(!g.is_walkable(r(Vec2::new(0.05, 1.5))), "wall blocked at {angle}");
            assert!(!g.is_walkable(r(Vec2::new(2.0, -1.0))), "outside blocked at {angle}");
            assert!(g.is_walkable(r(Vec2::new(2.0, 0.65))), "door channel at {angle}");
            assert_eq!(g.room_at(r(Vec2::new(2.0, 1.5))), Some(7));
            assert!(g.free_count() > 10);
        }
    }

    #[test]
    fn degenerate_floors_do_not_panic() {
        let mut f = floor(0.0);
        f.rooms[0].shape.0[1] = Vec2::NAN;
        f.walls[0].b = Vec2::NAN;
        let _ = walkable_grid(&f, f32::NAN);
        f.rooms.clear();
        assert_eq!(walkable_grid(&f, 0.5).cells.len(), 0);
        let mut huge = floor(0.0);
        huge.rooms[0].shape = Polygon::rect(Vec2::ZERO, Vec2::splat(1.0e5));
        let g = walkable_grid(&huge, 0.01);
        assert!(g.cells.len() <= MAX_CELLS);
    }
}
