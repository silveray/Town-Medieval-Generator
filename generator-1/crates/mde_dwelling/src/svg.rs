//! Architectural plan drawings: paper background, solid dark walls with door
//! gaps and swing arcs, windows as thin double lines, stair treads with arrows,
//! furniture outlines, room labels, scale bar and north arrow.

use std::fmt::Write as _;

use mde_core::{Dwelling, Floor, Opening, OpeningKind, Prop, PropKind, RoomKind, Stair, Vec2};

use crate::validate::dominant_angle;
use crate::walk::{point_in_polygon, Obb};
use crate::{level_name, room_label};

/// Pixels per metre.
const SCALE: f32 = 42.0;
/// Margin around each plan, pixels.
const MARGIN: f32 = 46.0;
/// Space for the floor title above each plan, pixels.
const TITLE_H: f32 = 34.0;

const PAPER: &str = "#efe8d9";
const FLOOR: &str = "#fbf8f0";
const STONE_FLOOR: &str = "#f1ece1";
const INK: &str = "#2a231e";
const FURN_STROKE: &str = "#5b4a3b";
const FURN_FILL: &str = "#ece2cd";
const TEXT: &str = "#3b3129";

/// Maps plan points into the drawing: rotate so walls are axis-aligned and
/// the entrance faces down, then scale and flip y.
#[derive(Clone, Copy)]
struct View {
    rot: Vec2,
    min: Vec2,
    max: Vec2,
    off: Vec2,
}

impl View {
    fn new(d: &Dwelling) -> View {
        let mut angle = dominant_angle(d);
        // Turn by a multiple of 90° so the entrance's outward side points down.
        if let Some(e) = crate::entrance(d) {
            let dir = (e.b - e.a).try_normalize().unwrap_or(Vec2::X);
            let mut n = Vec2::new(dir.y, -dir.x);
            let mid = (e.a + e.b) * 0.5;
            let inside = d.floors.iter().filter(|f| f.level == 0).any(|f| f.rooms.iter().any(|r| point_in_polygon(&r.shape.0, mid + n * 0.5)));
            if inside {
                n = -n;
            }
            let local = Vec2::from_angle(-angle).rotate(n);
            // Rotate local so `local` maps to (0, -1).
            let extra = if local.y < -0.7 {
                0.0
            } else if local.y > 0.7 {
                std::f32::consts::PI
            } else if local.x > 0.7 {
                -std::f32::consts::FRAC_PI_2
            } else {
                std::f32::consts::FRAC_PI_2
            };
            angle -= extra;
        }
        let rot = Vec2::from_angle(-angle);
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for f in &d.floors {
            for &p in &f.outline.0 {
                let q = rot.rotate(p);
                min = min.min(q);
                max = max.max(q);
            }
        }
        if !min.is_finite() || !max.is_finite() {
            min = Vec2::ZERO;
            max = Vec2::ONE;
        }
        View { rot, min, max, off: Vec2::ZERO }
    }

    /// Drawing size of one plan including margins, pixels.
    fn size(&self) -> Vec2 {
        (self.max - self.min) * SCALE + Vec2::splat(2.0 * MARGIN) + Vec2::new(0.0, TITLE_H)
    }

    fn p(&self, plan: Vec2) -> Vec2 {
        let q = self.rot.rotate(plan);
        Vec2::new((q.x - self.min.x) * SCALE + MARGIN, (self.max.y - q.y) * SCALE + MARGIN + TITLE_H) + self.off
    }

    /// Plan direction → drawing direction (unit, y down).
    fn dir(&self, d: Vec2) -> Vec2 {
        let q = self.rot.rotate(d);
        Vec2::new(q.x, -q.y)
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\'', "&#39;")
}

fn poly_path(view: &View, pts: &[Vec2]) -> String {
    let mut s = String::new();
    for (k, &p) in pts.iter().enumerate() {
        let q = view.p(p);
        let _ = write!(s, "{}{:.1},{:.1}", if k == 0 { "M" } else { "L" }, q.x, q.y);
    }
    s.push('Z');
    s
}

