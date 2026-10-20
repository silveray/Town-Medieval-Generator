//! Fast massing houses for the whole-town view (≤ ~4k triangles each).
//!
//! The footprint is extruded storey by storey as a shell whose outer face lies
//! on the footprint. Windows and the door are shallow recessed niches cut into
//! that face (deep, bare niches for custom parts), dressed with the same
//! procedural frames, sills, shutters and surrounds as full dwellings. Roofs
//! come from the shared roof builder. If a house would exceed the triangle
//! budget it is rebuilt with less decoration.

use mde_core::{
    Appearance, Building, BuildingKind, DoorKind, MaterialSlot as S, MeshData, Model, OpeningKind, Polygon, RoofKind,
    RoofMaterial, Rng, Vec2, Vec3, WindowKind,
};

use crate::dwelling::OpeningPlacement;
use crate::geom::{jitter, mix_seed, poly_facing, rot_y, w3, wdir, yaw_of_x, Mb};
use crate::openings::{self, door_hole, window_hole, LeafSpec, OpeningColors};
use crate::palette::{fixed, mix, scale, srgb_to_linear, Linear, WHITE};
use crate::roof::{self, Detail, GableStyle, Roof, RoofParams, Trim};
use crate::style::StyleProfile;
use crate::walls::{self, face_with_holes, hole_pane, hole_reveals, Framing, Head, Hole, WallFrame};

/// Nominal wall thickness of a lot house (depth of bare niches for custom parts).
const LOT_T: f32 = 0.3;
/// Triangle budget per lot house.
const BUDGET: usize = 4000;

#[derive(Clone, Copy, Debug, PartialEq)]
enum WallMat {
    Plaster,
    Stone,
    Boards,
}

#[derive(Clone, Debug)]
struct Storey {
    y0: f32,
    y1: f32,
    outline: Polygon,
    mat: WallMat,
    framed: bool,
}

#[derive(Clone, Debug)]
struct LotOpening {
    storey: usize,
    edge: usize,
    kind: OpeningKind,
    hole: Hole,
    /// Nominal rectangle (for placements).
    s0: f32,
    s1: f32,
    y0: f32,
    h: f32,
}

struct Plan {
    house_like: bool,
    /// Unit street direction (zero if unknown).
    front: Vec2,
    storeys: Vec<Storey>,
    openings: Vec<LotOpening>,
    roof: RoofParams,
    tower: Option<Polygon>,
    front_edge: usize,
    jetty: bool,
    /// Entrance lantern forced by the profile (`None`: rolled per house).
    lantern: Option<bool>,
}

/// Clean, CCW footprint or `None` if unusable.
fn clean(poly: &Polygon) -> Option<Polygon> {
    let pts: Vec<Vec2> = poly.0.iter().copied().filter(|p| p.is_finite()).collect();
    if pts.len() < 3 {
        return None;
    }
    let mut p = Polygon(pts).simplified(0.02);
    if p.len() < 3 || p.area() < 1.0 {
        return None;
    }
    p.make_ccw();
    let b = p.bounds().size();
    (b.x.min(b.y) > 0.8 && b.x.max(b.y) < 500.0).then_some(p)
}

fn edge_frame(outline: &Polygon, i: usize) -> (WallFrame, f32) {
    let a = outline.at(i as isize);
    let b = outline.at(i as isize + 1);
    let len = a.distance(b);
    let dir = if len > 1e-6 { (b - a) / len } else { Vec2::X };
    (WallFrame { origin: a, dir, out: Vec2::new(dir.y, -dir.x) }, len)
}

fn front_edge(outline: &Polygon, front: Vec2) -> usize {
    let front = if front.is_finite() { front.normalize_or_zero() } else { Vec2::ZERO };
    if front == Vec2::ZERO {
        return outline.longest_edge();
    }
    let mut best = (f32::NEG_INFINITY, 0);
    for i in 0..outline.len() {
        let (f, len) = edge_frame(outline, i);
        // Prefer street-facing, then long edges.
        let score = f.out.dot(front) * 10.0 + len.min(12.0) * 0.1;
        if score > best.0 {
            best = (score, i);
        }
    }
    best.1
}

