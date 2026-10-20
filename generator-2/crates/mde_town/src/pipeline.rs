//! The generation pipeline: seed sites → Voronoi cells → inner city, river,
//! wall, gates, citadel → streets → wards → lots, fields, trees → [`Town`].

use std::collections::{BTreeSet, VecDeque};
use std::f32::consts::TAU;

use mde_core::{
    Bridge, Building, BuildingKind, Patch, Polygon, Polyline, River, Rng, SCHEMA_VERSION, Street, StreetKind, Town,
    TownParams, Vec2, Wall, WardKind,
};

use crate::farms;
use crate::geom::{Obstacles, Shape, centroid, signed_area};
use crate::lots::{
    BlockEnv, Piece, clear_of, clear_of_specials, finish_lot, fit_rect, lot_ok, lot_style, pick_floors, pick_kind, seg,
    subdivide,
};
use crate::river::{self, RiverGeom, Water};
use crate::streets::{
    self, ALLEY_WIDTH, STREET_WIDTH, claim, components, polylines, shortest_path, street_width, verts_of,
};
use crate::style::{self, Region};
use crate::topo::Topo;
use crate::trees::{self, TreeEnv};
use crate::voronoi::{Pt, voronoi_box};
use crate::wall::{
    CASTLE_THICKNESS, LANE_OFFSET, TOWER_RADIUS, WALL_THICKNESS, castle_gate, choose_citadel, choose_gates, open_gate,
    outer_ring, towers_along,
};
use crate::wards::{self, WardInput};

/// How an edge of the cell graph relates to the settlement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EdgeClass {
    /// Between two inner (non-castle) cells.
    Inside,
    /// Between two outer cells.
    Outside,
    /// Between an inner and an outer cell: the wall line (if walled).
    Ring,
    /// On the citadel boundary.
    Castle,
    /// On the map boundary.
    Boundary,
}

/// Sanitised parameters.
struct Cfg {
    n: usize,
    lot: f32,
    l0: f32,
    walls: bool,
    citadel: bool,
    plaza: bool,
    temple: bool,
    river: bool,
    dense: bool,
}

impl Cfg {
    fn new(p: &TownParams) -> Self {
        let n = (p.patches as usize).clamp(1, 400);
        let lot = if p.lot_size.is_finite() { p.lot_size.clamp(3.0, 30.0) } else { 8.0 };
        Self {
            n,
            lot,
            l0: 64.0 * (lot / 8.0).powf(0.35),
            walls: p.walls,
            citadel: p.citadel,
            plaza: p.plaza,
            temple: p.temple,
            river: p.river,
            dense: p.walls || n >= 25,
        }
    }
}