fn stone_floor(kind: RoomKind) -> bool {
    matches!(
        kind,
        RoomKind::Kitchen | RoomKind::Hall | RoomKind::Taproom | RoomKind::Cellar | RoomKind::Storage | RoomKind::Pantry | RoomKind::Workshop | RoomKind::Chapel | RoomKind::Privy | RoomKind::Bath
    )
}

/// Openings lying on wall `a–b` as parameter intervals `[t0, t1]` in metres.
fn gaps_on(a: Vec2, b: Vec2, openings: &[Opening]) -> Vec<(f32, f32, usize)> {
    let len = a.distance(b);
    if len < 1e-6 {
        return Vec::new();
    }
    let dir = (b - a) / len;
    let n = Vec2::new(-dir.y, dir.x);
    let mut out = Vec::new();
    for (k, o) in openings.iter().enumerate() {
        let da = (o.a - a).dot(n).abs();
        let db = (o.b - a).dot(n).abs();
        if da > 1e-3 || db > 1e-3 {
            continue;
        }
        let t0 = (o.a - a).dot(dir);
        let t1 = (o.b - a).dot(dir);
        let (t0, t1) = (t0.min(t1), t0.max(t1));
        if t1 < -1e-3 || t0 > len + 1e-3 {
            continue;
        }
        out.push((t0.max(0.0), t1.min(len), k));
    }
    out.sort_by(|x, y| x.0.total_cmp(&y.0));
    out
}

fn room_at(f: &Floor, p: Vec2) -> Option<&mde_core::Room> {
    f.rooms.iter().find(|r| point_in_polygon(&r.shape.0, p))
}

/// Draw one floor into `out` using `view`.
fn draw_floor(out: &mut String, d: &Dwelling, f: &Floor, view: &View) {
    let _ = writeln!(out, "<g class=\"floor\" data-level=\"{}\">", f.level);
    // Shadow and outline footprint.
    let outline = poly_path(view, &f.outline.0);
    let _ = writeln!(out, "<path d=\"{outline}\" fill=\"#000\" opacity=\"0.10\" transform=\"translate(5,6)\"/>");
    let _ = writeln!(out, "<path d=\"{outline}\" fill=\"{INK}\"/>");

    // Room floors.
    for r in &f.rooms {
        let fill = if stone_floor(r.kind) { STONE_FLOOR } else { FLOOR };
        let _ = writeln!(out, "<path d=\"{}\" fill=\"{fill}\"/>", poly_path(view, &r.shape.0));
        if stone_floor(r.kind) {
            let _ = writeln!(out, "<path d=\"{}\" fill=\"url(#flags)\" opacity=\"0.55\"/>", poly_path(view, &r.shape.0));
        }
    }

    // Stairs: flights leaving this level and the void above flights arriving.
    let all: Vec<&Stair> = d.floors.iter().flat_map(|fl| fl.stairs.iter()).collect();
    for s in all.iter().filter(|s| s.to_level == f.level) {
        draw_stair(out, view, s, false);
    }
    for s in all.iter().filter(|s| s.from_level == f.level) {
        draw_stair(out, view, s, true);
    }

    // Furniture.
    for p in &f.props {
        draw_prop(out, view, p);
    }

    // Walls with gaps.
    let mut wall_d = String::new();
    for w in &f.walls {
        let len = w.a.distance(w.b);
        if len < 1e-6 {
            continue;
        }
        let dir = (w.b - w.a) / len;
        let n = Vec2::new(-dir.y, dir.x) * (w.thickness * 0.5);
        let ext = w.thickness * 0.5;
        let mut t = -ext;
        let mut pieces = Vec::new();
        for (g0, g1, _) in gaps_on(w.a, w.b, &f.openings) {
            if g0 > t {
                pieces.push((t, g0));
            }
            t = t.max(g1);
        }
        if len + ext > t {
            pieces.push((t, len + ext));
        }
        for (t0, t1) in pieces {
            let a = w.a + dir * t0;
            let b = w.a + dir * t1;
            wall_d.push_str(&poly_path(view, &[a - n, b - n, b + n, a + n]));
        }
    }
    let _ = writeln!(out, "<path d=\"{wall_d}\" fill=\"{INK}\"/>");

    // Openings.
    for o in &f.openings {
        draw_opening(out, view, f, o);
    }

    // Labels.
    for r in &f.rooms {
        if r.kind == RoomKind::Stairwell {
            continue;
        }
        draw_label(out, view, f, r);
    }
    let _ = writeln!(out, "</g>");
}

