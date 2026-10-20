//! Navigation graph data types, A* path finding and path geometry.
//!
//! A [`NavGraph`] is plain serialisable data: nodes in world space (Y up) and
//! undirected edges with a traversal cost. Edges may carry intermediate
//! [`NavEdge::via`] points (a path bending around furniture, a stair flight), so
//! the graph stays small while the walked geometry stays exact.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use mde_core::Vec3;
use serde::{Deserialize, Serialize};

/// Extra cost (in metres of walking) for passing through a door: NPCs prefer
/// staying in one room when the detour is short.
pub const DOOR_PENALTY: f32 = 0.5;

/// Cost multiplier for climbing a stair flight (slower than walking on the flat).
pub const STAIR_FACTOR: f32 = 1.5;

/// What a [`NavNode`] stands for. Building ids are [`mde_core::Building::id`];
/// levels are [`mde_core::Floor::level`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NavNodeKind {
    /// Point on a street centreline (street vertices, junctions and resampled points).
    Street,
    /// Point on an open square.
    Plaza,
    /// Point on a bridge deck; `bridge` indexes [`mde_core::Town::bridges`].
    Bridge { bridge: u32 },
    /// Just outside a building's entrance, where the street network joins it.
    Doorstep { building: u32 },
    /// Centre of a passable opening (door, doorway, entrance) on its wall
    /// centreline. `opening` indexes [`mde_core::Floor::openings`] of floor
    /// `level`; `leaf` is `false` for open archways (nothing to animate).
    Door { building: u32, level: i32, opening: u32, leaf: bool },
    /// A place to stand inside a room; big rooms get several.
    Room { building: u32, level: i32, room: u32 },
    /// Landing in front of the bottom of a stair flight (on floor `level`).
    /// `stair` counts flights over all floors of the dwelling in order
    /// (floor by floor, then [`mde_core::Floor::stairs`] order).
    StairBottom { building: u32, level: i32, stair: u32 },
    /// Landing beyond the top of stair flight `stair` (on floor `level`).
    StairTop { building: u32, level: i32, stair: u32 },
    /// Free-standing helper point.
    Waypoint,
}

impl NavNodeKind {
    /// Building this node belongs to, if any (interior nodes and doorsteps).
    pub fn building(&self) -> Option<u32> {
        match *self {
            NavNodeKind::Doorstep { building }
            | NavNodeKind::Door { building, .. }
            | NavNodeKind::Room { building, .. }
            | NavNodeKind::StairBottom { building, .. }
            | NavNodeKind::StairTop { building, .. } => Some(building),
            _ => None,
        }
    }

    /// Floor level of an interior node.
    pub fn level(&self) -> Option<i32> {
        match *self {
            NavNodeKind::Door { level, .. }
            | NavNodeKind::Room { level, .. }
            | NavNodeKind::StairBottom { level, .. }
            | NavNodeKind::StairTop { level, .. } => Some(level),
            _ => None,
        }
    }

    /// `true` for nodes inside a building (doors count as inside).
    pub fn is_interior(&self) -> bool {
        self.level().is_some()
    }

    /// `true` for public outdoor walkways: streets, plazas and bridge decks.
    pub fn is_public(&self) -> bool {
        matches!(self, NavNodeKind::Street | NavNodeKind::Plaza | NavNodeKind::Bridge { .. })
    }
}

/// One graph node, in world space (`(plan.x, height, -plan.y)`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NavNode {
    pub pos: Vec3,
    pub kind: NavNodeKind,
}

/// How an edge is traversed; drives animation and cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NavEdgeKind {
    Walk,
    /// Passes through a door (one end is a [`NavNodeKind::Door`]).
    Door,
    /// Climbs a stair flight.
    Stair,
    /// Runs along a bridge deck.
    Bridge,
}

/// Undirected edge between nodes `a` and `b`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NavEdge {
    pub a: u32,
    pub b: u32,
    /// Traversal cost in metre-equivalents, never below the walked length
    /// (keeps the A* Euclidean heuristic admissible).
    pub cost: f32,
    pub kind: NavEdgeKind,
    /// Intermediate world-space points from `a` to `b` (empty = straight line).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub via: Vec<Vec3>,
}

