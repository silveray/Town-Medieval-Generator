//! Dwelling → mesh: walls with real openings, half-timbering, windows and
//! doors, floor slabs, stairs, furniture and the roof.

use mde_core::{
    Appearance, DoorKind, Dwelling, DwellingStyle, Floor, MaterialSlot as S, MeshData, Model, OpeningKind, Polygon, PropKind,
    RoomKind, Rng, Vec2, Vec3,
};

use crate::geom::{jitter, mix_seed, rot_y, w3, wdir, yaw_of_x, Mb};
use crate::interior;
use crate::openings::{self, door_hole, window_hole, LeafSpec, OpeningColors};
use crate::palette::{fixed, mix, scale, srgb_to_linear, WHITE};
use crate::roof::{self, Detail, GableStyle, Roof, RoofParams, Trim};
use crate::style::StyleProfile;
use crate::walls::{self, wall_joins, wall_thickness, Framing, Head, Hole, Solid, WallFrame};

/// What [`build_dwelling`] includes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DwellingMeshOptions {
    /// `Some(l)`: only floors with `level <= l` and no roof (dollhouse
    /// cut-away); the top shown floor's walls stop at its ceiling.
    pub max_level: Option<i32>,
    /// Build the roof (ignored when `max_level` is set).
    pub roof: bool,
    /// Interior walls, interior doors, upper floor slabs, ceilings and stairs.
    pub interior: bool,
    /// Furniture.
    pub props: bool,
    /// Door leaves in their closed pose. Set to `false` when the viewer spawns
    /// animated leaves from [`door_leaves`].
    pub door_leaves: bool,
    /// Look that overrides the per-house style rolls (wall treatment, lantern,
    /// chimney). Pair a storybook profile with
    /// [`crate::Storybook::apply_to_dwelling`] on the data.
    pub profile: StyleProfile,
}

impl Default for DwellingMeshOptions {
    fn default() -> Self {
        Self { max_level: None, roof: true, interior: true, props: true, door_leaves: true, profile: StyleProfile::default() }
    }
}

/// One door leaf, ready to be spawned and animated by the viewer.
#[derive(Clone, Debug, PartialEq)]
pub struct DoorLeaf {
    /// Local space: hinge axis through the origin, closed pose spanning `+X`
    /// by `width`, `Y` up from the floor, centred on `z = 0`.
    pub model: Model,
    /// World position of the bottom of the hinge axis (on the wall centreline).
    pub hinge: Vec3,
    /// Closed-pose rotation about `Y` in radians (right-handed: local `+X`
    /// maps to `(cos yaw, 0, -sin yaw)`).
    pub yaw: f32,
    pub width: f32,
    pub height: f32,
    pub level: i32,
    /// Index into `Floor::openings` of the floor with this `level`.
    pub opening: usize,
    /// `+1` / `-1`: sign of the yaw change that swings the leaf inwards (into
    /// the house for entrances, into the private room for interior doors).
    pub open_dir: f32,
}

/// Where an opening sits in the world, so custom parts can be dropped in.
#[derive(Clone, Debug, PartialEq)]
pub struct OpeningPlacement {
    pub kind: OpeningKind,
    pub level: i32,
    /// Index into `Floor::openings` (dwellings) or the opening's index in
    /// [`crate::lot_house_openings`]'s own order (lot houses).
    pub opening: usize,
    /// World centre of the opening rectangle on the wall's centre plane.
    pub center: Vec3,
    /// Rotation about `Y` that turns a part modelled facing `+Z` to face
    /// outwards (`+Z` maps to `(sin yaw, 0, cos yaw)`). Interior openings face
    /// the left side of their segment `a → b`.
    pub yaw: f32,
    pub width: f32,
    pub height: f32,
    pub wall_thickness: f32,
}

// ---------------------------------------------------------------------------
// Shared analysis
// ---------------------------------------------------------------------------

/// Per-house style decisions derived from the seed and style.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HouseStyle {
    pub stone_ground: bool,
    pub all_stone: bool,
    pub frame_ground: bool,
    pub frame_upper: bool,
    pub lantern: bool,
    pub gable_timber: bool,
    pub brick_chimney: bool,
}