fn plan(b: &Building, entrance: Option<(Vec2, Vec2)>, fine: bool, profile: &StyleProfile) -> Option<Plan> {
    let fp = clean(&b.footprint)?;
    let app = &b.appearance;
    let mut rng = Rng::new(mix_seed(b.seed, 0x107));
    let floors = b.floors.clamp(1, 8) as usize;
    let kind = b.kind;
    let house_like = matches!(
        kind,
        BuildingKind::House
            | BuildingKind::Shop
            | BuildingKind::Workshop
            | BuildingKind::Tavern
            | BuildingKind::Mansion
            | BuildingKind::Farmhouse
            | BuildingKind::Mill
    );
    let fe = front_edge(&fp, b.front);

    // Storeys.
    let mut storeys = Vec::new();
    let p_stone = match kind {
        BuildingKind::Mansion => 0.7,
        BuildingKind::Farmhouse => 0.45,
        BuildingKind::Mill => 0.6,
        _ => 0.3,
    };
    let mut stone_ground = rng.chance(p_stone);
    let mut frame_upper = rng.chance(0.85);
    let mut frame_ground = !stone_ground && rng.chance(0.4);
    let jetty = house_like && floors >= 2 && rng.chance(0.5);
    // A storybook preset restyles houses only (towers, keeps, temples... keep theirs).
    let story = profile.storybook.filter(|_| house_like);
    if let Some(w) = story.map(|s| s.walls()) {
        // Overridden after the rolls so the random stream is unchanged.
        (stone_ground, frame_ground, frame_upper) = (w.stone_ground, w.frame_ground, w.frame_upper);
    }
    match kind {
        BuildingKind::Temple => {
            let h = (4.5 * floors as f32).max(7.0);
            storeys.push(Storey { y0: -0.3, y1: h, outline: fp.clone(), mat: WallMat::Stone, framed: false });
        }
        BuildingKind::Keep | BuildingKind::Tower => {
            let h = (3.6 * floors as f32).max(8.0);
            storeys.push(Storey { y0: -0.3, y1: h, outline: fp.clone(), mat: WallMat::Stone, framed: false });
        }
        BuildingKind::Barn => {
            storeys.push(Storey { y0: -0.3, y1: 4.2, outline: fp.clone(), mat: WallMat::Boards, framed: false });
        }
        BuildingKind::Barracks => {
            for k in 0..floors {
                let y0 = if k == 0 { -0.3 } else { 3.2 * k as f32 };
                storeys.push(Storey { y0, y1: 3.2 * (k + 1) as f32, outline: fp.clone(), mat: WallMat::Stone, framed: false });
            }
        }
        _ => {
            let g = if matches!(kind, BuildingKind::Shop | BuildingKind::Tavern) { 3.2 } else { 3.0 };
            let mut y = 0.0;
            for k in 0..floors {
                let h = if k == 0 { g } else { 2.8 };
                let mut outline = fp.clone();
                if jetty && k >= 1 {
                    let mut d = vec![0.0; fp.len()];
                    d[fe] = -0.35;
                    if let Some(o) = fp.inset_edges(&d) {
                        outline = o;
                    }
                }
                let stone = k == 0 && stone_ground;
                let framed = !stone && if k == 0 { frame_ground } else { frame_upper };
                storeys.push(Storey {
                    y0: if k == 0 { -0.3 } else { y },
                    y1: y + h,
                    outline,
                    mat: if stone { WallMat::Stone } else { WallMat::Plaster },
                    framed,
                });
                y += h;
            }
        }
    }

    // Door.
    let ground = &storeys[0].outline;
    let (door_edge, door_c, door_w) = {
        let dw = match (kind, app.door_kind) {
            (BuildingKind::Barn, _) => 2.6,
            (BuildingKind::Temple, _) => 1.8,
            (BuildingKind::Keep, _) | (BuildingKind::Barracks, _) => 1.6,
            (_, DoorKind::Double) => 1.6,
            _ => 1.1,
        };
        match entrance.filter(|(a, b)| a.is_finite() && b.is_finite() && a.distance(*b) > 0.3) {
            Some((a, b)) => {
                let mid = (a + b) * 0.5;
                let mut best = (f32::INFINITY, 0);
                for i in 0..ground.len() {
                    let pa = ground.at(i as isize);
                    let pb = ground.at(i as isize + 1);
                    let d = mid.distance(mde_core::poly::closest_point_on_segment(mid, pa, pb));
                    if d < best.0 {
                        best = (d, i);
                    }
                }
                let (f, _) = edge_frame(ground, best.1);
                let sa = (a - f.origin).dot(f.dir);
                let sb = (b - f.origin).dot(f.dir);
                (best.1, (sa + sb) * 0.5, (sa - sb).abs().clamp(0.9, 3.0))
            }
            None => {
                let (_, len) = edge_frame(ground, fe);
                (fe, len * 0.5, dw)
            }
        }
    };

    // Windows.
    let (ww, wh) = match app.window_kind {
        WindowKind::Mullioned => (1.1, 1.3),
        WindowKind::Round => (0.75, 0.75),
        WindowKind::Gothic => (0.75, 1.5),
        WindowKind::Arched => (0.8, 1.4),
        _ => (0.85, 1.2),
    };
    let (ww, wh) = match kind {
        BuildingKind::Temple => (1.2, (storeys[0].y1 * 0.45).clamp(2.0, 4.0)),
        BuildingKind::Keep | BuildingKind::Tower => (0.6, 1.0),
        BuildingKind::Barn => (0.7, 0.7),
        _ => (ww, wh),
    };
    let spacing = match kind {
        BuildingKind::Temple => 3.6,
        BuildingKind::Keep | BuildingKind::Tower => 3.5,
        BuildingKind::Barn => 6.0,
        _ => 3.1,
    };
    let custom_w = app.custom_window.is_some();
    let custom_d = app.custom_door.is_some();
    let front_dir = if b.front.is_finite() { b.front.normalize_or_zero() } else { Vec2::ZERO };
    let mut cands: Vec<(usize, usize, f32)> = Vec::new(); // storey, edge, centre
    let count_on = |len: f32, sp: f32| -> usize {
        let usable = len - 2.2;
        if usable < ww { 0 } else { ((usable - ww) / sp).floor() as usize + 1 }
    };
    let mut sp = spacing;
    for _ in 0..8 {
        cands.clear();
        for (si, st) in storeys.iter().enumerate() {
            for e in 0..st.outline.len() {
                let (f, len) = edge_frame(&st.outline, e);
                // Side walls of town houses are often shared with neighbours:
                // one window at most, and only upstairs.
                let side = house_like && front_dir != Vec2::ZERO && f.out.dot(front_dir).abs() < 0.5;
                let tall_stone = matches!(kind, BuildingKind::Keep | BuildingKind::Tower);
                let n = if side {
                    usize::from(si >= 1 && len >= 4.0)
                } else if tall_stone && st.outline.len() > 4 {
                    // Towers: staggered single windows.
                    usize::from(e % 2 == si % 2)
                } else {
                    count_on(len, sp)
                };
                for k in 0..n {
                    let c = len * 0.5 + (k as f32 - (n - 1) as f32 * 0.5) * sp;
                    cands.push((si, e, c));
                }
            }
        }
        if cands.len() <= 20 {
            break;
        }
        sp *= 1.3;
    }
    // Hard cap (many edges × many storeys): keep street-facing, lower windows first.
    const MAX_WINDOWS: usize = 24;
    if cands.len() > MAX_WINDOWS {
        let facing = |si: usize, e: usize| -> u32 {
            let (f, _) = edge_frame(&storeys[si].outline, e);
            let d = if front_dir == Vec2::ZERO { 0.0 } else { f.out.dot(front_dir) };
            if d > 0.5 {
                0
            } else if d < -0.5 {
                1
            } else {
                2
            }
        };
        cands.sort_by_key(|&(si, e, _)| (facing(si, e), si, e));
        cands.truncate(MAX_WINDOWS);
    }
    let mut openings = Vec::new();
    let door_y = 0.0;
    {
        let st = &storeys[0];
        let (_, len) = edge_frame(&st.outline, door_edge);
        let half = (door_w * 0.5).min(len * 0.5 - 0.25).max(0.3);
        let c = door_c.clamp(half + 0.2, (len - half - 0.2).max(half + 0.2));
        let (s0, s1) = (c - half, c + half);
        let dh = match kind {
            BuildingKind::Barn => 3.2,
            BuildingKind::Temple | BuildingKind::Keep => 2.8,
            _ => 2.2,
        };
        let hole = door_hole(app.door_kind, custom_d, s0, s1, door_y, dh, 2.0, st.y1 - 0.15).coarse();
        openings.push(LotOpening { storey: 0, edge: door_edge, kind: OpeningKind::Entrance, hole, s0, s1, y0: door_y, h: dh });
    }
    for (si, e, c) in cands {
        let st = &storeys[si];
        let (s0, s1) = (c - ww * 0.5, c + ww * 0.5);
        if si == 0 && e == door_edge {
            let d = &openings[0];
            if s0 < d.s1 + 0.5 && s1 > d.s0 - 0.5 {
                continue;
            }
        }
        let floor_y = if si == 0 { 0.0 } else { st.y0 };
        let room = st.y1 - floor_y;
        let (sill, h) = match kind {
            BuildingKind::Temple => (2.2_f32.min(room * 0.3), wh.min(room - 2.6)),
            BuildingKind::Keep | BuildingKind::Tower => (room * 0.55, wh),
            BuildingKind::Barn => (2.4, wh),
            _ => (if si == 0 { 0.95 } else { 0.85 }, wh.min(room - 1.2)),
        };
        if h < 0.4 || sill + h > room - 0.15 {
            continue;
        }
        let y0 = floor_y + sill;
        let hole = window_hole(app.window_kind, custom_w, s0, s1, y0, h).coarse();
        openings.push(LotOpening { storey: si, edge: e, kind: OpeningKind::Window, hole, s0, s1, y0, h });
    }

    // Roof.
    let top = storeys.last().map(|s| (s.outline.clone(), s.y1))?;
    let mut roof_kind = match kind {
        BuildingKind::Keep => RoofKind::Flat,
        BuildingKind::Tower => RoofKind::Tent,
        BuildingKind::Temple => RoofKind::Gable,
        BuildingKind::Barracks => *rng.pick(&[RoofKind::Hip, RoofKind::Gable]),
        BuildingKind::Barn => *rng.pick(&[RoofKind::Gable, RoofKind::Gambrel, RoofKind::HalfHip]),
        BuildingKind::Mansion => *rng.pick(&[RoofKind::Hip, RoofKind::HalfHip, RoofKind::Gable]),
        BuildingKind::Farmhouse => *rng.pick(&[RoofKind::HalfHip, RoofKind::Gable, RoofKind::Hip]),
        _ => {
            let r = rng.weighted(&[0.55, 0.15, 0.15, 0.05, 0.1]);
            [RoofKind::Gable, RoofKind::Hip, RoofKind::HalfHip, RoofKind::Gambrel, RoofKind::Gable][r]
        }
    };
    let mut pitch = match app.roof_material {
        RoofMaterial::Thatch => 50.0,
        RoofMaterial::Slate => 46.0,
        RoofMaterial::Shingles => 44.0,
        RoofMaterial::Tiles => 40.0,
    } + rng.range_f32(-3.0, 4.0)
        + if kind == BuildingKind::Temple { 8.0 } else { 0.0 };
    let mut overhang = if app.roof_material == RoofMaterial::Thatch { 0.5 } else { 0.38 };
    // Storybook roof (a preset, or the default) on house-like buildings.
    let story_roof = house_like && profile.story_roof();
    if story_roof {
        let (p, o) = profile.roof_shape(b.seed);
        (roof_kind, pitch, overhang) = (RoofKind::Gable, p, o);
    }
    let (fr, _) = edge_frame(&top.0, fe.min(top.0.len() - 1));
    let (mut wmin, mut wmax, mut dmin, mut dmax) = (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY);
    for &q in &top.0 .0 {
        wmin = wmin.min(q.dot(fr.dir));
        wmax = wmax.max(q.dot(fr.dir));
        dmin = dmin.min(q.dot(fr.out));
        dmax = dmax.max(q.dot(fr.out));
    }
    let ridge = if wmax - wmin >= (dmax - dmin) * 0.9 { fr.dir } else { fr.out };
    if story_roof && profile.storybook.is_none() {
        let half = if ridge == fr.dir { dmax - dmin } else { wmax - wmin } * 0.5;
        pitch = crate::style::cap_pitch(pitch, top.1, half);
    }
    let timber = srgb_to_linear(app.timber);
    let wall_col = srgb_to_linear(app.wall);
    let top_mat = storeys.last().map_or(WallMat::Plaster, |s| s.mat);
    let gable = match top_mat {
        WallMat::Stone => GableStyle { slot: S::Stone, col: WHITE, timber: None },
        WallMat::Boards => GableStyle { slot: S::Timber, col: scale(timber, 1.1), timber: None },
        WallMat::Plaster => GableStyle { slot: S::Plaster, col: wall_col, timber: Some(timber) },
    };
    let roof = RoofParams {
        kind: roof_kind,
        pitch_deg: pitch,
        overhang,
        ridge,
        base: top.1,
        material: app.roof_material,
        roof: srgb_to_linear(app.roof),
        timber,
        gable,
        parapet: (S::Stone, WHITE, true),
        detail: if fine { Detail::Lot } else { Detail::Low },
        seed: b.seed,
        trim: if story_roof { Trim::Boards } else { Trim::None },
        // Lot houses stay planar with whole courses: cheaper, and their
        // one-segment bargeboards could not follow a curve.
        curved: false,
        tiles: false,
        front: (story_roof && front_dir != Vec2::ZERO).then_some(front_dir),
    };

    // Temple bell tower on the front.
    let tower = (kind == BuildingKind::Temple).then(|| {
        let (f, len) = edge_frame(&fp, fe);
        let size = (len * 0.38).clamp(3.0, 6.5);
        let c = f.plan(len * 0.5, -size * 0.35);
        Polygon::rect(Vec2::ZERO, Vec2::splat(size)).0.iter().map(|&q| c + f.dir * q.x + f.out * q.y).collect::<Vec<_>>()
    });
    let lantern = story.map(|s| s.walls().lantern);
    Some(Plan { house_like, front: front_dir, storeys, openings, roof, tower: tower.map(Polygon), front_edge: fe, jetty, lantern })
}