impl NavEdge {
    /// The node at the other end, or `None` if `n` is not an end of this edge.
    pub fn other(&self, n: u32) -> Option<u32> {
        if self.a == n {
            Some(self.b)
        } else if self.b == n {
            Some(self.a)
        } else {
            None
        }
    }
}

/// Cost of an edge of `kind` whose walked polyline is `length` metres long.
pub fn edge_cost(kind: NavEdgeKind, length: f32) -> f32 {
    match kind {
        NavEdgeKind::Walk | NavEdgeKind::Bridge => length,
        NavEdgeKind::Door => length + DOOR_PENALTY,
        NavEdgeKind::Stair => length * STAIR_FACTOR,
    }
}

/// Walkable navigation graph for streets, bridges and building interiors.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NavGraph {
    pub nodes: Vec<NavNode>,
    pub edges: Vec<NavEdge>,
}

impl NavGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Add a node and return its id.
    pub fn add_node(&mut self, pos: Vec3, kind: NavNodeKind) -> u32 {
        self.nodes.push(NavNode { pos, kind });
        (self.nodes.len() - 1) as u32
    }

    /// Add a straight edge, costed from its length. Returns the edge index, or
    /// `None` for self-loops and unknown node ids.
    pub fn add_edge(&mut self, a: u32, b: u32, kind: NavEdgeKind) -> Option<usize> {
        self.add_edge_via(a, b, kind, Vec::new())
    }

    /// Add an edge through intermediate points `via` (from `a` towards `b`),
    /// costed from the polyline length.
    pub fn add_edge_via(&mut self, a: u32, b: u32, kind: NavEdgeKind, via: Vec<Vec3>) -> Option<usize> {
        if a == b || a as usize >= self.nodes.len() || b as usize >= self.nodes.len() {
            return None;
        }
        let pa = self.nodes[a as usize].pos;
        let pb = self.nodes[b as usize].pos;
        let length = polyline_length3(std::iter::once(pa).chain(via.iter().copied()).chain(std::iter::once(pb)));
        self.edges.push(NavEdge { a, b, cost: edge_cost(kind, length), kind, via });
        Some(self.edges.len() - 1)
    }

    /// World-space polyline of edge `e` from end `a` to end `b`.
    pub fn edge_points(&self, e: &NavEdge) -> Vec<Vec3> {
        let mut pts = Vec::with_capacity(e.via.len() + 2);
        pts.push(self.nodes[e.a as usize].pos);
        pts.extend_from_slice(&e.via);
        pts.push(self.nodes[e.b as usize].pos);
        pts
    }

    /// Append `other`, re-basing its node ids. Returns the id offset.
    pub fn append(&mut self, other: &NavGraph) -> u32 {
        let base = self.nodes.len() as u32;
        self.nodes.extend_from_slice(&other.nodes);
        self.edges.extend(other.edges.iter().map(|e| NavEdge { a: e.a + base, b: e.b + base, ..e.clone() }));
        base
    }

    /// Keep only nodes for which `keep(id, node)` is true, dropping edges that
    /// touch removed nodes. Returns the old-id → new-id map.
    pub fn retain_nodes(&mut self, mut keep: impl FnMut(u32, &NavNode) -> bool) -> Vec<Option<u32>> {
        let mut map = Vec::with_capacity(self.nodes.len());
        let mut next = 0u32;
        for (i, n) in self.nodes.iter().enumerate() {
            if keep(i as u32, n) {
                map.push(Some(next));
                next += 1;
            } else {
                map.push(None);
            }
        }
        let mut i = 0;
        self.nodes.retain(|_| {
            let k = map[i].is_some();
            i += 1;
            k
        });
        self.edges.retain_mut(|e| {
            match (map.get(e.a as usize).copied().flatten(), map.get(e.b as usize).copied().flatten()) {
                (Some(a), Some(b)) => {
                    e.a = a;
                    e.b = b;
                    true
                }
                _ => false,
            }
        });
        map
    }

    /// Number of edges touching each node.
    pub fn degrees(&self) -> Vec<u32> {
        let n = self.nodes.len();
        let mut d = vec![0u32; n];
        for e in self.edges.iter().filter(|e| (e.a as usize) < n && (e.b as usize) < n) {
            d[e.a as usize] += 1;
            d[e.b as usize] += 1;
        }
        d
    }

    /// Connected-component label per node (labels are `0..count`, assigned in
    /// node order, so they are deterministic).
    pub fn components(&self) -> Vec<u32> {
        let n = self.nodes.len();
        let mut parent: Vec<u32> = (0..n as u32).collect();
        fn find(p: &mut [u32], mut x: u32) -> u32 {
            while p[x as usize] != x {
                let up = p[p[x as usize] as usize];
                p[x as usize] = up;
                x = up;
            }
            x
        }
        for e in &self.edges {
            if (e.a as usize) < n && (e.b as usize) < n {
                let ra = find(&mut parent, e.a);
                let rb = find(&mut parent, e.b);
                if ra != rb {
                    let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
                    parent[hi as usize] = lo;
                }
            }
        }
        let mut label = vec![u32::MAX; n];
        let mut next = 0;
        for i in 0..n as u32 {
            let r = find(&mut parent, i);
            if label[r as usize] == u32::MAX {
                label[r as usize] = next;
                next += 1;
            }
            label[i as usize] = label[r as usize];
        }
        label
    }

    /// Ids of all nodes belonging to `building` (doorsteps and interior).
    pub fn building_nodes(&self, building: u32) -> Vec<u32> {
        (0..self.nodes.len() as u32).filter(|&i| self.nodes[i as usize].kind.building() == Some(building)).collect()
    }
}

