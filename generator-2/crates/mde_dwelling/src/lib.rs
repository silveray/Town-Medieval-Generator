//! Procedural dwelling generator: multi-storey medieval house plans with named
//! rooms, walls, doors, windows, stairs and furniture, walkable by NPCs.
//!
//! [`generate`] turns [`DwellingParams`] into a [`Dwelling`]: plain serde data
//! in plan space (metres, `x` east, `y` north, CCW polygons) that the mesh
//! builder and the navigation graph consume.
//!
//! # Pipeline
//!
//! 1. **Site.** A local frame is aligned with the footprint's street-facing edge
//!    (or its longest edge), the footprint is inset by half an exterior wall
//!    and rasterised into planning cells of about `cell_size` metres (stretched
//!    so a rectangle fits exactly). An empty footprint is synthesised from the
//!    style: rectangles and L / T / U plans.
//! 2. **Stairwell.** A two-lane straight stair well (about 2 × 3 cells)
//!    occupying the same cells on every level and touching an exterior wall.
//!    Flights alternate lanes and directions, so each level is entered and
//!    left at the same end of the well, through a landing zone kept clear in
//!    the adjoining room (the hall in manors, the kitchen at the back of town
//!    houses). Every flight is a straight run listed on the floor it starts
//!    from; on the floor above it is a void.
//! 3. **Rooms.** A per-style, per-floor programme is partitioned with a
//!    recursive BSP along grid lines (area weights, minimum widths, front/back
//!    preferences, entry room on the street and at the stair). Upper floors
//!    with several rooms get a ≥ 1 m corridor — a spine plus a connector from
//!    the stair landing, or a stair hall across narrow houses. A repair pass
//!    merges any room that could not get a usable door.
//! 4. **Openings.** Walls on every edge between different rooms; a door
//!    spanning tree grown from the entry (hubs preferred, private rooms as
//!    leaves, a few loops in big houses); the street entrance; windows on a
//!    regular bay rhythm that lines up between storeys.
//! 5. **Furniture** per room kind, kept out of door clearance zones, stair
//!    landings and the paths between doors: every piece is checked on a
//!    0.25 m walk grid and rejected if it would cut a door or landing off, or
//!    leave less than about 0.7 m to walk past it.
//! 6. **Roof** kind, pitch, overhang and ridge by style.
//!
//! Everything is deterministic for a given seed ([`mde_core::Rng`] only).
//! [`validate()`] checks the invariants NPC navigation relies on.

mod build;
mod frame;
mod grid;
mod layout;
mod program;
mod props;
mod runs;
mod style;
mod svg;
pub mod validate;
pub mod walk;

use mde_core::{
    Building, BuildingKind, Dwelling, DwellingParams, DwellingStyle, Floor, Opening, OpeningKind, Polygon, Prop, Rng, Room,
    RoomKind, Stair, Vec2, WallSegment, SCHEMA_VERSION,
};

use crate::build::{LOpening, LStair, LWall};
use crate::frame::{Frame, Side};
use crate::grid::Grid;
use crate::layout::{LayoutCtx, LevelLayout};
use crate::props::{Furnisher, LProp};

pub use svg::{render_svg, render_svg_all};
pub use validate::validate;

/// Exterior wall thickness, metres.
pub const EXTERIOR_WALL: f32 = 0.4;
/// Interior partition thickness, metres.
pub const INTERIOR_WALL: f32 = 0.15;
/// Narrowest door or doorway the generator produces, metres.
pub const DOOR_MIN_WIDTH: f32 = 0.9;
/// Interior door height, metres.
pub const DOOR_HEIGHT: f32 = 2.1;
/// Thickness of the floor structure between storeys (storey height minus
/// [`Floor::height`]), metres.
pub const FLOOR_THICKNESS: f32 = 0.25;
/// Most storeys above ground the generator builds.
pub const MAX_FLOORS: u8 = 8;

/// Salt mixed into the seed so dwelling streams differ from other generators'.
const SEED_SALT: u64 = 0x6D64_6577_656C_6C21;