fn colors(app: &Appearance) -> OpeningColors {
    OpeningColors {
        frame: srgb_to_linear(app.window),
        timber: srgb_to_linear(app.timber),
        stone: WHITE,
        lite: true,
    }
}

/// Decoration level for the budget fallback (0 = everything).
fn build_with(b: &Building, p: &Plan, level: u32) -> Model {
    let mut mb = Mb::new();
    let app = &b.appearance;
    let cols = colors(app);
    let mut rng = Rng::new(mix_seed(b.seed, 0x10_7B));
    let wall_col = srgb_to_linear(app.wall);
    let timber = cols.timber;
    let custom_w = app.custom_window.is_some();
    let custom_d = app.custom_door.is_some();
    let n_storeys = p.storeys.len();

    for (si, st) in p.storeys.iter().enumerate() {
        let outline = &st.outline;
        let n = outline.len();
        let (slot, col) = match st.mat {
            WallMat::Plaster => (S::Plaster, jitter(wall_col, &mut rng, 0.02)),
            WallMat::Stone => (S::Stone, jitter(WHITE, &mut rng, 0.04)),
            WallMat::Boards => (S::Timber, scale(timber, 1.1)),
        };
        let stone = st.mat == WallMat::Stone;
        for e in 0..n {
            let (f, len) = edge_frame(outline, e);
            if len < 0.05 {
                continue;
            }
            // Timber framing on street / yard faces (side walls are often shared).
            let shared_side = p.house_like && p.front != Vec2::ZERO && f.out.dot(p.front).abs() < 0.5;
            let framed_here = st.framed && (level < 2 || (level == 2 && e == p.front_edge)) && !shared_side;
            let mut holes: Vec<(usize, Hole)> =
                p.openings.iter().enumerate().filter(|(_, o)| o.storey == si && o.edge == e).map(|(i, o)| (i, o.hole)).collect();
            holes.sort_by(|a, b| a.1.s0.total_cmp(&b.1.s0));
            let hs: Vec<Hole> = holes.iter().map(|x| x.1).collect();
            let wall_slot = if st.mat == WallMat::Boards && level < 2 { S::Timber } else { slot };
            let first = mb.m(wall_slot).positions.len();
            if st.mat == WallMat::Boards && level < 2 {
                boards(mb.m(S::Timber), &f, len, st.y0, st.y1, &hs, col, &mut rng);
            } else {
                face_with_holes(mb.m(slot), &f, 0.0, 0.0, len, st.y0, st.y1, &hs, true, col);
            }
            if si == 0 {
                walls::ground_grime(mb.m(wall_slot), first, 0.0);
            }
            for &(oi, h) in &holes {
                let o = &p.openings[oi];
                let custom = match o.kind {
                    OpeningKind::Window => custom_w,
                    _ => custom_d,
                };
                let depth = if custom {
                    LOT_T
                } else if o.kind == OpeningKind::Window {
                    0.14
                } else {
                    0.2
                };
                hole_reveals(mb.m(slot), &f, &h, -depth, 0.0, col);
                if custom {
                    hole_pane(mb.m(S::Plaster), &f, &h, -depth, true, fixed::void());
                    continue;
                }
                match o.kind {
                    OpeningKind::Window => {
                        let wg = -depth + 0.045;
                        if level < 2 {
                            openings::window_insert(&mut mb, &f, &h, app.window_kind, wg, &cols, &mut rng);
                            let boxes = app.flower_boxes && level == 0 && (si == 0 || e == p.front_edge);
                            let shutters = app.shutters && level == 0;
                            openings::window_exterior(&mut mb, &f, &h, &hs, 0.0, wg, stone, shutters, boxes, &cols, &mut rng);
                        } else {
                            hole_pane(mb.m(S::Glass), &f, &h, wg, true, WHITE);
                            if level < 3 {
                                let c = jitter(cols.frame, &mut rng, 0.05);
                                let skip = crate::geom::faces::NZ | crate::geom::faces::NX | crate::geom::faces::PX;
                                f.bar_skip(mb.m(S::WindowFrame), (h.sc(), h.y0), (h.sc(), h.top()), 0.05, wg, wg + 0.03, c, skip);
                            }
                        }
                    }
                    _ => {
                        hole_pane(mb.m(S::Plaster), &f, &h, -depth - 0.01, true, fixed::void());
                        let rolled = level == 0 && rng.chance(0.6);
                        let lantern = p.lantern.map_or(rolled, |l| l && level == 0);
                        openings::door_surround(&mut mb, &f, &h, 0.0, 0.0, stone, framed_here, lantern, &cols, &mut rng);
                        lot_door(&mut mb, &f, &h, app, depth, b.seed, level);
                    }
                }
            }
            // Dressing.
            let convex_end = |at: usize| -> bool {
                let a = outline.at(at as isize - 1);
                let v = outline.at(at as isize);
                let c = outline.at(at as isize + 1);
                mde_core::poly::cross(v - a, c - v) > 1e-4
            };
            let corner = [convex_end(e), convex_end((e + 1) % n)];
            if framed_here {
                let rail = hs.iter().filter(|h| h.y0 > st.y0 + 0.5).map(|h| h.y0 - st.y0.max(0.0)).fold(0.9f32, f32::min);
                let fr = Framing {
                    f: &f,
                    w_face: 0.0,
                    s0: 0.0,
                    s1: len,
                    y0: st.y0.max(0.0),
                    y1: st.y1,
                    holes: &hs,
                    ext: [0.0, if corner[1] { 0.05 } else { 0.0 }],
                    corner,
                    trim: [0.0, if corner[1] { 0.0 } else { 0.07 }],
                    col: timber,
                    rail: rail.max(0.6),
                    sill_beam: true,
                    lite: true,
                };
                walls::half_timber(mb.m(S::Timber), &fr, &mut rng);
            } else if level < 2 && p.house_like && si == 0 && !st.framed && !shared_side {
                let dressed = mix(WHITE, fixed::dressed_stone(), 0.9);
                for (k, &cv) in corner.iter().enumerate() {
                    if cv {
                        let s = if k == 0 { 0.0 } else { len };
                        let ext = if k == 1 { 0.03 } else { 0.0 };
                        walls::quoins(mb.m(S::Stone), &f, 0.0, s, k == 1, 0.0, st.y1.min(4.5), ext, k == 1, dressed, &mut rng, true);
                    }
                }
            }
            if si == 0 && !st.framed {
                let mut runs = vec![(0.0, len + if corner[1] { 0.05 } else { -0.06 })];
                for h in hs.iter().filter(|h| h.y0 <= 0.05) {
                    runs = walls::cut_runs(&runs, h.s0 - 0.02, h.s1 + 0.02);
                }
                for (a, bb) in runs {
                    let c = jitter(scale(WHITE, 0.8), &mut rng, 0.04);
                    f.block(mb.m(S::Stone), a, bb, -0.3, 0.38, 0.0, 0.05, c, crate::geom::faces::NZ);
                }
            }
        }
        // Jetty: underside of an overhanging storey, beam and brackets.
        if si > 0 && p.jetty {
            let below = &p.storeys[si - 1].outline;
            if below != outline {
                let pts: Vec<Vec3> = outline.0.iter().map(|&q| w3(q, st.y0)).collect();
                poly_facing(mb.m(S::Timber), &pts, -Vec3::Y, scale(timber, 0.8));
                let (f, len) = edge_frame(below, p.front_edge);
                let (fu, _) = edge_frame(outline, p.front_edge);
                let off = (fu.origin - f.origin).dot(f.out);
                let m = mb.m(S::Timber);
                let c = jitter(timber, &mut rng, 0.05);
                f.block(m, 0.0, len, st.y0 - 0.16, st.y0 + 0.02, off - 0.18, off + 0.02, scale(c, 0.9), 0);
                if level < 2 {
                    let count = ((len / 1.6).floor() as usize).max(2);
                    for k in 0..count {
                        let s = len * (k as f32 + 0.5) / count as f32;
                        f.bar(m, (s, st.y0 - 0.6), (s, st.y0 - 0.15), 0.12, 0.0, off.max(0.05) - 0.02, c);
                    }
                }
            }
        }
        if si + 1 == n_storeys {
            // Ceiling cap closing the shell under the roof.
            let pts: Vec<Vec3> = outline.0.iter().map(|&q| w3(q, st.y1)).collect();
            poly_facing(mb.m(S::Timber), &pts, Vec3::Y, scale(timber, 0.6));
        }
    }

    // Tower (temples).
    if let Some(t) = &p.tower {
        let h = p.storeys.first().map_or(8.0, |s| s.y1) * 1.75;
        let m = mb.m(S::Stone);
        crate::geom::prism(m, &t.0, -0.3, h, jitter(WHITE, &mut rng, 0.03), false, false);
        for e in 0..t.len() {
            let (f, len) = edge_frame(t, e);
            let hole = Hole { s0: len * 0.5 - 0.45, s1: len * 0.5 + 0.45, y0: h - 3.2, y1: h - 1.6, head: Head::Pointed, rise: 0.7, coarse: true };
            f.block(mb.m(S::Stone), hole.s0 - 0.1, hole.s1 + 0.1, hole.y0 - 0.12, hole.y0, -0.05, 0.08, scale(WHITE, 1.05), 0);
            hole_pane(mb.m(S::Plaster), &f, &hole, 0.002, true, fixed::void());
            if level < 2 {
                openings::arch_ring(mb.m(S::Stone), &f, &hole, 0.18, 0.0, 0.04, mix(WHITE, fixed::dressed_stone(), 0.8), scale(WHITE, 0.92));
            }
        }
        let tp = RoofParams { kind: RoofKind::Tent, pitch_deg: 62.0, overhang: 0.3, base: h, detail: Detail::Low, ..p.roof.clone() };
        if let Some(r) = Roof::plan(t, &tp) {
            r.build(&mut mb, &tp);
        }
    }

    // Roof and chimney.
    let mut roof_params = p.roof.clone();
    if level >= 2 {
        roof_params.detail = Detail::Low;
    }
    if level >= 2 {
        roof_params.gable.timber = None;
        roof_params.trim = Trim::None;
    }
    if let Some(top) = p.storeys.last()
        && let Some(r) = Roof::plan(&top.outline, &roof_params)
    {
        r.build(&mut mb, &roof_params);
        let mut crng = Rng::new(mix_seed(b.seed, 0xC41));
        let wants = matches!(
            b.kind,
            BuildingKind::House | BuildingKind::Shop | BuildingKind::Tavern | BuildingKind::Mansion | BuildingKind::Farmhouse | BuildingKind::Workshop
        ) && crng.chance(0.75);
        if wants
            && level < 3
            && roof_params.kind != RoofKind::Flat
            && let Some((a, bb)) = r.ridges().first().copied()
        {
            let t = if crng.chance(0.5) { 0.22 } else { 0.78 };
            let pos = a + (bb - a) * t;
            roof::chimney(&mut mb, &r, pos, (bb - a).normalize_or(Vec2::X), WHITE, crng.chance(0.5), &mut crng);
        }
    }
    mb.into_model(format!("building_{}", b.id))
}

