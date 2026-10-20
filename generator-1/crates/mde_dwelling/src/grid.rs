//! Planning grid: the house footprint rasterised into cells in the local frame,
//! plus generic cell-set helpers (connectivity, outline tracing).
//!
//! Cell `(i, j)` covers `[x(i), x(i+1)] × [y(j), y(j+1)]`; cell edges are wall
//! centrelines. Cells may be slightly non-square: the grid is stretched so a
//! rectangular footprint is covered exactly.

use std::collections::BTreeMap;

use mde_core::{DwellingStyle, Polygon, Rng, Vec2};

/// Upper bound on cells per axis; huge footprints get coarser cells instead.
const MAX_CELLS_PER_AXIS: i32 = 64;

/// Grid axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Axis {
    X,
    Y,
}

impl Axis {
    /// The perpendicular axis.
    pub fn other(self) -> Axis {
        match self {
            Axis::X => Axis::Y,
            Axis::Y => Axis::X,
        }
    }

    /// Component of a cell coordinate along this axis.
    pub fn of(self, c: (i32, i32)) -> i32 {
        match self {
            Axis::X => c.0,
            Axis::Y => c.1,
        }
    }

    /// Build a cell coordinate from an along-axis and an across-axis component.
    pub fn cell(self, along: i32, across: i32) -> (i32, i32) {
        match self {
            Axis::X => (along, across),
            Axis::Y => (across, along),
        }
    }

    /// Unit vector of the axis in the local frame.
    pub fn unit(self) -> Vec2 {
        match self {
            Axis::X => Vec2::X,
            Axis::Y => Vec2::Y,
        }
    }
}

/// Half-open cell rectangle `[i0, i1) × [j0, j1)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CellRect {
    pub i0: i32,
    pub j0: i32,
    pub i1: i32,
    pub j1: i32,
}

impl CellRect {
    pub fn new(i0: i32, j0: i32, i1: i32, j1: i32) -> Self {
        CellRect { i0, j0, i1, j1 }
    }

    /// Rectangle from along/across ranges for `axis`.
    pub fn from_axis(axis: Axis, along: (i32, i32), across: (i32, i32)) -> Self {
        match axis {
            Axis::X => CellRect::new(along.0, across.0, along.1, across.1),
            Axis::Y => CellRect::new(across.0, along.0, across.1, along.1),
        }
    }

    pub fn contains(&self, c: (i32, i32)) -> bool {
        c.0 >= self.i0 && c.0 < self.i1 && c.1 >= self.j0 && c.1 < self.j1
    }

    /// Low bound along `axis`.
    pub fn lo(&self, axis: Axis) -> i32 {
        match axis {
            Axis::X => self.i0,
            Axis::Y => self.j0,
        }
    }

    /// High (exclusive) bound along `axis`.
    pub fn hi(&self, axis: Axis) -> i32 {
        match axis {
            Axis::X => self.i1,
            Axis::Y => self.j1,
        }
    }

    /// All cells, row-major.
    pub fn cells(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        (self.j0..self.j1).flat_map(move |j| (self.i0..self.i1).map(move |i| (i, j)))
    }
}

/// Rasterised footprint.
#[derive(Clone, Debug)]
pub(crate) struct Grid {
    pub nx: i32,
    pub ny: i32,
    /// Cell width (local x), metres.
    pub cx: f32,
    /// Cell depth (local y), metres.
    pub cy: f32,
    /// Local coordinates of the grid's min corner.
    pub x0: f32,
    pub y0: f32,
    /// Cells that are part of the house.
    pub mask: Vec<bool>,
}

impl Grid {
    pub fn new(nx: i32, ny: i32, cx: f32, cy: f32, x0: f32, y0: f32) -> Self {
        Grid { nx, ny, cx, cy, x0, y0, mask: vec![false; (nx.max(0) * ny.max(0)) as usize] }
    }

    pub fn in_bounds(&self, c: (i32, i32)) -> bool {
        c.0 >= 0 && c.1 >= 0 && c.0 < self.nx && c.1 < self.ny
    }

    /// Flat index of an in-bounds cell.
    pub fn idx(&self, c: (i32, i32)) -> usize {
        (c.1 * self.nx + c.0) as usize
    }

    pub fn coords(&self, idx: usize) -> (i32, i32) {
        let i = idx as i32;
        (i % self.nx, i / self.nx)
    }

