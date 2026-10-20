//! Pure helpers for the free-design editor: drag-to-build bridges, drag an end
//! to change the length, cut a bridge in half, erase.
//!
//! They work on the plain [`mde_core`] data so the Bevy viewer and other tools
//! can share them.

use mde_core::{Bridge, BridgeKind, River, Town, Vec2};

use crate::geom::point_seg_dist;
use crate::river::{bridge_across_path, rise_for_span};

/// Shortest deck the editor creates (m).
pub const MIN_BRIDGE_SPAN: f32 = 2.0;

/// One end of a bridge deck.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BridgeEnd {
    /// [`Bridge::a`].
    A,
    /// [`Bridge::b`].
    B,
}

/// Default mid-span rise for a deck of `span` metres (flat-ish for short
/// spans, at most 3 m).
pub fn bridge_rise(span: f32) -> f32 {
    rise_for_span(span.max(0.0))
}

/// A bridge dragged from `a` to `b` with deck `width`. Degenerate drags are
/// extended to [`MIN_BRIDGE_SPAN`] along +x.
pub fn bridge_between(a: Vec2, b: Vec2, kind: BridgeKind, width: f32) -> Bridge {
    let b = if a.distance(b) < MIN_BRIDGE_SPAN {
        let d = (b - a).try_normalize().unwrap_or(Vec2::X);
        a + d * MIN_BRIDGE_SPAN
    } else {
        b
    };
    Bridge { kind, a, b, width: width.max(1.0), height: bridge_rise(a.distance(b)) }
}

/// Snap a drag `a → b` across `river` to a bridge spanning exactly the water
/// plus a 2 m landing on each bank, centred on the first crossing. `None` if
/// the drag does not cross the river.
pub fn bridge_across(river: &River, a: Vec2, b: Vec2, width: f32, kind: BridgeKind) -> Option<Bridge> {
    bridge_across_path(&river.path.0, river.width, a, b, width, kind)
}

/// Move one end of a bridge (drag-to-resize); the other end stays put and the
/// rise follows the new span.
pub fn move_bridge_end(bridge: &Bridge, end: BridgeEnd, to: Vec2) -> Bridge {
    match end {
        BridgeEnd::B => bridge_between(bridge.a, to, bridge.kind, bridge.width),
        BridgeEnd::A => {
            let r = bridge_between(bridge.b, to, bridge.kind, bridge.width);
            Bridge { a: r.b, b: r.a, ..r }
        }
    }
}

/// Change the deck length symmetrically about its centre.
pub fn set_bridge_length(bridge: &Bridge, length: f32) -> Bridge {
    let c = (bridge.a + bridge.b) * 0.5;
    let d = (bridge.b - bridge.a).try_normalize().unwrap_or(Vec2::X);
    let h = length.max(MIN_BRIDGE_SPAN) * 0.5;
    bridge_between(c - d * h, c + d * h, bridge.kind, bridge.width)
}

/// Cut a bridge at `t ∈ (0, 1)` along its deck, leaving `gap` metres between
/// the two new decks. `None` if either piece would be shorter than
/// [`MIN_BRIDGE_SPAN`].
pub fn cut_bridge(bridge: &Bridge, t: f32, gap: f32) -> Option<(Bridge, Bridge)> {
    let len = bridge.a.distance(bridge.b);
    if len < 1e-3 {
        return None;
    }
    let d = (bridge.b - bridge.a) / len;
    let at = len * t.clamp(0.0, 1.0);
    let g = gap.max(0.0) * 0.5;
    let (l1, l2) = (at - g, len - at - g);
    if l1 < MIN_BRIDGE_SPAN || l2 < MIN_BRIDGE_SPAN {
        return None;
    }
    let first = bridge_between(bridge.a, bridge.a + d * l1, bridge.kind, bridge.width);
    let second = bridge_between(bridge.a + d * (at + g), bridge.b, bridge.kind, bridge.width);
    Some((first, second))
}

/// Index of the bridge whose deck lies under `p` (within `tol` of its edge),
/// nearest first. Use with `town.bridges.remove(i)` to erase.
pub fn bridge_at(town: &Town, p: Vec2, tol: f32) -> Option<usize> {
    town.bridges
        .iter()
        .enumerate()
        .map(|(i, b)| (i, point_seg_dist(p, b.a, b.b) - b.width * 0.5))
        .filter(|&(_, d)| d <= tol)
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .map(|(i, _)| i)
}

/// Index of the building whose footprint contains `p`.
pub fn building_at(town: &Town, p: Vec2) -> Option<usize> {
    town.buildings.iter().position(|b| b.footprint.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mde_core::Polyline;

    #[test]
    fn drag_resize_cut() {
        let b = bridge_between(Vec2::ZERO, Vec2::new(20.0, 0.0), BridgeKind::Stone, 4.0);
        assert!((b.height - 2.4).abs() < 1e-4);
        let longer = move_bridge_end(&b, BridgeEnd::B, Vec2::new(30.0, 0.0));
        assert_eq!(longer.b, Vec2::new(30.0, 0.0));
        assert!(longer.height > b.height);
        let (l, r) = cut_bridge(&longer, 0.5, 1.0).unwrap();
        assert!((l.a.distance(l.b) - 14.5).abs() < 1e-3 && (r.a.distance(r.b) - 14.5).abs() < 1e-3);
        assert!(cut_bridge(&b, 0.05, 0.0).is_none());
        let s = set_bridge_length(&b, 10.0);
        assert!((s.a.distance(s.b) - 10.0).abs() < 1e-4);
    }

    #[test]
    fn snap_across_river() {
        let river = River { path: Polyline(vec![Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)]), width: 10.0 };
        let b = bridge_across(&river, Vec2::new(3.0, -30.0), Vec2::new(3.0, 40.0), 4.0, BridgeKind::Wood).unwrap();
        assert!((b.a.y + 7.0).abs() < 1e-3 && (b.b.y - 7.0).abs() < 1e-3);
        assert!(bridge_across(&river, Vec2::new(0.0, 5.0), Vec2::new(0.0, 30.0), 4.0, BridgeKind::Wood).is_none());
    }
}