/// Vertical boarding for barns: alternating plank strips around the holes.
#[allow(clippy::too_many_arguments)]
fn boards(m: &mut MeshData, f: &WallFrame, len: f32, y0: f32, y1: f32, holes: &[Hole], col: Linear, rng: &mut Rng) {
    let n = ((len / 0.9).ceil() as usize).max(1);
    let mut tmp = MeshData::default();
    face_with_holes(&mut tmp, f, 0.0, 0.0, len, y0, y1, holes, true, col);
    // Re-colour faces by their position along the wall to read as boards.
    for (i, pos) in tmp.positions.iter().enumerate() {
        let p = Vec3::from_array(*pos);
        let s = (mde_core::world_to_plan(p) - f.origin).dot(f.dir);
        let k = ((s / len.max(1e-3)) * n as f32).floor() as i64;
        let tone = if k % 2 == 0 { 1.0 } else { 0.9 };
        tmp.colors[i] = scale(col, tone);
    }
    let _ = rng;
    m.append(&tmp);
    // Battens hide the colour seams.
    for k in 1..n {
        let s = len * k as f32 / n as f32;
        if holes.iter().any(|h| s > h.s0 - 0.05 && s < h.s1 + 0.05) {
            continue;
        }
        f.block(m, s - 0.03, s + 0.03, y0, y1, 0.0, 0.025, scale(col, 0.8), crate::geom::faces::NZ);
    }
}