    /// Cell is in bounds and part of the house.
    pub fn inside(&self, c: (i32, i32)) -> bool {
        self.in_bounds(c) && self.mask[self.idx(c)]
    }

    pub fn n(&self, axis: Axis) -> i32 {
        match axis {
            Axis::X => self.nx,
            Axis::Y => self.ny,
        }
    }

    /// Cell size along `axis`, metres.
    pub fn size(&self, axis: Axis) -> f32 {
        match axis {
            Axis::X => self.cx,
            Axis::Y => self.cy,
        }
    }

    /// Coordinate of grid line `k` along `axis`.
    pub fn line(&self, axis: Axis, k: i32) -> f32 {
        match axis {
            Axis::X => self.x0 + k as f32 * self.cx,
            Axis::Y => self.y0 + k as f32 * self.cy,
        }
    }

    /// Grid vertex `(i, j)` in local coordinates.
    pub fn vertex(&self, v: (i32, i32)) -> Vec2 {
        Vec2::new(self.line(Axis::X, v.0), self.line(Axis::Y, v.1))
    }

    pub fn center(&self, c: (i32, i32)) -> Vec2 {
        Vec2::new(self.x0 + (c.0 as f32 + 0.5) * self.cx, self.y0 + (c.1 as f32 + 0.5) * self.cy)
    }

    pub fn cell_area(&self) -> f32 {
        self.cx * self.cy
    }

    /// Number of house cells.
    pub fn count(&self) -> usize {
        self.mask.iter().filter(|&&m| m).count()
    }

    /// All house cells, row-major.
    pub fn cells(&self) -> Vec<(i32, i32)> {
        (0..self.mask.len()).filter(|&k| self.mask[k]).map(|k| self.coords(k)).collect()
    }

    /// Number of cells needed along `axis` to span at least `metres`.
    pub fn cells_for(&self, axis: Axis, metres: f32) -> i32 {
        ((metres / self.size(axis)) - 1e-3).ceil().max(1.0) as i32
    }

