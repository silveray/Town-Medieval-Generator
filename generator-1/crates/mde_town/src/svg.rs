//! Ink-on-parchment map rendering of a [`Town`] as SVG.

use std::fmt::Write;

use mde_core::{BridgeKind, Polygon, StreetKind, Town, Vec2, WardKind};

const PARCHMENT: &str = "#efe5c9";
const PARCHMENT_EDGE: &str = "#d6c49b";
const STREET: &str = "#f7f0de";
const CASING: &str = "#b9a682";
const INK: &str = "#33291f";
const WATER: &str = "#a7c3cd";
const BANK: &str = "#7f9ea9";
const WALL: &str = "#4a3d30";
const TOWER: &str = "#665645";
const FIELD_TINTS: [&str; 5] = ["#e2d39f", "#d8d29a", "#e7dbb0", "#ced096", "#dccb94"];

fn ward_tint(w: WardKind) -> &'static str {
    match w {
        WardKind::Craftsmen => "#e7d9b8",
        WardKind::Merchant => "#e9d5ae",
        WardKind::Patriciate => "#e2dabf",
        WardKind::Administration => "#e5d4b5",
        WardKind::Military => "#dbcdb0",
        WardKind::Slum => "#dfcba6",
        WardKind::Market => "#efe4c7",
        WardKind::Cathedral => "#e4dac2",
        WardKind::Castle => "#d7caad",
        WardKind::Park => "#cbd5a1",
        WardKind::Gate => "#e7dbb8",
        WardKind::Farm => "#e3d9ad",
        WardKind::Empty => "#e5dfbd",
    }
}

/// Plan → SVG coordinates (y flipped so north is up).
fn p(v: Vec2) -> (f32, f32) {
    (v.x, -v.y)
}

fn path_d(pts: &[Vec2], closed: bool) -> String {
    let mut s = String::with_capacity(pts.len() * 14);
    for (i, &v) in pts.iter().enumerate() {
        let (x, y) = p(v);
        let _ = write!(s, "{}{:.1} {:.1}", if i == 0 { "M" } else { "L" }, x, y);
    }
    if closed {
        s.push('Z');
    }
    s
}