/// Generate a town. See the crate docs.
pub(crate) fn generate(params: &TownParams) -> Town {
    let cfg = Cfg::new(params);
    let mut root = Rng::new(params.seed ^ 0x6D64_655F_746F_776E);
    let mut r_sites = root.fork(1);
    let mut r_river = root.fork(2);
    let mut r_wall = root.fork(3);
    let mut r_wards = root.fork(4);
    let mut r_streets = root.fork(5);
    let mut r_lots = root.fork(6);
    let mut r_fields = root.fork(7);
    let mut r_trees = root.fork(8);
    let mut r_style = root.fork(9);
    let region = Region::pick(&mut r_style);

    // 1. Sites and Voronoi cells.
    let (raw_sites, half) = spiral(&cfg, &mut r_sites);
    let (mut topo, sites) = relaxed_cells(raw_sites, cfg.n, half);
    if topo.cells.is_empty() {
        // Cannot happen for finite parameters (the centre site always yields
        // a cell); return an empty town rather than panic.
        return empty_town(params);
    }

    // 2. Inner city.
    let (mut inner, centre) = select_inner(&topo, &sites, cfg.n);
    let r_city = city_radius(&topo, &inner, Vec2::ZERO);

    // 3. River.
    let river: Option<RiverGeom> = cfg.river.then(|| river::make_river(&mut r_river, half, r_city, &topo.pts(centre)));
    let near_water = |p: Vec2, extra: f32| river.as_ref().is_some_and(|r| r.dist(p) < r.half() + extra);

    // 4. Citadel, wall ring and gates.
    let castle = if cfg.citadel {
        let dry = |pts: &[Vec2]| river.as_ref().is_none_or(|r| r.poly_clear(pts, r.half() + 8.0));
        choose_citadel(&topo, &inner, centre, &dry, &mut r_wall)
    } else {
        None
    };
    let ring = outer_ring(&topo, &inner);
    let gate_count = if cfg.walls {
        (2 + cfg.n / 12 + r_wall.range_i32(0, 2) as usize).clamp(2, 5)
    } else {
        (2 + cfg.n / 10 + r_wall.range_i32(0, 2) as usize).clamp(2, 4)
    };
    let wet_gate = |p: Vec2| near_water(p, river::VERTEX_CLEARANCE + 8.0);
    let gates = choose_gates(&topo, &ring, &inner, castle, centre, Vec2::ZERO, &wet_gate, gate_count, &mut r_wall);
    for &g in &gates {
        if open_gate(&mut topo, &inner, g, Vec2::ZERO).is_some() {
            inner.push(false);
        }
    }
    let ring = outer_ring(&topo, &inner);
    let ring_set: BTreeSet<usize> = ring.iter().copied().collect();
    let castle_gate_v = castle.and_then(|c| castle_gate(&topo, &inner, c, &ring_set, cfg.walls, Vec2::ZERO));
    let nc = topo.cells.len();
    let centres: Vec<Vec2> = (0..nc).map(|c| centroid(&topo.pts(c))).collect();
    let dense_cell: Vec<bool> =
        (0..nc).map(|c| cfg.walls || (cfg.n >= 25 && inner[c] && centres[c].length() < 0.62 * r_city)).collect();

    // 5. Edge classes and wetness.
    let class = classify_edges(&topo, &inner, castle);
    let wet: Vec<Water> = topo
        .edges
        .iter()
        .map(|e| river.as_ref().map_or(Water::Dry, |r| river::classify(r, topo.verts[e.a], topo.verts[e.b])))
        .collect();

    // 6. Trunk network: arteries gate → centre, roads gate → map edge.
    let mut nb =
        NetBuilder { topo: &topo, class: &class, wet: &wet, walls: cfg.walls, net: vec![None; topo.edges.len()] };
    nb.trunk(&gates, centre, &ring_set, half, &mut r_streets);

    // 7. Wards.
    let dry: Vec<bool> =
        (0..nc).map(|c| river.as_ref().is_none_or(|r| r.poly_clear(&topo.pts(c), r.half() + 4.0))).collect();
    let (ward, cathedral) = wards::assign(
        &WardInput {
            topo: &topo,
            inner: &inner,
            centres: &centres,
            centre_cell: centre,
            castle,
            r_city,
            dense: cfg.dense,
            walls: cfg.walls,
            temple: cfg.temple,
            net: &nb.net,
            gates: &gates,
            dry: &dry,
        },
        &mut r_wards,
    );

    // 8. Secondary streets, alleys, connectivity.
    nb.fill(&inner, &ward, &centres, &dense_cell, centre, castle, castle_gate_v, cfg.dense, &mut r_streets);
    let net = nb.net;
    let on_net = |v: usize| topo.vert_edges[v].iter().any(|&e| net[e].is_some());
    // Gates the network never reached are dropped; the citadel gate is
    // re-picked among castle corners the streets reach.
    let gates: Vec<usize> = gates.into_iter().filter(|&g| on_net(g)).collect();
    let castle_gate_v = match (castle, castle_gate_v) {
        (_, Some(g)) if on_net(g) => Some(g),
        (Some(c), _) => topo.cells[c]
            .iter()
            .copied()
            .filter(|&v| on_net(v) && !(cfg.walls && ring_set.contains(&v)))
            .min_by(|&a, &b| topo.verts[a].length().total_cmp(&topo.verts[b].length()).then(a.cmp(&b))),
        _ => None,
    };

    // 9. Bridges.
    let bridge_kind = params.bridge_kind;
    let mut bridges: Vec<Bridge> = Vec::new();
    for (e, ed) in topo.edges.iter().enumerate() {
        if let (Some(k), Water::Cross { ta, tb }) = (net[e], wet[e]) {
            bridges.push(river::bridge_on_edge(
                topo.verts[ed.a],
                topo.verts[ed.b],
                ta,
                tb,
                street_width(k),
                bridge_kind,
            ));
        }
    }

    // Intramural lane along the inside of the wall, between gates.
    let castle_verts: BTreeSet<usize> = castle.map(|c| topo.cells[c].iter().copied().collect()).unwrap_or_default();
    let (lanes, lane_edges) = if cfg.walls {
        wall_lanes(&topo, &ring, &gates, &castle_verts, &wet, river.as_ref())
    } else {
        (Vec::new(), BTreeSet::new())
    };

    let env = BlockEnv { topo: &topo, net: &net, class: &class, walls: cfg.walls, lane: &lane_edges, inner: &inner };

    // 10. Plaza.
    let plaza: Option<Polygon> =
        if cfg.plaza { env.block(centre, 0.3, 0.6, false).map(|p| Polygon(p.pts)) } else { None };

    // 11. Walls.
    let wall: Option<Wall> = (cfg.walls && ring.len() >= 3).then(|| {
        let pts: Vec<Vec2> = ring.iter().map(|&v| topo.verts[v]).collect();
        let gate_pos: Vec<Vec2> = gates.iter().map(|&g| topo.verts[g]).collect();
        let ok: Vec<bool> = ring
            .iter()
            .map(|&v| {
                !gates.contains(&v) && !castle_verts.contains(&v) && !near_water(topo.verts[v], TOWER_RADIUS + 2.0)
            })
            .collect();
        let castle_pts = castle.map(|c| topo.pts(c)).unwrap_or_default();
        let mut towers = towers_along(&pts, &ok, 14.0, 70.0);
        towers.retain(|t| {
            gate_pos.iter().all(|g| g.distance(*t) > 9.0)
                && !near_water(*t, TOWER_RADIUS + 2.0)
                && (castle_pts.len() < 3 || crate::geom::boundary_dist(&castle_pts, *t) > 6.0)
        });
        let mut shape = Polygon(pts);
        shape.make_ccw();
        Wall { shape, towers, gates: gate_pos, height: r_wall.range_f32(8.0, 10.0), thickness: WALL_THICKNESS }
    });
    let citadel: Option<Wall> = castle.map(|c| {
        let pts = topo.pts(c);
        let gate_pos: Vec<Vec2> = castle_gate_v.map(|g| topo.verts[g]).into_iter().collect();
        let ok: Vec<bool> = topo.cells[c].iter().map(|&v| Some(v) != castle_gate_v).collect();
        let mut towers = towers_along(&pts, &ok, 10.0, 45.0);
        towers.retain(|t| gate_pos.iter().all(|g| g.distance(*t) > 7.0));
        let mut shape = Polygon(pts);
        shape.make_ccw();
        Wall { shape, towers, gates: gate_pos, height: r_wall.range_f32(11.0, 13.0), thickness: CASTLE_THICKNESS }
    });

    // 12. Special buildings: temple and keep, plus the castle's inner street.
    let mut specials: Vec<(usize, Vec<Vec2>, Vec2, BuildingKind)> = Vec::new();
    let water_ok = |pts: &[Vec2], gap: f32| river.as_ref().is_none_or(|r| r.poly_clear(pts, r.half() + gap));
    if let Some(c) = cathedral
        && let Some(block) = env.block(c, 1.0, 1.0, false)
        && let Some((fp, front)) = place_temple(&env, c, &block, cfg.dense, &mut r_lots)
        && water_ok(&fp, 2.0)
    {
        specials.push((c, fp, front, BuildingKind::Temple));
    }
    let mut extra_streets: Vec<Street> = Vec::new();
    if let Some((c, fp, front, _)) = specials.iter().find(|s| s.3 == BuildingKind::Temple)
        && let Some(path) = temple_path(&topo, &net, *c, fp, *front)
    {
        extra_streets.push(Street { kind: StreetKind::Alley, path: Polyline(path), width: ALLEY_WIDTH });
    }
    if let Some(c) = castle
        && let Some(block) = env.block(c, 0.0, 0.0, true)
    {
        let gate_p = castle_gate_v.map(|g| topo.verts[g]);
        let mut keep = place_keep(&block, gate_p, &mut r_lots);
        let mut path = match (&keep, gate_p) {
            (Some((fp, front)), Some(g)) => {
                // Straight in from the gate, or via a point square in front of
                // the keep if the straight line would graze its corner.
                let end = front_mid(fp, *front) + *front * (STREET_WIDTH * 0.5 + 0.6);
                let clear = |a: Vec2, b: Vec2| crate::geom::poly_seg_dist(fp, a, b) >= STREET_WIDTH * 0.5 + 0.4;
                let via = end + *front * 8.0;
                if clear(g, end) {
                    vec![g, end]
                } else if clear(g, via) && crate::lots::inside_by(&block.pts, via, 0.5) {
                    vec![g, via, end]
                } else {
                    keep = None;
                    vec![g, centroid(&block.pts)]
                }
            }
            (None, Some(g)) => vec![g, centroid(&block.pts)],
            _ => Vec::new(),
        };
        path.dedup_by(|a, b| a.distance(*b) < 2.0);
        if path.len() >= 2 {
            extra_streets.push(Street { kind: StreetKind::Street, path: Polyline(path), width: STREET_WIDTH });
        }
        if let Some((fp, front)) = keep
            && water_ok(&fp, 2.0)
        {
            specials.push((c, fp, front, BuildingKind::Keep));
        }
    }

    let mut streets = polylines(&topo, &net);
    streets.extend(lanes);
    streets.extend(extra_streets);

    // 13. Obstacles that lots must keep clear of.
    let mut obs = Obstacles::new(half);
    for s in &streets {
        for w in s.path.0.windows(2) {
            obs.add(seg(w[0], w[1], s.width * 0.5 + 0.35));
        }
    }
    for wl in wall.iter().chain(citadel.iter()) {
        let n = wl.shape.len();
        for i in 0..n {
            obs.add(seg(wl.shape.0[i], wl.shape.0[(i + 1) % n], wl.thickness * 0.5 + 0.6));
        }
        for t in &wl.towers {
            obs.add(Shape::Circle { c: *t, r: TOWER_RADIUS + 0.5 });
        }
        for g in &wl.gates {
            obs.add(Shape::Circle { c: *g, r: 5.0 });
        }
    }
    for b in &bridges {
        obs.add(seg(b.a, b.b, b.width * 0.5 + 0.6));
    }
    if let Some(p) = &plaza {
        obs.add(Shape::Poly { pts: p.0.clone() });
    }

    // 14. Lots and farms, cell by cell from the centre outwards.
    struct Raw {
        cell: usize,
        pts: Vec<Vec2>,
        front: Vec2,
        kind: Option<BuildingKind>,
        faces_market: bool,
    }
    let mut raws: Vec<Raw> = Vec::new();
    let mut fields: Vec<Polygon> = Vec::new();
    let mut blocks: Vec<Option<Vec<Vec2>>> = vec![None; nc];
    let mut order: Vec<usize> = (0..nc).collect();
    order.sort_by(|&a, &b| {
        inner[b].cmp(&inner[a]).then(centres[a].length().total_cmp(&centres[b].length())).then(a.cmp(&b))
    });
    let special_fps: Vec<Vec<Vec2>> = specials.iter().map(|s| s.1.clone()).collect();
    let mut mill_done = false;
    for &c in &order {
        let w = ward[c];
        let dense = dense_cell[c];
        let mut crng = r_lots.fork(c as u64);
        // Specials of this cell first (sequential ids follow cell order).
        for (sc, fp, front, kind) in &specials {
            if *sc == c {
                raws.push(Raw { cell: c, pts: fp.clone(), front: *front, kind: Some(*kind), faces_market: false });
            }
        }
        match w {
            WardKind::Market if plaza.is_some() => continue,
            WardKind::Empty => continue,
            WardKind::Park => {
                blocks[c] = env.block(c, 0.8, 0.8, false).map(|p| p.pts);
                continue;
            }
            WardKind::Farm => {
                let margin = if inner[c] { 1.5 } else { 1.8 };
                let Some(block) = env.block(c, margin, 2.0, false) else { continue };
                // Farmstead along the widest street side.
                let m = topo.cells[c].len();
                let mut best: Option<(i32, f32)> = None;
                for i in 0..m {
                    let sw = env.side_width(c, i);
                    if sw <= 0.0 {
                        continue;
                    }
                    let Some((a, b)) = block.tagged_edge(i as i32) else { continue };
                    let score = a.distance(b) * (1.0 + sw);
                    if best.is_none_or(|bb| score > bb.1) {
                        best = Some((i as i32, score));
                    }
                }
                // Farmsteads only where a road gives access (NPCs must reach the doors).
                let front_tag = best.map(|b| b.0);
                let stead = front_tag.is_some() && crng.chance(0.85);
                let mill = stead && !mill_done;
                let out = farms::farm(&block, front_tag, stead, mill, &mut r_fields);
                for (pts, front, kind) in out.buildings {
                    if let Some(pts) = clear_of(pts, &mut obs)
                        && lot_ok(&pts)
                        && water_ok(&pts, 1.0)
                    {
                        if kind == BuildingKind::Mill {
                            mill_done = true;
                        }
                        raws.push(Raw { cell: c, pts, front, kind: Some(kind), faces_market: false });
                    }
                }
                for f in out.fields {
                    if let Some(pts) = clear_of(f, &mut obs)
                        && pts.len() >= 3
                        && signed_area(&pts) >= 100.0
                        && water_ok(&pts, 2.0)
                    {
                        fields.push(Polygon(pts));
                    }
                }
                blocks[c] = Some(block.pts);
                continue;
            }
            _ => {}
        }
        let Some(style) = lot_style(w, dense, cfg.lot) else { continue };
        let is_castle = w == WardKind::Castle;
        let (fm, bm) = if is_castle {
            (0.0, 0.0)
        } else if dense {
            (0.5, 0.45)
        } else {
            (crng.range_f32(1.5, 3.0), 1.2)
        };
        let Some(block) = env.block(c, fm, bm, is_castle) else { continue };
        blocks[c] = Some(block.pts.clone());
        let side_w = |t: i32| -> f32 {
            if t < 0 {
                return 0.0;
            }
            let i = t as usize;
            if is_castle {
                if env.side_class(c, i) == EdgeClass::Castle { 1.0 } else { 0.0 }
            } else {
                env.side_width(c, i)
            }
        };
        let mut pieces = Vec::new();
        subdivide(block, &|t| side_w(t) > 0.0, &style, &mut crng, &mut pieces, 0);
        for piece in &pieces {
            let Some(lot) = finish_lot(piece, &side_w, &style, is_castle, &mut crng) else { continue };
            if crng.chance(style.gap) {
                continue;
            }
            let Some(pts) = clear_of(lot.pts, &mut obs) else { continue };
            if !lot_ok(&pts) || !water_ok(&pts, 1.0) || !clear_of_specials(&pts, &special_fps, 2.0) {
                continue;
            }
            let faces_market = lot.side >= 0
                && topo.edges[topo.cell_sides[c][lot.side as usize]].other_cell(c) == Some(centre)
                && !is_castle;
            raws.push(Raw { cell: c, pts, front: lot.front, kind: None, faces_market });
        }
    }

    // 15. Buildings with kinds, floors, seeds and appearance.
    let mut r_meta = r_lots.fork(0xB01D);
    let mut buildings: Vec<Building> = Vec::with_capacity(raws.len());
    for raw in raws {
        let id = buildings.len() as u32;
        let mut brng = r_meta.fork(id as u64);
        let seed = brng.next_u64();
        let w = ward[raw.cell];
        let dense = dense_cell[raw.cell];
        let kind = raw.kind.unwrap_or_else(|| pick_kind(w, raw.faces_market, dense, &mut brng));
        let d = centres[raw.cell].length() / r_city.max(1.0);
        let floors = pick_floors(kind, w, dense, d, &mut brng);
        let rustic = if dense { 0.0 } else { 0.6 };
        let appearance = style::appearance(kind, w, region, rustic, &mut brng);
        let mut footprint = Polygon(raw.pts);
        footprint.make_ccw();
        let front = raw.front.try_normalize().unwrap_or(Vec2::Y);
        buildings.push(Building { id, patch: raw.cell as u32, kind, footprint, floors, front, seed, appearance });
    }

    // 16. Trees.
    let mut hard = Obstacles::new(half);
    for s in &streets {
        for w in s.path.0.windows(2) {
            hard.add(seg(w[0], w[1], s.width * 0.5 + 0.6));
        }
    }
    if let Some(r) = &river {
        for w in r.path.windows(2) {
            hard.add(seg(w[0], w[1], r.half() + 0.8));
        }
    }
    for b in &bridges {
        hard.add(seg(b.a, b.b, b.width * 0.5 + 1.0));
    }
    for wl in wall.iter().chain(citadel.iter()) {
        let n = wl.shape.len();
        for i in 0..n {
            hard.add(seg(wl.shape.0[i], wl.shape.0[(i + 1) % n], wl.thickness * 0.5 + 1.5));
        }
        for t in &wl.towers {
            hard.add(Shape::Circle { c: *t, r: TOWER_RADIUS + 1.0 });
        }
    }
    for b in &buildings {
        hard.add(Shape::Poly { pts: b.footprint.0.clone() });
    }
    if let Some(p) = &plaza {
        hard.add(Shape::Poly { pts: p.0.clone() });
    }
    if let Some(c) = castle {
        hard.add(Shape::Poly { pts: topo.pts(c) });
    }
    let mut field_obs = Obstacles::new(half);
    for f in &fields {
        field_obs.add(Shape::Poly { pts: f.0.clone() });
    }
    let cell_pts: Vec<Vec<Vec2>> = (0..nc).map(|c| topo.pts(c)).collect();
    let trees = trees::plant(
        &TreeEnv {
            wards: &ward,
            inner: &inner,
            dense: &dense_cell,
            cells: &cell_pts,
            blocks: &blocks,
            streets: &streets,
            half,
        },
        &mut hard,
        &mut field_obs,
        &mut r_trees,
    );

    // 17. Assemble, renumbering patches inner-first from the centre out.
    let mut new_id = vec![0u32; nc];
    for (i, &c) in order.iter().enumerate() {
        new_id[c] = i as u32;
    }
    let patches: Vec<Patch> = order
        .iter()
        .map(|&c| Patch {
            id: new_id[c],
            shape: Polygon(cell_pts[c].clone()),
            ward: ward[c],
            within_walls: inner[c],
            within_city: inner[c],
        })
        .collect();
    for b in &mut buildings {
        b.patch = new_id[b.patch as usize];
    }
    let center = centres[centre];
    let radius = (0..nc)
        .filter(|&c| inner[c])
        .flat_map(|c| topo.cells[c].iter().map(|&v| topo.verts[v].distance(center)))
        .fold(0.0f32, f32::max);
    Town {
        schema: SCHEMA_VERSION,
        params: params.clone(),
        center,
        radius,
        patches,
        wall,
        citadel,
        plaza,
        streets,
        buildings,
        fields,
        trees,
        river: river.map(|r| River { path: Polyline(r.path), width: r.width }),
        bridges,
    }
}