/// Closed door leaf at the back of a door niche.
fn lot_door(mb: &mut Mb, f: &WallFrame, h: &Hole, app: &Appearance, depth: f32, seed: u64, level: u32) {
    let double = app.door_kind == DoorKind::Double || h.width() > 1.7;
    let spans: Vec<(f32, f32)> = if double { vec![(h.s0, h.sc()), (h.s1, h.sc())] } else { vec![(h.s0, h.s1)] };
    for (k, &(sh, sl)) in spans.iter().enumerate() {
        let along = if sl > sh { f.dir } else { -f.dir };
        let width = (sl - sh).abs() - 0.008;
        let top = match h.head {
            Head::Flat => None,
            _ => Some(
                h.profile()
                    .iter()
                    .filter(|q| q.0 >= sh.min(sl) - 1e-4 && q.0 <= sh.max(sl) + 1e-4)
                    .map(|q| ((q.0 - sh).abs(), q.2 - h.y0 - 0.01))
                    .collect::<Vec<_>>(),
            ),
        };
        let height = h.y1 - h.y0 - 0.01;
        let yaw = yaw_of_x(wdir(along));
        let front_pos_z = rot_y(Vec3::Z, yaw).dot(wdir(f.out)) > 0.0;
        let kind = if level >= 2 || (double && app.door_kind == DoorKind::Double) { DoorKind::Plank } else { app.door_kind };
        let spec = LeafSpec {
            kind,
            width,
            height: top.as_ref().map_or(height, |t| t.iter().map(|q| q.1).fold(height, f32::max)),
            top,
            thickness: 0.06,
            door: srgb_to_linear(app.door),
            front_pos_z,
            seed: mix_seed(seed, k as u64 + 11),
            lite: true,
        };
        let model = openings::door_leaf_model(&spec);
        let hinge = f.p(sh, h.y0, -depth + 0.04);
        mb.append_transformed(&model, |q| hinge + rot_y(q, yaw), |n| rot_y(n, yaw));
    }
}