/// Length of a 3D polyline.
pub(crate) fn polyline_length3(pts: impl IntoIterator<Item = Vec3>) -> f32 {
    let mut it = pts.into_iter();
    let Some(mut prev) = it.next() else { return 0.0 };
    let mut len = 0.0;
    for p in it {
        len += prev.distance(p);
        prev = p;
    }
    len
}

/// Cost actually used for search: the stored cost if sane, else the length.
fn search_cost(g: &NavGraph, e: &NavEdge) -> f32 {
    if e.cost.is_finite() && e.cost >= 0.0 {
        e.cost
    } else {
        let len = polyline_length3(g.edge_points(e));
        if len.is_finite() { len } else { f32::MAX / 4.0 }
    }
}

/// A path found by [`Router::find_route`].
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    /// Node ids from start to goal (inclusive).
    pub nodes: Vec<u32>,
    /// Edge index used between `nodes[i]` and `nodes[i + 1]`.
    pub edges: Vec<usize>,
    /// Total cost.
    pub cost: f32,
}

/// Cached adjacency of a [`NavGraph`] for repeated queries. Rebuild it after
/// the graph changes (it is cheap: one pass over the edges).
#[derive(Clone, Debug, Default)]
pub struct Router {
    /// `adj[offsets[n]..offsets[n + 1]]` = `(neighbour, edge index)` of node `n`.
    offsets: Vec<u32>,
    adj: Vec<(u32, u32)>,
}

#[derive(Clone, Copy, PartialEq)]
struct HeapItem {
    f: f32,
    node: u32,
}

impl Eq for HeapItem {}

impl Ord for HeapItem {
    fn cmp(&self, o: &Self) -> Ordering {
        // Min-heap on f, ties broken towards the lower node id.
        o.f.total_cmp(&self.f).then_with(|| o.node.cmp(&self.node))
    }
}

impl PartialOrd for HeapItem {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Router {
    /// Build the adjacency of `g`. Edges with out-of-range ends or self-loops
    /// are ignored.
    pub fn new(g: &NavGraph) -> Self {
        let n = g.nodes.len();
        let valid = |e: &NavEdge| e.a != e.b && (e.a as usize) < n && (e.b as usize) < n;
        let mut count = vec![0u32; n + 1];
        for e in g.edges.iter().filter(|e| valid(e)) {
            count[e.a as usize + 1] += 1;
            count[e.b as usize + 1] += 1;
        }
        for i in 0..n {
            count[i + 1] += count[i];
        }
        let offsets = count.clone();
        let mut fill = count;
        let mut adj = vec![(0u32, 0u32); offsets[n] as usize];
        for (ei, e) in g.edges.iter().enumerate().filter(|(_, e)| valid(e)) {
            adj[fill[e.a as usize] as usize] = (e.b, ei as u32);
            fill[e.a as usize] += 1;
            adj[fill[e.b as usize] as usize] = (e.a, ei as u32);
            fill[e.b as usize] += 1;
        }
        Router { offsets, adj }
    }