/// A town with nothing in it (degenerate input safety net).
fn empty_town(params: &TownParams) -> Town {
    Town {
        schema: SCHEMA_VERSION,
        params: params.clone(),
        center: Vec2::ZERO,
        radius: 1.0,
        patches: Vec::new(),
        wall: None,
        citadel: None,
        plaza: None,
        streets: Vec::new(),
        buildings: Vec::new(),
        fields: Vec::new(),
        trees: Vec::new(),
        river: None,
        bridges: Vec::new(),
    }
}

/// Jittered spiral of sites: fine near the centre, coarser in the countryside.
/// Returns the sites (centre first) and the half-size of the square map.
fn spiral(cfg: &Cfg, rng: &mut Rng) -> (Vec<Vec2>, f32) {
    let n = cfg.n;
    let l0 = cfg.l0;
    let spacing = |i: usize| l0 * (0.78 + 0.5 * (i as f32 / n.max(1) as f32).powf(0.8));
    let mut pts = vec![Vec2::new(rng.range_f32(-0.08, 0.08), rng.range_f32(-0.08, 0.08)) * l0];
    let mut r = 0.62 * l0;
    let mut a = rng.range_f32(0.0, TAU);
    let mut r_city = 0.55 * l0;
    let mut l_edge = spacing(n);
    let mut half = if n <= 1 { (r_city * 2.3).max(r_city + 280.0) } else { f32::INFINITY };
    let mut i = 1usize;
    while i < 50_000 {
        let l = if i < n { spacing(i) } else { l_edge * (1.0 + 0.85 * (r - r_city).max(0.0) / r_city.max(1.0)) };
        let jr = rng.range_f32(-0.2, 0.2) * l;
        let ja = rng.range_f32(-0.2, 0.2) * l / r;
        pts.push(Vec2::from_angle(a + ja) * (r + jr));
        if i + 1 == n {
            r_city = r + 0.5 * l;
            l_edge = l;
            half = (r_city * 2.3).max(r_city + 280.0);
        }
        r += l * l / (TAU * r);
        a += l / r;
        i += 1;
        if i >= n && r > half * 1.45 {
            break;
        }
    }
    (pts, half)
}

