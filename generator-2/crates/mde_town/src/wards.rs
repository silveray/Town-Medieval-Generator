//! Ward (district) assignment for inner and outer cells.

use std::collections::VecDeque;
use std::f32::consts::PI;

use mde_core::{Rng, StreetKind, Vec2, WardKind};

use crate::geom::signed_area;
use crate::topo::Topo;

/// Inputs to [`assign`].
pub(crate) struct WardInput<'a> {
    pub topo: &'a Topo,
    pub inner: &'a [bool],
    pub centres: &'a [Vec2],
    pub centre_cell: usize,
    pub castle: Option<usize>,
    pub r_city: f32,
    pub dense: bool,
    pub walls: bool,
    pub temple: bool,
    pub net: &'a [Option<StreetKind>],
    pub gates: &'a [usize],
    /// Cells clear of the river (eligible for the cathedral).
    pub dry: &'a [bool],
}

const KINDS: [WardKind; 8] = [
    WardKind::Craftsmen,
    WardKind::Merchant,
    WardKind::Patriciate,
    WardKind::Administration,
    WardKind::Military,
    WardKind::Slum,
    WardKind::Park,
    WardKind::Farm,
];

fn kind_index(k: WardKind) -> Option<usize> {
    KINDS.iter().position(|&x| x == k)
}