    /// Keep only the largest 4-connected component of the mask and crop the
    /// grid to its bounding box. Returns `false` if the mask is empty.
    pub fn keep_largest_component(&mut self) -> bool {
        let comps = components(self.nx, self.ny, |c| self.inside(c));
        let Some(best) = comps.into_iter().max_by_key(|c| c.len()) else {
            return false;
        };
        let (mut i0, mut j0, mut i1, mut j1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &(i, j) in &best {
            i0 = i0.min(i);
            j0 = j0.min(j);
            i1 = i1.max(i + 1);
            j1 = j1.max(j + 1);
        }
        let mut g = Grid::new(i1 - i0, j1 - j0, self.cx, self.cy, self.line(Axis::X, i0), self.line(Axis::Y, j0));
        for &(i, j) in &best {
            let k = g.idx((i - i0, j - j0));
            g.mask[k] = true;
        }
        *self = g;
        true
    }
}

/// Rasterise a local-frame polygon (wall centrelines) into a grid whose cells
/// are as close to `cell` metres as possible while covering the polygon's
/// bounding box exactly. A cell is in if its centre is inside; only the largest
/// 4-connected component is kept. `None` if nothing remains.
pub(crate) fn rasterize(poly: &Polygon, cell: f32) -> Option<Grid> {
    if poly.len() < 3 {
        return None;
    }
    let b = poly.bounds();
    let size = b.size();
    if !(size.x.is_finite() && size.y.is_finite()) || size.x < 0.5 || size.y < 0.5 {
        return None;
    }
    let nx = ((size.x / cell).round() as i32).clamp(2, MAX_CELLS_PER_AXIS);
    let ny = ((size.y / cell).round() as i32).clamp(2, MAX_CELLS_PER_AXIS);
    let mut g = Grid::new(nx, ny, size.x / nx as f32, size.y / ny as f32, b.min.x, b.min.y);
    for j in 0..ny {
        for i in 0..nx {
            if poly.contains(g.center((i, j))) {
                let k = g.idx((i, j));
                g.mask[k] = true;
            }
        }
    }
    g.keep_largest_component().then_some(g)
}

/// Synthesise a footprint for `style` directly on the grid (front = local
/// south). Sizes follow the style (cottage ~6×5 m, townhouse ~6×10 m, farmhouse
/// ~12×7 m, manor ~16×12 m with wings, tavern ~12×10 m, shop ~7×9 m) with
/// seeded variation and occasional L / T / U wings.
pub(crate) fn synthesize(style: DwellingStyle, cell: f32, rng: &mut Rng) -> Grid {
    let m = |rng: &mut Rng, a: f32, b: f32| -> i32 { (rng.range_f32(a, b) / cell).round().max(2.0) as i32 };
    let mut rects: Vec<CellRect> = Vec::new();
    match style {
        DwellingStyle::Cottage => {
            let w = m(rng, 5.6, 7.6);
            let d = m(rng, 4.6, 5.8);
            rects.push(CellRect::new(0, 0, w, d));
            if rng.chance(0.35) {
                // Lean-to at the back, on one side.
                let ew = m(rng, 2.6, 3.6).min(w - 1);
                let ed = m(rng, 2.2, 3.2);
                let i0 = if rng.chance(0.5) { 0 } else { w - ew };
                rects.push(CellRect::new(i0, d, i0 + ew, d + ed));
            }
        }
        DwellingStyle::Townhouse => {
            let w = m(rng, 5.2, 7.2);
            let d = m(rng, 9.0, 12.0);
            rects.push(CellRect::new(0, 0, w, d));
            if rng.chance(0.3) {
                let ew = (w - m(rng, 1.6, 2.6)).max(3);
                let ed = m(rng, 2.6, 3.6);
                let i0 = if rng.chance(0.5) { 0 } else { w - ew };
                rects.push(CellRect::new(i0, d, i0 + ew, d + ed));
            }
        }
        DwellingStyle::Farmhouse => {
            let w = m(rng, 11.0, 14.0);
            let d = m(rng, 6.6, 8.0);
            rects.push(CellRect::new(0, 0, w, d));
            if rng.chance(0.4) {
                let ew = m(rng, 4.2, 5.6);
                let ed = m(rng, 3.2, 5.0);
                let i0 = if rng.chance(0.5) { 0 } else { w - ew };
                rects.push(CellRect::new(i0, d, i0 + ew, d + ed));
            }
        }
        DwellingStyle::Manor => {
            let w = m(rng, 15.0, 18.0);
            let d = m(rng, 8.0, 10.0);
            match rng.weighted(&[0.45, 0.25, 0.2, 0.1]) {
                0 => {
                    // U: two wings reaching forward around a front courtyard.
                    let ww = m(rng, 4.2, 5.6);
                    let wd = m(rng, 3.6, 5.0);
                    rects.push(CellRect::new(0, wd, w, wd + d));
                    rects.push(CellRect::new(0, 0, ww, wd));
                    rects.push(CellRect::new(w - ww, 0, w, wd));
                }
                1 => {
                    // T: a wing behind the middle of the main block.
                    let ww = m(rng, 5.0, 7.0);
                    let wd = m(rng, 4.0, 6.0);
                    let i0 = (w - ww) / 2;
                    rects.push(CellRect::new(0, 0, w, d));
                    rects.push(CellRect::new(i0, d, i0 + ww, d + wd));
                }
                2 => {
                    // L: one forward wing.
                    let ww = m(rng, 4.6, 6.0);
                    let wd = m(rng, 3.6, 5.0);
                    rects.push(CellRect::new(0, wd, w, wd + d));
                    let i0 = if rng.chance(0.5) { 0 } else { w - ww };
                    rects.push(CellRect::new(i0, 0, i0 + ww, wd));
                }
                _ => {
                    let d2 = m(rng, 11.0, 13.0);
                    rects.push(CellRect::new(0, 0, w, d2));
                }
            }
        }
        DwellingStyle::Tavern => {
            let w = m(rng, 11.0, 14.0);
            let d = m(rng, 9.0, 11.0);
            rects.push(CellRect::new(0, 0, w, d));
            if rng.chance(0.3) {
                let ew = m(rng, 4.2, 5.6);
                let ed = m(rng, 3.0, 4.4);
                let i0 = if rng.chance(0.5) { 0 } else { w - ew };
                rects.push(CellRect::new(i0, d, i0 + ew, d + ed));
            }
        }
        DwellingStyle::Shop => {
            let w = m(rng, 6.0, 8.0);
            let d = m(rng, 8.0, 10.0);
            rects.push(CellRect::new(0, 0, w, d));
        }
    }
    let nx = rects.iter().map(|r| r.i1).max().unwrap_or(2).max(2);
    let ny = rects.iter().map(|r| r.j1).max().unwrap_or(2).max(2);
    let mut g = Grid::new(nx, ny, cell, cell, 0.0, 0.0);
    for r in &rects {
        for c in r.cells() {
            if g.in_bounds(c) {
                let k = g.idx(c);
                g.mask[k] = true;
            }
        }
    }
    g
}

/// 4-connected components of the cells in `[0,nx)×[0,ny)` for which `member`
/// is true, each sorted row-major. Components are ordered by their first cell.
pub(crate) fn components(nx: i32, ny: i32, member: impl Fn((i32, i32)) -> bool) -> Vec<Vec<(i32, i32)>> {
    let w = nx.max(0) as usize;
    let mut seen = vec![false; w * ny.max(0) as usize];
    let mut out = Vec::new();
    for j in 0..ny {
        for i in 0..nx {
            let k = j as usize * w + i as usize;
            if seen[k] || !member((i, j)) {
                continue;
            }
            seen[k] = true;
            let mut comp = Vec::new();
            let mut stack = vec![(i, j)];
            while let Some(c) = stack.pop() {
                comp.push(c);
                for n in neighbours4(c) {
                    if n.0 < 0 || n.1 < 0 || n.0 >= nx || n.1 >= ny {
                        continue;
                    }
                    let kn = n.1 as usize * w + n.0 as usize;
                    if !seen[kn] && member(n) {
                        seen[kn] = true;
                        stack.push(n);
                    }
                }
            }
            comp.sort_by_key(|c| (c.1, c.0));
            out.push(comp);
        }
    }
    out
}

/// The four edge neighbours of a cell (S, E, N, W).
pub(crate) fn neighbours4(c: (i32, i32)) -> [(i32, i32); 4] {
    [(c.0, c.1 - 1), (c.0 + 1, c.1), (c.0, c.1 + 1), (c.0 - 1, c.1)]
}

/// Whether a set of cells is 4-connected (an empty set counts as connected).
pub(crate) fn is_connected(cells: &[(i32, i32)]) -> bool {
    let Some(&first) = cells.first() else {
        return true;
    };
    let (mut i0, mut j0, mut i1, mut j1) = (first.0, first.1, first.0, first.1);
    for &(i, j) in cells {
        i0 = i0.min(i);
        j0 = j0.min(j);
        i1 = i1.max(i);
        j1 = j1.max(j);
    }
    let w = (i1 - i0 + 1) as usize;
    let h = (j1 - j0 + 1) as usize;
    let mut member = vec![false; w * h];
    for &(i, j) in cells {
        member[(j - j0) as usize * w + (i - i0) as usize] = true;
    }
    let mut seen = vec![false; w * h];
    let mut stack = vec![first];
    seen[(first.1 - j0) as usize * w + (first.0 - i0) as usize] = true;
    let mut count = 0usize;
    while let Some(c) = stack.pop() {
        count += 1;
        for n in neighbours4(c) {
            if n.0 < i0 || n.1 < j0 || n.0 > i1 || n.1 > j1 {
                continue;
            }
            let k = (n.1 - j0) as usize * w + (n.0 - i0) as usize;
            if member[k] && !seen[k] {
                seen[k] = true;
                stack.push(n);
            }
        }
    }
    count == cells.len()
}

/// Boundary loops of the cell set given by `member` (which must be false outside
/// `[0,nx)×[0,ny)`), as grid-vertex coordinates. The set lies to the left of
/// every loop, so outer boundaries are CCW and holes CW. Diagonal pinches are
/// resolved into separate loops. Collinear vertices are removed.
pub(crate) fn trace_loops(nx: i32, ny: i32, member: impl Fn((i32, i32)) -> bool) -> Vec<Vec<(i32, i32)>> {
    let inside = |c: (i32, i32)| c.0 >= 0 && c.1 >= 0 && c.0 < nx && c.1 < ny && member(c);
    let mut out_edges: BTreeMap<(i32, i32), Vec<(i32, i32)>> = BTreeMap::new();
    let mut add = |a: (i32, i32), b: (i32, i32)| out_edges.entry(a).or_default().push(b);
    for j in 0..ny {
        for i in 0..nx {
            if !inside((i, j)) {
                continue;
            }
            if !inside((i, j - 1)) {
                add((i, j), (i + 1, j));
            }
            if !inside((i + 1, j)) {
                add((i + 1, j), (i + 1, j + 1));
            }
            if !inside((i, j + 1)) {
                add((i + 1, j + 1), (i, j + 1));
            }
            if !inside((i - 1, j)) {
                add((i, j + 1), (i, j));
            }
        }
    }
    let mut loops = Vec::new();
    while let Some(start) = out_edges.iter().find(|(_, v)| !v.is_empty()).map(|(&s, _)| s) {
        let mut pts = vec![start];
        let mut cur = start;
        let mut prev_dir: Option<(i32, i32)> = None;
        // Each step consumes one edge, so this terminates.
        while let Some(outs) = out_edges.get_mut(&cur).filter(|o| !o.is_empty()) {
            let pick = match prev_dir {
                Some(d) if outs.len() > 1 => {
                    // Prefer the sharpest left turn so pinched cells close their own loop.
                    let mut best = 0;
                    let mut best_rank = i32::MIN;
                    for (k, &o) in outs.iter().enumerate() {
                        let e = (o.0 - cur.0, o.1 - cur.1);
                        let cross = d.0 * e.1 - d.1 * e.0;
                        let dot = d.0 * e.0 + d.1 * e.1;
                        let rank = if cross > 0 { 2 } else if dot > 0 { 1 } else { 0 };
                        if rank > best_rank {
                            best_rank = rank;
                            best = k;
                        }
                    }
                    best
                }
                _ => 0,
            };
            let next = outs.remove(pick);
            prev_dir = Some((next.0 - cur.0, next.1 - cur.1));
            if next == start {
                break;
            }
            pts.push(next);
            cur = next;
        }
        loops.push(remove_collinear(pts));
    }
    loops
}

/// Drop vertices of a rectilinear loop that lie on a straight line.
fn remove_collinear(pts: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    let n = pts.len();
    if n < 4 {
        return pts;
    }
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let a = pts[(k + n - 1) % n];
        let b = pts[k];
        let c = pts[(k + 1) % n];
        let cross = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
        if cross != 0 {
            out.push(b);
        }
    }
    out
}