fn poly(out: &mut String, pts: &[Vec2], attrs: &str) {
    if pts.len() >= 3 {
        let _ = writeln!(out, r#"<path d="{}" {attrs}/>"#, path_d(pts, true));
    }
}

/// `[t0, t1]` of the line `o + d t` inside a convex CCW ring.
fn clip_line(ring: &[Vec2], o: Vec2, d: Vec2) -> Option<(f32, f32)> {
    let (mut t0, mut t1) = (f32::NEG_INFINITY, f32::INFINITY);
    let n = ring.len();
    for i in 0..n {
        let a = ring[i];
        let e = ring[(i + 1) % n] - a;
        let inward = Vec2::new(-e.y, e.x);
        let num = (o - a).dot(inward);
        let den = d.dot(inward);
        if den.abs() < 1e-9 {
            if num < 0.0 {
                return None;
            }
            continue;
        }
        let t = -num / den;
        if den > 0.0 {
            t0 = t0.max(t);
        } else {
            t1 = t1.min(t);
        }
    }
    (t1 > t0).then_some((t0, t1))
}

/// Furrow hatching along a field's longest edge.
fn furrows(field: &Polygon, spacing: f32) -> String {
    let pts = &field.0;
    let n = pts.len();
    let mut dir = Vec2::X;
    let mut best = 0.0;
    for i in 0..n {
        let d = pts[(i + 1) % n] - pts[i];
        if d.length() > best {
            best = d.length();
            dir = d / best;
        }
    }
    let nrm = Vec2::new(-dir.y, dir.x);
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for v in pts {
        lo = lo.min(v.dot(nrm));
        hi = hi.max(v.dot(nrm));
    }
    let mut s = String::new();
    let mut k = lo + spacing * 0.5;
    while k < hi {
        let o = nrm * k;
        if let Some((t0, t1)) = clip_line(pts, o, dir) {
            let (a, b) = (o + dir * (t0 + 0.6), o + dir * (t1 - 0.6));
            if t1 - t0 > 1.5 {
                let (ax, ay) = p(a);
                let (bx, by) = p(b);
                let _ = write!(s, "M{ax:.1} {ay:.1}L{bx:.1} {by:.1}");
            }
        }
        k += spacing;
    }
    s
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> String {
    let c = |i: usize| ((a[i] * (1.0 - t) + b[i] * t).clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", c(0), c(1), c(2))
}

/// Render the town as a watabou-style ink map: parchment ground, ward-tinted
/// blocks, dark building footprints, light streets, thick walls with towers
/// and gates, striped fields, trees as circles, river and bridges.
pub fn render_svg(town: &Town) -> String {
    let map_half =
        town.patches.iter().flat_map(|pa| pa.shape.0.iter().map(|v| v.x.abs().max(v.y.abs()))).fold(100.0f32, f32::max);
    let view = (town.radius * 1.55 + 60.0).min(map_half);
    let (cx, cy) = p(town.center);
    let (x0, y0, size) = (cx - view, cy - view, view * 2.0);
    let px = 1600.0;
    let k = size / px; // metres per pixel
    let mut s = String::with_capacity(1 << 20);
    let _ = writeln!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{x0:.1} {y0:.1} {size:.1} {size:.1}" width="{px}" height="{px}">"#
    );
    let _ = writeln!(
        s,
        r#"<defs><radialGradient id="vig" cx="50%" cy="50%" r="72%"><stop offset="62%" stop-color="{PARCHMENT_EDGE}" stop-opacity="0"/><stop offset="100%" stop-color="{PARCHMENT_EDGE}" stop-opacity="0.85"/></radialGradient></defs>"#
    );
    let _ = writeln!(s, r#"<rect x="{x0:.1}" y="{y0:.1}" width="{size:.1}" height="{size:.1}" fill="{PARCHMENT}"/>"#);

    // Meadow tint on unbuilt countryside.
    for pa in &town.patches {
        if !pa.within_city && pa.ward == WardKind::Empty {
            poly(&mut s, &pa.shape.0, &format!(r#"fill="{}" opacity="0.7""#, ward_tint(WardKind::Empty)));
        }
    }
    // Fields with furrows.
    for (i, f) in town.fields.iter().enumerate() {
        let tint = FIELD_TINTS[(i * 7 + f.0.len()) % FIELD_TINTS.len()];
        poly(&mut s, &f.0, &format!(r##"fill="{tint}" stroke="#bfae80" stroke-width="0.5""##));
        let d = furrows(f, 2.6);
        if !d.is_empty() {
            let _ = writeln!(
                s,
                r#"<path d="{d}" stroke="rgb(130,110,70)" stroke-opacity="0.35" stroke-width="0.45" fill="none"/>"#
            );
        }
    }
    // Ward-tinted blocks (inner patches and built suburbs).
    for pa in &town.patches {
        if pa.within_city || matches!(pa.ward, WardKind::Slum | WardKind::Gate) {
            poly(&mut s, &pa.shape.0, &format!(r#"fill="{}""#, ward_tint(pa.ward)));
        }
    }
    // River.
    if let Some(r) = &town.river {
        let d = path_d(&r.path.0, false);
        let _ = writeln!(
            s,
            r#"<path d="{d}" fill="none" stroke="{BANK}" stroke-width="{:.1}" stroke-linejoin="round" stroke-linecap="round"/>"#,
            r.width + 2.6
        );
        let _ = writeln!(
            s,
            r#"<path d="{d}" fill="none" stroke="{WATER}" stroke-width="{:.1}" stroke-linejoin="round" stroke-linecap="round"/>"#,
            r.width
        );
        let _ = writeln!(
            s,
            r##"<path d="{d}" fill="none" stroke="#c4d8de" stroke-width="{:.1}" stroke-opacity="0.6" stroke-linejoin="round" stroke-dasharray="14 22"/>"##,
            r.width * 0.25
        );
    }
    // Streets: casing then fill, widest last so junctions read cleanly.
    let mut order: Vec<&mde_core::Street> = town.streets.iter().collect();
    order.sort_by(|a, b| a.width.total_cmp(&b.width));
    for st in &order {
        let d = path_d(&st.path.0, false);
        let _ = writeln!(
            s,
            r#"<path d="{d}" fill="none" stroke="{CASING}" stroke-width="{:.2}" stroke-linejoin="round" stroke-linecap="round"/>"#,
            st.width + 1.1
        );
    }
    for st in &order {
        let d = path_d(&st.path.0, false);
        let col = if st.kind == StreetKind::Road { "#f1e7cc" } else { STREET };
        let _ = writeln!(
            s,
            r#"<path d="{d}" fill="none" stroke="{col}" stroke-width="{:.2}" stroke-linejoin="round" stroke-linecap="round"/>"#,
            st.width
        );
    }
    // Plaza / green.
    if let Some(pl) = &town.plaza {
        let fill = if town.wall.is_none() && town.params.patches < 25 { "#cfd8a3" } else { "#f1e8d0" };
        poly(&mut s, &pl.0, &format!(r#"fill="{fill}" stroke="{CASING}" stroke-width="0.6""#));
    }
    // Bridges.
    for b in &town.bridges {
        let d = (b.b - b.a).normalize_or_zero();
        let nrm = Vec2::new(-d.y, d.x) * (b.width * 0.5);
        let deck = [b.a - nrm, b.b - nrm, b.b + nrm, b.a + nrm];
        let (fill, rail) = match b.kind {
            BridgeKind::Wood => ("#a3845d", "#5a4530"),
            BridgeKind::Stone => ("#b3ab9b", "#5b5449"),
        };
        poly(&mut s, &deck, &format!(r#"fill="{fill}" stroke="{rail}" stroke-width="0.8""#));
        if b.kind == BridgeKind::Wood {
            let len = b.a.distance(b.b);
            let mut t = 0.8;
            let mut planks = String::new();
            while t < len {
                let c = b.a + d * t;
                let (ax, ay) = p(c - nrm * 0.9);
                let (bx, by) = p(c + nrm * 0.9);
                let _ = write!(planks, "M{ax:.1} {ay:.1}L{bx:.1} {by:.1}");
                t += 1.2;
            }
            let _ = writeln!(s, r#"<path d="{planks}" stroke="{rail}" stroke-width="0.25" stroke-opacity="0.6"/>"#);
        }
    }
    // Buildings.
    for b in &town.buildings {
        let special = matches!(b.kind, mde_core::BuildingKind::Temple | mde_core::BuildingKind::Keep);
        let r = b.appearance.roof;
        let fill = if special { "#5d4f3f".to_string() } else { mix([0.48, 0.42, 0.34], [r[0], r[1], r[2]], 0.28) };
        poly(&mut s, &b.footprint.0, &format!(r#"fill="{fill}" stroke="{INK}" stroke-width="0.45""#));
    }
    // Trees: shadow then crown.
    let mut shadows = String::new();
    let mut crowns = String::new();
    for t in &town.trees {
        let (x, y) = p(t.pos);
        let _ = write!(shadows, r#"<circle cx="{:.1}" cy="{:.1}" r="{:.1}"/>"#, x + 0.7, y + 0.7, t.radius);
        let _ = write!(crowns, r#"<circle cx="{x:.1}" cy="{y:.1}" r="{:.1}"/>"#, t.radius);
    }
    let _ = writeln!(s, r##"<g fill="#7b8650" opacity="0.55">{shadows}</g>"##);
    let _ = writeln!(s, r##"<g fill="#a9b57c" stroke="#5c6640" stroke-width="0.4">{crowns}</g>"##);
    // Walls, towers, gates.
    for (wall, scale) in town.wall.iter().map(|w| (w, 1.6)).chain(town.citadel.iter().map(|w| (w, 1.5))) {
        let d = path_d(&wall.shape.0, true);
        let _ = writeln!(
            s,
            r#"<path d="{d}" fill="none" stroke="{WALL}" stroke-width="{:.2}" stroke-linejoin="round"/>"#,
            wall.thickness * scale
        );
        let n = wall.shape.len();
        for t in &wall.towers {
            let (x, y) = p(*t);
            let _ = writeln!(
                s,
                r#"<circle cx="{x:.1}" cy="{y:.1}" r="{:.1}" fill="{TOWER}" stroke="{INK}" stroke-width="0.6"/>"#,
                3.2 * scale / 1.6
            );
        }
        for g in &wall.gates {
            let i = wall.shape.0.iter().position(|v| v.distance(*g) < 0.01);
            let dir = match i {
                Some(i) => (wall.shape.0[(i + 1) % n] - wall.shape.0[(i + n - 1) % n]).normalize_or_zero(),
                None => Vec2::X,
            };
            let (ax, ay) = p(*g - dir * 2.8);
            let (bx, by) = p(*g + dir * 2.8);
            let _ = writeln!(
                s,
                r#"<path d="M{ax:.1} {ay:.1}L{bx:.1} {by:.1}" stroke="{STREET}" stroke-width="{:.1}"/>"#,
                wall.thickness * scale + 0.4
            );
            for side in [-1.0f32, 1.0] {
                let (x, y) = p(*g + dir * (4.6 * side));
                let _ = writeln!(
                    s,
                    r#"<rect x="{:.1}" y="{:.1}" width="4.2" height="4.2" fill="{TOWER}" stroke="{INK}" stroke-width="0.6"/>"#,
                    x - 2.1,
                    y - 2.1
                );
            }
        }
    }
    // Vignette, frame, compass and scale bar.
    let _ = writeln!(s, r#"<rect x="{x0:.1}" y="{y0:.1}" width="{size:.1}" height="{size:.1}" fill="url(#vig)"/>"#);
    let _ = writeln!(
        s,
        r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="none" stroke="{INK}" stroke-width="{:.2}"/>"#,
        x0 + 10.0 * k,
        y0 + 10.0 * k,
        size - 20.0 * k,
        size - 20.0 * k,
        2.0 * k
    );
    let bar = [50.0f32, 100.0, 200.0, 500.0].into_iter().find(|b| *b > size * 0.08).unwrap_or(500.0);
    let (bx, by) = (x0 + 40.0 * k, y0 + size - 50.0 * k);
    let _ = writeln!(
        s,
        r#"<g stroke="{INK}" stroke-width="{:.2}" fill="none"><path d="M{bx:.1} {by:.1}h{bar:.1}M{bx:.1} {:.1}v{:.1}M{:.1} {:.1}v{:.1}M{:.1} {:.1}v{:.1}"/></g>"#,
        2.0 * k,
        by - 6.0 * k,
        12.0 * k,
        bx + bar,
        by - 6.0 * k,
        12.0 * k,
        bx + bar * 0.5,
        by - 4.0 * k,
        8.0 * k
    );
    let _ = writeln!(
        s,
        r#"<text x="{:.1}" y="{:.1}" font-family="Georgia, serif" font-size="{:.1}" fill="{INK}">{bar} m</text>"#,
        bx + bar + 8.0 * k,
        by + 5.0 * k,
        16.0 * k
    );
    let (nx, ny) = (x0 + size - 60.0 * k, y0 + 70.0 * k);
    let _ = writeln!(
        s,
        r#"<g fill="{INK}"><path d="M{nx:.1} {:.1}L{:.1} {:.1}L{nx:.1} {:.1}L{:.1} {:.1}Z" fill-opacity="0.85"/><text x="{:.1}" y="{:.1}" font-family="Georgia, serif" font-size="{:.1}">N</text></g>"#,
        ny - 32.0 * k,
        nx + 10.0 * k,
        ny + 10.0 * k,
        ny + 2.0 * k,
        nx - 10.0 * k,
        ny + 10.0 * k,
        nx - 6.0 * k,
        ny + 32.0 * k,
        18.0 * k
    );
    s.push_str("</svg>\n");
    s
}