    /// `(neighbour, edge index)` pairs of node `n`, in edge order.
    pub fn neighbours(&self, n: u32) -> &[(u32, u32)] {
        let i = n as usize;
        if i + 1 >= self.offsets.len() {
            return &[];
        }
        &self.adj[self.offsets[i] as usize..self.offsets[i + 1] as usize]
    }

    /// A* from `from` to `to` with a Euclidean heuristic. Deterministic: ties
    /// are broken by node id and edges are scanned in index order.
    pub fn find_route(&self, g: &NavGraph, from: u32, to: u32) -> Option<Route> {
        let n = g.nodes.len();
        if from as usize >= n || to as usize >= n || self.offsets.len() != n + 1 {
            return None;
        }
        if from == to {
            return Some(Route { nodes: vec![from], edges: Vec::new(), cost: 0.0 });
        }
        let goal = g.nodes[to as usize].pos;
        let h = |i: u32| {
            let d = g.nodes[i as usize].pos.distance(goal);
            if d.is_finite() { d } else { 0.0 }
        };
        let mut gs = vec![f32::INFINITY; n];
        let mut came: Vec<(u32, u32)> = vec![(u32::MAX, u32::MAX); n];
        let mut closed = vec![false; n];
        let mut heap = BinaryHeap::new();
        gs[from as usize] = 0.0;
        heap.push(HeapItem { f: h(from), node: from });
        while let Some(HeapItem { node, .. }) = heap.pop() {
            if closed[node as usize] {
                continue;
            }
            closed[node as usize] = true;
            if node == to {
                break;
            }
            let g0 = gs[node as usize];
            for &(nb, ei) in self.neighbours(node) {
                if closed[nb as usize] {
                    continue;
                }
                let ng = g0 + search_cost(g, &g.edges[ei as usize]);
                if ng < gs[nb as usize] {
                    gs[nb as usize] = ng;
                    came[nb as usize] = (node, ei);
                    heap.push(HeapItem { f: ng + h(nb), node: nb });
                }
            }
        }
        if !closed[to as usize] {
            return None;
        }
        let mut nodes = vec![to];
        let mut edges = Vec::new();
        let mut cur = to;
        while cur != from {
            let (prev, ei) = came[cur as usize];
            if prev == u32::MAX {
                return None;
            }
            nodes.push(prev);
            edges.push(ei as usize);
            cur = prev;
        }
        nodes.reverse();
        edges.reverse();
        Some(Route { nodes, edges, cost: gs[to as usize] })
    }

    /// Node ids of the cheapest path, see [`Router::find_route`].
    pub fn find_path(&self, g: &NavGraph, from: u32, to: u32) -> Option<Vec<u32>> {
        self.find_route(g, from, to).map(|r| r.nodes)
    }

    /// Cheapest edge joining `a` and `b` (lowest index on ties).
    pub fn edge_between(&self, g: &NavGraph, a: u32, b: u32) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        for &(nb, ei) in self.neighbours(a) {
            if nb == b {
                let c = search_cost(g, &g.edges[ei as usize]);
                if best.is_none_or(|(bc, _)| c < bc) {
                    best = Some((c, ei as usize));
                }
            }
        }
        best.map(|(_, e)| e)
    }

    /// World-space points along `path` (node positions plus edge `via`
    /// points), see [`path_points`].
    pub fn path_points(&self, g: &NavGraph, path: &[u32]) -> Vec<Vec3> {
        let mut out: Vec<Vec3> = Vec::new();
        let push = |out: &mut Vec<Vec3>, p: Vec3| {
            if out.last().is_none_or(|q: &Vec3| q.distance_squared(p) > 1e-8) {
                out.push(p);
            }
        };
        for (i, &n) in path.iter().enumerate() {
            let Some(node) = g.nodes.get(n as usize) else { continue };
            if i > 0 {
                let prev = path[i - 1];
                if let Some(ei) = self.edge_between(g, prev, n) {
                    let e = &g.edges[ei];
                    if e.a == prev {
                        e.via.iter().for_each(|&p| push(&mut out, p));
                    } else {
                        e.via.iter().rev().for_each(|&p| push(&mut out, p));
                    }
                }
            }
            push(&mut out, node.pos);
        }
        out
    }
}