fn draw_stair(out: &mut String, view: &View, s: &Stair, leaving: bool) {
    let up = s.up.try_normalize().unwrap_or(Vec2::Y);
    let across = Vec2::new(-up.y, up.x);
    let (mut a0, mut a1, mut b0, mut b1) = (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY);
    for p in &s.footprint.0 {
        a0 = a0.min(p.dot(up));
        a1 = a1.max(p.dot(up));
        b0 = b0.min(p.dot(across));
        b1 = b1.max(p.dot(across));
    }
    let at = |a: f32, b: f32| up * a + across * b;
    let (stroke, fill, dash) = if leaving { (INK, "#e6dccb", "") } else { ("#8a7d6e", "#d9d0c0", " stroke-dasharray=\"4 3\"") };
    let _ = writeln!(
        out,
        "<path d=\"{}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"1.2\"{dash}/>",
        poly_path(view, &[at(a0, b0), at(a1, b0), at(a1, b1), at(a0, b1)])
    );
    let run = a1 - a0;
    let n = ((run / 0.24).round() as i32).max(2);
    let mut treads = String::new();
    for k in 1..n {
        let a = a0 + run * k as f32 / n as f32;
        let p = view.p(at(a, b0));
        let q = view.p(at(a, b1));
        let _ = write!(treads, "M{:.1},{:.1}L{:.1},{:.1}", p.x, p.y, q.x, q.y);
    }
    let _ = writeln!(out, "<path d=\"{treads}\" stroke=\"{stroke}\" stroke-width=\"0.8\" fill=\"none\"{dash}/>");
    // Walking line with arrow head at the top.
    let mid = (b0 + b1) * 0.5;
    let start = view.p(at(a0 + 0.15, mid));
    let end = view.p(at(a1 - 0.12, mid));
    let dirv = (end - start).try_normalize().unwrap_or(Vec2::X);
    let side = Vec2::new(-dirv.y, dirv.x);
    let head = 7.0;
    let h1 = end - dirv * head + side * head * 0.55;
    let h2 = end - dirv * head - side * head * 0.55;
    let colour = if leaving { INK } else { "#8a7d6e" };
    let _ = writeln!(
        out,
        "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"2.6\" fill=\"{colour}\"/><path d=\"M{:.1},{:.1}L{:.1},{:.1}M{:.1},{:.1}L{:.1},{:.1}L{:.1},{:.1}\" stroke=\"{colour}\" stroke-width=\"1.3\" fill=\"none\"/>",
        start.x, start.y, start.x, start.y, end.x, end.y, h1.x, h1.y, end.x, end.y, h2.x, h2.y
    );
    let label = if leaving { "up" } else { "down" };
    let lp = view.p(at(a0 + run * 0.5, mid));
    // Place the word beside the walking line.
    let shift = side * 9.0;
    let _ = writeln!(
        out,
        "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"9\" fill=\"{colour}\" text-anchor=\"middle\" dominant-baseline=\"middle\" font-family=\"Georgia, serif\" font-style=\"italic\">{label}</text>",
        lp.x + shift.x,
        lp.y + shift.y
    );
}

