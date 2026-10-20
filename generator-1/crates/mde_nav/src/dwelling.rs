//! Interior navigation graph of a [`Dwelling`]: room nodes on a walkable grid,
//! door nodes at every passable opening, stair landings joined by stair edges,
//! and a doorstep outside each ground-floor exterior door.
//!
//! Inside a room every pair of "anchors" (door approaches, stair landings,
//! room nodes) that can reach each other on the grid is joined by one edge:
//! straight when there is line of sight, otherwise a grid path string-pulled
//! into a few corner points stored in [`NavEdge::via`](crate::NavEdge::via).
//! Edges never cross a wall except straight through a door opening, and never
//! cross furniture.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use mde_core::{Dwelling, Floor, OpeningKind, RoomKind, Stair, Vec2, Vec3, plan_to_world};

use crate::geom::perp;
use crate::graph::{NavEdgeKind, NavGraph, NavNodeKind};
use crate::grid::{CellState, DEFAULT_CELL, FloorGeom, NO_ROOM, WalkGrid, build_grid, floor_flights};

/// Distance of a doorstep node outside the outer wall face, metres.
pub const DOORSTEP_DISTANCE: f32 = 0.8;

/// Distance of a door's approach point from the wall face, metres.
const APPROACH: f32 = 0.4;

/// Distance of a stair landing node beyond the end of the flight, metres.
const LANDING: f32 = 0.55;

/// Tuning for [`dwelling_graph_with`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DwellingNavParams {
    /// Walk-grid cell size, metres.
    pub cell: f32,
    /// Target spacing between room nodes in big rooms, metres.
    pub room_spacing: f32,
    /// Maximum number of room nodes per room.
    pub max_room_nodes: usize,
}

impl Default for DwellingNavParams {
    fn default() -> Self {
        Self { cell: DEFAULT_CELL, room_spacing: 2.5, max_room_nodes: 6 }
    }
}

/// A node that joins a room's internal edge mesh.
#[derive(Clone, Copy, Debug)]
struct Anchor {
    node: u32,
    room: u32,
    /// Plan point where walking inside the room starts (the approach point for
    /// doors, the node itself otherwise).
    pos: Vec2,
    /// Grid cell of `pos`.
    cell: usize,
    door: bool,
}

/// Interior graph of `d` with default parameters; all nodes are tagged with
/// `building` ([`mde_core::Building::id`]). Positions are world space using the
/// dwelling's own plan coordinates, so a dwelling generated for a town lot
/// lines up with [`town_graph`](crate::town_graph).
pub fn dwelling_graph(d: &Dwelling, building: u32) -> NavGraph {
    dwelling_graph_with(d, building, &DwellingNavParams::default())
}