fn to_pts(v: &[Vec2]) -> Vec<Pt> {
    v.iter().map(|p| Pt::new(p.x as f64, p.y as f64)).collect()
}

/// Voronoi cells with two Lloyd steps on the innermost sites.
fn relaxed_cells(sites: Vec<Vec2>, n: usize, half: f32) -> (Topo, Vec<Vec2>) {
    let mut kept: Vec<Vec2> = Vec::with_capacity(sites.len());
    for p in sites {
        if p.x.abs() < half - 2.0 && p.y.abs() < half - 2.0 && kept.iter().all(|q| q.distance(p) > 2.0) {
            kept.push(p);
        }
    }
    let mut sites = kept;
    let relax = (n + n / 4 + 2).min(sites.len());
    for _ in 0..2 {
        let d = voronoi_box(&to_pts(&sites), half as f64);
        for (site, cell) in sites.iter_mut().zip(&d.cells).take(relax) {
            if cell.len() >= 3 {
                let pts: Vec<Vec2> =
                    cell.iter().map(|&v| Vec2::new(d.vertices[v].x as f32, d.vertices[v].y as f32)).collect();
                let c = centroid(&pts);
                if c.x.abs() < half - 2.0 && c.y.abs() < half - 2.0 {
                    *site = c;
                }
            }
        }
    }
    let d = voronoi_box(&to_pts(&sites), half as f64);
    let verts: Vec<Vec2> = d.vertices.iter().map(|p| Vec2::new(p.x as f32, p.y as f32)).collect();
    let mut cells = Vec::new();
    let mut out_sites = Vec::new();
    for (i, cell) in d.cells.into_iter().enumerate() {
        if cell.len() >= 3 {
            cells.push(cell);
            out_sites.push(sites[i]);
        }
    }
    (Topo::new(verts, cells), out_sites)
}