/// Copy of `params` with out-of-range values clamped and defaults filled in:
/// floors in `1..=MAX_FLOORS`, storey height 2.4–6 m, cell size 0.6–2 m,
/// `front` normalised (or zero).
pub fn sanitize(params: &DwellingParams) -> DwellingParams {
    let mut p = params.clone();
    p.floors = p.floors.clamp(1, MAX_FLOORS);
    p.storey_height = if p.storey_height.is_finite() { p.storey_height.clamp(2.4, 6.0) } else { 3.0 };
    p.cell_size = if p.cell_size.is_finite() { p.cell_size.clamp(0.6, 2.0) } else { 1.0 };
    p.front = if p.front.is_finite() { p.front.normalize_or_zero() } else { Vec2::ZERO };
    p
}

/// Generate a dwelling.
///
/// `Dwelling::params` holds the sanitised inputs with the resolved appearance;
/// if no stairwell fits the footprint the house is reduced to a single storey
/// (and `floors` / `basement` say so). Generating again from the returned
/// params reproduces the same dwelling.
pub fn generate(params: &DwellingParams) -> Dwelling {
    let mut p = sanitize(params);
    let mut root = Rng::new(p.seed ^ SEED_SALT);
    let mut rng_site = root.fork(1);
    let mut rng_stair = root.fork(2);
    let mut rng_layout = root.fork(3);
    let mut rng_open = root.fork(4);
    let mut rng_props = root.fork(5);
    let mut rng_roof = root.fork(6);
    let mut rng_app = root.fork(7);

    let appearance = match &p.appearance {
        Some(a) => a.clone(),
        None => style::pick_appearance(p.style, &mut rng_app),
    };
    p.appearance = Some(appearance.clone());

    let site = Site::prepare(&p, &mut rng_site);
    let g = &site.grid;

    let mut levels: Vec<i32> = (if p.basement { -1 } else { 0 }..p.floors as i32).collect();
    let mut stair = None;
    if levels.len() > 1 {
        let ground_index = levels.iter().position(|&l| l == 0).unwrap_or(0);
        stair = layout::place_stairwell(g, p.style, site.front, p.storey_height, ground_index, &mut rng_stair);
        if stair.is_none() {
            p.basement = false;
            p.floors = 1;
            levels = vec![0];
        }
    }

    let ctx = LayoutCtx { grid: g, front: site.front, stair: stair.as_ref(), style: p.style, floors: p.floors };
    let layouts: Vec<LevelLayout> = levels
        .iter()
        .enumerate()
        .map(|(m, &level)| layout::layout_level(&ctx, level, m, &mut rng_layout.fork(m as u64)))
        .collect();

    let clear = p.storey_height - FLOOR_THICKNESS;
    let window_style = style::window_style(p.style, appearance.window_kind, clear, &mut rng_open);
    let flights: Vec<LStair> = stair.map(|s| build::flights(&s, g, &levels)).unwrap_or_default();
    let outline = site.outline();

    let mut floors = Vec::with_capacity(layouts.len());
    let mut next_id = 0u32;
    for (m, lay) in layouts.iter().enumerate() {
        let mut r = rng_open.fork(m as u64);
        let walls = build::walls(g, &lay.labels);
        let mut openings = build::place_doors(lay, g, stair.as_ref(), &mut r);
        let entrance = if lay.level == 0 { build::place_entrance(lay, g, site.front, &appearance, &mut r) } else { None };
        let windows = build::place_windows(lay, g, &window_style, site.front, entrance.as_ref(), clear);
        openings.extend(entrance);
        openings.extend(windows);
        let stairs_here: Vec<LStair> = flights.iter().filter(|s| s.from == lay.level || s.to == lay.level).cloned().collect();
        let furnisher = Furnisher::new(g, lay, p.style, &walls, &openings, &stairs_here, stair.map(|s| s.rect));
        let props = furnisher.furnish(&mut rng_props.fork(m as u64));
        let floor = site.floor(lay, &walls, &openings, &flights, &props, next_id, p.storey_height, &outline);
        next_id += lay.room_count() as u32;
        floors.push(floor);
    }

    let (long_axis, front_plan) = site.axes();
    let roof = style::pick_roof(p.style, long_axis, front_plan, &mut rng_roof);
    Dwelling { schema: SCHEMA_VERSION, params: p, floors, roof, appearance }
}

