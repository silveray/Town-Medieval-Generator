//! Floor layout on the planning grid: stairwell placement, corridor spine,
//! recursive BSP of the remaining cells into the room programme, and a repair
//! pass that guarantees every room can be reached through a door.

use mde_core::{DwellingStyle, RoomKind, Rng};

use crate::frame::Side;
use crate::grid::{self, Axis, CellRect, Grid};
use crate::program::{level_program, LevelRole, RoomSpec};
use crate::runs::{self, Run, JUNCTION_CLEARANCE, OUTSIDE};
use crate::{DOOR_MIN_WIDTH, INTERIOR_WALL};

/// A straight stacked stairwell: two lanes side by side along `axis`. Flight
/// `k` (from the `k`-th lowest level) climbs in lane `k % 2`, starting at end
/// `(first_end + k) % 2` (0 = low end) and arriving at the other end one level
/// up, so every level enters and leaves the stairwell at the same end.
/// Landings are the cells just beyond each end, outside the stairwell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StairWell {
    /// Cells of the whole stairwell.
    pub rect: CellRect,
    /// Axis the flights run along.
    pub axis: Axis,
    /// Cells per lane, across the axis.
    pub lane_cells: i32,
    /// Depth of each landing zone in cells (along the axis).
    pub landing_cells: i32,
    /// End the lowest flight starts from.
    pub first_end: usize,
}

impl StairWell {
    /// Cells of lane `k` (0 = low side across the axis).
    pub fn lane(&self, k: usize) -> CellRect {
        let across = self.axis.other();
        let lo = self.rect.lo(across) + k as i32 * self.lane_cells;
        CellRect::from_axis(self.axis, (self.rect.lo(self.axis), self.rect.hi(self.axis)), (lo, lo + self.lane_cells))
    }

    /// Landing zone beyond end `e` (0 = low end along the axis, 1 = high end).
    pub fn landing(&self, e: usize) -> CellRect {
        let a = self.axis;
        let along = if e == 0 {
            (self.rect.lo(a) - self.landing_cells, self.rect.lo(a))
        } else {
            (self.rect.hi(a), self.rect.hi(a) + self.landing_cells)
        };
        CellRect::from_axis(a, along, (self.rect.lo(a.other()), self.rect.hi(a.other())))
    }

    /// End flight `k` starts from.
    pub fn start_end(&self, k: usize) -> usize {
        (self.first_end + k) % 2
    }

    /// End used (arrival and departure) on the `m`-th lowest level.
    pub fn active_end(&self, m: usize) -> usize {
        (self.first_end + m) % 2
    }

    /// Grid line index of end `e`.
    pub fn end_line(&self, e: usize) -> i32 {
        if e == 0 { self.rect.lo(self.axis) } else { self.rect.hi(self.axis) }
    }

    /// Whether `run` is the end wall at end `e` (between the stairwell and its landing).
    pub fn is_end_run(&self, run: &Run, e: usize) -> bool {
        run.axis == self.axis.other() && run.line == self.end_line(e)
    }
}

/// Normalised depth of a cell from the front (0) to the back (1).
pub(crate) fn depth_of(g: &Grid, front: Side, c: (i32, i32)) -> f32 {
    match front {
        Side::South => (c.1 as f32 + 0.5) / g.ny as f32,
        Side::North => 1.0 - (c.1 as f32 + 0.5) / g.ny as f32,
        Side::West => (c.0 as f32 + 0.5) / g.nx as f32,
        Side::East => 1.0 - (c.0 as f32 + 0.5) / g.nx as f32,
    }
}

/// Normalised lateral position of a cell across the front (0..1).
fn lateral_of(g: &Grid, front: Side, c: (i32, i32)) -> f32 {
    match front {
        Side::South | Side::North => (c.0 as f32 + 0.5) / g.nx as f32,
        Side::West | Side::East => (c.1 as f32 + 0.5) / g.ny as f32,
    }
}

/// Neighbour of cell `c` across `side`.
fn step(c: (i32, i32), side: Side) -> (i32, i32) {
    match side {
        Side::South => (c.0, c.1 - 1),
        Side::North => (c.0, c.1 + 1),
        Side::West => (c.0 - 1, c.1),
        Side::East => (c.0 + 1, c.1),
    }
}

/// Cells of `member` (grid-sized) covered by at least one `k`×`k` block lying
/// entirely inside it: a morphological opening that strips slivers narrower
/// than `k` cells.
pub(crate) fn thick_cells(g: &Grid, member: &[bool], k: i32) -> Vec<bool> {
    let mut thick = vec![false; member.len()];
    for j in 0..=(g.ny - k) {
        for i in 0..=(g.nx - k) {
            if (0..k).all(|dj| (0..k).all(|di| member[g.idx((i + di, j + dj))])) {
                for dj in 0..k {
                    for di in 0..k {
                        thick[g.idx((i + di, j + dj))] = true;
                    }
                }
            }
        }
    }
    thick
}