/// [`dwelling_graph`] with explicit tuning.
pub fn dwelling_graph_with(d: &Dwelling, building: u32, params: &DwellingNavParams) -> NavGraph {
    let geoms: Vec<FloorGeom> = d.floors.iter().map(FloorGeom::new).collect();
    let grids: Vec<WalkGrid> = d
        .floors
        .iter()
        .zip(&geoms)
        .map(|(f, geom)| build_grid(f, geom, params.cell, crate::grid::AGENT_RADIUS, &floor_flights(d, f.level)))
        .collect();
    let elev: Vec<f32> = d.floors.iter().map(|f| if f.elevation.is_finite() { f.elevation } else { 0.0 }).collect();
    let ground = d.floors.iter().position(|f| f.level == 0).or_else(|| {
        (0..d.floors.len()).min_by(|&a, &b| elev[a].abs().total_cmp(&elev[b].abs()))
    });

    let mut g = NavGraph::new();
    let mut anchors: Vec<Vec<Anchor>> = vec![Vec::new(); d.floors.len()];

    // Doors, doorways and entrances.
    for (fi, f) in d.floors.iter().enumerate() {
        let (grid, geom, e) = (&grids[fi], &geoms[fi], elev[fi]);
        for p in &geom.passages {
            let half = p.thickness * 0.5;
            let mut room_sides = Vec::new();
            let mut outside_sides = Vec::new();
            for s in [1.0f32, -1.0] {
                let probe = p.mid + p.n * (s * (half + APPROACH));
                match room_containing(f, probe) {
                    Some(r) => {
                        if let Some((cell, pos)) = approach_cell(grid, geom, probe, r, p.mid + p.n * (s * half)) {
                            room_sides.push((r, cell, pos));
                        }
                    }
                    None => outside_sides.push(s),
                }
            }
            if room_sides.is_empty() {
                continue;
            }
            let leaf = p.kind != OpeningKind::Doorway;
            let kind = NavNodeKind::Door { building, level: f.level, opening: p.opening, leaf };
            let node = g.add_node(plan_to_world(p.mid, e), kind);
            for (room, cell, pos) in room_sides {
                anchors[fi].push(Anchor { node, room, pos, cell, door: true });
            }
            if Some(fi) == ground {
                for s in outside_sides {
                    let ds = p.mid + p.n * (s * (half + DOORSTEP_DISTANCE));
                    let step = g.add_node(plan_to_world(ds, e), NavNodeKind::Doorstep { building });
                    g.add_edge(node, step, NavEdgeKind::Door);
                }
            }
        }
    }

    // Stair flights: landing nodes on both floors joined along the flight.
    let level_index = |level: i32| d.floors.iter().position(|f| f.level == level);
    let mut stair = 0u32;
    for f in &d.floors {
        for s in &f.stairs {
            let id = stair;
            stair += 1;
            let (Some(bi), Some(ti)) = (level_index(s.from_level), level_index(s.to_level)) else { continue };
            if bi == ti {
                continue;
            }
            let Some((bottom_end, top_end, up)) = flight_ends(s) else { continue };
            let Some((bcell, bpos)) = snap_landing(&grids[bi], &geoms[bi], bottom_end, -up) else { continue };
            let Some((tcell, tpos)) = snap_landing(&grids[ti], &geoms[ti], top_end, up) else { continue };
            let (be, te) = (elev[bi], elev[ti]);
            let (bl, tl) = (d.floors[bi].level, d.floors[ti].level);
            let nb = g.add_node(plan_to_world(bpos, be), NavNodeKind::StairBottom { building, level: bl, stair: id });
            let nt = g.add_node(plan_to_world(tpos, te), NavNodeKind::StairTop { building, level: tl, stair: id });
            g.add_edge_via(nb, nt, NavEdgeKind::Stair, vec![plan_to_world(bottom_end, be), plan_to_world(top_end, te)]);
            let broom = grids[bi].rooms[bcell];
            let troom = grids[ti].rooms[tcell];
            anchors[bi].push(Anchor { node: nb, room: broom, pos: bpos, cell: bcell, door: false });
            anchors[ti].push(Anchor { node: nt, room: troom, pos: tpos, cell: tcell, door: false });
        }
    }

    // Rooms: room nodes, then one edge per connected anchor pair.
    for (fi, f) in d.floors.iter().enumerate() {
        let (grid, geom, e) = (&grids[fi], &geoms[fi], elev[fi]);
        for room in &f.rooms {
            let mut anc: Vec<Anchor> = anchors[fi].iter().copied().filter(|a| a.room == room.id).collect();
            if anc.is_empty() {
                continue;
            }
            let mut fields: Vec<Field> = anc.iter().map(|a| Field::new(grid, a.cell, room.id)).collect();
            if room.kind != RoomKind::Stairwell {
                for cell in room_node_cells(grid, &fields, room.id, params) {
                    let kind = NavNodeKind::Room { building, level: f.level, room: room.id };
                    let pos = grid.center_of(cell);
                    let node = g.add_node(plan_to_world(pos, e), kind);
                    anc.push(Anchor { node, room: room.id, pos, cell, door: false });
                    fields.push(Field::new(grid, cell, room.id));
                }
            }
            let clear = |a: Vec2, b: Vec2| grid.segment_clear_in(a, b, Some(room.id)) && geom.segment_clear(a, b);
            for i in 0..anc.len() {
                for j in i + 1..anc.len() {
                    let (ai, aj) = (anc[i], anc[j]);
                    if ai.node == aj.node || !fields[i].dist[aj.cell].is_finite() {
                        continue;
                    }
                    let mut via: Vec<Vec2> = Vec::new();
                    if ai.door {
                        via.push(ai.pos);
                    }
                    if !clear(ai.pos, aj.pos) {
                        let cells = fields[i].path_to(aj.cell);
                        let mut pts = vec![ai.pos];
                        pts.extend(cells[..cells.len().saturating_sub(1)].iter().map(|&c| grid.center_of(c)));
                        pts.push(aj.pos);
                        via.extend(string_pull(&pts, &clear));
                    }
                    if aj.door {
                        via.push(aj.pos);
                    }
                    let kind = if ai.door || aj.door { NavEdgeKind::Door } else { NavEdgeKind::Walk };
                    let via3: Vec<Vec3> = via.iter().map(|&p| plan_to_world(p, e)).collect();
                    g.add_edge_via(ai.node, aj.node, kind, via3);
                }
            }
        }
    }
    g
}