/// Dwelling parameters for a town building: style from the building kind
/// (houses become cottages or townhouses by size, mansions / keeps / temples
/// manors, barracks taverns, barns / mills farmhouses), footprint, storeys,
/// street front, seed and the building's appearance. Taverns, mansions and
/// keeps get a cellar; other kinds sometimes do (decided from the seed).
pub fn from_building(b: &Building) -> DwellingParams {
    use BuildingKind as B;
    let area = b.footprint.area();
    let style = match b.kind {
        B::House => {
            if area > 0.0 && area < 45.0 {
                DwellingStyle::Cottage
            } else {
                DwellingStyle::Townhouse
            }
        }
        B::Tower => DwellingStyle::Townhouse,
        B::Shop | B::Workshop => DwellingStyle::Shop,
        B::Tavern | B::Barracks => DwellingStyle::Tavern,
        B::Mansion | B::Keep | B::Temple => DwellingStyle::Manor,
        B::Farmhouse | B::Barn | B::Mill => DwellingStyle::Farmhouse,
    };
    let mut rng = Rng::new(b.seed ^ 0xB417_D1A6);
    let basement = match b.kind {
        B::Tavern | B::Mansion | B::Keep => true,
        B::Shop | B::Workshop | B::Temple => rng.chance(0.5),
        B::House => rng.chance(0.25),
        _ => false,
    };
    DwellingParams {
        seed: b.seed,
        footprint: b.footprint.clone(),
        floors: b.floors.max(1),
        basement,
        front: b.front,
        style,
        storey_height: if style == DwellingStyle::Manor { 3.4 } else { 3.0 },
        cell_size: 1.0,
        appearance: Some(b.appearance.clone()),
    }
}

/// The ground-floor street entrance.
pub fn entrance(d: &Dwelling) -> Option<&mde_core::Opening> {
    fn find(f: &Floor) -> Option<&Opening> {
        f.openings.iter().find(|o| o.kind == OpeningKind::Entrance)
    }
    d.floors.iter().filter(|f| f.level == 0).find_map(find).or_else(|| d.floors.iter().find_map(find))
}

/// Human-readable room name ("Living room", "Servants' room", ...).
pub fn room_label(kind: RoomKind) -> &'static str {
    use RoomKind as K;
    match kind {
        K::Hall => "Hall",
        K::Corridor => "Corridor",
        K::Stairwell => "Stairs",
        K::Living => "Living room",
        K::Kitchen => "Kitchen",
        K::Dining => "Dining room",
        K::Bedroom => "Bedroom",
        K::Study => "Study",
        K::Library => "Library",
        K::Storage => "Storage",
        K::Pantry => "Pantry",
        K::Workshop => "Workshop",
        K::Shop => "Shop",
        K::Taproom => "Taproom",
        K::Bath => "Bath",
        K::Privy => "Privy",
        K::Chapel => "Chapel",
        K::Nursery => "Nursery",
        K::Servants => "Servants' room",
        K::Cellar => "Cellar",
        K::Attic => "Attic",
    }
}

/// Human-readable level name: "Cellar", "Ground floor", "First floor", ...
pub fn level_name(level: i32) -> String {
    const ORD: [&str; 9] = ["Ground", "First", "Second", "Third", "Fourth", "Fifth", "Sixth", "Seventh", "Eighth"];
    match level {
        l if l < -1 => format!("Basement {}", -l),
        -1 => "Cellar".to_string(),
        l if (l as usize) < ORD.len() => format!("{} floor", ORD[l as usize]),
        l => format!("Floor {l}"),
    }
}