/// Choose the stairwell cells. `ground_index` is the position of level 0 in
/// the level list (1 with a basement). `None` if no placement keeps both
/// landings inside the house and the rest of the floor connected.
pub(crate) fn place_stairwell(g: &Grid, style: DwellingStyle, front: Side, storey_height: f32, ground_index: usize, rng: &mut Rng) -> Option<StairWell> {
    let widths: &[bool] = if style == DwellingStyle::Manor { &[true, false] } else { &[false] };
    let (target_depth, target_side) = match style {
        DwellingStyle::Cottage => (0.75, 1.0),
        DwellingStyle::Townhouse => (0.5, 1.0),
        DwellingStyle::Farmhouse => (0.65, 0.3),
        DwellingStyle::Manor => (0.5, 0.0),
        DwellingStyle::Tavern => (0.8, 0.7),
        DwellingStyle::Shop => (0.65, 1.0),
    };
    // Where the ground floor should meet the stair (depth 0 front .. 1 back).
    let ground_landing_depth = match style {
        DwellingStyle::Townhouse | DwellingStyle::Shop | DwellingStyle::Tavern => Some(1.0),
        _ => None,
    };
    for &wide in widths {
        let mut best: Option<(f32, StairWell)> = None;
        for axis in [Axis::X, Axis::Y] {
            let across = axis.other();
            let ca = g.size(axis);
            let run = ((storey_height * 0.92 / ca).round() as i32).max((storey_height * 0.68 / ca).ceil() as i32).max(2);
            let lane_cells = if wide { g.cells_for(across, 1.25) } else { g.cells_for(across, 0.8) };
            let landing_cells = g.cells_for(axis, 0.75);
            let width = 2 * lane_cells;
            for a0 in landing_cells..=(g.n(axis) - run - landing_cells) {
                for b0 in 0..=(g.n(across) - width) {
                    let mut s = StairWell {
                        rect: CellRect::from_axis(axis, (a0, a0 + run), (b0, b0 + width)),
                        axis,
                        lane_cells,
                        landing_cells,
                        first_end: 0,
                    };
                    if !s.rect.cells().all(|c| g.inside(c)) {
                        continue;
                    }
                    if !(0..2).all(|e| s.landing(e).cells().all(|c| g.inside(c))) {
                        continue;
                    }
                    // Must touch the exterior along a long side so no room can
                    // wrap all the way around it.
                    let touches = (a0..a0 + run).any(|a| !g.inside(axis.cell(a, b0 - 1)) || !g.inside(axis.cell(a, b0 + width)));
                    if !touches {
                        continue;
                    }
                    let mut member = g.mask.clone();
                    for c in s.rect.cells() {
                        member[g.idx(c)] = false;
                    }
                    let rest: Vec<(i32, i32)> = (0..member.len()).filter(|&k| member[k]).map(|k| g.coords(k)).collect();
                    if rest.is_empty() || !grid::is_connected(&rest) {
                        continue;
                    }
                    let cells: Vec<(i32, i32)> = s.rect.cells().collect();
                    let n = cells.len() as f32;
                    let depth = cells.iter().map(|&c| depth_of(g, front, c)).sum::<f32>() / n;
                    let lateral = cells.iter().map(|&c| lateral_of(g, front, c)).sum::<f32>() / n;
                    let sideness = ((lateral - 0.5).abs() * 2.0).min(1.0);
                    let mut score = (depth - target_depth).abs() * 3.0 + (sideness - target_side).abs() * 1.5;
                    if cells.iter().any(|&c| !g.inside(step(c, front))) {
                        score += 4.0;
                    }
                    // No narrow necks: the rest of the floor must stay connected
                    // through ≥ 2-cell-wide space, landings included.
                    let thick = thick_cells(g, &member, 2);
                    if grid::components(g.nx, g.ny, |c| thick[g.idx(c)]).len() > 1 {
                        score += 6.0;
                    }
                    for e in 0..2 {
                        if !s.landing(e).cells().any(|c| thick[g.idx(c)]) {
                            score += 2.0;
                        }
                    }
                    score += rng.next_f32() * 0.6;
                    // Which end the ground floor uses: one that opens onto the
                    // house rather than into a corner pocket, at the preferred
                    // depth for the style.
                    let mut end_scores = [0.0f32; 2];
                    for (first_end, es) in end_scores.iter_mut().enumerate() {
                        let ge = (first_end + ground_index) % 2;
                        let l = s.landing(ge);
                        let near = CellRect::new(l.i0 - 2, l.j0 - 2, l.i1 + 2, l.j1 + 2);
                        let open = near.cells().filter(|&c| g.in_bounds(c) && member[g.idx(c)]).count();
                        *es = -(open as f32) * 0.3 + rng.next_f32() * 0.1;
                        if let Some(t) = ground_landing_depth {
                            let d = l.cells().map(|c| depth_of(g, front, c)).sum::<f32>() / l.cells().count().max(1) as f32;
                            *es += (d - t).abs() * 3.0;
                        }
                    }
                    s.first_end = if end_scores[1] < end_scores[0] { 1 } else { 0 };
                    score += end_scores[s.first_end];
                    if best.as_ref().is_none_or(|(b, _)| score < *b) {
                        best = Some((score, s));
                    }
                }
            }
        }
        if let Some((_, s)) = best {
            return Some(s);
        }
    }
    None
}