fn draw_opening(out: &mut String, view: &View, f: &Floor, o: &Opening) {
    let len = o.a.distance(o.b);
    if len < 1e-4 {
        return;
    }
    let dir = (o.b - o.a) / len;
    let n = Vec2::new(-dir.y, dir.x);
    let th = f
        .walls
        .iter()
        .find(|w| !gaps_on(w.a, w.b, std::slice::from_ref(o)).is_empty())
        .map(|w| w.thickness)
        .unwrap_or(0.15);
    let h = th * 0.5;
    match o.kind {
        OpeningKind::Window => {
            let quad = [o.a - n * h, o.b - n * h, o.b + n * h, o.a + n * h];
            let _ = writeln!(out, "<path d=\"{}\" fill=\"{FLOOR}\" stroke=\"{INK}\" stroke-width=\"1\"/>", poly_path(view, &quad));
            let p = view.p(o.a);
            let q = view.p(o.b);
            let _ = writeln!(out, "<path d=\"M{:.1},{:.1}L{:.1},{:.1}\" stroke=\"{INK}\" stroke-width=\"0.9\"/>", p.x, p.y, q.x, q.y);
        }
        OpeningKind::Doorway => {
            // Jambs only, plus a faint arch line.
            let p = view.p(o.a);
            let q = view.p(o.b);
            let _ = writeln!(
                out,
                "<path d=\"M{:.1},{:.1}L{:.1},{:.1}\" stroke=\"{INK}\" stroke-width=\"0.7\" stroke-dasharray=\"2 2.5\" opacity=\"0.6\"/>",
                p.x, p.y, q.x, q.y
            );
        }
        OpeningKind::Door | OpeningKind::Entrance => {
            // Swing into the more private room (inwards for the entrance).
            let side_a = room_at(f, (o.a + o.b) * 0.5 + n * (h + 0.3));
            let side_b = room_at(f, (o.a + o.b) * 0.5 - n * (h + 0.3));
            let rank = |r: Option<&mde_core::Room>| match r {
                None => -10,
                Some(r) => match r.kind {
                    RoomKind::Corridor | RoomKind::Hall | RoomKind::Stairwell | RoomKind::Taproom => 0,
                    RoomKind::Living | RoomKind::Dining | RoomKind::Shop | RoomKind::Kitchen => 1,
                    _ => 2,
                },
            };
            let into = if rank(side_a) >= rank(side_b) { n } else { -n };
            if o.kind == OpeningKind::Entrance {
                // Threshold stone outside.
                let outside = -into;
                let quad = [
                    o.a - dir * 0.1 + outside * h,
                    o.b + dir * 0.1 + outside * h,
                    o.b + dir * 0.1 + outside * (h + 0.35),
                    o.a - dir * 0.1 + outside * (h + 0.35),
                ];
                let _ = writeln!(out, "<path d=\"{}\" fill=\"#d6cbb6\" stroke=\"{INK}\" stroke-width=\"0.8\"/>", poly_path(view, &quad));
            }
            let leaves: Vec<(Vec2, Vec2, f32)> = if o.kind == OpeningKind::Entrance && len > 1.45 {
                vec![(o.a, dir, len * 0.5), (o.b, -dir, len * 0.5)]
            } else {
                vec![(o.a, dir, len)]
            };
            for (hinge, along, w) in leaves {
                let hinge = hinge + into * h;
                let tip = hinge + into * w;
                let close = hinge + along * w;
                let p0 = view.p(hinge);
                let p1 = view.p(tip);
                let p2 = view.p(close);
                let r = w * SCALE;
                // Sweep direction from the cross product in drawing space.
                let c = (p1 - p0).perp_dot(p2 - p0);
                let sweep = if c > 0.0 { 1 } else { 0 };
                let _ = writeln!(
                    out,
                    "<path d=\"M{:.1},{:.1}L{:.1},{:.1}\" stroke=\"{INK}\" stroke-width=\"1.6\"/><path d=\"M{:.1},{:.1}A{r:.1},{r:.1} 0 0 {sweep} {:.1},{:.1}\" stroke=\"{INK}\" stroke-width=\"0.6\" fill=\"none\" stroke-dasharray=\"3 2\"/>",
                    p0.x, p0.y, p1.x, p1.y, p1.x, p1.y, p2.x, p2.y
                );
            }
        }
    }
}