impl HouseStyle {
    fn new(style: DwellingStyle, seed: u64) -> Self {
        let mut r = Rng::new(mix_seed(seed, 0x51_7E));
        let (p_stone, p_frame_ground, p_all_stone) = match style {
            DwellingStyle::Manor => (0.85, 0.2, 0.4),
            DwellingStyle::Farmhouse => (0.5, 0.3, 0.15),
            DwellingStyle::Tavern => (0.45, 0.6, 0.0),
            DwellingStyle::Townhouse => (0.35, 0.6, 0.0),
            DwellingStyle::Shop => (0.3, 0.6, 0.0),
            DwellingStyle::Cottage => (0.35, 0.55, 0.1),
        };
        let stone_ground = r.chance(p_stone);
        let all_stone = stone_ground && r.chance(p_all_stone);
        let frame_ground = !stone_ground && r.chance(p_frame_ground);
        let frame_upper = !all_stone && r.chance(0.9);
        // Formerly a chimney without a hearth; still drawn so later rolls stay put.
        let _ = r.chance(0.75);
        HouseStyle {
            stone_ground,
            all_stone,
            frame_ground,
            frame_upper,
            lantern: r.chance(0.65),
            gable_timber: r.chance(0.8),
            brick_chimney: r.chance(0.5),
        }
    }

    /// Force what the profile's preset fixes: wall treatment, lantern and
    /// stone chimneys.
    fn with_profile(mut self, profile: &StyleProfile) -> Self {
        if let Some(w) = profile.storybook.map(|s| s.walls()) {
            self.stone_ground = w.stone_ground;
            self.all_stone = false;
            self.frame_ground = w.frame_ground;
            self.frame_upper = w.frame_upper;
            self.lantern = w.lantern;
            self.gable_timber = true;
            self.brick_chimney = false;
        }
        self
    }

    fn stone(&self, level: i32) -> bool {
        level < 0 || (level == 0 && self.stone_ground) || self.all_stone
    }

