//! Room programmes: which rooms each style wants on each floor, with relative
//! areas and front/back preferences.

use mde_core::{DwellingStyle, RoomKind, Rng};

/// One room the floor should contain.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoomSpec {
    pub kind: RoomKind,
    /// Relative area share.
    pub weight: f32,
    /// Preferred position from the street front (0) to the back (1).
    pub depth: f32,
    /// Smallest acceptable area, m².
    pub min_area: f32,
    /// Lower = more important; rooms are dropped from the highest value first.
    pub priority: u8,
    /// The ground-floor room the entrance should open into.
    pub entry: bool,
}

impl RoomSpec {
    fn new(kind: RoomKind, weight: f32, depth: f32, min_area: f32, priority: u8) -> Self {
        RoomSpec { kind, weight, depth, min_area, priority, entry: false }
    }

    fn entry(mut self) -> Self {
        self.entry = true;
        self
    }

    /// Rooms that may be narrower than the normal minimum width.
    pub fn is_small_kind(&self) -> bool {
        matches!(self.kind, RoomKind::Privy | RoomKind::Pantry | RoomKind::Bath | RoomKind::Storage)
    }
}

/// Rooms wanted on one floor.
#[derive(Clone, Debug)]
pub(crate) struct LevelProgram {
    pub rooms: Vec<RoomSpec>,
    /// Serve the rooms from a corridor spine.
    pub corridor: bool,
}

/// What a level is within the house.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LevelRole {
    Basement,
    Ground,
    /// An upper storey below the top one, or the top one of a two-storey house.
    Upper,
    /// Top storey of a house with three or more storeys (attic-like).
    Top,
}

impl LevelRole {
    /// Role of `level` in a house with `floors` storeys above ground.
    pub fn of(level: i32, floors: u8) -> LevelRole {
        if level < 0 {
            LevelRole::Basement
        } else if level == 0 {
            LevelRole::Ground
        } else if floors >= 3 && level == floors as i32 - 1 {
            LevelRole::Top
        } else {
            LevelRole::Upper
        }
    }
}