/// Points of a prop's local shape → drawing space.
fn prop_xf<'a>(view: &'a View, p: &Prop) -> impl Fn(f32, f32) -> Vec2 + 'a {
    let r = Vec2::from_angle(p.rotation);
    let pos = p.pos;
    move |x: f32, y: f32| view.p(pos + r.rotate(Vec2::new(x, y)))
}

fn path_of(pts: &[Vec2]) -> String {
    let mut s = String::new();
    for (k, q) in pts.iter().enumerate() {
        let _ = write!(s, "{}{:.1},{:.1}", if k == 0 { "M" } else { "L" }, q.x, q.y);
    }
    s
}

fn draw_prop(out: &mut String, view: &View, p: &Prop) {
    let xf = prop_xf(view, p);
    let (w, d) = (p.size.x * 0.5, p.size.y * 0.5);
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| -> String {
        let mut s = path_of(&[xf(x0, y0), xf(x1, y0), xf(x1, y1), xf(x0, y1)]);
        s.push('Z');
        s
    };
    let line = |x0: f32, y0: f32, x1: f32, y1: f32| -> String { path_of(&[xf(x0, y0), xf(x1, y1)]) };
    let body = rect(-w, -d, w, d);
    let sw = 0.9;
    let mut extra = String::new();
    let mut fill = FURN_FILL;
    match p.kind {
        PropKind::Bed | PropKind::Cot => {
            // Head against the wall at -y.
            let pw = if p.size.x > 1.2 { (p.size.x - 0.3) * 0.5 } else { p.size.x - 0.2 };
            let pd = 0.32_f32.min(p.size.y * 0.25);
            if p.size.x > 1.2 {
                extra += &rect(-w + 0.1, -d + 0.08, -w + 0.1 + pw, -d + 0.08 + pd);
                extra += &rect(w - 0.1 - pw, -d + 0.08, w - 0.1, -d + 0.08 + pd);
            } else {
                extra += &rect(-pw * 0.5, -d + 0.08, pw * 0.5, -d + 0.08 + pd);
            }
            let fold = -d + p.size.y * 0.38;
            extra += &line(-w, fold, w, fold);
            extra += &line(-w, fold, -w + 0.25, fold + 0.2);
        }
        PropKind::Table | PropKind::Desk => {
            if p.kind == PropKind::Desk {
                extra += &rect(-w + 0.08, -d + 0.08, -w + 0.4, -d + 0.3);
            }
        }
        PropKind::Chair => {
            extra += &rect(-w, -d, w, -d + 0.08);
        }
        PropKind::Bench => {}
        PropKind::Hearth => {
            fill = "#d8ccb5";
            let fw = w * 0.62;
            extra += &rect(-fw, -d, fw, -d + p.size.y * 0.6);
            extra += &line(-fw * 0.5, -d + 0.1, fw * 0.3, -d + p.size.y * 0.45);
            extra += &line(fw * 0.4, -d + 0.12, -fw * 0.2, -d + p.size.y * 0.5);
        }
        PropKind::Stove => {
            fill = "#d8ccb5";
            for cx in [-w * 0.45, w * 0.45] {
                let c = xf(cx, 0.0);
                let _ = write!(extra, "M{:.1},{:.1}m-{r:.1},0a{r:.1},{r:.1} 0 1 0 {d2:.1},0a{r:.1},{r:.1} 0 1 0 -{d2:.1},0", c.x, c.y, r = 0.17 * SCALE, d2 = 0.34 * SCALE);
            }
        }
        PropKind::Counter => {
            extra += &line(-w, d - 0.12, w, d - 0.12);
        }
        PropKind::Shelf => {
            extra += &line(-w, 0.0, w, 0.0);
        }
        PropKind::Bookshelf => {
            let n = ((p.size.x / 0.12) as i32).max(2);
            for k in 1..n {
                let x = -w + p.size.x * k as f32 / n as f32;
                extra += &line(x, -d + 0.06, x, d - 0.06);
            }
        }
        PropKind::Wardrobe => {
            extra += &line(0.0, -d, 0.0, d);
            extra += &line(-w, d - 0.06, w, d - 0.06);
        }
        PropKind::Chest => {
            extra += &line(-w, -d + p.size.y * 0.35, w, -d + p.size.y * 0.35);
        }
        PropKind::Barrel => {
            let c = xf(0.0, 0.0);
            let r = w.min(d) * SCALE;
            let _ = writeln!(
                out,
                "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"{r:.1}\" fill=\"#e2d3b4\" stroke=\"{FURN_STROKE}\" stroke-width=\"{sw}\"/><circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"{:.1}\" fill=\"none\" stroke=\"{FURN_STROKE}\" stroke-width=\"0.6\"/>",
                c.x, c.y, c.x, c.y, r * 0.62
            );
            return;
        }
        PropKind::Crate => {
            fill = "#e2d3b4";
            extra += &line(-w, -d, w, d);
            extra += &line(-w, d, w, -d);
        }
        PropKind::Bathtub => {
            extra += &rect(-w + 0.1, -d + 0.1, w - 0.1, d - 0.1);
        }
        PropKind::Altar => {
            fill = "#e6dcc8";
            extra += &line(0.0, -d + 0.12, 0.0, d - 0.12);
            extra += &line(-0.18, -d + 0.3, 0.18, -d + 0.3);
        }
        PropKind::Anvil => {
            fill = "#b9b2a6";
            let mut s = path_of(&[xf(-w, -d * 0.6), xf(w * 0.5, -d * 0.6), xf(w, 0.0), xf(w * 0.5, d * 0.6), xf(-w, d * 0.6)]);
            s.push('Z');
            let _ = writeln!(out, "<path d=\"{s}\" fill=\"{fill}\" stroke=\"{FURN_STROKE}\" stroke-width=\"{sw}\"/>");
            return;
        }
    }
    let _ = writeln!(out, "<path d=\"{body}\" fill=\"{fill}\" stroke=\"{FURN_STROKE}\" stroke-width=\"{sw}\"/>");
    if !extra.is_empty() {
        let _ = writeln!(out, "<path d=\"{extra}\" fill=\"none\" stroke=\"{FURN_STROKE}\" stroke-width=\"0.6\"/>");
    }
}