/// The house on its lot: local frame, planning grid and street side.
struct Site {
    frame: Frame,
    grid: Grid,
    front: Side,
}

impl Site {
    /// Rasterise the footprint, or synthesise one for the style.
    fn prepare(p: &DwellingParams, rng: &mut Rng) -> Site {
        let mut anchor = Vec2::ZERO;
        if let Some(poly) = clean_polygon(&p.footprint) {
            anchor = poly.centroid();
            let (a, b) = choose_edge(&poly, p.front);
            let frame = Frame::new(a, b - a);
            let local = Polygon(poly.0.iter().map(|&q| frame.to_local(q)).collect());
            let centre = local.inset(EXTERIOR_WALL * 0.5).unwrap_or(local);
            if let Some(grid) = grid::rasterize(&centre, p.cell_size)
                && grid.count() >= 4
                && grid.count() as f32 * grid.cell_area() >= 6.0
            {
                let front = if p.front.length_squared() > 0.5 { Side::from_dir(frame.dir_to_local(p.front)) } else { Side::South };
                return Site { frame, grid, front };
            }
        }
        let grid = grid::synthesize(p.style, p.cell_size, rng);
        let front_dir = if p.front.length_squared() > 0.5 { p.front } else { Vec2::new(0.0, -1.0) };
        let v = -front_dir;
        let u = Vec2::new(v.y, -v.x);
        let cells = grid.cells();
        let centroid = cells.iter().map(|&c| grid.center(c)).sum::<Vec2>() / cells.len().max(1) as f32;
        let origin = anchor - u * centroid.x - v * centroid.y;
        Site { frame: Frame::new(origin, u), grid, front: Side::South }
    }

    fn poly_to_plan(&self, pts: &[Vec2]) -> Polygon {
        Polygon(pts.iter().map(|&q| self.frame.to_plan(q)).collect())
    }

    /// Outer face of the exterior walls, plan space.
    fn outline(&self) -> Polygon {
        let g = &self.grid;
        let loops = grid::trace_loops(g.nx, g.ny, |c| g.inside(c));
        let best = loops.into_iter().max_by_key(|l| grid::loop_area(l)).unwrap_or_default();
        let local = Polygon(best.iter().map(|&v| g.vertex(v)).collect());
        let outer = local.inset(-EXTERIOR_WALL * 0.5).unwrap_or(local);
        self.poly_to_plan(&outer.0)
    }

    /// Plan directions of the long axis and of the street.
    fn axes(&self) -> (Vec2, Vec2) {
        let g = &self.grid;
        let long = if g.nx as f32 * g.cx >= g.ny as f32 * g.cy { Vec2::X } else { Vec2::Y };
        (self.frame.dir_to_plan(long), self.frame.dir_to_plan(self.front.normal()))
    }