/// Rooms of one level on the grid.
#[derive(Clone, Debug)]
pub(crate) struct LevelLayout {
    pub level: i32,
    /// Room index per grid cell, `OUTSIDE` for cells not in the house.
    pub labels: Vec<i32>,
    /// Kind per room index.
    pub kinds: Vec<RoomKind>,
    pub stairwell: Option<usize>,
    pub corridor: Option<usize>,
    /// Landing zone used on this level (at the stairwell's active end).
    pub landing: Option<CellRect>,
    /// Active stairwell end on this level.
    pub active_end: usize,
    /// Room the level is entered from: the entry room on the ground floor,
    /// the landing room elsewhere.
    pub root: usize,
}

impl LevelLayout {
    /// Cells of room `r`.
    pub fn cells_of(&self, g: &Grid, r: usize) -> Vec<(i32, i32)> {
        (0..self.labels.len()).filter(|&k| self.labels[k] == r as i32).map(|k| g.coords(k)).collect()
    }

    pub fn room_count(&self) -> usize {
        self.kinds.len()
    }

    /// Label of the cell containing local point `p` (`OUTSIDE` if none).
    pub fn labels_side(&self, g: &Grid, p: mde_core::Vec2) -> i32 {
        let i = ((p.x - g.x0) / g.cx).floor() as i32;
        let j = ((p.y - g.y0) / g.cy).floor() as i32;
        runs::label_at(g, &self.labels, (i, j))
    }
}

/// Inputs shared by all levels.
pub(crate) struct LayoutCtx<'a> {
    pub grid: &'a Grid,
    pub front: Side,
    pub stair: Option<&'a StairWell>,
    pub style: DwellingStyle,
    pub floors: u8,
}

/// Lay out level `level`, which is the `index`-th lowest level of the house.
pub(crate) fn layout_level(ctx: &LayoutCtx, level: i32, index: usize, rng: &mut Rng) -> LevelLayout {
    let g = ctx.grid;
    let active_end = ctx.stair.map(|s| s.active_end(index)).unwrap_or(0);
    let in_stair = |c: (i32, i32)| ctx.stair.is_some_and(|s| s.rect.contains(c));
    let region: Vec<(i32, i32)> = g.cells().into_iter().filter(|&c| !in_stair(c)).collect();
    let landing = ctx.stair.map(|s| s.landing(active_end));
    let role = LevelRole::of(level, ctx.floors);
    let area = region.len() as f32 * g.cell_area();

    // Corridor first (it takes area from the programme).
    let prog_rng = rng.clone();
    let mut program = level_program(ctx.style, role, level, ctx.floors, area, rng);
    let mut corridor_cells: Vec<(i32, i32)> = Vec::new();
    if program.corridor
        && let Some(cells) = build_corridor(g, &region, landing)
    {
        let left = area - cells.len() as f32 * g.cell_area();
        let refit = level_program(ctx.style, role, level, ctx.floors, left, &mut prog_rng.clone());
        if refit.rooms.len() >= 2 {
            program = refit;
            corridor_cells = cells;
        }
    }

    let mut corridor_mask = vec![false; g.mask.len()];
    for &c in &corridor_cells {
        corridor_mask[g.idx(c)] = true;
    }
    let rest: Vec<(i32, i32)> = region.iter().copied().filter(|&c| !corridor_mask[g.idx(c)]).collect();
    let mut regions = grid::components(g.nx, g.ny, |c| rest.binary_search_by_key(&(c.1, c.0), |r| (r.1, r.0)).is_ok());
    if !corridor_cells.is_empty() {
        // Slivers left beside the corridor (no 2×2 block) become alcoves of it.
        regions.retain(|r| {
            let mut member = vec![false; g.mask.len()];
            for &c in r {
                member[g.idx(c)] = true;
            }
            let usable = thick_cells(g, &member, 2).iter().any(|&t| t);
            if !usable {
                for &c in r {
                    corridor_mask[g.idx(c)] = true;
                    corridor_cells.push(c);
                }
            }
            usable
        });
    }
    regions.sort_by_key(|r| std::cmp::Reverse(r.len()));

    let reserved: Vec<CellRect> = landing.into_iter().collect();
    let bctx = BspCtx {
        grid: g,
        front: ctx.front,
        reserved,
        corridor: (!corridor_cells.is_empty()).then_some(corridor_mask.clone()),
        ground: level == 0,
        stair_from_entry: match ctx.style {
            DwellingStyle::Manor => 6.0,
            DwellingStyle::Cottage | DwellingStyle::Farmhouse => 1.0,
            _ => 0.0,
        },
    };

    let groups = allocate(g, ctx.front, &regions, &program.rooms);
    let mut leaves: Vec<(Vec<(i32, i32)>, RoomSpec)> = Vec::new();
    for (cells, rooms) in regions.into_iter().zip(groups) {
        if rooms.is_empty() {
            // Leftover pocket: storage closet; repair merges it if unusable.
            let kind = if level < 0 { RoomKind::Cellar } else { RoomKind::Storage };
            let spec = RoomSpec { kind, weight: 1.0, depth: 0.5, min_area: 0.0, priority: 9, entry: false };
            leaves.push((cells, spec));
        } else {
            bsp(&bctx, cells, rooms, rng, &mut leaves);
        }
    }

    // Assemble labels: stairwell, corridor, then rooms.
    let mut labels = vec![OUTSIDE; g.mask.len()];
    let mut kinds = Vec::new();
    let mut stairwell = None;
    if let Some(s) = ctx.stair {
        stairwell = Some(kinds.len());
        for c in s.rect.cells() {
            labels[g.idx(c)] = kinds.len() as i32;
        }
        kinds.push(RoomKind::Stairwell);
    }
    let mut corridor = None;
    if !corridor_cells.is_empty() {
        corridor = Some(kinds.len());
        for &c in &corridor_cells {
            labels[g.idx(c)] = kinds.len() as i32;
        }
        kinds.push(RoomKind::Corridor);
    }
    let mut entry_room = None;
    for (cells, spec) in &leaves {
        if spec.entry {
            entry_room = Some(kinds.len());
        }
        for &c in cells {
            labels[g.idx(c)] = kinds.len() as i32;
        }
        kinds.push(spec.kind);
    }

    let mut lay = LevelLayout { level, labels, kinds, stairwell, corridor, landing, active_end, root: 0 };
    lay.root = pick_root(&lay, g, ctx.front, entry_room);
    repair(&mut lay, g, ctx.stair);
    // A privy is a closet; anything bigger reads better as storage.
    for r in 0..lay.kinds.len() {
        if lay.kinds[r] == RoomKind::Privy && lay.cells_of(g, r).len() as f32 * g.cell_area() > 5.0 {
            lay.kinds[r] = RoomKind::Storage;
        }
    }
    // A closet that ended up holding the stair landing is really a stair hall.
    if level >= 0
        && let Some(c) = landing.and_then(|l| l.cells().next())
        && let Ok(r) = usize::try_from(lay.labels[g.idx(c)])
        && matches!(lay.kinds[r], RoomKind::Privy | RoomKind::Pantry | RoomKind::Storage | RoomKind::Bath)
        && lay.cells_of(g, r).len() as f32 * g.cell_area() <= 14.0
    {
        lay.kinds[r] = if level == 0 { RoomKind::Hall } else { RoomKind::Corridor };
    }
    lay
}