/// Distance from `p` to the polygon boundary.
fn dist_to_boundary(poly: &[Vec2], p: Vec2) -> f32 {
    let n = poly.len();
    (0..n)
        .map(|k| mde_core::poly::closest_point_on_segment(p, poly[k], poly[(k + 1) % n]).distance(p))
        .fold(f32::INFINITY, f32::min)
}

/// Distance from `p` to an oriented box (0 inside).
fn dist_to_obb(o: &Obb, p: Vec2) -> f32 {
    let d = p - o.center;
    let v = Vec2::new(-o.u.y, o.u.x);
    let q = Vec2::new(d.dot(o.u).abs() - o.half.x, d.dot(v).abs() - o.half.y);
    q.max(Vec2::ZERO).length()
}

fn draw_label(out: &mut String, view: &View, f: &Floor, r: &mde_core::Room) {
    let poly = &r.shape.0;
    if poly.len() < 3 {
        return;
    }
    let b = r.shape.bounds();
    let centroid = r.shape.centroid();
    let boxes: Vec<Obb> = f.props.iter().filter(|p| p.room == r.id).map(|p| Obb::from_prop(p.pos, p.size, p.rotation)).collect();
    let name = room_label(r.kind);
    let area = r.shape.area();
    let mut best = (f32::NEG_INFINITY, centroid, 0.0f32);
    let step = 0.2;
    let nx = ((b.max.x - b.min.x) / step).ceil() as i32;
    let ny = ((b.max.y - b.min.y) / step).ceil() as i32;
    for j in 0..=ny {
        for i in 0..=nx {
            let p = b.min + Vec2::new(i as f32 * step, j as f32 * step);
            if !point_in_polygon(poly, p) {
                continue;
            }
            let wall = dist_to_boundary(poly, p);
            let furn = boxes.iter().map(|o| dist_to_obb(o, p)).fold(f32::INFINITY, f32::min);
            // Text is wide: weigh horizontal room (in drawing space) more.
            let clear = wall.min(furn * 1.6);
            let score = clear.min(1.2) - p.distance(centroid) * 0.05;
            if score > best.0 {
                best = (score, p, clear);
            }
        }
    }
    let (_, p, clear) = best;
    let fs = if clear > 0.8 && area > 9.0 { 12.5 } else if clear > 0.45 { 10.5 } else { 9.0 };
    let q = view.p(p);
    let _ = writeln!(
        out,
        "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"{fs}\" fill=\"{TEXT}\" text-anchor=\"middle\" font-family=\"Georgia, 'Times New Roman', serif\" letter-spacing=\"0.6\" style=\"font-variant: small-caps\" paint-order=\"stroke\" stroke=\"{FLOOR}\" stroke-width=\"3\" stroke-linejoin=\"round\">{}</text>",
        q.x,
        q.y - 1.0,
        esc(name)
    );
    if area >= 4.0 && r.kind != RoomKind::Corridor {
        let _ = writeln!(
            out,
            "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"{:.1}\" fill=\"#7a6b5b\" text-anchor=\"middle\" font-family=\"Georgia, serif\" font-style=\"italic\" paint-order=\"stroke\" stroke=\"{FLOOR}\" stroke-width=\"3\" stroke-linejoin=\"round\">{:.0} m²</text>",
            q.x,
            q.y + fs * 0.95,
            fs * 0.75,
            area
        );
    }
}