/// Grow the inner city from the centre cell by site distance, then fill holes.
fn select_inner(topo: &Topo, sites: &[Vec2], n: usize) -> (Vec<bool>, usize) {
    let nc = topo.cells.len();
    let dist = |c: usize| sites[c].length();
    let centre = (0..nc).min_by(|&a, &b| dist(a).total_cmp(&dist(b)).then(a.cmp(&b))).unwrap_or(0);
    let on_map_edge: Vec<bool> =
        (0..nc).map(|c| topo.cell_sides[c].iter().any(|&e| topo.edges[e].other_cell(c).is_none())).collect();
    let mut inner = vec![false; nc];
    let mut frontier: BTreeSet<usize> = BTreeSet::new();
    frontier.insert(centre);
    let mut count = 0;
    while count < n {
        let Some(&c) = frontier.iter().min_by(|&&a, &&b| dist(a).total_cmp(&dist(b)).then(a.cmp(&b))) else { break };
        frontier.remove(&c);
        if on_map_edge[c] && c != centre {
            continue;
        }
        inner[c] = true;
        count += 1;
        for o in topo.neighbours(c) {
            if !inner[o] {
                frontier.insert(o);
            }
        }
    }
    // Holes: outer cells cut off from the map edge become inner; compensate
    // by releasing the farthest edge cells so the count stays `n`.
    for _ in 0..4 {
        let reach = reachable_outside(topo, &inner, &on_map_edge);
        let holes: Vec<usize> = (0..nc).filter(|&c| !inner[c] && !reach[c]).collect();
        if holes.is_empty() {
            break;
        }
        for &h in &holes {
            inner[h] = true;
        }
        for _ in 0..holes.len() {
            let reach = reachable_outside(topo, &inner, &on_map_edge);
            let mut cands: Vec<usize> = (0..nc)
                .filter(|&c| inner[c] && c != centre && topo.neighbours(c).iter().any(|&o| !inner[o] && reach[o]))
                .collect();
            cands.sort_by(|&a, &b| dist(b).total_cmp(&dist(a)).then(a.cmp(&b)));
            for c in cands {
                inner[c] = false;
                if inner_connected(topo, &inner, centre) {
                    break;
                }
                inner[c] = true;
            }
        }
    }
    (inner, centre)
}

fn reachable_outside(topo: &Topo, inner: &[bool], on_edge: &[bool]) -> Vec<bool> {
    let nc = topo.cells.len();
    let mut seen = vec![false; nc];
    let mut q: VecDeque<usize> = (0..nc).filter(|&c| on_edge[c] && !inner[c]).collect();
    for &c in &q {
        seen[c] = true;
    }
    while let Some(c) = q.pop_front() {
        for o in topo.neighbours(c) {
            if !inner[o] && !seen[o] {
                seen[o] = true;
                q.push_back(o);
            }
        }
    }
    seen
}

fn inner_connected(topo: &Topo, inner: &[bool], centre: usize) -> bool {
    let nc = topo.cells.len();
    let mut seen = vec![false; nc];
    let mut stack = vec![centre];
    seen[centre] = true;
    while let Some(c) = stack.pop() {
        for o in topo.neighbours(c) {
            if inner[o] && !seen[o] {
                seen[o] = true;
                stack.push(o);
            }
        }
    }
    (0..nc).all(|c| !inner[c] || seen[c])
}