/// Root room: the entry room on the ground floor (or the room with the most
/// street frontage), the landing room on other levels.
fn pick_root(lay: &LevelLayout, g: &Grid, front: Side, entry: Option<usize>) -> usize {
    if lay.level != 0 {
        if let Some(c) = lay.landing.and_then(|l| l.cells().next())
            && lay.labels[g.idx(c)] >= 0
        {
            return lay.labels[g.idx(c)] as usize;
        }
    } else if let Some(e) = entry {
        return e;
    }
    // Most front-facing exterior cells, skipping the stairwell.
    let mut best = (0usize, if lay.stairwell == Some(0) && lay.kinds.len() > 1 { 1 } else { 0 });
    for r in 0..lay.kinds.len() {
        if Some(r) == lay.stairwell {
            continue;
        }
        let n = lay.cells_of(g, r).into_iter().filter(|&c| !g.inside(step(c, front))).count();
        if n > best.0 {
            best = (n, r);
        }
    }
    best.1
}

/// A set of grid cells.
type Cells = Vec<(i32, i32)>;

/// Region shared by the BSP recursion.
struct BspCtx<'a> {
    grid: &'a Grid,
    front: Side,
    /// Cell rectangles no cut may pass through (landing zones).
    reserved: Vec<CellRect>,
    /// Corridor cells (grid-sized mask), if the level has a corridor.
    corridor: Option<Vec<bool>>,
    ground: bool,
    /// Penalty for a ground-floor stair landing outside the entry room.
    stair_from_entry: f32,
}

/// Measurements of one side of a candidate cut.
struct SideStats {
    count: usize,
    /// Minimum thickness normal to the cut, cells.
    thickness: i32,
    /// Bounding box size in cells (x, y).
    bbox: (i32, i32),
    depth: f32,
    front_cells: usize,
    corridor_touch: usize,
    /// Contains part of a stair landing zone.
    landing: bool,
}