    fn framed(&self, level: i32) -> bool {
        if self.stone(level) {
            return false;
        }
        if level == 0 { self.frame_ground } else { self.frame_upper }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WallA {
    pub seg: usize,
    pub f: WallFrame,
    pub t: f32,
    pub len: f32,
    pub exterior: bool,
    pub s_out: [f32; 2],
    pub s_in: [f32; 2],
    pub caps: [bool; 2],
    pub mitre: [Option<usize>; 2],
}

impl WallA {
    /// Usable face range for openings.
    fn usable(&self) -> (f32, f32) {
        (self.s_out[0].max(self.s_in[0]) + 0.02, self.s_out[1].min(self.s_in[1]) - 0.02)
    }

    /// Convex building corner at end `e` (outer face runs past the centreline end).
    fn convex(&self, e: usize) -> bool {
        self.exterior && if e == 0 { self.s_out[0] < -0.01 } else { self.s_out[1] > self.len + 0.01 }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct OpeningA {
    pub index: usize,
    pub wall: usize,
    pub hole: Hole,
    pub kind: OpeningKind,
    /// Plan direction into which a door swings.
    pub inward: Vec2,
}

pub(crate) struct FloorA<'a> {
    pub floor: &'a Floor,
    pub y0: f32,
    pub ceil: f32,
    pub top_ext: f32,
    pub walls: Vec<WallA>,
    pub openings: Vec<OpeningA>,
}

fn is_hub(k: RoomKind) -> bool {
    matches!(k, RoomKind::Hall | RoomKind::Corridor | RoomKind::Stairwell | RoomKind::Taproom | RoomKind::Living | RoomKind::Shop)
}

/// Default storey slab thickness when it cannot be inferred.
const SLAB: f32 = 0.25;

pub(crate) fn analyse<'a>(d: &'a Dwelling) -> Vec<FloorA<'a>> {
    let mut floors: Vec<&Floor> = d.floors.iter().filter(|f| f.elevation.is_finite() && f.height.is_finite()).collect();
    floors.sort_by_key(|f| f.level);
    let app = &d.appearance;
    let mut out = Vec::with_capacity(floors.len());
    for (fi, floor) in floors.iter().enumerate() {
        let y0 = floor.elevation;
        let ceil = y0 + floor.height.clamp(1.8, 12.0);
        let top_ext = floors
            .get(fi + 1)
            .map(|n| n.elevation)
            .filter(|&e| e > ceil + 0.02 && e < ceil + 1.5)
            .unwrap_or(ceil + SLAB);
        let joins = wall_joins(&floor.walls);
        let outline = &floor.outline;
        let centroid = {
            let mids: Vec<Vec2> =
                floor.walls.iter().filter(|w| w.a.is_finite() && w.b.is_finite()).map(|w| (w.a + w.b) * 0.5).collect();
            if mids.is_empty() { Vec2::ZERO } else { mids.iter().copied().sum::<Vec2>() / mids.len() as f32 }
        };
        let mut walls = Vec::new();
        let mut wall_of_seg = vec![None; floor.walls.len()];
        for (i, w) in floor.walls.iter().enumerate() {
            if !w.a.is_finite() || !w.b.is_finite() {
                continue;
            }
            let len = w.a.distance(w.b);
            if len < 0.05 {
                continue;
            }
            let t = wall_thickness(w);
            let d = (w.b - w.a) / len;
            let n = Vec2::new(-d.y, d.x);
            let mid = (w.a + w.b) * 0.5;
            let flip = if w.exterior {
                let probe = mid + n * (t * 0.5 + 0.05);
                if outline.len() >= 3 { outline.contains(probe) } else { n.dot(mid - centroid) < 0.0 }
            } else {
                false
            };
            let out = if flip { -n } else { n };
            let j = joins[i];
            let (s_out, s_in) = if flip {
                ([j[0].s_neg, j[1].s_neg], [j[0].s_pos, j[1].s_pos])
            } else {
                ([j[0].s_pos, j[1].s_pos], [j[0].s_neg, j[1].s_neg])
            };
            wall_of_seg[i] = Some(walls.len());
            walls.push(WallA {
                seg: i,
                f: WallFrame { origin: w.a, dir: d, out },
                t,
                len,
                exterior: w.exterior,
                s_out,
                s_in,
                caps: [j[0].cap, j[1].cap],
                mitre: [j[0].mitre, j[1].mitre],
            });
        }
        // Mitre partners as indices into `walls`.
        for w in &mut walls {
            for e in 0..2 {
                w.mitre[e] = w.mitre[e].and_then(|s| wall_of_seg.get(s).copied().flatten());
            }
        }
        // Openings.
        let mut openings: Vec<OpeningA> = Vec::new();
        for (oi, o) in floor.openings.iter().enumerate() {
            if !o.a.is_finite() || !o.b.is_finite() || !o.sill.is_finite() || !o.height.is_finite() {
                continue;
            }
            let mut best: Option<(f32, usize)> = None;
            for (wi, w) in walls.iter().enumerate() {
                let seg = &floor.walls[w.seg];
                let da = o.a.distance(mde_core::poly::closest_point_on_segment(o.a, seg.a, seg.b));
                let db = o.b.distance(mde_core::poly::closest_point_on_segment(o.b, seg.a, seg.b));
                let score = da + db;
                if da < 0.06 && db < 0.06 && best.is_none_or(|(s, _)| score < s) {
                    best = Some((score, wi));
                }
            }
            let Some((_, wi)) = best else { continue };
            let w = &walls[wi];
            let sa = (o.a - w.f.origin).dot(w.f.dir);
            let sb = (o.b - w.f.origin).dot(w.f.dir);
            let (u0, u1) = w.usable();
            let s0 = sa.min(sb).max(u0);
            let s1 = sa.max(sb).min(u1);
            if s1 - s0 < 0.25 {
                continue;
            }
            if openings.iter().any(|q| q.wall == wi && s0 < q.hole.s1 + 0.05 && s1 > q.hole.s0 - 0.05) {
                continue;
            }
            let wall_top = if w.exterior { top_ext } else { ceil };
            let sill = o.sill.clamp(0.0, (wall_top - y0 - 0.4).max(0.0));
            let yb = y0 + sill;
            let room = wall_top - 0.1 - yb;
            if room < 0.3 {
                continue;
            }
            let h = o.height.clamp(0.3, room);
            let hole = match o.kind {
                OpeningKind::Window => {
                    let hole = window_hole(app.window_kind, app.custom_window.is_some(), s0, s1, yb, h);
                    if hole.top() > wall_top - 0.08 {
                        let excess = hole.top() - (wall_top - 0.08);
                        window_hole(app.window_kind, app.custom_window.is_some(), s0, s1, yb, (h - excess).max(0.3))
                    } else {
                        hole
                    }
                }
                OpeningKind::Entrance => {
                    door_hole(app.door_kind, app.custom_door.is_some(), s0, s1, y0, h, 2.0, wall_top - 0.12)
                }
                OpeningKind::Door => door_hole(DoorKind::Plank, false, s0, s1, y0, h, 2.0, wall_top - 0.1),
                OpeningKind::Doorway => {
                    let kind = if s1 - s0 <= 1.8 { DoorKind::Arched } else { DoorKind::Plank };
                    door_hole(kind, false, s0, s1, y0, h, 2.0, wall_top - 0.1)
                }
            };
            // Which way doors swing.
            let n = w.f.out;
            let inward = match o.kind {
                OpeningKind::Entrance | OpeningKind::Window if w.exterior => -n,
                _ => {
                    let mid = (o.a + o.b) * 0.5;
                    let side_of = |id: Option<u32>| -> Option<(f32, RoomKind)> {
                        let r = floor.rooms.iter().find(|r| Some(r.id) == id)?;
                        let s = if r.shape.contains(mid + n * 0.4) {
                            1.0
                        } else if r.shape.contains(mid - n * 0.4) {
                            -1.0
                        } else {
                            return None;
                        };
                        Some((s, r.kind))
                    };
                    let a = side_of(o.rooms[0]);
                    let b = side_of(o.rooms[1]);
                    let pick = match (a, b) {
                        (Some((sa, ka)), Some((sb, kb))) => {
                            if is_hub(ka) && !is_hub(kb) {
                                sb
                            } else if is_hub(kb) && !is_hub(ka) {
                                sa
                            } else {
                                sb
                            }
                        }
                        (Some((s, k)), None) | (None, Some((s, k))) => {
                            if is_hub(k) {
                                -s
                            } else {
                                s
                            }
                        }
                        _ => 1.0,
                    };
                    n * pick
                }
            };
            openings.push(OpeningA { index: oi, wall: wi, hole, kind: o.kind, inward });
        }
        out.push(FloorA { floor, y0, ceil, top_ext, walls, openings });
    }
    out
}

fn colors(app: &Appearance) -> OpeningColors {
    OpeningColors {
        frame: srgb_to_linear(app.window),
        timber: srgb_to_linear(app.timber),
        stone: WHITE,
        lite: false,
    }
}

/// Leaves for one door opening (two for double doors).
fn leaves_for(d: &Dwelling, fa: &FloorA, o: &OpeningA) -> Vec<DoorLeaf> {
    let app = &d.appearance;
    let w = &fa.walls[o.wall];
    let h = &o.hole;
    let kind = match o.kind {
        OpeningKind::Entrance => app.door_kind,
        _ => DoorKind::Plank,
    };
    let double = kind == DoorKind::Double || (o.kind == OpeningKind::Entrance && h.width() > 1.7);
    // Hinge on the end nearer to the wall's end (the leaf folds against the wall).
    let hinge_low = h.s0 < w.len - h.s1;
    let spans: Vec<(f32, f32)> = if double {
        vec![(h.s0, h.sc()), (h.s1, h.sc())]
    } else if hinge_low {
        vec![(h.s0, h.s1)]
    } else {
        vec![(h.s1, h.s0)]
    };
    let floor_y = fa.y0;
    let mut out = Vec::new();
    for (k, &(sh, sl)) in spans.iter().enumerate() {
        let along = if sl > sh { w.f.dir } else { -w.f.dir };
        let width = (sl - sh).abs() - 0.008;
        if width < 0.2 {
            continue;
        }
        let x_of = |s: f32| (s - sh).abs();
        let top = match h.head {
            Head::Flat => None,
            _ => Some(
                h.profile()
                    .iter()
                    .filter(|p| {
                        let (lo, hi) = if sh < sl { (sh, sl) } else { (sl, sh) };
                        p.0 >= lo - 1e-4 && p.0 <= hi + 1e-4
                    })
                    .map(|p| (x_of(p.0), p.2 - floor_y - 0.01))
                    .collect::<Vec<_>>(),
            ),
        };
        let height = h.y1 - floor_y - 0.01;
        let yaw = yaw_of_x(wdir(along));
        let local_z = rot_y(Vec3::Z, yaw);
        let inward3 = wdir(o.inward);
        let front_pos_z = local_z.dot(-inward3) > 0.0;
        let open_dir = if rot_y(-Vec3::Z, yaw).dot(inward3) > 0.0 { 1.0 } else { -1.0 };
        let spec = LeafSpec {
            kind: if double && kind == DoorKind::Double { DoorKind::Plank } else { kind },
            width,
            height: top.as_ref().map_or(height, |t| t.iter().map(|p| p.1).fold(height, f32::max)),
            top,
            thickness: if o.kind == OpeningKind::Entrance { 0.07 } else { 0.05 },
            door: srgb_to_linear(app.door),
            front_pos_z,
            seed: mix_seed(d.params.seed, (fa.floor.level as u64) << 32 | (o.index as u64) << 4 | k as u64),
            lite: false,
        };
        let model = openings::door_leaf_model(&spec);
        let hinge = w.f.p(sh + along.dot(w.f.dir) * 0.004, floor_y, 0.0);
        out.push(DoorLeaf {
            model,
            hinge,
            yaw,
            width,
            height: spec.height,
            level: fa.floor.level,
            opening: o.index,
            open_dir,
        });
    }
    out
}

/// Door leaves of every `Door` and `Entrance` opening (not `Doorway`s, and not
/// the entrance when [`Appearance::custom_door`] is set), in their closed pose.
pub fn door_leaves(d: &Dwelling) -> Vec<DoorLeaf> {
    let mut out = Vec::new();
    for fa in analyse(d) {
        for o in &fa.openings {
            let wanted = match o.kind {
                OpeningKind::Door => true,
                OpeningKind::Entrance => d.appearance.custom_door.is_none(),
                _ => false,
            };
            if wanted {
                out.extend(leaves_for(d, &fa, o));
            }
        }
    }
    out
}

/// Placement of every opening that sits on a wall (the same set the mesh cuts).
pub fn opening_placements(d: &Dwelling) -> Vec<OpeningPlacement> {
    let mut out = Vec::new();
    for fa in analyse(d) {
        for o in &fa.openings {
            let w = &fa.walls[o.wall];
            let op = &fa.floor.openings[o.index];
            let s = (o.hole.s0 + o.hole.s1) * 0.5;
            let width = o.hole.s1 - o.hole.s0;
            let height = (o.hole.top() - o.hole.y0).max(0.01);
            let cy = match o.kind {
                OpeningKind::Window => fa.y0 + op.sill.max(0.0) + op.height.min(height) * 0.5,
                _ => o.hole.y0 + height * 0.5,
            };
            let height = if o.kind == OpeningKind::Window { op.height.min(height) } else { height };
            let out3 = wdir(w.f.out);
            out.push(OpeningPlacement {
                kind: o.kind,
                level: fa.floor.level,
                opening: o.index,
                center: w.f.p(s, cy, 0.0),
                yaw: out3.x.atan2(out3.z),
                width,
                height,
                wall_thickness: w.t,
            });
        }
    }
    out
}

/// Build the mesh of a dwelling (world space, metres, Y up).
pub fn build_dwelling(d: &Dwelling, opts: &DwellingMeshOptions) -> Model {
    let mut mb = Mb::new();
    let app = &d.appearance;
    let style = HouseStyle::new(d.params.style, d.params.seed).with_profile(&opts.profile);
    let cols = colors(app);
    let wall_col = srgb_to_linear(app.wall);
    let timber = cols.timber;
    let wash = fixed::whitewash();
    let mut rng = Rng::new(mix_seed(d.params.seed, 0xD3E1));
    let analysis = analyse(d);
    let shown: Vec<&FloorA> = analysis.iter().filter(|f| opts.max_level.is_none_or(|l| f.floor.level <= l)).collect();
    let lowest = shown.first().map(|f| f.floor.level);
    let draw_roof = opts.roof && opts.max_level.is_none();
    let elevation_of = |level: i32| analysis.iter().find(|f| f.floor.level == level).map(|f| f.y0);

    for (si, fa) in shown.iter().enumerate() {
        let level = fa.floor.level;
        let cutaway_top = opts.max_level.is_some() && si + 1 == shown.len();
        let top_ext = if cutaway_top { fa.ceil } else { fa.top_ext };
        let stone = style.stone(level);
        let framed = style.framed(level);
        let ext_slot = if stone { S::Stone } else { S::Plaster };
        let (in_slot, in_col) = if level < 0 { (S::Stone, scale(WHITE, 0.92)) } else { (S::Plaster, wash) };
        for (wi, w) in fa.walls.iter().enumerate() {
            if !w.exterior && !opts.interior {
                continue;
            }
            let holes: Vec<Hole> = {
                let mut v: Vec<Hole> = fa.openings.iter().filter(|o| o.wall == wi).map(|o| o.hole).collect();
                v.sort_by(|a, b| a.s0.total_cmp(&b.s0));
                v
            };
            let y1 = if w.exterior { top_ext } else { fa.ceil };
            // Exterior wall tops are covered by the next floor or the roof.
            let covered = w.exterior && !cutaway_top && (si + 1 < shown.len() || draw_roof);
            let solid = Solid {
                f: w.f,
                t: w.t,
                y0: fa.y0,
                y1,
                s_out: w.s_out,
                s_in: w.s_in,
                caps: w.caps,
                holes: &holes,
                top: !covered,
            };
            let ht = w.t * 0.5;
            if w.exterior {
                let oc = if stone { jitter(WHITE, &mut rng, 0.04) } else { jitter(wall_col, &mut rng, 0.025) };
                let split = ht - (w.t * 0.35).min(0.13);
                let (mut outer, mut inner) = (MeshData::default(), MeshData::default());
                walls::wall_solid(&mut outer, oc, &mut inner, in_col, &solid, split);
                if level == 0 {
                    walls::ground_grime(&mut outer, 0, fa.y0);
                }
                mb.m(ext_slot).append(&outer);
                mb.m(in_slot).append(&inner);
                exterior_dressing(&mut mb, d, fa, wi, w, &holes, y1, stone, framed, &style, &cols, &mut rng);
            } else {
                let (mut a, mut b) = (MeshData::default(), MeshData::default());
                walls::wall_solid(&mut a, wash, &mut b, wash, &solid, 0.0);
                mb.m(S::Plaster).append(&a);
                mb.m(S::Plaster).append(&b);
            }
        }
        // Openings: windows, entrance dressing, door casings and leaves.
        for o in &fa.openings {
            let w = &fa.walls[o.wall];
            if !w.exterior && !opts.interior {
                continue;
            }
            let ht = w.t * 0.5;
            let others: Vec<Hole> = fa.openings.iter().filter(|q| q.wall == o.wall).map(|q| q.hole).collect();
            match o.kind {
                OpeningKind::Window => {
                    if app.custom_window.is_some() {
                        continue;
                    }
                    let wg = if w.exterior { ht - (w.t * 0.35).min(0.13) } else { 0.0 };
                    openings::window_insert(&mut mb, &w.f, &o.hole, app.window_kind, wg, &cols, &mut rng);
                    if w.exterior {
                        let boxes = app.flower_boxes && level >= 0 && o.hole.y0 - fa.y0 > 0.5;
                        openings::window_exterior(
                            &mut mb,
                            &w.f,
                            &o.hole,
                            &others,
                            ht,
                            wg,
                            stone,
                            app.shutters,
                            boxes,
                            &cols,
                            &mut rng,
                        );
                    }
                }
                OpeningKind::Entrance => {
                    if w.exterior {
                        let lantern = style.lantern && level == 0;
                        openings::door_surround(
                            &mut mb,
                            &w.f,
                            &o.hole,
                            ht,
                            fa.y0,
                            stone,
                            framed,
                            lantern,
                            &cols,
                            &mut rng,
                        );
                    }
                    if opts.door_leaves && app.custom_door.is_none() {
                        add_leaves(&mut mb, &leaves_for(d, fa, o));
                    }
                }
                OpeningKind::Door => {
                    let c = jitter(timber, &mut rng, 0.05);
                    openings::door_casing(mb.m(S::Timber), &w.f, &o.hole, ht, true, c);
                    openings::door_casing(mb.m(S::Timber), &w.f, &o.hole, -ht, false, c);
                    if opts.door_leaves {
                        add_leaves(&mut mb, &leaves_for(d, fa, o));
                    }
                }
                OpeningKind::Doorway => {
                    if o.hole.head == Head::Flat {
                        let c = jitter(timber, &mut rng, 0.05);
                        openings::door_casing(mb.m(S::Timber), &w.f, &o.hole, ht, true, c);
                        openings::door_casing(mb.m(S::Timber), &w.f, &o.hole, -ht, false, c);
                    }
                }
            }
        }
        // Floor slab with stair holes.
        let outline = clean_outline(&fa.floor.outline, &fa.walls);
        if opts.interior || Some(level) == lowest {
            let holes: Vec<Vec<Vec2>> = analysis
                .iter()
                .flat_map(|f| f.floor.stairs.iter())
                .filter(|s| s.to_level == level && s.footprint.len() >= 3)
                .map(|s| s.footprint.0.clone())
                .filter(|h| h.iter().all(|&p| outline.contains(p)))
                .collect();
            let (top_slot, top_col) = if level >= 1 { (S::WoodFloor, WHITE) } else { (S::Stone, scale(WHITE, 0.85)) };
            let under = scale(timber, 0.85);
            let slab_t = if Some(level) == lowest { 0.3 } else { (fa.y0 - prev_ceil(&analysis, level).unwrap_or(fa.y0 - SLAB)).clamp(0.1, 0.6) };
            // Inset so the slab's edges stay hidden inside the exterior walls.
            let slab_outline = outline.inset(0.1).unwrap_or_else(|| outline.clone());
            interior::slab(&mut mb, &slab_outline.0, &holes, fa.y0, slab_t, top_slot, top_col, S::Timber, under, Some(level) != lowest);
            if Some(level) != lowest && opts.interior {
                interior::joists(mb.m(S::Timber), &outline, &holes, fa.y0 - slab_t, scale(timber, 0.9), &mut rng);
            }
        }
        if opts.interior {
            for st in &fa.floor.stairs {
                let rise = elevation_of(st.to_level).map_or(3.0 * (st.to_level - st.from_level) as f32, |e| e - fa.y0);
                interior::stair_flight(&mut mb, st, fa.y0, rise, timber, &mut rng);
            }
        }
        if opts.props {
            for p in &fa.floor.props {
                interior::prop(&mut mb, p, fa.y0, timber, WHITE, &mut rng);
            }
        }
    }

    if draw_roof && let Some(top) = analysis.last() {
        let outline = clean_outline(&top.floor.outline, &top.walls);
        let base = top.top_ext;
        // Attic floor / top ceiling.
        if opts.interior {
            let slab_outline = outline.inset(0.1).unwrap_or_else(|| outline.clone());
            interior::slab(&mut mb, &slab_outline.0, &[], base, (base - top.ceil).clamp(0.1, 0.6), S::Timber, scale(timber, 0.7), S::Timber, scale(timber, 0.85), true);
            interior::joists(mb.m(S::Timber), &outline, &[], top.ceil, scale(timber, 0.9), &mut rng);
        } else {
            let pts: Vec<Vec3> = outline.0.iter().map(|&q| w3(q, base)).collect();
            crate::geom::poly_facing(mb.m(S::Timber), &pts, Vec3::Y, scale(timber, 0.6));
        }
        let level = top.floor.level;
        let stone = style.stone(level);
        let story = opts.profile.story_roof();
        // The default storybook roof overrides the generator's roll (a preset
        // has set it on the data already, see `Storybook::apply_to_dwelling`).
        let (kind, pitch_deg, overhang, ridge) = if story && opts.profile.storybook.is_none() {
            let (pitch, overhang) = opts.profile.roof_shape(d.params.seed);
            let ridge = crate::style::street_ridge(d).unwrap_or(d.roof.ridge);
            // Half-span across the ridge, wall height from the ground floor.
            let across = Vec2::new(-ridge.y, ridge.x);
            let (lo, hi) = outline.0.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), q| (lo.min(q.dot(across)), hi.max(q.dot(across))));
            let ground = d.floors.iter().find(|f| f.level == 0).map_or(0.0, |f| f.elevation);
            let pitch = crate::style::cap_pitch(pitch, base - ground, (hi - lo) * 0.5);
            (mde_core::RoofKind::Gable, pitch, overhang, ridge)
        } else {
            (d.roof.kind, d.roof.pitch_deg, d.roof.overhang, d.roof.ridge)
        };
        let params = RoofParams {
            kind,
            pitch_deg,
            overhang,
            ridge,
            base,
            material: app.roof_material,
            roof: srgb_to_linear(app.roof),
            timber,
            gable: GableStyle {
                slot: if stone { S::Stone } else { S::Plaster },
                col: if stone { WHITE } else { wall_col },
                timber: (style.framed(level) || style.gable_timber && !stone).then_some(timber),
            },
            parapet: (S::Stone, WHITE, true),
            detail: Detail::Full,
            seed: d.params.seed,
            trim: if story { Trim::Boards } else { Trim::None },
            curved: story,
            tiles: story,
            front: if story { crate::style::dwelling_front(d) } else { None },
        };
        if let Some(r) = Roof::plan(&outline, &params) {
            r.build(&mut mb, &params);
            // Chimneys only over a hearth or stove inside the house.
            let mut placed = 0;
            let mut chimney_rng = rng.fork(77);
            for f in &analysis {
                for p in &f.floor.props {
                    if placed < 2 && matches!(p.kind, PropKind::Hearth | PropKind::Stove) && outline.contains(p.pos) {
                        let dir = r.ridges().first().map_or(Vec2::X, |(a, b)| (*b - *a).normalize_or(Vec2::X));
                        if roof::chimney(&mut mb, &r, p.pos, dir, WHITE, style.brick_chimney, &mut chimney_rng) {
                            placed += 1;
                        }
                    }
                }
            }
        }
    }
    mb.into_model(format!("dwelling_{}", d.params.seed))
}