/// Programme for floor `level` with `area` m² available to rooms (stairwell already
/// excluded). Rooms that do not fit are dropped by priority; at least one room
/// is always kept.
pub(crate) fn level_program(style: DwellingStyle, role: LevelRole, level: i32, floors: u8, area: f32, rng: &mut Rng) -> LevelProgram {
    use DwellingStyle as S;
    use RoomKind as K;
    let single = floors <= 1;
    let mut rooms: Vec<RoomSpec> = Vec::new();
    let r = RoomSpec::new;
    match role {
        LevelRole::Basement => {
            // Vaulted cellars of roughly 20–40 m² each, plus a store room.
            let n = ((area / 32.0).round() as usize).clamp(1, 5);
            for k in 0..n {
                rooms.push(r(K::Cellar, 1.0, k as f32 / n.max(2) as f32, 5.0, k as u8));
            }
            if area > 50.0 {
                rooms.push(r(K::Storage, 0.6, 0.0, 5.0, 6));
            }
        }
        LevelRole::Ground => match style {
            S::Cottage => {
                rooms.push(r(K::Living, 5.0, 0.3, 9.0, 0).entry());
                rooms.push(r(K::Kitchen, 3.0, 0.85, 5.0, 1));
                if single {
                    rooms.push(r(K::Bedroom, 3.0, 0.9, 6.0, 2));
                }
                rooms.push(r(K::Storage, 1.2, 1.0, 2.5, 3));
                if area > 45.0 {
                    rooms.push(r(K::Pantry, 0.9, 1.0, 2.5, 5));
                }
            }
            S::Townhouse => {
                let front = if rng.chance(0.5) { K::Shop } else { K::Workshop };
                rooms.push(r(front, 4.0, 0.0, 8.0, 0).entry());
                rooms.push(r(K::Kitchen, 3.0, 1.0, 6.0, 1));
                if single {
                    rooms.push(r(K::Bedroom, 2.8, 0.8, 6.0, 2));
                }
                if rng.chance(0.5) {
                    rooms.push(r(K::Pantry, 0.9, 1.0, 2.5, 4));
                } else {
                    rooms.push(r(K::Privy, 0.6, 1.0, 1.6, 4));
                }
            }
            S::Farmhouse => {
                rooms.push(r(K::Kitchen, 6.0, 0.3, 12.0, 0).entry());
                rooms.push(r(K::Workshop, 3.0, 0.7, 7.0, 1));
                rooms.push(r(K::Storage, 2.5, 1.0, 4.0, 2));
                if single {
                    rooms.push(r(K::Bedroom, 3.0, 0.9, 6.0, 1));
                    rooms.push(r(K::Bedroom, 2.6, 0.9, 6.0, 4));
                }
                rooms.push(r(K::Pantry, 1.0, 1.0, 2.5, 3));
            }
            S::Manor => {
                rooms.push(r(K::Hall, 6.0, 0.4, 18.0, 0).entry());
                rooms.push(r(K::Dining, 3.5, 0.6, 10.0, 1));
                rooms.push(r(K::Kitchen, 3.0, 0.9, 8.0, 1));
                let study = if rng.chance(0.5) { K::Library } else { K::Study };
                rooms.push(r(study, 2.5, 0.4, 7.0, 2));
                rooms.push(r(K::Pantry, 1.0, 1.0, 3.0, 3));
                rooms.push(r(K::Servants, 2.0, 1.0, 6.0, 4));
                if rng.chance(0.4) {
                    rooms.push(r(K::Chapel, 2.5, 0.5, 8.0, 5));
                }
                if single {
                    rooms.push(r(K::Bedroom, 3.0, 0.8, 8.0, 2));
                    rooms.push(r(K::Bedroom, 2.6, 0.8, 7.0, 4));
                    rooms.push(r(K::Bath, 1.2, 1.0, 3.0, 5));
                }
            }
            S::Tavern => {
                rooms.push(r(K::Taproom, 7.0, 0.0, 20.0, 0).entry());
                rooms.push(r(K::Kitchen, 3.0, 1.0, 8.0, 1));
                rooms.push(r(K::Pantry, 1.0, 1.0, 3.0, 2));
                rooms.push(r(K::Storage, 1.5, 1.0, 4.0, 3));
                rooms.push(r(K::Privy, 0.6, 1.0, 1.6, 4));
                if single {
                    rooms.push(r(K::Bedroom, 2.6, 1.0, 7.0, 2));
                }
            }
            S::Shop => {
                rooms.push(r(K::Shop, 4.0, 0.0, 8.0, 0).entry());
                rooms.push(r(K::Workshop, 3.0, 0.7, 7.0, 1));
                rooms.push(r(K::Storage, 1.5, 1.0, 3.0, 2));
                if single {
                    rooms.push(r(K::Kitchen, 2.5, 0.9, 5.0, 1));
                    rooms.push(r(K::Bedroom, 2.5, 1.0, 6.0, 2));
                }
            }
        },
        LevelRole::Upper => match style {
            S::Cottage => {
                rooms.push(r(K::Bedroom, 3.0, 0.4, 6.0, 0));
                rooms.push(r(K::Bedroom, 2.5, 0.8, 6.0, 2));
                rooms.push(r(K::Storage, 1.0, 1.0, 2.5, 3));
            }
            S::Townhouse if level == 1 => {
                rooms.push(r(K::Living, 4.0, 0.0, 9.0, 0));
                rooms.push(r(K::Bedroom, 3.0, 1.0, 6.0, 1));
                if rng.chance(0.5) {
                    rooms.push(r(K::Study, 1.6, 0.5, 4.5, 4));
                } else {
                    rooms.push(r(K::Bedroom, 2.4, 0.6, 6.0, 4));
                }
            }
            S::Townhouse | S::Shop if level >= 2 => {
                rooms.push(r(K::Bedroom, 3.0, 0.0, 6.0, 0));
                rooms.push(r(K::Bedroom, 3.0, 1.0, 6.0, 1));
                if rng.chance(0.5) {
                    rooms.push(r(K::Nursery, 1.8, 0.5, 5.0, 4));
                } else {
                    rooms.push(r(K::Storage, 1.2, 0.6, 3.0, 4));
                }
            }
            S::Townhouse => {}
            S::Farmhouse => {
                rooms.push(r(K::Bedroom, 3.0, 0.2, 7.0, 0));
                rooms.push(r(K::Bedroom, 3.0, 0.8, 7.0, 1));
                rooms.push(r(K::Storage, 1.5, 1.0, 3.0, 2));
                rooms.push(r(K::Bedroom, 2.6, 0.5, 6.0, 3));
            }
            S::Manor if level == 1 => {
                rooms.push(r(K::Bedroom, 3.0, 0.2, 9.0, 0));
                rooms.push(r(K::Bedroom, 3.0, 0.8, 9.0, 1));
                rooms.push(r(K::Bath, 1.2, 1.0, 3.0, 2));
                rooms.push(r(K::Bedroom, 2.8, 0.4, 8.0, 3));
                let study = if rng.chance(0.5) { K::Library } else { K::Study };
                rooms.push(r(study, 2.4, 0.5, 7.0, 4));
                rooms.push(r(K::Nursery, 2.0, 0.7, 6.0, 5));
                rooms.push(r(K::Bedroom, 2.6, 0.6, 8.0, 6));
            }
            S::Manor => {
                rooms.push(r(K::Bedroom, 3.0, 0.2, 9.0, 0));
                rooms.push(r(K::Bedroom, 3.0, 0.8, 9.0, 1));
                rooms.push(r(K::Bedroom, 2.8, 0.5, 8.0, 2));
                rooms.push(r(K::Servants, 2.4, 1.0, 7.0, 3));
                rooms.push(r(K::Bath, 1.2, 1.0, 3.0, 4));
                rooms.push(r(K::Storage, 1.4, 0.6, 3.0, 5));
            }
            S::Tavern => {
                let n = ((area / 11.0) as usize).clamp(2, 8);
                for k in 0..n {
                    let depth = if k % 2 == 0 { 0.1 } else { 0.9 };
                    rooms.push(r(K::Bedroom, 2.5, depth, 6.0, k as u8));
                }
                rooms.push(r(K::Privy, 0.5, 1.0, 1.6, 9));
            }
            S::Shop => {
                rooms.push(r(K::Living, 3.0, 0.0, 8.0, 0));
                rooms.push(r(K::Kitchen, 2.5, 1.0, 5.0, 1));
                rooms.push(r(K::Bedroom, 2.6, 0.6, 6.0, 2));
            }
        },
        LevelRole::Top => match style {
            S::Manor => {
                rooms.push(r(K::Servants, 3.0, 0.5, 8.0, 0));
                rooms.push(r(K::Bedroom, 2.6, 0.2, 8.0, 1));
                rooms.push(r(K::Attic, 3.0, 0.8, 6.0, 2));
                rooms.push(r(K::Storage, 1.5, 1.0, 3.0, 3));
                rooms.push(r(K::Servants, 2.4, 0.3, 7.0, 4));
            }
            S::Tavern => {
                let n = ((area / 12.0) as usize).clamp(2, 6);
                for k in 0..n {
                    let depth = if k % 2 == 0 { 0.1 } else { 0.9 };
                    rooms.push(r(K::Bedroom, 2.5, depth, 6.0, k as u8));
                }
                rooms.push(r(K::Storage, 1.4, 1.0, 3.0, 8));
            }
            _ => {
                rooms.push(r(K::Attic, 3.0, 0.4, 5.0, 0));
                rooms.push(r(K::Bedroom, 2.4, 0.2, 6.0, 2));
                rooms.push(r(K::Storage, 1.0, 1.0, 2.5, 3));
            }
        },
    }
    fit_to_area(&mut rooms, area);
    // Upper floors with several rooms are served from a landing corridor
    // (a stair hall in narrow town houses, a spine in big houses).
    let corridor = match role {
        LevelRole::Upper => match style {
            S::Townhouse | S::Shop | S::Tavern | S::Manor => rooms.len() >= 3,
            S::Cottage | S::Farmhouse => rooms.len() >= 4,
        },
        LevelRole::Top => matches!(style, S::Tavern | S::Manor) && rooms.len() >= 3,
        _ => false,
    };
    LevelProgram { rooms, corridor }
}