fn defs() -> String {
    format!(
        "<defs><pattern id=\"flags\" width=\"{s:.1}\" height=\"{h:.1}\" patternUnits=\"userSpaceOnUse\"><path d=\"M0,0H{s:.1}M0,{hh:.1}H{s:.1}M{q:.1},0V{hh:.1}M{tq:.1},{hh:.1}V{h:.1}\" stroke=\"#d9cfbd\" stroke-width=\"0.8\" fill=\"none\"/></pattern></defs>",
        s = SCALE * 1.2,
        h = SCALE * 1.2,
        hh = SCALE * 0.6,
        q = SCALE * 0.3,
        tq = SCALE * 0.9
    )
}

fn scale_bar(out: &mut String, x: f32, y: f32) {
    let seg = SCALE;
    let _ = write!(out, "<g font-family=\"Georgia, serif\" font-size=\"9\" fill=\"{TEXT}\">");
    for k in 0..5 {
        let fill = if k % 2 == 0 { INK } else { FLOOR };
        let _ = write!(out, "<rect x=\"{:.1}\" y=\"{y:.1}\" width=\"{seg:.1}\" height=\"5\" fill=\"{fill}\" stroke=\"{INK}\" stroke-width=\"0.8\"/>", x + k as f32 * seg);
    }
    let _ = write!(out, "<text x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\">0</text>", y + 16.0);
    let _ = write!(out, "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">5 m</text>", x + 5.0 * seg, y + 16.0);
    let _ = writeln!(out, "</g>");
}

fn north_arrow(out: &mut String, view: &View, x: f32, y: f32) {
    let n = view.dir(Vec2::Y);
    let tip = Vec2::new(x, y) + n * 13.0;
    let tail = Vec2::new(x, y) - n * 13.0;
    let side = Vec2::new(-n.y, n.x) * 5.0;
    let _ = writeln!(
        out,
        "<g><path d=\"M{:.1},{:.1}L{:.1},{:.1}L{:.1},{:.1}Z\" fill=\"{INK}\"/><path d=\"M{:.1},{:.1}L{:.1},{:.1}L{:.1},{:.1}Z\" fill=\"{FLOOR}\" stroke=\"{INK}\" stroke-width=\"0.8\"/><text x=\"{:.1}\" y=\"{:.1}\" font-size=\"10\" font-family=\"Georgia, serif\" fill=\"{TEXT}\" text-anchor=\"middle\" dominant-baseline=\"middle\">N</text></g>",
        tip.x, tip.y, (tail + side).x, (tail + side).y, Vec2::new(x, y).x, Vec2::new(x, y).y,
        tip.x, tip.y, (tail - side).x, (tail - side).y, Vec2::new(x, y).x, Vec2::new(x, y).y,
        (tip + n * 9.0).x, (tip + n * 9.0).y
    );
}

fn title(out: &mut String, x: f32, y: f32, text: &str, size: f32) {
    let _ = writeln!(
        out,
        "<text x=\"{x:.1}\" y=\"{y:.1}\" font-size=\"{size}\" fill=\"{TEXT}\" text-anchor=\"middle\" font-family=\"Georgia, 'Times New Roman', serif\" letter-spacing=\"1.5\" style=\"font-variant: small-caps\">{}</text>",
        esc(text)
    );
}

/// One floor plan as a standalone SVG document. An unknown `level` gives an
/// empty sheet with a note.
pub fn render_svg(d: &Dwelling, level: i32) -> String {
    let view = View::new(d);
    let size = view.size();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{:.0}\" height=\"{:.0}\" viewBox=\"0 0 {:.0} {:.0}\">",
        size.x,
        size.y + 24.0,
        size.x,
        size.y + 24.0
    );
    out.push_str(&defs());
    let _ = writeln!(out, "<rect width=\"100%\" height=\"100%\" fill=\"{PAPER}\"/>");
    match d.floors.iter().find(|f| f.level == level) {
        Some(f) => {
            title(&mut out, size.x * 0.5, MARGIN * 0.5 + 12.0, &level_name(level), 15.0);
            draw_floor(&mut out, d, f, &view);
        }
        None => title(&mut out, size.x * 0.5, size.y * 0.5, &format!("No level {level}"), 15.0),
    }
    scale_bar(&mut out, MARGIN, size.y - 4.0);
    north_arrow(&mut out, &view, size.x - MARGIN * 0.6, size.y - 2.0);
    out.push_str("</svg>\n");
    out
}