/// A* path from node `from` to node `to` (inclusive), or `None` if they are not
/// connected. Builds a [`Router`] per call; keep one around for many queries.
pub fn find_path(g: &NavGraph, from: u32, to: u32) -> Option<Vec<u32>> {
    Router::new(g).find_path(g, from, to)
}

/// Closest node to world point `p` whose kind passes `filter` (lowest id on ties).
pub fn nearest_node(g: &NavGraph, p: Vec3, filter: impl Fn(&NavNodeKind) -> bool) -> Option<u32> {
    let mut best: Option<(f32, u32)> = None;
    for (i, n) in g.nodes.iter().enumerate() {
        if !filter(&n.kind) {
            continue;
        }
        let d = n.pos.distance_squared(p);
        if d.is_finite() && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, i as u32));
        }
    }
    best.map(|(_, i)| i)
}

/// World-space polyline for a node path: node positions plus the `via` points
/// of the edges between them (interior paths are already string-pulled around
/// walls and furniture when the graph is built). Consecutive duplicates are
/// removed.
pub fn path_points(g: &NavGraph, path: &[u32]) -> Vec<Vec3> {
    Router::new(g).path_points(g, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_graph(n: u32) -> NavGraph {
        let mut g = NavGraph::new();
        for i in 0..n {
            g.add_node(Vec3::new(i as f32, 0.0, 0.0), NavNodeKind::Street);
        }
        for i in 1..n {
            g.add_edge(i - 1, i, NavEdgeKind::Walk);
        }
        g
    }

    #[test]
    fn astar_on_a_line_and_a_shortcut() {
        let mut g = line_graph(5);
        assert_eq!(find_path(&g, 0, 4), Some(vec![0, 1, 2, 3, 4]));
        assert_eq!(find_path(&g, 4, 0), Some(vec![4, 3, 2, 1, 0]));
        let extra = g.add_node(Vec3::new(10.0, 0.0, 0.0), NavNodeKind::Waypoint);
        assert_eq!(find_path(&g, 0, extra), None);
        g.add_edge(0, 4, NavEdgeKind::Walk);
        assert_eq!(find_path(&g, 0, 4), Some(vec![0, 4]));
        assert_eq!(find_path(&g, 2, 2), Some(vec![2]));
        assert_eq!(find_path(&g, 0, 99), None);
    }

    #[test]
    fn via_points_follow_direction() {
        let mut g = NavGraph::new();
        let a = g.add_node(Vec3::ZERO, NavNodeKind::Waypoint);
        let b = g.add_node(Vec3::new(2.0, 0.0, 0.0), NavNodeKind::Waypoint);
        g.add_edge_via(a, b, NavEdgeKind::Walk, vec![Vec3::new(1.0, 0.0, 1.0)]);
        assert!((g.edges[0].cost - 2.0 * 2f32.sqrt()).abs() < 1e-5);
        let fwd = path_points(&g, &[a, b]);
        let back = path_points(&g, &[b, a]);
        assert_eq!(fwd.len(), 3);
        assert_eq!(fwd[1], Vec3::new(1.0, 0.0, 1.0));
        assert_eq!(back.iter().rev().copied().collect::<Vec<_>>(), fwd);
    }

    #[test]
    fn retain_remaps_edges() {
        let mut g = line_graph(4);
        let map = g.retain_nodes(|i, _| i != 1);
        assert_eq!(map, vec![Some(0), None, Some(1), Some(2)]);
        assert_eq!(g.nodes.len(), 3);
        assert_eq!(g.edges.len(), 1);
        assert_eq!((g.edges[0].a, g.edges[0].b), (1, 2));
        assert_eq!(g.components(), vec![0, 1, 1]);
    }

    #[test]
    fn bad_costs_do_not_break_search() {
        let mut g = line_graph(3);
        g.edges[0].cost = f32::NAN;
        g.edges[1].cost = -5.0;
        g.edges.push(NavEdge { a: 0, b: 77, cost: 1.0, kind: NavEdgeKind::Walk, via: vec![] });
        assert_eq!(find_path(&g, 0, 2), Some(vec![0, 1, 2]));
        assert!(nearest_node(&g, Vec3::new(1.2, 0.0, 0.0), |_| true) == Some(1));
    }
}