/// First room of `f` whose outline contains `p`.
fn room_containing(f: &Floor, p: Vec2) -> Option<u32> {
    f.rooms.iter().find(|r| r.shape.len() >= 3 && r.shape.contains(p)).map(|r| r.id)
}

/// Walkable cell (and point in it) in front of a door on the side of `room`.
fn approach_cell(grid: &WalkGrid, geom: &FloorGeom, probe: Vec2, room: u32, face: Vec2) -> Option<(usize, Vec2)> {
    if let Some(i) = grid.index_of(probe)
        && grid.cells[i] == CellState::Free
        && grid.rooms[i] == room
    {
        return Some((i, probe));
    }
    grid.cells_near(probe, 1.0)
        .into_iter()
        .filter(|&i| grid.cells[i] == CellState::Free && grid.rooms[i] == room)
        .map(|i| (i, grid.center_of(i)))
        .filter(|&(_, c)| geom.segment_clear(face, c))
        .min_by(|a, b| a.1.distance(probe).total_cmp(&b.1.distance(probe)).then(a.0.cmp(&b.0)))
}

/// Bottom-end centre, top-end centre and unit climbing direction of a flight.
fn flight_ends(s: &Stair) -> Option<(Vec2, Vec2, Vec2)> {
    let pts: Vec<Vec2> = s.footprint.0.iter().copied().filter(|p| p.is_finite()).collect();
    if pts.len() < 3 {
        return None;
    }
    let up = s.up.try_normalize()?;
    let across = perp(up);
    let (mut a0, mut a1, mut b0, mut b1) = (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY);
    for p in &pts {
        a0 = a0.min(p.dot(up));
        a1 = a1.max(p.dot(up));
        b0 = b0.min(p.dot(across));
        b1 = b1.max(p.dot(across));
    }
    if a1 - a0 < 0.1 {
        return None;
    }
    let mid = across * ((b0 + b1) * 0.5);
    Some((up * a0 + mid, up * a1 + mid, up))
}

/// Walkable landing cell beyond a flight end (`out` points away from the flight).
fn snap_landing(grid: &WalkGrid, geom: &FloorGeom, end: Vec2, out: Vec2) -> Option<(usize, Vec2)> {
    let start = end + out * 0.01;
    let probe = end + out * LANDING;
    let ok = |i: usize| grid.cells[i] == CellState::Free && grid.rooms[i] != NO_ROOM;
    if let Some(i) = grid.index_of(probe)
        && ok(i)
        && geom.segment_clear(start, probe)
    {
        return Some((i, probe));
    }
    grid.cells_near(probe, 1.5)
        .into_iter()
        .filter(|&i| ok(i))
        .map(|i| (i, grid.center_of(i)))
        .filter(|&(_, c)| (c - end).dot(out) > 0.05 && geom.segment_clear(start, c))
        .min_by(|a, b| a.1.distance(probe).total_cmp(&b.1.distance(probe)).then(a.0.cmp(&b.0)))
}

/// Dijkstra distance field over the walkable cells of one room.
struct Field {
    dist: Vec<f32>,
    parent: Vec<u32>,
}

#[derive(PartialEq)]
struct QItem(f32, usize);

impl Eq for QItem {}

impl Ord for QItem {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0).then_with(|| o.1.cmp(&self.1))
    }
}

