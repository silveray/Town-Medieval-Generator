//! Planar subdivision of the map into convex cells that share vertices by index.

use std::collections::BTreeMap;

use mde_core::Vec2;

/// An undirected edge between two shared vertices (`a < b`).
#[derive(Clone, Debug)]
pub(crate) struct Edge {
    pub a: usize,
    pub b: usize,
    /// Cell that walks this edge `a → b` (the cell lies to its left).
    pub left: Option<usize>,
    /// Cell that walks this edge `b → a`.
    pub right: Option<usize>,
}

impl Edge {
    /// The endpoint that is not `v`.
    pub fn other(&self, v: usize) -> usize {
        if v == self.a { self.b } else { self.a }
    }

    /// The cell on the other side of the edge from `c`.
    pub fn other_cell(&self, c: usize) -> Option<usize> {
        if self.left == Some(c) {
            self.right
        } else if self.right == Some(c) {
            self.left
        } else {
            None
        }
    }

    /// Both adjacent cells (one may be missing on the map boundary).
    pub fn cells(&self) -> [Option<usize>; 2] {
        [self.left, self.right]
    }
}

/// Cells (CCW rings of vertex indices) plus derived edge adjacency.
#[derive(Clone, Debug, Default)]
pub(crate) struct Topo {
    pub verts: Vec<Vec2>,
    pub cells: Vec<Vec<usize>>,
    pub edges: Vec<Edge>,
    edge_map: BTreeMap<(usize, usize), usize>,
    /// Edge ids incident to each vertex.
    pub vert_edges: Vec<Vec<usize>>,
    /// Edge id of side `i` (vertex `i → i+1`) of each cell.
    pub cell_sides: Vec<Vec<usize>>,
}

impl Topo {
    pub fn new(verts: Vec<Vec2>, cells: Vec<Vec<usize>>) -> Self {
        let mut t = Self { verts, cells, ..Default::default() };
        t.rebuild();
        t
    }

    /// Recompute edges and adjacency after editing `verts` / `cells`.
    pub fn rebuild(&mut self) {
        self.edges.clear();
        self.edge_map.clear();
        self.vert_edges = vec![Vec::new(); self.verts.len()];
        self.cell_sides = Vec::with_capacity(self.cells.len());
        for (c, cell) in self.cells.iter().enumerate() {
            let m = cell.len();
            let mut sides = Vec::with_capacity(m);
            for i in 0..m {
                let u = cell[i];
                let v = cell[(i + 1) % m];
                let key = (u.min(v), u.max(v));
                let id = match self.edge_map.get(&key) {
                    Some(&id) => id,
                    None => {
                        self.edges.push(Edge { a: key.0, b: key.1, left: None, right: None });
                        let id = self.edges.len() - 1;
                        self.edge_map.insert(key, id);
                        id
                    }
                };
                if u < v {
                    self.edges[id].left = Some(c);
                } else {
                    self.edges[id].right = Some(c);
                }
                sides.push(id);
            }
            self.cell_sides.push(sides);
        }
        for (id, e) in self.edges.iter().enumerate() {
            self.vert_edges[e.a].push(id);
            self.vert_edges[e.b].push(id);
        }
    }

    pub fn edge_len(&self, e: usize) -> f32 {
        let e = &self.edges[e];
        self.verts[e.a].distance(self.verts[e.b])
    }

    /// Cell vertex positions.
    pub fn pts(&self, c: usize) -> Vec<Vec2> {
        self.cells[c].iter().map(|&v| self.verts[v]).collect()
    }

    /// Cells sharing an edge with `c`, ascending.
    pub fn neighbours(&self, c: usize) -> Vec<usize> {
        let mut out: Vec<usize> = self.cell_sides[c].iter().filter_map(|&e| self.edges[e].other_cell(c)).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Cells having `v` as a vertex, ascending.
    pub fn cells_at(&self, v: usize) -> Vec<usize> {
        let mut out: Vec<usize> = self.vert_edges[v].iter().flat_map(|&e| self.edges[e].cells()).flatten().collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Insert a new vertex at `p` on the edge `u–v` of every cell using it.
    pub fn insert_on_edge(&mut self, u: usize, v: usize, p: Vec2) -> usize {
        let w = self.verts.len();
        self.verts.push(p);
        for cell in self.cells.iter_mut() {
            let m = cell.len();
            for i in 0..m {
                let a = cell[i];
                let b = cell[(i + 1) % m];
                if (a == u && b == v) || (a == v && b == u) {
                    cell.insert(i + 1, w);
                    break;
                }
            }
        }
        w
    }

    /// Split cell `c` along the chord between its local vertex indices `i` and
    /// `j`. Cell `c` keeps the ring `i..=j`; the returned new cell gets `j..=i`.
    /// Call [`Topo::rebuild`] afterwards.
    pub fn split_cell(&mut self, c: usize, i: usize, j: usize) -> usize {
        let cell = self.cells[c].clone();
        let m = cell.len();
        let mut a = Vec::new();
        let mut k = i;
        loop {
            a.push(cell[k]);
            if k == j {
                break;
            }
            k = (k + 1) % m;
        }
        let mut b = Vec::new();
        k = j;
        loop {
            b.push(cell[k]);
            if k == i {
                break;
            }
            k = (k + 1) % m;
        }
        self.cells[c] = a;
        self.cells.push(b);
        self.cells.len() - 1
    }
}
