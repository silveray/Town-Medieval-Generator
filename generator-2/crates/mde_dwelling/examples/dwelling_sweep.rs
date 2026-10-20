//! Stress sweep: generate many dwellings and summarise validation problems.
//! `cargo run -p mde_dwelling --example dwelling_sweep -- [seeds]`
use std::collections::BTreeMap;

use mde_core::{DwellingParams, DwellingStyle, Polygon, Rng, Vec2};

fn main() {
    let seeds: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(40);
    let styles = [DwellingStyle::Cottage, DwellingStyle::Townhouse, DwellingStyle::Farmhouse, DwellingStyle::Manor, DwellingStyle::Tavern, DwellingStyle::Shop];
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut examples: BTreeMap<String, String> = BTreeMap::new();
    let mut total = 0;
    let mut bad = 0;
    let t = std::time::Instant::now();
    for seed in 0..seeds {
        for style in styles {
            for floors in 1..=4u8 {
                for basement in [false, true] {
                    let p = DwellingParams { seed, style, floors, basement, ..Default::default() };
                    check(&p, &format!("{style:?} s{seed} f{floors} b{}", basement as u8), &mut kinds, &mut examples, &mut total, &mut bad);
                }
            }
            // Random rotated convex quad lot.
            let mut r = Rng::new(seed * 31 + style as u64);
            let area = r.range_f32(40.0, 300.0);
            let aspect = r.range_f32(0.5, 2.0);
            let w = (area * aspect).sqrt();
            let h = area / w;
            let jit = |r: &mut Rng| Vec2::new(r.range_f32(-0.12, 0.12) * w, r.range_f32(-0.12, 0.12) * h);
            let pts = vec![
                Vec2::new(-w / 2.0, -h / 2.0) + jit(&mut r),
                Vec2::new(w / 2.0, -h / 2.0) + jit(&mut r),
                Vec2::new(w / 2.0, h / 2.0) + jit(&mut r),
                Vec2::new(-w / 2.0, h / 2.0) + jit(&mut r),
            ];
            let ang = r.range_f32(0.0, std::f32::consts::TAU);
            let lot = Polygon(pts).rotated(Vec2::ZERO, ang).translated(Vec2::new(r.range_f32(-50.0, 50.0), r.range_f32(-50.0, 50.0)));
            let front = Vec2::from_angle(ang - std::f32::consts::FRAC_PI_2 + r.range_f32(-0.3, 0.3));
            let floors = r.range_i32(1, 5) as u8;
            let p = DwellingParams { seed, style, floors, basement: r.chance(0.5), footprint: lot, front, ..Default::default() };
            check(&p, &format!("LOT {style:?} s{seed} f{floors} b{}", p.basement as u8), &mut kinds, &mut examples, &mut total, &mut bad);
        }
    }
    println!("{bad}/{total} dwellings with problems in {:.1}s (generate {:.1}s, validate {:.1}s)", t.elapsed().as_secs_f32(), GEN.with(|g| g.get()), VAL.with(|g| g.get()));
    for (k, n) in &kinds {
        println!("{n:6}  {k}   e.g. {}", examples[k]);
    }
}

thread_local! {
    static GEN: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
    static VAL: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
}

fn check(p: &DwellingParams, name: &str, kinds: &mut BTreeMap<String, usize>, ex: &mut BTreeMap<String, String>, total: &mut usize, bad: &mut usize) {
    let t0 = std::time::Instant::now();
    let d = mde_dwelling::generate(p);
    let t1 = std::time::Instant::now();
    let errs = mde_dwelling::validate(&d);
    GEN.with(|g| g.set(g.get() + (t1 - t0).as_secs_f64()));
    VAL.with(|g| g.set(g.get() + t1.elapsed().as_secs_f64()));
    *total += 1;
    if !errs.is_empty() {
        *bad += 1;
    }
    for e in errs {
        // Normalise numbers away to group problems.
        let key: String = e.chars().map(|c| if c.is_ascii_digit() || c == '.' || c == '-' { '#' } else { c }).collect();
        let key = key.split("##").collect::<Vec<_>>().join("#");
        *kinds.entry(key.clone()).or_default() += 1;
        ex.entry(key).or_insert_with(|| {
            let _ = std::fs::create_dir_all("out/mde_dwelling/fail");
            let file = format!("out/mde_dwelling/fail/{}.json", name.replace(' ', "_"));
            let _ = std::fs::write(&file, serde_json::to_string(p).unwrap_or_default());
            format!("{name}: {e}  [{file}]")
        });
    }
}
