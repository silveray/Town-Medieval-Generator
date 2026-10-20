//! Prints triangle counts per material slot for the fixture models and the
//! lot-house decoration levels (diagnostics for the triangle budgets).

use mde_core::{Model, Rng};
use mde_mesh::{build_dwelling, fixtures, lot_house_at_level, DwellingMeshOptions};

fn breakdown(name: &str, m: &Model) {
    let parts: Vec<String> = m.parts.iter().map(|p| format!("{:?}:{}", p.material, p.mesh.triangle_count())).collect();
    println!("{name:<24} {:>6}  {}", m.triangle_count(), parts.join(" "));
}

fn main() {
    for m in mde_mesh::build_town(&fixtures::town(), &Default::default()) {
        winding_report(&m.name.clone(), &m);
    }
    winding_report("rect_house", &build_dwelling(&fixtures::rect_house(), &DwellingMeshOptions::default()));
    winding_report("l_house", &build_dwelling(&fixtures::l_house(), &DwellingMeshOptions::default()));
    breakdown("rect_house", &build_dwelling(&fixtures::rect_house(), &DwellingMeshOptions::default()));
    breakdown("l_house", &build_dwelling(&fixtures::l_house(), &DwellingMeshOptions::default()));
    let town = fixtures::town();
    let mut rng = Rng::new(1);
    for b in town.buildings.iter().take(8).chain(town.buildings.iter().rev().take(8)) {
        for level in 0..5 {
            breakdown(&format!("{:?}#{} L{level}", b.kind, b.id), &lot_house_at_level(b, None, level));
        }
        let _ = rng.next_u64();
    }
}

/// Count triangles whose winding disagrees with their vertex normals.
#[allow(dead_code)]
pub fn winding_report(name: &str, m: &Model) {
    for p in &m.parts {
        let mm = &p.mesh;
        let mut bad = 0;
        let mut first = None;
        for t in mm.indices.as_chunks::<3>().0 {
            let v = |i: u32| mde_core::Vec3::from_array(mm.positions[i as usize]);
            let (a, b, c) = (v(t[0]), v(t[1]), v(t[2]));
            let n = (b - a).cross(c - a);
            if n.length() < 1e-9 {
                continue;
            }
            let vn = mde_core::Vec3::from_array(mm.normals[t[0] as usize])
                + mde_core::Vec3::from_array(mm.normals[t[1] as usize])
                + mde_core::Vec3::from_array(mm.normals[t[2] as usize]);
            if n.normalize().dot(vn.normalize()) < 0.0 {
                bad += 1;
                if first.is_none() {
                    first = Some((a, b, c));
                }
            }
        }
        if bad > 0 {
            println!("{name}: {:?} {bad} flipped of {} (first {:?})", p.material, mm.triangle_count(), first);
        }
    }
}