fn city_radius(topo: &Topo, inner: &[bool], centre: Vec2) -> f32 {
    (0..topo.cells.len())
        .filter(|&c| inner[c])
        .flat_map(|c| topo.cells[c].iter().map(|&v| topo.verts[v].distance(centre)))
        .fold(1.0f32, f32::max)
}

fn classify_edges(topo: &Topo, inner: &[bool], castle: Option<usize>) -> Vec<EdgeClass> {
    topo.edges
        .iter()
        .map(|e| match (e.left, e.right) {
            (Some(l), Some(r)) => {
                if castle.is_some_and(|k| k == l || k == r) {
                    EdgeClass::Castle
                } else if inner[l] && inner[r] {
                    EdgeClass::Inside
                } else if !inner[l] && !inner[r] {
                    EdgeClass::Outside
                } else {
                    EdgeClass::Ring
                }
            }
            _ => EdgeClass::Boundary,
        })
        .collect()
}

fn angle_dist(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(TAU);
    d.min(TAU - d)
}

/// Builds the street network edge by edge.
struct NetBuilder<'a> {
    topo: &'a Topo,
    class: &'a [EdgeClass],
    wet: &'a [Water],
    walls: bool,
    net: Vec<Option<StreetKind>>,
}

impl NetBuilder<'_> {
    fn passable(&self, e: usize) -> bool {
        !matches!(self.class[e], EdgeClass::Boundary | EdgeClass::Castle)
            && !(self.walls && self.class[e] == EdgeClass::Ring)
            && self.wet[e] != Water::Wet
    }

    fn cost(&self, e: usize) -> f32 {
        self.topo.edge_len(e) + if matches!(self.wet[e], Water::Cross { .. }) { 45.0 } else { 0.0 }
    }

    /// Usable by streets of the settlement itself.
    fn town_edge(&self, e: usize) -> bool {
        self.passable(e) && (self.class[e] == EdgeClass::Inside || (!self.walls && self.class[e] == EdgeClass::Ring))
    }

    /// Usable by country roads.
    fn country_edge(&self, e: usize) -> bool {
        self.passable(e) && self.class[e] == EdgeClass::Outside
    }

    fn claim_all(&mut self, path: &[usize], kind: StreetKind) {
        for &e in path {
            claim(&mut self.net, e, kind);
        }
    }

    fn net_vertices(&self, pred: impl Fn(usize) -> bool) -> Vec<usize> {
        let mut out: BTreeSet<usize> = BTreeSet::new();
        for (e, k) in self.net.iter().enumerate() {
            if k.is_some() && pred(e) {
                out.insert(self.topo.edges[e].a);
                out.insert(self.topo.edges[e].b);
            }
        }
        out.into_iter().collect()
    }

    /// Arteries from every gate to the market, roads from every gate to the map edge.
    fn trunk(&mut self, gates: &[usize], centre: usize, ring: &BTreeSet<usize>, half: f32, rng: &mut Rng) {
        let topo = self.topo;
        let centre_v: BTreeSet<usize> = topo.cells[centre].iter().copied().collect();
        for &g in gates {
            let found = shortest_path(topo, &[g], &|v| centre_v.contains(&v), &|e| {
                if !self.town_edge(e) {
                    return None;
                }
                let mut c = self.cost(e);
                if self.net[e] == Some(StreetKind::Artery) {
                    c *= 0.5;
                }
                if !self.walls && self.class[e] == EdgeClass::Ring {
                    c *= 1.5;
                }
                Some(c)
            });
            if let Some((path, _)) = found {
                self.claim_all(&path, StreetKind::Artery);
            }
        }
        let on_box = |v: usize| {
            let p = topo.verts[v];
            p.x.abs() >= half - 0.01 || p.y.abs() >= half - 0.01
        };
        let boundary: Vec<usize> =
            (0..topo.verts.len()).filter(|&v| on_box(v) && !topo.vert_edges[v].is_empty()).collect();
        for &g in gates {
            let gp = topo.verts[g];
            let ang = gp.y.atan2(gp.x) + rng.range_f32(-0.25, 0.25);
            let mut cands = boundary.clone();
            cands.sort_by(|&a, &b| {
                let pa = topo.verts[a];
                let pb = topo.verts[b];
                angle_dist(pa.y.atan2(pa.x), ang).total_cmp(&angle_dist(pb.y.atan2(pb.x), ang)).then(a.cmp(&b))
            });
            let cost = |e: usize| {
                if !self.country_edge(e) {
                    return None;
                }
                let ed = &topo.edges[e];
                let mut c = self.cost(e);
                if self.net[e] == Some(StreetKind::Road) {
                    c *= 0.55;
                }
                if (ring.contains(&ed.a) && ed.a != g) || (ring.contains(&ed.b) && ed.b != g) {
                    c *= 2.5;
                }
                Some(c)
            };
            let mut found = None;
            for &t in cands.iter().take(4) {
                found = shortest_path(topo, &[g], &|v| v == t, &cost);
                if found.is_some() {
                    break;
                }
            }
            if found.is_none() {
                found = shortest_path(topo, &[g], &on_box, &cost);
            }
            if let Some((path, _)) = found {
                self.claim_all(&path, StreetKind::Road);
            }
        }
    }

    /// Market ring, frontage streets for every built inner cell, alleys,
    /// citadel access, then make the network connected.
    #[allow(clippy::too_many_arguments)]
    fn fill(
        &mut self,
        inner: &[bool],
        ward: &[WardKind],
        centres: &[Vec2],
        dense_cell: &[bool],
        centre: usize,
        castle: Option<usize>,
        castle_gate: Option<usize>,
        dense: bool,
        rng: &mut Rng,
    ) {
        let topo = self.topo;
        for i in 0..topo.cell_sides[centre].len() {
            let e = topo.cell_sides[centre][i];
            if self.town_edge(e) {
                claim(&mut self.net, e, StreetKind::Street);
            }
        }
        let mut order: Vec<usize> =
            (0..topo.cells.len()).filter(|&c| inner[c] && Some(c) != castle && ward[c] != WardKind::Park).collect();
        order.sort_by(|&a, &b| centres[a].length().total_cmp(&centres[b].length()).then(a.cmp(&b)));
        for c in order {
            if topo.cell_sides[c].iter().any(|&e| self.net[e].is_some() && self.town_edge(e)) {
                continue;
            }
            let mut sources = self.net_vertices(|e| self.town_edge(e));
            if sources.is_empty() {
                sources = topo.cells[centre].clone();
            }
            let cell_v: BTreeSet<usize> = topo.cells[c].iter().copied().collect();
            let side_at = |v: usize| -> Option<usize> {
                topo.cell_sides[c]
                    .iter()
                    .copied()
                    .filter(|&e| self.town_edge(e) && (topo.edges[e].a == v || topo.edges[e].b == v))
                    .max_by(|&x, &y| topo.edge_len(x).total_cmp(&topo.edge_len(y)))
            };
            let found = shortest_path(topo, &sources, &|v| cell_v.contains(&v) && side_at(v).is_some(), &|e| {
                self.town_edge(e).then(|| self.cost(e))
            });
            if let Some((path, end)) = found {
                let side = side_at(end);
                self.claim_all(&path, StreetKind::Street);
                if let Some(e) = side {
                    claim(&mut self.net, e, StreetKind::Street);
                }
            }
        }
        // Village lanes between properties.
        for e in 0..topo.edges.len() {
            let ed = &topo.edges[e];
            let open = |c: Option<usize>| c.is_some_and(|c| inner[c] && !dense_cell[c] && ward[c] != WardKind::Park);
            if self.net[e].is_none()
                && self.class[e] == EdgeClass::Inside
                && self.passable(e)
                && open(ed.left)
                && open(ed.right)
                && rng.chance(0.6)
            {
                self.net[e] = Some(StreetKind::Street);
            }
        }
        if dense {
            for e in 0..topo.edges.len() {
                let ed = &topo.edges[e];
                if self.net[e].is_none()
                    && self.class[e] == EdgeClass::Inside
                    && self.passable(e)
                    && ed.left.is_some_and(|l| dense_cell[l])
                    && ed.right.is_some_and(|r| dense_cell[r])
                {
                    self.net[e] = Some(StreetKind::Alley);
                }
            }
        }
        if let Some(g) = castle_gate
            && !topo.vert_edges[g].iter().any(|&e| self.net[e].is_some())
        {
            let sources = self.net_vertices(|e| self.town_edge(e));
            let found = shortest_path(topo, &sources, &|v| v == g, &|e| self.town_edge(e).then(|| self.cost(e)));
            if let Some((path, _)) = found {
                self.claim_all(&path, StreetKind::Street);
            }
        }
        // Connectivity: join or drop stray components.
        let centre_v: BTreeSet<usize> = topo.cells[centre].iter().copied().collect();
        for _ in 0..64 {
            let comps = components(topo, &self.net);
            if comps.len() <= 1 {
                break;
            }
            let main =
                comps.iter().position(|comp| verts_of(topo, comp).iter().any(|v| centre_v.contains(v))).unwrap_or(0);
            let main_v = verts_of(topo, &comps[main]);
            let k = if main == 0 { 1 } else { 0 };
            let src: Vec<usize> = verts_of(topo, &comps[k]).into_iter().collect();
            let found =
                shortest_path(topo, &src, &|v| main_v.contains(&v), &|e| self.passable(e).then(|| self.cost(e)));
            match found {
                Some((path, _)) if !path.is_empty() => {
                    for e in path {
                        let kind =
                            if self.class[e] == EdgeClass::Outside { StreetKind::Road } else { StreetKind::Street };
                        claim(&mut self.net, e, kind);
                    }
                }
                _ => {
                    for &e in &comps[k] {
                        self.net[e] = None;
                    }
                }
            }
        }
    }
}