/// Fast massing house for the whole-town view (≤ ~4k triangles): the
/// footprint extruded by `b.floors` storeys, roof by building kind and
/// [`mde_core::RoofMaterial`], windows and door from `b.appearance`.
/// `entrance` is a plan-space segment on the footprint boundary; without it the
/// door is centred on the edge facing `b.front`. Degenerate footprints give an
/// empty model.
pub fn build_lot_house(b: &Building, entrance: Option<(Vec2, Vec2)>) -> Model {
    build_lot_house_with(b, entrance, &StyleProfile::default())
}

/// [`build_lot_house`] with a [`StyleProfile`]: a storybook preset gives house-like
/// buildings a gable roof at the preset pitch and overhang and its wall
/// treatment. Pair it with [`crate::Storybook::apply_to_building`] for the colours.
pub fn build_lot_house_with(b: &Building, entrance: Option<(Vec2, Vec2)>, profile: &StyleProfile) -> Model {
    let Some(p) = plan(b, entrance, true, profile) else { return Model::new(format!("building_{}", b.id)) };
    let mut last = Model::new(format!("building_{}", b.id));
    for level in 0..5 {
        let m = build_with(b, &p, level);
        if m.triangle_count() <= BUDGET {
            return m;
        }
        last = m;
    }
    last
}