/// All floors side by side (lowest on the left), each titled, on one sheet.
pub fn render_svg_all(d: &Dwelling) -> String {
    let base = View::new(d);
    let cell = base.size();
    let n = d.floors.len().max(1);
    let cols = if n > 3 { n.div_ceil(2) } else { n };
    let rows = n.div_ceil(cols);
    let head = 40.0;
    let w = cell.x * cols as f32;
    let h = cell.y * rows as f32 + head + 30.0;
    let mut out = String::new();
    let _ = writeln!(out, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" viewBox=\"0 0 {w:.0} {h:.0}\">");
    out.push_str(&defs());
    let _ = writeln!(out, "<rect width=\"100%\" height=\"100%\" fill=\"{PAPER}\"/>");
    let style = format!("{:?}", d.params.style);
    title(&mut out, w * 0.5, 30.0, &format!("{style} · seed {}", d.params.seed), 20.0);
    for (k, f) in d.floors.iter().enumerate() {
        let col = k % cols;
        let row = k / cols;
        let mut view = base;
        view.off = Vec2::new(col as f32 * cell.x, head + row as f32 * cell.y);
        title(&mut out, view.off.x + cell.x * 0.5, view.off.y + MARGIN * 0.5 + 10.0, &level_name(f.level), 14.0);
        draw_floor(&mut out, d, f, &view);
    }
    scale_bar(&mut out, MARGIN, h - 22.0);
    north_arrow(&mut out, &base, w - MARGIN * 0.6, h - 22.0);
    out.push_str("</svg>\n");
    out
}