/// Intramural lanes: alleys running just inside the wall from each gate until
/// the next gate (or until the citadel or the river interrupts them). Returns
/// the lanes and the ring edges they serve.
fn wall_lanes(
    topo: &Topo,
    ring: &[usize],
    gates: &[usize],
    castle_verts: &BTreeSet<usize>,
    wet: &[Water],
    river: Option<&RiverGeom>,
) -> (Vec<Street>, BTreeSet<usize>) {
    let m = ring.len();
    let mut lanes = Vec::new();
    let mut covered: BTreeSet<usize> = BTreeSet::new();
    if m < 3 || gates.is_empty() {
        return (lanes, covered);
    }
    let pts: Vec<Vec2> = ring.iter().map(|&v| topo.verts[v]).collect();
    let left = |a: Vec2, b: Vec2| -crate::geom::outward(a, b);
    let off: Vec<Vec2> = (0..m)
        .map(|i| {
            let (a, b, c) = (pts[(i + m - 1) % m], pts[i], pts[(i + 1) % m]);
            let (n1, n2) = (left(a, b), left(b, c));
            match (n1 + n2).try_normalize() {
                Some(bis) => b + bis * (LANE_OFFSET / bis.dot(n1).max(0.35)),
                None => b + n1 * LANE_OFFSET,
            }
        })
        .collect();
    let wet_p = |p: Vec2| river.is_some_and(|r| r.dist(p) < r.half() + 3.0);
    let bad_v = |i: usize| castle_verts.contains(&ring[i]) || wet_p(pts[i]) || wet_p(off[i]);
    let edge_of = |i: usize| {
        topo.vert_edges[ring[i]].iter().copied().find(|&e| topo.edges[e].other(ring[i]) == ring[(i + 1) % m])
    };
    let bad_e = |i: usize| {
        let j = (i + 1) % m;
        edge_of(i).is_none_or(|e| wet[e] != Water::Dry)
            || river.is_some_and(|r| r.seg_dist(off[i], off[j]) < r.half() + 2.5)
    };
    let gate_idx: Vec<usize> = (0..m).filter(|&i| gates.contains(&ring[i])).collect();
    for dir in [1usize, m - 1] {
        for &s in &gate_idx {
            if bad_v(s) && !gates.contains(&ring[s]) {
                continue;
            }
            let mut path = vec![pts[s], off[s]];
            let mut i = s;
            let mut edges = Vec::new();
            for _ in 0..m {
                let j = (i + dir) % m;
                let e_idx = if dir == 1 { i } else { j };
                let Some(e) = edge_of(e_idx) else { break };
                if covered.contains(&e) || bad_e(e_idx) || (bad_v(j) && !gates.contains(&ring[j])) {
                    break;
                }
                edges.push(e);
                // Skip offset points that would make the lane double back.
                let last = path[path.len() - 1];
                if (off[j] - last).dot(pts[j] - pts[i]) > 0.0 {
                    path.push(off[j]);
                }
                if gates.contains(&ring[j]) {
                    path.push(pts[j]);
                    break;
                }
                i = j;
            }
            if !edges.is_empty() {
                covered.extend(edges);
                lanes.push(Street { kind: StreetKind::Alley, path: Polyline(path), width: ALLEY_WIDTH });
            }
        }
    }
    (lanes, covered)
}