/// Placements of the lot house's windows and entrance, in the order the
/// builder cuts them (entrance first), on the centre plane of a nominal
/// [`LOT_T`]-thick wall.
pub fn lot_house_openings(b: &Building, entrance: Option<(Vec2, Vec2)>) -> Vec<OpeningPlacement> {
    lot_house_openings_with(b, entrance, &StyleProfile::default())
}

/// [`lot_house_openings`] for a house built with [`build_lot_house_with`].
pub fn lot_house_openings_with(b: &Building, entrance: Option<(Vec2, Vec2)>, profile: &StyleProfile) -> Vec<OpeningPlacement> {
    let Some(p) = plan(b, entrance, true, profile) else { return Vec::new() };
    p.openings
        .iter()
        .enumerate()
        .map(|(i, o)| {
            let st = &p.storeys[o.storey];
            let (f, _) = edge_frame(&st.outline, o.edge);
            let out3 = wdir(f.out);
            let (h, cy) = match o.kind {
                OpeningKind::Window => (o.h, o.y0 + o.h * 0.5),
                _ => {
                    let h = o.hole.top() - o.hole.y0;
                    (h, o.hole.y0 + h * 0.5)
                }
            };
            OpeningPlacement {
                kind: o.kind,
                level: o.storey as i32,
                opening: i,
                center: f.p((o.s0 + o.s1) * 0.5, cy, -LOT_T * 0.5),
                yaw: out3.x.atan2(out3.z),
                width: o.s1 - o.s0,
                height: h,
                wall_thickness: LOT_T,
            }
        })
        .collect()
}

/// Lot house at a fixed decoration level (0 = full), ignoring the budget.
/// For diagnostics and tests.
#[doc(hidden)]
pub fn lot_house_at_level(b: &Building, entrance: Option<(Vec2, Vec2)>, level: u32) -> Model {
    lot_house_at_level_with(b, entrance, level, &StyleProfile::default())
}

/// [`lot_house_at_level`] with a [`StyleProfile`].
#[doc(hidden)]
pub fn lot_house_at_level_with(b: &Building, entrance: Option<(Vec2, Vec2)>, level: u32, profile: &StyleProfile) -> Model {
    match plan(b, entrance, true, profile) {
        Some(p) => build_with(b, &p, level),
        None => Model::new(format!("building_{}", b.id)),
    }
}