    /// Convert one laid-out level to output data.
    #[allow(clippy::too_many_arguments)]
    fn floor(
        &self,
        lay: &LevelLayout,
        walls: &[LWall],
        openings: &[LOpening],
        flights: &[LStair],
        props: &[LProp],
        base_id: u32,
        storey_height: f32,
        outline: &Polygon,
    ) -> Floor {
        let g = &self.grid;
        let f = &self.frame;
        let rooms = (0..lay.room_count())
            .map(|r| {
                let loops = grid::trace_loops(g.nx, g.ny, |c| g.in_bounds(c) && lay.labels[g.idx(c)] == r as i32);
                let best = loops.into_iter().max_by_key(|l| grid::loop_area(l)).unwrap_or_default();
                let pts: Vec<Vec2> = best.iter().map(|&v| g.vertex(v)).collect();
                Room { id: base_id + r as u32, kind: lay.kinds[r], shape: self.poly_to_plan(&pts) }
            })
            .collect();
        let walls = walls
            .iter()
            .map(|w| WallSegment { a: f.to_plan(w.a), b: f.to_plan(w.b), thickness: w.thickness, exterior: w.exterior })
            .collect();
        let openings = openings
            .iter()
            .map(|o| Opening {
                kind: o.kind,
                a: f.to_plan(o.a),
                b: f.to_plan(o.b),
                sill: o.sill,
                height: o.height,
                rooms: o.rooms.map(|r| r.map(|r| base_id + r as u32)),
            })
            .collect();
        let stairs = flights
            .iter()
            .filter(|s| s.from == lay.level)
            .map(|s| Stair {
                footprint: self.poly_to_plan(&[s.min, Vec2::new(s.max.x, s.min.y), s.max, Vec2::new(s.min.x, s.max.y)]),
                up: f.dir_to_plan(s.up),
                from_level: s.from,
                to_level: s.to,
            })
            .collect();
        let props = props
            .iter()
            .map(|pr| {
                let facing = f.dir_to_plan(pr.facing);
                Prop {
                    kind: pr.kind,
                    room: base_id + pr.room as u32,
                    pos: f.to_plan(pr.center()),
                    size: pr.size(),
                    rotation: (-facing.x).atan2(facing.y),
                }
            })
            .collect();
        Floor {
            level: lay.level,
            elevation: lay.level as f32 * storey_height,
            height: storey_height - FLOOR_THICKNESS,
            outline: outline.clone(),
            rooms,
            walls,
            openings,
            stairs,
            props,
        }
    }
}

/// Finite, de-duplicated, CCW copy of a footprint, or `None` if degenerate.
fn clean_polygon(poly: &Polygon) -> Option<Polygon> {
    let pts: Vec<Vec2> = poly.0.iter().copied().filter(|p| p.is_finite()).collect();
    if pts.len() < 3 {
        return None;
    }
    let mut p = Polygon(pts).simplified(1e-3);
    if p.len() < 3 {
        return None;
    }
    p.make_ccw();
    let b = p.bounds().size();
    (p.area() >= 4.0 && b.x.min(b.y) >= 1.5 && b.x.max(b.y) < 2000.0).then_some(p)
}

/// Edge to align the planning grid with: the longest edge facing the street
/// (outward normal within ~45° of `front`), else the longest edge.
fn choose_edge(poly: &Polygon, front: Vec2) -> (Vec2, Vec2) {
    let mut best: Option<(f32, (Vec2, Vec2))> = None;
    if front.length_squared() > 0.5 {
        for (a, b) in poly.edges() {
            let d = (b - a).normalize_or_zero();
            let outward = Vec2::new(d.y, -d.x);
            let len = a.distance(b);
            if outward.dot(front) > 0.7 && best.is_none_or(|(l, _)| len > l) {
                best = Some((len, (a, b)));
            }
        }
    }
    match best {
        Some((_, e)) => e,
        None => {
            let i = poly.longest_edge();
            (poly.at(i as isize), poly.at(i as isize + 1))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_names() {
        assert_eq!(level_name(-1), "Cellar");
        assert_eq!(level_name(0), "Ground floor");
        assert_eq!(level_name(2), "Second floor");
    }

    #[test]
    fn sanitize_clamps() {
        let p = sanitize(&DwellingParams { floors: 0, storey_height: f32::NAN, cell_size: 99.0, front: Vec2::new(3.0, 4.0), ..Default::default() });
        assert_eq!(p.floors, 1);
        assert_eq!(p.storey_height, 3.0);
        assert_eq!(p.cell_size, 2.0);
        assert!((p.front.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn edge_choice_prefers_street() {
        // 10 × 6 rectangle; street to the west (short side).
        let poly = Polygon::rect(Vec2::ZERO, Vec2::new(10.0, 6.0));
        let (a, b) = choose_edge(&poly, Vec2::new(-1.0, 0.0));
        assert!((a.x + 5.0).abs() < 1e-5 && (b.x + 5.0).abs() < 1e-5);
        let (a, b) = choose_edge(&poly, Vec2::ZERO);
        assert!((a.distance(b) - 10.0).abs() < 1e-5);
    }
}
