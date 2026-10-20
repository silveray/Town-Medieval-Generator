//! Tree placement by rejection sampling: parks, gardens, road verges,
//! hedgerows and countryside groves.

use mde_core::{Rng, Street, StreetKind, Tree, Vec2, WardKind};

use crate::geom::{Obstacles, Shape, contains, signed_area};

/// What the planter needs to know about the town.
pub(crate) struct TreeEnv<'a> {
    /// Per cell: ward, inner flag, dense flag, and the cell polygon.
    pub wards: &'a [WardKind],
    pub inner: &'a [bool],
    pub dense: &'a [bool],
    pub cells: &'a [Vec<Vec2>],
    /// Buildable block per cell (if any), used for gardens and hedgerows.
    pub blocks: &'a [Option<Vec<Vec2>>],
    pub streets: &'a [Street],
    pub half: f32,
}

fn bounds(pts: &[Vec2]) -> (Vec2, Vec2) {
    let mut lo = Vec2::splat(f32::INFINITY);
    let mut hi = Vec2::splat(f32::NEG_INFINITY);
    for p in pts {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    (lo, hi)
}

struct Planter<'a> {
    hard: &'a mut Obstacles,
    fields: &'a mut Obstacles,
    trees: Vec<Tree>,
    half: f32,
}

impl Planter<'_> {
    fn try_plant(&mut self, p: Vec2, r: f32) -> bool {
        if p.x.abs() > self.half - r || p.y.abs() > self.half - r {
            return false;
        }
        if !self.hard.disc_clear(p, r, 0.3) || !self.fields.disc_clear(p, 0.0, 0.4) {
            return false;
        }
        self.trees.push(Tree { pos: p, radius: r });
        // Neighbouring crowns may touch but trunks keep apart.
        self.hard.add(Shape::Circle { c: p, r: r * 0.55 });
        true
    }
}

fn random_in(poly: &[Vec2], rng: &mut Rng) -> Option<Vec2> {
    let (lo, hi) = bounds(poly);
    for _ in 0..12 {
        let p = Vec2::new(rng.range_f32(lo.x, hi.x), rng.range_f32(lo.y, hi.y));
        if contains(poly, p) {
            return Some(p);
        }
    }
    None
}

/// Plant trees. `hard` holds everything trees must not touch (streets,
/// water, bridges, walls, buildings, plaza); `fields` holds field polygons
/// (a trunk may not stand in a field, but crowns may overhang its edge).
pub(crate) fn plant(env: &TreeEnv, hard: &mut Obstacles, fields: &mut Obstacles, rng: &mut Rng) -> Vec<Tree> {
    let mut pl = Planter { hard, fields, trees: Vec::new(), half: env.half };
    let n = env.cells.len();
    for c in 0..n {
        let cell = &env.cells[c];
        if cell.len() < 3 {
            continue;
        }
        let area = signed_area(cell);
        match env.wards[c] {
            WardKind::Park => {
                let poly = env.blocks[c].as_deref().unwrap_or(cell);
                let tries = (signed_area(poly) / 22.0) as usize + 4;
                for _ in 0..tries {
                    if let Some(p) = random_in(poly, rng) {
                        pl.try_plant(p, rng.range_f32(2.4, 4.6));
                    }
                }
            }
            WardKind::Empty => {
                let groves = (area / 14_000.0) as usize + 1;
                for _ in 0..groves {
                    let Some(centre) = random_in(cell, rng) else { continue };
                    let spread = rng.range_f32(18.0, 55.0);
                    let count = rng.range_i32(10, 34);
                    for _ in 0..count {
                        let off = Vec2::new(rng.normal3() - 0.5, rng.normal3() - 0.5) * (spread * 3.0);
                        let p = centre + off;
                        if contains(cell, p) {
                            pl.try_plant(p, rng.range_f32(2.6, 4.8));
                        }
                    }
                }
                for _ in 0..(area / 3500.0) as usize {
                    if let Some(p) = random_in(cell, rng) {
                        pl.try_plant(p, rng.range_f32(2.2, 4.2));
                    }
                }
            }
            WardKind::Farm if !env.inner[c] || !env.dense[c] => {
                // Hedgerow trees along the farm's boundary.
                let ring = env.blocks[c].as_deref().unwrap_or(cell);
                let m = ring.len();
                for i in 0..m {
                    let a = ring[i];
                    let b = ring[(i + 1) % m];
                    let len = a.distance(b);
                    let mut t = rng.range_f32(2.0, 10.0);
                    while t < len {
                        if rng.chance(0.42) {
                            let p = a + (b - a) * (t / len);
                            pl.try_plant(p, rng.range_f32(2.0, 3.6));
                        }
                        t += rng.range_f32(7.0, 14.0);
                    }
                }
            }
            w => {
                // Gardens and orchards behind houses.
                let density = if !env.dense[c] && env.inner[c] {
                    1.0 / 140.0
                } else {
                    match w {
                        WardKind::Patriciate | WardKind::Administration => 1.0 / 260.0,
                        WardKind::Cathedral => 1.0 / 320.0,
                        WardKind::Gate | WardKind::Slum if !env.inner[c] => 1.0 / 450.0,
                        WardKind::Craftsmen | WardKind::Merchant => 1.0 / 1600.0,
                        _ => 0.0,
                    }
                };
                if density > 0.0
                    && let Some(poly) = env.blocks[c].as_deref()
                {
                    let tries = (signed_area(poly) * density) as usize;
                    for _ in 0..tries {
                        if let Some(p) = random_in(poly, rng) {
                            pl.try_plant(p, rng.range_f32(2.0, 3.6));
                        }
                    }
                }
            }
        }
    }
    // Rows along country roads.
    for s in env.streets {
        if s.kind != StreetKind::Road {
            continue;
        }
        for w in s.path.0.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = a.distance(b);
            if len < 1.0 {
                continue;
            }
            let u = (b - a) / len;
            let nrm = Vec2::new(-u.y, u.x);
            let mut t = rng.range_f32(3.0, 12.0);
            while t < len {
                for side in [1.0f32, -1.0] {
                    if rng.chance(0.5) {
                        let r = rng.range_f32(2.2, 3.8);
                        let p = a + u * t + nrm * side * (s.width * 0.5 + r + rng.range_f32(1.0, 2.5));
                        pl.try_plant(p, r);
                    }
                }
                t += rng.range_f32(9.0, 16.0);
            }
        }
    }
    pl.trees
}