/// Drop the least important rooms until the minimum areas fit and every room
/// keeps a sensible share of the floor.
fn fit_to_area(rooms: &mut Vec<RoomSpec>, area: f32) {
    loop {
        if rooms.len() <= 1 {
            return;
        }
        let total_min: f32 = rooms.iter().map(|r| r.min_area).sum();
        let too_many = rooms.len() as f32 * 4.5 > area;
        if total_min <= area * 0.92 && !too_many {
            return;
        }
        // Remove the room with the highest priority value (last one on ties).
        let mut worst = 0;
        for (k, r) in rooms.iter().enumerate() {
            if r.priority >= rooms[worst].priority {
                worst = k;
            }
        }
        if rooms[worst].entry {
            // Never drop the entry room; drop the next worst instead.
            let alt = (0..rooms.len()).filter(|&k| !rooms[k].entry).max_by_key(|&k| (rooms[k].priority, k));
            match alt {
                Some(k) => {
                    rooms.remove(k);
                }
                None => return,
            }
        } else {
            rooms.remove(worst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_area_keeps_one_room() {
        let p = level_program(DwellingStyle::Manor, LevelRole::Ground, 0, 2, 6.0, &mut Rng::new(1));
        assert_eq!(p.rooms.len(), 1);
        assert_eq!(p.rooms[0].kind, RoomKind::Hall);
    }

    #[test]
    fn tavern_upper_has_corridor() {
        let p = level_program(DwellingStyle::Tavern, LevelRole::Upper, 1, 2, 110.0, &mut Rng::new(1));
        assert!(p.corridor);
        assert!(p.rooms.iter().filter(|r| r.kind == RoomKind::Bedroom).count() >= 4);
    }
}