/// Footpath from the temple door to the nearest street junction of its cell,
/// so the entrance is reachable on the street network: straight if possible,
/// else with a dog-leg out of the door first.
fn temple_path(topo: &Topo, net: &[Option<StreetKind>], c: usize, fp: &[Vec2], front: Vec2) -> Option<Vec<Vec2>> {
    let door = front_mid(fp, front) + front * (ALLEY_WIDTH * 0.5 + 0.6);
    let cell = topo.pts(c);
    let clear = |a: Vec2, b: Vec2| crate::geom::poly_seg_dist(fp, a, b) >= ALLEY_WIDTH * 0.5 + 0.3;
    let mut targets: Vec<Vec2> = topo.cells[c]
        .iter()
        .copied()
        .filter(|&v| topo.vert_edges[v].iter().any(|&e| net[e].is_some()))
        .map(|v| topo.verts[v])
        .collect();
    targets.sort_by(|a, b| a.distance(door).total_cmp(&b.distance(door)));
    let step = door + front * 5.0;
    for t in targets {
        if door.distance(t) < 1.0 {
            continue;
        }
        if (t - door).dot(front) > 0.0 && clear(door, t) {
            return Some(vec![door, t]);
        }
        if crate::lots::inside_by(&cell, step, 0.5) && clear(door, step) && clear(step, t) {
            return Some(vec![door, step, t]);
        }
    }
    None
}

/// Midpoint of the footprint edge whose outward normal is `front`.
fn front_mid(fp: &[Vec2], front: Vec2) -> Vec2 {
    let n = fp.len();
    (0..n)
        .max_by(|&i, &j| {
            let ni = crate::geom::outward(fp[i], fp[(i + 1) % n]).dot(front);
            let nj = crate::geom::outward(fp[j], fp[(j + 1) % n]).dot(front);
            ni.total_cmp(&nj)
        })
        .map(|i| (fp[i] + fp[(i + 1) % n]) * 0.5)
        .unwrap_or_else(|| centroid(fp))
}

/// Temple: an east–west oriented rectangle fitted into the cathedral block,
/// its west (or street-facing) end as the front.
fn place_temple(env: &BlockEnv, c: usize, block: &Piece, dense: bool, rng: &mut Rng) -> Option<(Vec<Vec2>, Vec2)> {
    let len = if dense { rng.range_f32(30.0, 44.0) } else { rng.range_f32(16.0, 22.0) };
    let wid = len * rng.range_f32(0.38, 0.46);
    let centre = centroid(&block.pts);
    let ew = Vec2::from_angle(rng.range_f32(-0.12, 0.12));
    let long = {
        let n = block.pts.len();
        (0..n)
            .map(|i| block.pts[(i + 1) % n] - block.pts[i])
            .max_by(|a, b| a.length().total_cmp(&b.length()))
            .and_then(|d| d.try_normalize())
            .unwrap_or(Vec2::X)
    };
    let a = fit_rect(&block.pts, centre, ew, len, wid, 2.0, 0.45);
    let b = fit_rect(&block.pts, centre, long, len, wid, 2.0, 0.45);
    let (fp, axis) = match (a, b) {
        (Some(x), Some(y)) => {
            if x.1 * 1.15 >= y.1 {
                (x.0, ew)
            } else {
                (y.0, long)
            }
        }
        (Some(x), None) => (x.0, ew),
        (None, Some(y)) => (y.0, long),
        (None, None) => return None,
    };
    // Face the nearest side carrying a network street (the footpath joins the
    // network there); default west.
    let m = env.topo.cells[c].len();
    let mut target: Option<(Vec2, f32)> = None;
    for i in 0..m {
        if streets::width_on(env.net, env.topo.cell_sides[c][i]) <= 0.0 {
            continue;
        }
        let Some((p, q)) = block.tagged_edge(i as i32) else { continue };
        let mid = (p + q) * 0.5;
        let d = mid.distance(centre);
        if target.is_none_or(|t| d < t.1) {
            target = Some((mid, d));
        }
    }
    let front = match target {
        Some((mid, _)) => {
            if (mid - centre).dot(axis) >= 0.0 {
                axis
            } else {
                -axis
            }
        }
        None => -axis,
    };
    Some((fp, front))
}

/// Keep: a near-square block in the castle courtyard, facing the gate.
fn place_keep(block: &Piece, gate: Option<Vec2>, rng: &mut Rng) -> Option<(Vec<Vec2>, Vec2)> {
    let len = rng.range_f32(18.0, 26.0);
    let wid = len * rng.range_f32(0.75, 1.0);
    let centre = centroid(&block.pts);
    let n = block.pts.len();
    let axis = (0..n)
        .map(|i| block.pts[(i + 1) % n] - block.pts[i])
        .max_by(|a, b| a.length().total_cmp(&b.length()))
        .and_then(|d| d.try_normalize())
        .unwrap_or(Vec2::X);
    let (fp, _) = fit_rect(&block.pts, centre, axis, len, wid, 7.0, 0.4)?;
    let to_gate = gate.map(|g| g - centre).and_then(|d| d.try_normalize()).unwrap_or(-axis);
    let perp = Vec2::new(-axis.y, axis.x);
    let front = [axis, -axis, perp, -perp].into_iter().max_by(|a, b| a.dot(to_gate).total_cmp(&b.dot(to_gate)))?;
    Some((fp, front))
}
