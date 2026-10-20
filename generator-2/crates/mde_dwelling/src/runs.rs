//! Maximal straight runs of grid edges separating two labels (rooms or the
//! outside), and the door-placement margins at their ends.

use mde_core::Vec2;

use crate::grid::{Axis, Grid};
use crate::{EXTERIOR_WALL, INTERIOR_WALL};

/// Label of cells outside the house.
pub(crate) const OUTSIDE: i32 = -1;

/// Extra clearance kept between an opening and a wall junction, metres.
pub(crate) const JUNCTION_CLEARANCE: f32 = 0.06;

/// A maximal straight run of edges with the same label pair on either side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Run {
    /// Direction the run extends in (`X` = lies on a horizontal grid line).
    pub axis: Axis,
    /// Index of the grid line the run lies on (perpendicular to `axis`).
    pub line: i32,
    /// Vertex range along `axis`, `s0 < s1`.
    pub s0: i32,
    pub s1: i32,
    /// Label on the low side (below a horizontal run, left of a vertical one).
    pub lo: i32,
    /// Label on the high side.
    pub hi: i32,
}

impl Run {
    pub fn exterior(&self) -> bool {
        self.lo == OUTSIDE || self.hi == OUTSIDE
    }

    pub fn thickness(&self) -> f32 {
        if self.exterior() { EXTERIOR_WALL } else { INTERIOR_WALL }
    }

    pub fn has(&self, label: i32) -> bool {
        self.lo == label || self.hi == label
    }

    /// The label on the other side from `label`.
    pub fn other(&self, label: i32) -> i32 {
        if self.lo == label { self.hi } else { self.lo }
    }

    /// Unit normal pointing from the low side to the high side (local frame).
    pub fn normal_lo_to_hi(&self) -> Vec2 {
        self.axis.other().unit()
    }

    /// Unit normal pointing into the region labelled `label`.
    pub fn normal_into(&self, label: i32) -> Vec2 {
        if self.hi == label { self.normal_lo_to_hi() } else { -self.normal_lo_to_hi() }
    }

    /// Metric start / end along `axis`.
    pub fn metric(&self, g: &Grid) -> (f32, f32) {
        (g.line(self.axis, self.s0), g.line(self.axis, self.s1))
    }

    /// Metric coordinate of the line across the run.
    pub fn line_coord(&self, g: &Grid) -> f32 {
        g.line(self.axis.other(), self.line)
    }

    /// Point at metric position `t` along the run.
    pub fn point(&self, g: &Grid, t: f32) -> Vec2 {
        let c = self.line_coord(g);
        match self.axis {
            Axis::X => Vec2::new(t, c),
            Axis::Y => Vec2::new(c, t),
        }
    }

    /// Usable interval for an opening: the run shortened at each end by the
    /// half-thickness of the walls meeting it there plus a clearance.
    pub fn usable(&self, g: &Grid, labels: &[i32], clearance: f32) -> (f32, f32) {
        let (a, b) = self.metric(g);
        let m0 = junction_half(g, labels, self.axis, self.line, self.s0) + clearance;
        let m1 = junction_half(g, labels, self.axis, self.line, self.s1) + clearance;
        (a + m0, b - m1)
    }

    pub fn usable_len(&self, g: &Grid, labels: &[i32], clearance: f32) -> f32 {
        let (a, b) = self.usable(g, labels, clearance);
        b - a
    }
}

/// Label of a cell, `OUTSIDE` when out of bounds.
pub(crate) fn label_at(g: &Grid, labels: &[i32], c: (i32, i32)) -> i32 {
    if g.in_bounds(c) { labels[g.idx(c)] } else { OUTSIDE }
}

/// Thickness of the wall between two cells, if there is one.
fn wall_between(g: &Grid, labels: &[i32], a: (i32, i32), b: (i32, i32)) -> Option<f32> {
    let la = label_at(g, labels, a);
    let lb = label_at(g, labels, b);
    if la == lb {
        None
    } else if la == OUTSIDE || lb == OUTSIDE {
        Some(EXTERIOR_WALL)
    } else {
        Some(INTERIOR_WALL)
    }
}

/// Half-thickness of the thickest wall meeting grid vertex `s` (along `axis`
/// on grid line `line`) perpendicular to a run along `axis`. 0 if none.
pub(crate) fn junction_half(g: &Grid, labels: &[i32], axis: Axis, line: i32, s: i32) -> f32 {
    // Perpendicular edges at the vertex lie on grid line `s` of the other
    // orientation; they separate the cells on either side of that line, one
    // row below and one above `line`.
    let mut t: f32 = 0.0;
    for across in [line - 1, line] {
        let a = axis.cell(s - 1, across);
        let b = axis.cell(s, across);
        if let Some(th) = wall_between(g, labels, a, b) {
            t = t.max(th);
        }
    }
    t * 0.5
}

/// All maximal runs of a labelled grid, horizontal lines first, deterministic order.
pub(crate) fn runs(g: &Grid, labels: &[i32]) -> Vec<Run> {
    let mut out = Vec::new();
    for axis in [Axis::X, Axis::Y] {
        let across = axis.other();
        for line in 0..=g.n(across) {
            let mut cur: Option<Run> = None;
            for s in 0..g.n(axis) {
                let lo = label_at(g, labels, axis.cell(s, line - 1));
                let hi = label_at(g, labels, axis.cell(s, line));
                if lo != hi {
                    match cur.as_mut() {
                        Some(r) if r.lo == lo && r.hi == hi && r.s1 == s => r.s1 = s + 1,
                        _ => {
                            if let Some(r) = cur.take() {
                                out.push(r);
                            }
                            cur = Some(Run { axis, line, s0: s, s1: s + 1, lo, hi });
                        }
                    }
                } else if let Some(r) = cur.take() {
                    out.push(r);
                }
            }
            if let Some(r) = cur.take() {
                out.push(r);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_two_rooms() {
        // 4×2 grid, rooms 0 | 1 split at x = 2.
        let mut g = Grid::new(4, 2, 1.0, 1.0, 0.0, 0.0);
        g.mask.iter_mut().for_each(|m| *m = true);
        let labels: Vec<i32> = (0..8).map(|k| if k % 4 < 2 { 0 } else { 1 }).collect();
        let rs = runs(&g, &labels);
        let shared: Vec<&Run> = rs.iter().filter(|r| r.lo == 0 && r.hi == 1).collect();
        assert_eq!(shared.len(), 1);
        assert_eq!((shared[0].s0, shared[0].s1), (0, 2));
        // Both ends meet the exterior wall: margins are 0.2 + clearance.
        let (a, b) = shared[0].usable(&g, &labels, 0.0);
        assert!((a - 0.2).abs() < 1e-5 && (b - 1.8).abs() < 1e-5);
        // Exterior runs: bottom 2, top 2, left 1, right 1.
        assert_eq!(rs.iter().filter(|r| r.exterior()).count(), 6);
    }
}