/// Signed area of a vertex loop in grid units (positive = CCW).
pub(crate) fn loop_area(pts: &[(i32, i32)]) -> i64 {
    let n = pts.len();
    let mut s = 0i64;
    for k in 0..n {
        let a = pts[k];
        let b = pts[(k + 1) % n];
        s += a.0 as i64 * b.1 as i64 - b.0 as i64 * a.1 as i64;
    }
    s / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterize_rect_exact() {
        let p = Polygon::rect(Vec2::new(3.0, 2.5), Vec2::new(5.6, 4.6));
        let g = rasterize(&p, 1.0).unwrap();
        assert_eq!((g.nx, g.ny), (6, 5));
        assert_eq!(g.count(), 30);
        assert!((g.cx * g.nx as f32 - 5.6).abs() < 1e-4);
    }

    #[test]
    fn trace_l_shape() {
        let cells = [(0, 0), (1, 0), (2, 0), (0, 1)];
        let loops = trace_loops(3, 2, |c| cells.contains(&c));
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].len(), 6);
        assert_eq!(loop_area(&loops[0]), 4);
    }

    #[test]
    fn trace_pinch_splits() {
        let cells = [(0, 0), (1, 1)];
        let loops = trace_loops(2, 2, |c| cells.contains(&c));
        assert_eq!(loops.len(), 2);
        assert!(loops.iter().all(|l| loop_area(l) == 1));
    }

    #[test]
    fn connectivity() {
        assert!(is_connected(&[(0, 0), (1, 0), (1, 1)]));
        assert!(!is_connected(&[(0, 0), (1, 1)]));
    }

    #[test]
    fn synth_all_styles() {
        for style in [
            DwellingStyle::Cottage,
            DwellingStyle::Townhouse,
            DwellingStyle::Farmhouse,
            DwellingStyle::Manor,
            DwellingStyle::Tavern,
            DwellingStyle::Shop,
        ] {
            for seed in 0..20 {
                let g = synthesize(style, 1.0, &mut Rng::new(seed));
                assert!(g.count() >= 20, "{style:?} {seed}");
                assert_eq!(components(g.nx, g.ny, |c| g.inside(c)).len(), 1);
            }
        }
    }
}
