//! Street network on the cell edges: shortest paths, kinds, connectivity and
//! conversion to polylines.

use std::cmp::Ordering;
use std::collections::{BTreeSet, BinaryHeap};

use mde_core::{Polyline, Street, StreetKind, Vec2};

use crate::topo::Topo;

/// Main road through a gate to the centre (m).
pub const ARTERY_WIDTH: f32 = 5.0;
/// Ordinary street (m).
pub const STREET_WIDTH: f32 = 3.5;
/// Narrow lane between blocks (m).
pub const ALLEY_WIDTH: f32 = 2.0;
/// Country road outside the walls (m).
pub const ROAD_WIDTH: f32 = 4.0;

/// Default carriageway width of a street kind, metres.
pub fn street_width(kind: StreetKind) -> f32 {
    match kind {
        StreetKind::Artery => ARTERY_WIDTH,
        StreetKind::Street => STREET_WIDTH,
        StreetKind::Alley => ALLEY_WIDTH,
        StreetKind::Road => ROAD_WIDTH,
    }
}

/// Priority when two kinds claim the same edge (higher wins).
pub(crate) fn rank(kind: StreetKind) -> u8 {
    match kind {
        StreetKind::Artery => 4,
        StreetKind::Road => 3,
        StreetKind::Street => 2,
        StreetKind::Alley => 1,
    }
}

/// Set `net[e]` to `kind` unless it already holds a higher-ranked kind.
pub(crate) fn claim(net: &mut [Option<StreetKind>], e: usize, kind: StreetKind) {
    if net[e].is_none_or(|k| rank(k) < rank(kind)) {
        net[e] = Some(kind);
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Item {
    cost: f32,
    v: usize,
}

impl Eq for Item {}

impl Ord for Item {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap on cost, then on vertex id for determinism.
        other.cost.total_cmp(&self.cost).then_with(|| other.v.cmp(&self.v))
    }
}

impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Multi-source Dijkstra over the cell edges. `cost(e)` returns `None` for
/// impassable edges. Returns the edge ids of the cheapest path from any source
/// to the first vertex satisfying `is_target` (empty if a source is a target),
/// plus the target vertex reached.
pub(crate) fn shortest_path(
    topo: &Topo,
    sources: &[usize],
    is_target: &dyn Fn(usize) -> bool,
    cost: &dyn Fn(usize) -> Option<f32>,
) -> Option<(Vec<usize>, usize)> {
    let n = topo.verts.len();
    let mut dist = vec![f32::INFINITY; n];
    let mut via: Vec<Option<usize>> = vec![None; n];
    let mut heap = BinaryHeap::new();
    for &s in sources {
        if s < n && dist[s] > 0.0 {
            dist[s] = 0.0;
            heap.push(Item { cost: 0.0, v: s });
        }
    }
    while let Some(Item { cost: d, v }) = heap.pop() {
        if d > dist[v] {
            continue;
        }
        if is_target(v) {
            let mut path = Vec::new();
            let mut cur = v;
            while let Some(e) = via[cur] {
                path.push(e);
                cur = topo.edges[e].other(cur);
            }
            path.reverse();
            return Some((path, v));
        }
        for &e in &topo.vert_edges[v] {
            let Some(c) = cost(e) else { continue };
            let w = topo.edges[e].other(v);
            let nd = d + c.max(0.0);
            if nd < dist[w] {
                dist[w] = nd;
                via[w] = Some(e);
                heap.push(Item { cost: nd, v: w });
            }
        }
    }
    None
}

/// Connected components of the network (edge ids grouped), largest first.
pub(crate) fn components(topo: &Topo, net: &[Option<StreetKind>]) -> Vec<Vec<usize>> {
    let mut seen = vec![false; topo.edges.len()];
    let mut out = Vec::new();
    for start in 0..topo.edges.len() {
        if net[start].is_none() || seen[start] {
            continue;
        }
        let mut comp = Vec::new();
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(e) = stack.pop() {
            comp.push(e);
            for v in [topo.edges[e].a, topo.edges[e].b] {
                for &f in &topo.vert_edges[v] {
                    if net[f].is_some() && !seen[f] {
                        seen[f] = true;
                        stack.push(f);
                    }
                }
            }
        }
        comp.sort_unstable();
        out.push(comp);
    }
    out.sort_by(|a, b| b.len().cmp(&a.len()).then(a[0].cmp(&b[0])));
    out
}

/// Vertices touched by a set of edges.
pub(crate) fn verts_of(topo: &Topo, edges: &[usize]) -> BTreeSet<usize> {
    edges.iter().flat_map(|&e| [topo.edges[e].a, topo.edges[e].b]).collect()
}

/// Chain the network into polylines, one kind at a time.
pub(crate) fn polylines(topo: &Topo, net: &[Option<StreetKind>]) -> Vec<Street> {
    let mut out = Vec::new();
    for kind in [StreetKind::Artery, StreetKind::Road, StreetKind::Street, StreetKind::Alley] {
        let of_kind = |e: usize| net[e] == Some(kind);
        let mut deg = vec![0u32; topo.verts.len()];
        for e in 0..topo.edges.len() {
            if of_kind(e) {
                deg[topo.edges[e].a] += 1;
                deg[topo.edges[e].b] += 1;
            }
        }
        let mut done = vec![false; topo.edges.len()];
        // Start chains at junctions / ends first so they break there.
        let mut order: Vec<usize> = (0..topo.edges.len()).filter(|&e| of_kind(e)).collect();
        order.sort_by_key(|&e| {
            let ed = &topo.edges[e];
            (deg[ed.a] == 2 && deg[ed.b] == 2, e)
        });
        for e0 in order {
            if done[e0] {
                continue;
            }
            done[e0] = true;
            let ed = &topo.edges[e0];
            let (mut a, mut b) = (ed.a, ed.b);
            if deg[b] != 2 && deg[a] == 2 {
                std::mem::swap(&mut a, &mut b);
            }
            let mut chain = vec![a, b];
            // Extend forward from the back.
            let extend = |chain: &mut Vec<usize>, done: &mut [bool]| {
                while let Some(&last) = chain.last() {
                    if deg[last] != 2 {
                        break;
                    }
                    let Some(f) = topo.vert_edges[last].iter().copied().find(|&f| of_kind(f) && !done[f]) else {
                        break;
                    };
                    done[f] = true;
                    chain.push(topo.edges[f].other(last));
                }
            };
            extend(&mut chain, &mut done);
            chain.reverse();
            extend(&mut chain, &mut done);
            let pts: Vec<Vec2> = chain.iter().map(|&v| topo.verts[v]).collect();
            out.push(Street { kind, path: Polyline(pts), width: street_width(kind) });
        }
    }
    out
}

/// Width of the network on edge `e` (0 if none).
pub(crate) fn width_on(net: &[Option<StreetKind>], e: usize) -> f32 {
    net[e].map_or(0.0, street_width)
}