impl PartialOrd for QItem {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Field {
    fn new(grid: &WalkGrid, start: usize, room: u32) -> Field {
        let n = grid.cells.len();
        let mut dist = vec![f32::INFINITY; n];
        let mut parent = vec![u32::MAX; n];
        let mut heap = BinaryHeap::new();
        let mut nb = Vec::with_capacity(8);
        if start < n {
            dist[start] = 0.0;
            heap.push(QItem(0.0, start));
        }
        while let Some(QItem(d, i)) = heap.pop() {
            if d > dist[i] {
                continue;
            }
            grid.neighbours_in(i, room, &mut nb);
            for &(k, c) in &nb {
                let nd = d + c;
                if nd < dist[k] {
                    dist[k] = nd;
                    parent[k] = i as u32;
                    heap.push(QItem(nd, k));
                }
            }
        }
        Field { dist, parent }
    }

    /// Cells from just after the start up to and including `target`.
    fn path_to(&self, target: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut cur = target;
        while self.parent.get(cur).is_some_and(|&p| p != u32::MAX) {
            out.push(cur);
            cur = self.parent[cur] as usize;
            if out.len() > self.parent.len() {
                break;
            }
        }
        out.reverse();
        out
    }
}

/// Greedy string pulling: interior corner points of `pts` (first and last
/// excluded) such that consecutive corners see each other.
fn string_pull(pts: &[Vec2], clear: &impl Fn(Vec2, Vec2) -> bool) -> Vec<Vec2> {
    let mut out = Vec::new();
    let n = pts.len();
    let mut i = 0;
    while i + 1 < n {
        let mut j = n - 1;
        while j > i + 1 && !clear(pts[i], pts[j]) {
            j -= 1;
        }
        if j < n - 1 {
            out.push(pts[j]);
        }
        i = j;
    }
    out
}

/// Cells for a room's standing nodes: the most open cell first, then
/// farthest-point sampling until every reachable cell is within
/// `room_spacing` of a node (or the cap is hit).
fn room_node_cells(grid: &WalkGrid, fields: &[Field], room: u32, p: &DwellingNavParams) -> Vec<usize> {
    let reach: Vec<usize> = (0..grid.cells.len()).filter(|&i| fields.iter().any(|f| f.dist[i].is_finite())).collect();
    if reach.is_empty() || p.max_room_nodes == 0 {
        return Vec::new();
    }
    let open = |i: usize| grid.cells[i] == CellState::Free && grid.rooms[i] == room;
    let clearance = |idx: usize| -> u32 {
        let (ci, cj) = grid.coords(idx);
        for k in 1..=3i32 {
            for dj in -k..=k {
                for di in -k..=k {
                    if di.abs() != k && dj.abs() != k {
                        continue;
                    }
                    let (i, j) = (ci as i32 + di, cj as i32 + dj);
                    if i < 0 || j < 0 || i >= grid.width as i32 || j >= grid.height as i32 {
                        return (k - 1) as u32;
                    }
                    if !open(grid.index(i as u32, j as u32)) {
                        return (k - 1) as u32;
                    }
                }
            }
        }
        3
    };
    let clear: Vec<u32> = reach.iter().map(|&i| clearance(i)).collect();
    let max_c = clear.iter().copied().max().unwrap_or(0);
    let need = max_c.min(1);
    let cands: Vec<(usize, u32, Vec2)> =
        reach.iter().zip(&clear).filter(|&(_, &c)| c >= need).map(|(&i, &c)| (i, c, grid.center_of(i))).collect();
    let centroid = cands.iter().map(|c| c.2).sum::<Vec2>() / cands.len().max(1) as f32;
    let mut chosen: Vec<(usize, Vec2)> = Vec::new();
    // Most open cell, nearest the centroid on ties.
    if let Some(first) = cands.iter().min_by(|a, b| {
        b.1.cmp(&a.1).then(a.2.distance(centroid).total_cmp(&b.2.distance(centroid))).then(a.0.cmp(&b.0))
    }) {
        chosen.push((first.0, first.2));
    }
    let spacing = if p.room_spacing.is_finite() { p.room_spacing.max(0.5) } else { 2.5 };
    while chosen.len() < p.max_room_nodes {
        let best = cands
            .iter()
            .map(|c| (c, chosen.iter().map(|q| q.1.distance(c.2)).fold(f32::INFINITY, f32::min)))
            .max_by(|(a, da), (b, db)| da.total_cmp(db).then(a.1.cmp(&b.1)).then(b.0.cmp(&a.0)));
        match best {
            Some((c, d)) if d >= spacing => chosen.push((c.0, c.2)),
            _ => break,
        }
    }
    chosen.into_iter().map(|c| c.0).collect()
}