fn prev_ceil(analysis: &[FloorA], level: i32) -> Option<f32> {
    analysis.iter().filter(|f| f.floor.level < level).map(|f| f.ceil).fold(None, |a: Option<f32>, c| Some(a.map_or(c, |x| x.max(c))))
}

/// Floor outline, or the exterior walls' outer bounds if missing.
fn clean_outline(outline: &Polygon, walls: &[WallA]) -> Polygon {
    let pts: Vec<Vec2> = outline.0.iter().copied().filter(|p| p.is_finite()).collect();
    let mut p = Polygon(pts).simplified(1e-3);
    if p.len() >= 3 && p.area() > 0.5 {
        p.make_ccw();
        return p;
    }
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    for w in walls.iter().filter(|w| w.exterior) {
        for s in [0.0, w.len] {
            for side in [-1.0, 1.0] {
                let q = w.f.plan(s, side * w.t * 0.5);
                min = min.min(q);
                max = max.max(q);
            }
        }
    }
    if min.x < max.x && min.y < max.y {
        Polygon::rect((min + max) * 0.5, max - min)
    } else {
        Polygon::default()
    }
}

fn add_leaves(mb: &mut Mb, leaves: &[DoorLeaf]) {
    for l in leaves {
        let (hinge, yaw) = (l.hinge, l.yaw);
        mb.append_transformed(&l.model, |p| hinge + rot_y(p, yaw), |n| rot_y(n, yaw));
    }
}