fn side_stats(ctx: &BspCtx, cells: &[(i32, i32)], member: &[bool], axis: Axis, k: i32, low: bool) -> SideStats {
    let g = ctx.grid;
    let (mut i0, mut j0, mut i1, mut j1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    let mut depth = 0.0;
    let mut front_cells = 0;
    let mut corridor_touch = 0;
    let mut landing = false;
    for &c in cells {
        i0 = i0.min(c.0);
        j0 = j0.min(c.1);
        i1 = i1.max(c.0);
        j1 = j1.max(c.1);
        depth += depth_of(g, ctx.front, c);
        if !g.inside(step(c, ctx.front)) {
            front_cells += 1;
        }
        if let Some(cm) = &ctx.corridor
            && grid::neighbours4(c).iter().any(|&n| g.in_bounds(n) && cm[g.idx(n)])
        {
            corridor_touch += 1;
        }
        if ctx.reserved.iter().any(|r| r.contains(c)) {
            landing = true;
        }
    }
    // Thickness away from the cut line.
    let mut thickness = i32::MAX;
    let edge_a = if low { k - 1 } else { k };
    let dir = if low { -1 } else { 1 };
    for &c in cells {
        if axis.of(c) != edge_a {
            continue;
        }
        let mut t = 0;
        let mut cur = c;
        while g.in_bounds(cur) && member[g.idx(cur)] {
            t += 1;
            cur = match axis {
                Axis::X => (cur.0 + dir, cur.1),
                Axis::Y => (cur.0, cur.1 + dir),
            };
        }
        thickness = thickness.min(t);
    }
    SideStats {
        count: cells.len(),
        thickness,
        bbox: (i1 - i0 + 1, j1 - j0 + 1),
        depth: depth / cells.len().max(1) as f32,
        front_cells,
        corridor_touch,
        landing,
    }
}

/// Penalty for a room kind holding the stair landing (the room everyone
/// passes through on that floor).
fn landing_penalty(kind: RoomKind) -> f32 {
    use RoomKind as K;
    match kind {
        K::Privy | K::Pantry | K::Bath | K::Storage => 5.0,
        K::Bedroom | K::Nursery | K::Study | K::Library | K::Chapel | K::Servants => 1.2,
        _ => 0.0,
    }
}

/// Recursive binary space partition of `cells` into `rooms`.
fn bsp(ctx: &BspCtx, cells: Vec<(i32, i32)>, mut rooms: Vec<RoomSpec>, rng: &mut Rng, out: &mut Vec<(Vec<(i32, i32)>, RoomSpec)>) {
    let g = ctx.grid;
    if rooms.len() <= 1 || cells.len() < 2 {
        out.push((cells, merge_specs(rooms)));
        return;
    }
    // Front-most first so contiguous groups follow the front→back order.
    rooms.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    let n = rooms.len();
    let total_w: f32 = rooms.iter().map(|r| r.weight).sum::<f32>().max(1e-3);
    let cell_area = g.cell_area();
    let door_run = DOOR_MIN_WIDTH + 2.0 * (INTERIOR_WALL * 0.5 + JUNCTION_CLEARANCE);

    let mut best: Option<(f32, Axis, i32, usize, bool)> = None;
    for axis in [Axis::X, Axis::Y] {
        let lo_b = cells.iter().map(|&c| axis.of(c)).min().unwrap_or(0);
        let hi_b = cells.iter().map(|&c| axis.of(c)).max().unwrap_or(0);
        let normal_cells = g.cells_for(axis, 1.8);
        let small_cells = g.cells_for(axis, 0.85);
        let par_normal = g.cells_for(axis.other(), 1.8);
        let par_small = g.cells_for(axis.other(), 0.85);
        let need_shared = g.cells_for(axis.other(), door_run);
        for k in (lo_b + 1)..=hi_b {
            if ctx.reserved.iter().any(|r| r.lo(axis) < k && k < r.hi(axis)) {
                continue;
            }
            let (low, high): (Cells, Cells) = cells.iter().partition(|&&c| axis.of(c) < k);
            if low.is_empty() || high.is_empty() || !grid::is_connected(&low) || !grid::is_connected(&high) {
                continue;
            }
            let mut m_low = vec![false; g.mask.len()];
            let mut m_high = vec![false; g.mask.len()];
            for &c in &low {
                m_low[g.idx(c)] = true;
            }
            for &c in &high {
                m_high[g.idx(c)] = true;
            }
            let shared = low
                .iter()
                .filter(|&&c| axis.of(c) == k - 1)
                .filter(|&&c| {
                    let n = match axis {
                        Axis::X => (c.0 + 1, c.1),
                        Axis::Y => (c.0, c.1 + 1),
                    };
                    g.in_bounds(n) && m_high[g.idx(n)]
                })
                .count() as i32;
            let sl = side_stats(ctx, &low, &m_low, axis, k, true);
            let sh = side_stats(ctx, &high, &m_high, axis, k, false);
            // Sides that can only meet through a sliver need a detour.
            let via_corridor = ctx.corridor.is_some() && sl.corridor_touch >= 2 && sh.corridor_touch >= 2;
            let neck = if shared < need_shared && !via_corridor { 6.0 } else { 0.0 };
            for s in 1..n {
                for flip in [false, true] {
                    let (gl, gh) = if flip { (&rooms[s..], &rooms[..s]) } else { (&rooms[..s], &rooms[s..]) };
                    let mut feasible = true;
                    let mut score = neck;
                    for (grp, st) in [(gl, &sl), (gh, &sh)] {
                        let small = grp.len() == 1 && grp[0].is_small_kind();
                        let (need, need_par) = if small { (small_cells, par_small) } else { (normal_cells, par_normal) };
                        let par = match axis {
                            Axis::X => st.bbox.1,
                            Axis::Y => st.bbox.0,
                        };
                        if st.thickness < need || par < need_par {
                            feasible = false;
                            break;
                        }
                        let area = st.count as f32 * cell_area;
                        let min_area: f32 = grp.iter().map(|r| r.min_area).sum();
                        if min_area > area {
                            score += 2.0 + (min_area - area) * 0.6;
                        }
                        score += grp.iter().map(|r| (r.depth - st.depth).abs()).sum::<f32>() * 2.5 / n as f32;
                        if ctx.ground && grp.iter().any(|r| r.entry) && st.front_cells < 2 {
                            score += 6.0;
                        }
                        if ctx.corridor.is_some() {
                            if grp.len() == 1 && st.corridor_touch < 2 {
                                score += 3.0;
                            } else if st.corridor_touch == 0 {
                                score += 2.0 * grp.len() as f32;
                            }
                        }
                        // Steer the entry room towards the stair landing at every
                        // level of the recursion, not just the last split.
                        if st.landing && ctx.ground && !grp.iter().any(|r| r.entry) {
                            score += ctx.stair_from_entry;
                        }
                        if grp.len() == 1 {
                            if st.landing {
                                score += landing_penalty(grp[0].kind);
                            }
                            let w = st.bbox.0 as f32 * g.cx;
                            let h = st.bbox.1 as f32 * g.cy;
                            let aspect = w.max(h) / w.min(h).max(1e-3);
                            score += (aspect - 2.0).max(0.0) * 3.0;
                            let fill = st.count as f32 / (st.bbox.0 * st.bbox.1).max(1) as f32;
                            score += (0.8 - fill).max(0.0) * 5.0;
                        }
                    }
                    if !feasible {
                        continue;
                    }
                    let w_low: f32 = gl.iter().map(|r| r.weight).sum();
                    let ratio = sl.count as f32 / cells.len() as f32;
                    score += (ratio - w_low / total_w).abs() * 8.0;
                    score += rng.next_f32() * 0.35;
                    if best.is_none_or(|b| score < b.0) {
                        best = Some((score, axis, k, s, flip));
                    }
                }
            }
        }
    }
    let Some((_, axis, k, s, flip)) = best else {
        out.push((cells, merge_specs(rooms)));
        return;
    };
    let (low, high): (Cells, Cells) = cells.into_iter().partition(|&c| axis.of(c) < k);
    let second = rooms.split_off(s);
    let (gl, gh) = if flip { (second, rooms) } else { (rooms, second) };
    bsp(ctx, low, gl, rng, out);
    bsp(ctx, high, gh, rng, out);
}

/// Collapse several specs into one leaf: keep the entry room, else the most
/// important one, with the combined weight.
fn merge_specs(rooms: Vec<RoomSpec>) -> RoomSpec {
    let weight: f32 = rooms.iter().map(|r| r.weight).sum();
    let keep = rooms
        .iter()
        .enumerate()
        .min_by_key(|(k, r)| (!r.entry, r.priority, *k))
        .map(|(_, r)| r.clone())
        .unwrap_or(RoomSpec { kind: RoomKind::Storage, weight: 1.0, depth: 0.5, min_area: 0.0, priority: 9, entry: false });
    RoomSpec { weight, ..keep }
}

/// Distribute rooms over regions (largest first) by area share and depth.
fn allocate(g: &Grid, front: Side, regions: &[Vec<(i32, i32)>], rooms: &[RoomSpec]) -> Vec<Vec<RoomSpec>> {
    let mut groups: Vec<Vec<RoomSpec>> = vec![Vec::new(); regions.len()];
    if regions.is_empty() {
        return groups;
    }
    let cell_area = g.cell_area();
    let areas: Vec<f32> = regions.iter().map(|r| r.len() as f32 * cell_area).collect();
    let depths: Vec<f32> = regions.iter().map(|r| r.iter().map(|&c| depth_of(g, front, c)).sum::<f32>() / r.len().max(1) as f32).collect();
    let total_area: f32 = areas.iter().sum::<f32>().max(1e-3);
    let total_w: f32 = rooms.iter().map(|r| r.weight).sum::<f32>().max(1e-3);
    let mut order: Vec<usize> = (0..rooms.len()).collect();
    order.sort_by_key(|&k| (rooms[k].priority, k));
    for k in order {
        let room = &rooms[k];
        let mut best: Option<(f32, usize)> = None;
        for (r, &a) in areas.iter().enumerate() {
            let used_min: f32 = groups[r].iter().map(|s| s.min_area).sum();
            let fits = a - used_min >= room.min_area * 0.9;
            let assigned: f32 = groups[r].iter().map(|s| s.weight).sum();
            let deficit = (a / total_area * total_w - assigned) / total_w;
            let score = deficit - (room.depth - depths[r]).abs() * 0.5 + if fits { 1000.0 } else { 0.0 };
            if best.is_none_or(|b| score > b.0) {
                best = Some((score, r));
            }
        }
        if let Some((_, r)) = best {
            groups[r].push(room.clone());
        }
    }
    groups
}

/// Score of a corridor candidate (lower is better): its own area, unusable
/// pockets and slivers it leaves, and how deep the rooms reach from it.
fn corridor_score(g: &Grid, in_region: &[bool], cor: &[bool]) -> f32 {
    let cell_area = g.cell_area();
    let rest: Vec<bool> = (0..cor.len()).map(|k| in_region[k] && !cor[k]).collect();
    let mut s = cor.iter().filter(|&&c| c).count() as f32 * cell_area * 0.12;
    let wide = thick_cells(g, &rest, 3);
    let thick = thick_cells(g, &rest, 2);
    for comp in grid::components(g.nx, g.ny, |c| rest[g.idx(c)]) {
        let area = comp.len() as f32 * cell_area;
        if area < 4.0 {
            s += 5.0;
        }
        let narrow = comp.iter().filter(|&&c| !wide[g.idx(c)]).count() as f32;
        let thin = comp.iter().filter(|&&c| !thick[g.idx(c)]).count() as f32;
        s += narrow * cell_area * 0.25 + thin * cell_area * 2.5;
    }
    // Mean walking distance (metres) from the corridor into the rooms.
    let mut dist = vec![-1i32; cor.len()];
    let mut queue = std::collections::VecDeque::new();
    for k in 0..cor.len() {
        if cor[k] {
            dist[k] = 0;
            queue.push_back(g.coords(k));
        }
    }
    while let Some(c) = queue.pop_front() {
        let d = dist[g.idx(c)];
        for n in grid::neighbours4(c) {
            if g.in_bounds(n) && rest[g.idx(n)] && dist[g.idx(n)] < 0 {
                dist[g.idx(n)] = d + 1;
                queue.push_back(n);
            }
        }
    }
    // Rooms should open off the corridor: floor area far from it means
    // rooms reached through other rooms.
    let step = g.cx.max(g.cy);
    let (mut sum, mut cnt) = (0.0f32, 0usize);
    for k in 0..cor.len() {
        if rest[k] {
            let d = dist[k].max(0) as f32 * step;
            sum += d;
            cnt += 1;
            s += (d - 4.5).max(0.0) * cell_area * 0.5;
        }
    }
    if cnt > 0 {
        s += sum / cnt as f32 * 0.6;
    }
    s
}

/// Shortest 4-connected path through `in_region` from any `from` cell to any
/// `to` cell, excluding the start and end cells. `None` if unreachable.
fn shortest_path(g: &Grid, in_region: &[bool], from: &[(i32, i32)], to: &[bool]) -> Option<Vec<(i32, i32)>> {
    let mut prev: Vec<i64> = vec![-2; in_region.len()];
    let mut queue = std::collections::VecDeque::new();
    for &c in from {
        if g.in_bounds(c) && in_region[g.idx(c)] {
            prev[g.idx(c)] = -1;
            queue.push_back(c);
        }
    }
    while let Some(c) = queue.pop_front() {
        for n in grid::neighbours4(c) {
            if !g.in_bounds(n) || !in_region[g.idx(n)] || prev[g.idx(n)] != -2 {
                continue;
            }
            prev[g.idx(n)] = g.idx(c) as i64;
            if to[g.idx(n)] {
                let mut path = Vec::new();
                let mut k = g.idx(c) as i64;
                while k >= 0 && prev[k as usize] != -1 {
                    path.push(g.coords(k as usize));
                    k = prev[k as usize];
                }
                return Some(path);
            }
            queue.push_back(n);
        }
    }
    None
}

/// Corridor of ≥ 1 m clear width: a straight spine through the floor plus a
/// connector from the stair landing. Every spine position along both axes is
/// scored; returns the best corridor's cells, or `None` if nothing fits.
fn build_corridor(g: &Grid, region: &[(i32, i32)], landing: Option<CellRect>) -> Option<Vec<(i32, i32)>> {
    let mut in_region = vec![false; g.mask.len()];
    for &c in region {
        in_region[g.idx(c)] = true;
    }
    let mut best: Option<(f32, Vec<bool>)> = None;
    for axis in [Axis::X, Axis::Y] {
        let b = axis.other();
        let w = g.cells_for(b, 1.0 + INTERIOR_WALL + 0.05);
        for b0 in 0..=(g.n(b) - w) {
            let mut spine = vec![false; g.mask.len()];
            for a in 0..g.n(axis) {
                if (b0..b0 + w).all(|bb| in_region[g.idx(axis.cell(a, bb))]) {
                    for bb in b0..b0 + w {
                        spine[g.idx(axis.cell(a, bb))] = true;
                    }
                }
            }
            let Some(main) = grid::components(g.nx, g.ny, |c| spine[g.idx(c)]).into_iter().max_by_key(|c| c.len()) else {
                continue;
            };
            if (main.len() as i32) < 4 * w {
                continue;
            }
            let mut cor = vec![false; g.mask.len()];
            for &c in &main {
                cor[g.idx(c)] = true;
            }
            if let Some(l) = landing {
                let lcells: Vec<(i32, i32)> = l.cells().filter(|&c| g.in_bounds(c) && in_region[g.idx(c)]).collect();
                let touching = lcells.iter().any(|&c| cor[g.idx(c)] || grid::neighbours4(c).iter().any(|&n| g.in_bounds(n) && cor[g.idx(n)]));
                for &c in &lcells {
                    cor[g.idx(c)] = true;
                }
                if !touching {
                    let Some(path) = shortest_path(g, &in_region, &lcells, &cor) else { continue };
                    for c in path {
                        // Widen the connector with a w×w brush kept inside the region.
                        let mut placed = false;
                        'brush: for dy in 0..w {
                            for dx in 0..w {
                                let o = (c.0 - dx, c.1 - dy);
                                let ok = (0..w).all(|j| (0..w).all(|i| {
                                    let q = (o.0 + i, o.1 + j);
                                    g.in_bounds(q) && in_region[g.idx(q)]
                                }));
                                if ok {
                                    for j in 0..w {
                                        for i in 0..w {
                                            cor[g.idx((o.0 + i, o.1 + j))] = true;
                                        }
                                    }
                                    placed = true;
                                    break 'brush;
                                }
                            }
                        }
                        if !placed {
                            cor[g.idx(c)] = true;
                        }
                    }
                }
            }
            let cells: Vec<(i32, i32)> = (0..cor.len()).filter(|&k| cor[k]).map(|k| g.coords(k)).collect();
            // A corridor must stay a minor part of the floor.
            if !grid::is_connected(&cells) || cells.len() as f32 > region.len() as f32 * 0.3 {
                continue;
            }
            let score = corridor_score(g, &in_region, &cor);
            if best.as_ref().is_none_or(|(s, _)| score < *s) {
                best = Some((score, cor));
            }
        }
    }
    best.map(|(_, cor)| (0..cor.len()).filter(|&k| cor[k]).map(|k| g.coords(k)).collect())
}