/// Assign a ward to every cell. Returns the wards and the cathedral cell.
pub(crate) fn assign(inp: &WardInput, rng: &mut Rng) -> (Vec<WardKind>, Option<usize>) {
    let topo = inp.topo;
    let nc = topo.cells.len();
    let mut ward = vec![WardKind::Empty; nc];
    let inner_cells: Vec<usize> = (0..nc).filter(|&c| inp.inner[c]).collect();
    let n = inner_cells.len();
    ward[inp.centre_cell] = WardKind::Market;
    if let Some(c) = inp.castle {
        ward[c] = WardKind::Castle;
    }
    let free = |c: usize| inp.inner[c] && c != inp.centre_cell && Some(c) != inp.castle;

    // Cathedral: next to the market, roomy and compact.
    let mut cathedral = None;
    if inp.temple {
        let mut best: Option<(usize, f32)> = None;
        let ok = |c: usize| free(c) && inp.dry[c];
        let mut cands: Vec<usize> = topo.neighbours(inp.centre_cell).into_iter().filter(|&c| ok(c)).collect();
        if cands.is_empty() {
            cands = inner_cells.iter().copied().filter(|&c| ok(c)).collect();
        }
        for c in cands {
            let pts = topo.pts(c);
            let a = signed_area(&pts);
            let per: f32 = (0..pts.len()).map(|i| pts[i].distance(pts[(i + 1) % pts.len()])).sum();
            let compact = 4.0 * PI * a / (per * per).max(1e-3);
            let score = a.sqrt() * compact * rng.range_f32(0.8, 1.2);
            if best.is_none_or(|b| score > b.1) {
                best = Some((c, score));
            }
        }
        if let Some((c, _)) = best {
            ward[c] = WardKind::Cathedral;
            cathedral = Some(c);
        }
    }

    // Remaining inner cells, centre outwards.
    let mut order: Vec<usize> = inner_cells.iter().copied().filter(|&c| free(c) && Some(c) != cathedral).collect();
    order.sort_by(|&a, &b| inp.centres[a].length().total_cmp(&inp.centres[b].length()).then(a.cmp(&b)));
    let mut counts = [0usize; KINDS.len()];
    let market_nb = topo.neighbours(inp.centre_cell);
    let castle_nb = inp.castle.map(|c| topo.neighbours(c)).unwrap_or_default();
    for c in order {
        let d = (inp.centres[c].length() / inp.r_city.max(1.0)).clamp(0.0, 1.0);
        let on_ring = topo.cell_sides[c].iter().any(|&e| topo.edges[e].other_cell(c).is_none_or(|o| !inp.inner[o]));
        let adj_market = market_nb.contains(&c);
        let adj_castle = castle_nb.contains(&c);
        let mut nb = [0f32; KINDS.len()];
        for o in topo.neighbours(c) {
            if let Some(i) = kind_index(ward[o]) {
                nb[i] += 1.0;
            }
        }
        let nf = n as f32;
        let lim = |k: usize, max: f32| counts[k] as f32 + 0.5 < max;
        let mut w = [0f32; KINDS.len()];
        if inp.dense {
            w[0] = 3.0 + 0.8 * nb[0];
            w[1] = 2.6 * (1.0 - d).powf(1.5) + if adj_market { 1.8 } else { 0.0 } + 0.6 * nb[1];
            if lim(2, nf / 5.0 + 1.0) {
                w[2] = 2.2 * (1.0 - d).powi(2) + 0.25 + 1.0 * nb[2];
            }
            if lim(3, 1.0 + nf / 25.0) {
                w[3] = 1.6 * (1.0 - d).powi(2) + if adj_market { 1.0 } else { 0.0 };
            }
            if lim(4, 1.0 + nf / 20.0) {
                w[4] = if adj_castle { 2.5 } else { 0.0 } + if on_ring && inp.walls { 0.5 * d } else { 0.0 };
            }
            w[5] = if on_ring { 2.0 * d * d + 0.6 * nb[5] } else { 0.1 * d };
            if lim(6, 1.0 + nf / 30.0) {
                w[6] = 0.3;
            }
        } else {
            w[0] = 3.0 + 0.5 * nb[0];
            w[1] = if adj_market { 1.4 } else { 0.15 };
            if lim(2, 1.0) {
                w[2] = 0.5;
            }
            w[7] = 1.4 * d + 0.3;
            if lim(6, 1.0 + nf / 8.0) {
                w[6] = 0.5;
            }
            w[5] = 0.15 * d;
        }
        let k = rng.weighted(&w);
        counts[k] += 1;
        ward[c] = KINDS[k];
    }

    // Outer cells by ring distance from the inner set.
    let mut ring_dist = vec![usize::MAX; nc];
    let mut q = VecDeque::new();
    for &c in &inner_cells {
        ring_dist[c] = 0;
        q.push_back(c);
    }
    while let Some(c) = q.pop_front() {
        for o in topo.neighbours(c) {
            if ring_dist[o] == usize::MAX {
                ring_dist[o] = ring_dist[c] + 1;
                q.push_back(o);
            }
        }
    }
    for c in 0..nc {
        if inp.inner[c] {
            continue;
        }
        let k = ring_dist[c];
        let road = topo.cell_sides[c].iter().any(|&e| inp.net[e] == Some(StreetKind::Road));
        let gate_cell = topo.cells[c].iter().any(|v| inp.gates.contains(v));
        let map_edge = topo.cell_sides[c].iter().any(|&e| topo.edges[e].other_cell(c).is_none());
        let opts: &[(WardKind, f32)] = if inp.walls {
            if gate_cell {
                &[(WardKind::Gate, 0.6), (WardKind::Slum, 0.4)]
            } else if k == 1 && road {
                &[(WardKind::Slum, 0.35), (WardKind::Farm, 0.45), (WardKind::Gate, 0.2)]
            } else if k == 1 {
                &[(WardKind::Farm, 0.6), (WardKind::Empty, 0.4)]
            } else if k == 2 && road {
                &[(WardKind::Farm, 0.7), (WardKind::Empty, 0.3)]
            } else if k == 2 {
                &[(WardKind::Farm, 0.5), (WardKind::Empty, 0.5)]
            } else if map_edge {
                &[(WardKind::Farm, 0.15), (WardKind::Empty, 0.85)]
            } else {
                &[(WardKind::Farm, 0.35), (WardKind::Empty, 0.65)]
            }
        } else if k == 1 && road && inp.dense {
            &[(WardKind::Slum, 0.3), (WardKind::Farm, 0.55), (WardKind::Empty, 0.15)]
        } else if k == 1 {
            &[(WardKind::Farm, if road { 0.8 } else { 0.7 }), (WardKind::Empty, 0.25)]
        } else if k == 2 {
            &[(WardKind::Farm, 0.55), (WardKind::Empty, 0.45)]
        } else if map_edge {
            &[(WardKind::Farm, 0.12), (WardKind::Empty, 0.88)]
        } else {
            &[(WardKind::Farm, 0.3), (WardKind::Empty, 0.7)]
        };
        let w: Vec<f32> = opts.iter().map(|o| o.1).collect();
        ward[c] = opts[rng.weighted(&w)].0;
    }
    (ward, cathedral)
}