/// Timber framing, quoins and plinth on one exterior wall's outer face.
#[allow(clippy::too_many_arguments)]
fn exterior_dressing(
    mb: &mut Mb,
    d: &Dwelling,
    fa: &FloorA,
    wi: usize,
    w: &WallA,
    holes: &[Hole],
    y1: f32,
    stone: bool,
    framed: bool,
    style: &HouseStyle,
    cols: &OpeningColors,
    rng: &mut Rng,
) {
    let _ = (d, style);
    let ht = w.t * 0.5;
    let level = fa.floor.level;
    // Extend corner pieces on the wall with the lower index so the notch at a
    // convex corner is closed exactly once.
    let ext = |e: usize, amount: f32| if w.convex(e) && w.mitre[e].is_none_or(|m| wi < m) { amount } else { 0.0 };
    // At inner (reflex) corners one of the two walls stops its timbers and
    // plinth short, so the pieces in front of both faces do not overlap.
    let trim = |e: usize| if !w.convex(e) && w.mitre[e].is_some_and(|m| wi < m) { 0.07 } else { 0.0 };
    if framed {
        let rail = holes
            .iter()
            .filter(|h| h.y0 > fa.y0 + 0.3)
            .map(|h| h.y0 - fa.y0)
            .fold(None, |a: Option<f32>, v| Some(a.map_or(v, |x: f32| x.min(v))))
            .unwrap_or(0.95);
        let fr = Framing {
            f: &w.f,
            w_face: ht,
            s0: w.s_out[0],
            s1: w.s_out[1],
            y0: fa.y0,
            y1,
            holes,
            ext: [ext(0, 0.05), ext(1, 0.05)],
            corner: [w.convex(0), w.convex(1)],
            trim: [trim(0), trim(1)],
            col: cols.timber,
            rail,
            sill_beam: true,
            lite: false,
        };
        walls::half_timber(mb.m(S::Timber), &fr, rng);
    } else if level >= 0 {
        let dressed = mix(WHITE, fixed::dressed_stone(), if stone { 0.8 } else { 1.0 });
        for e in 0..2 {
            if w.convex(e) {
                let parity = w.mitre[e].is_none_or(|m| wi < m);
                walls::quoins(mb.m(S::Stone), &w.f, ht, w.s_out[e], e == 1, fa.y0, y1, ext(e, 0.03), parity, dressed, rng, false);
            }
        }
    }
    if level == 0 && !framed {
        // Plinth: a proud stone base course, broken at doors.
        let mut runs = vec![(w.s_out[0] - ext(0, 0.05) + trim(0), w.s_out[1] + ext(1, 0.05) - trim(1))];
        for h in holes.iter().filter(|h| h.y0 <= fa.y0 + 0.05) {
            runs = walls::cut_runs(&runs, h.s0 - 0.02, h.s1 + 0.02);
        }
        for (a, b) in runs {
            let c = jitter(scale(WHITE, 0.82), rng, 0.04);
            w.f.block(mb.m(S::Stone), a, b, fa.y0 - 0.3, fa.y0 + 0.38, ht, ht + 0.05, c, crate::geom::faces::NZ);
        }
    }
}