/// Door-capable adjacency of a level: pairs of rooms sharing a run long enough
/// for a door. The stairwell only connects through its active end wall.
pub(crate) fn door_runs(lay: &LevelLayout, g: &Grid, stair: Option<&StairWell>) -> Vec<Run> {
    runs::runs(g, &lay.labels)
        .into_iter()
        .filter(|r| r.lo != OUTSIDE && r.hi != OUTSIDE)
        .filter(|r| {
            if let (Some(sw), Some(s)) = (lay.stairwell, stair)
                && r.has(sw as i32)
            {
                return s.is_end_run(r, lay.active_end) && r.other(sw as i32) >= 0;
            }
            true
        })
        .filter(|r| r.usable_len(g, &lay.labels, JUNCTION_CLEARANCE) >= DOOR_MIN_WIDTH)
        .collect()
}

/// Merge rooms until every room is reachable from the root through door-capable
/// walls, and drop rooms too small to use.
fn repair(lay: &mut LevelLayout, g: &Grid, stair: Option<&StairWell>) {
    let min_cells = ((1.6 / g.cell_area()).ceil() as usize).max(2);
    for _ in 0..64 {
        let n = lay.kinds.len();
        let mut counts = vec![0usize; n];
        for &l in &lay.labels {
            if l >= 0 {
                counts[l as usize] += 1;
            }
        }
        let stairwell = lay.stairwell;
        // Reachability from the root.
        let adj = door_runs(lay, g, stair);
        let mut seen = vec![false; n];
        let mut stack = vec![lay.root];
        if lay.root < n {
            seen[lay.root] = true;
        }
        while let Some(r) = stack.pop() {
            for run in adj.iter().filter(|x| x.has(r as i32)) {
                let o = run.other(r as i32) as usize;
                if !seen[o] {
                    seen[o] = true;
                    stack.push(o);
                }
            }
        }
        // Tiny rooms first, then unreachable ones (smallest first).
        let tiny = (0..n).find(|&r| counts[r] > 0 && counts[r] < min_cells && Some(r) != stairwell && r != lay.root);
        let victim = tiny.or_else(|| (0..n).filter(|&r| counts[r] > 0 && !seen[r] && Some(r) != stairwell).min_by_key(|&r| (counts[r], r)));
        let Some(u) = victim else { break };
        // Merge into the neighbour with the longest shared boundary, preferring
        // reachable rooms and never the stairwell.
        let mut shared = vec![0i32; n];
        for run in runs::runs(g, &lay.labels) {
            if run.has(u as i32) {
                let o = run.other(u as i32);
                if o >= 0 && Some(o as usize) != stairwell {
                    shared[o as usize] += run.s1 - run.s0;
                }
            }
        }
        let Some(v) = (0..n).filter(|&r| shared[r] > 0).max_by_key(|&r| (seen[r], shared[r], std::cmp::Reverse(r))) else {
            break;
        };
        if Some(v) != lay.corridor && counts[u] > counts[v] {
            lay.kinds[v] = lay.kinds[u];
        }
        for l in lay.labels.iter_mut() {
            if *l == u as i32 {
                *l = v as i32;
            }
        }
    }
    compact(lay);
}

/// Remove rooms without cells and renumber.
fn compact(lay: &mut LevelLayout) {
    let n = lay.kinds.len();
    let mut used = vec![false; n];
    for &l in &lay.labels {
        if l >= 0 {
            used[l as usize] = true;
        }
    }
    let mut map = vec![-1i32; n];
    let mut kinds = Vec::new();
    for r in 0..n {
        if used[r] {
            map[r] = kinds.len() as i32;
            kinds.push(lay.kinds[r]);
        }
    }
    for l in lay.labels.iter_mut() {
        if *l >= 0 {
            *l = map[*l as usize];
        }
    }
    let remap = |o: Option<usize>| o.and_then(|r| (map[r] >= 0).then_some(map[r] as usize));
    lay.stairwell = remap(lay.stairwell);
    lay.corridor = remap(lay.corridor);
    lay.root = remap(Some(lay.root)).unwrap_or(0);
    lay.kinds = kinds;
}

