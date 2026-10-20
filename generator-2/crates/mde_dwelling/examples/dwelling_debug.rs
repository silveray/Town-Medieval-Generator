//! Debug helper: generate one dwelling and print validation problems.
//! `cargo run -p mde_dwelling --example dwelling_debug -- <style> <seed> <floors> <basement>`
use mde_core::{DwellingParams, DwellingStyle};
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let style = match a.first().map(|s| s.as_str()).unwrap_or("cottage") {
        "townhouse" => DwellingStyle::Townhouse,
        "farmhouse" => DwellingStyle::Farmhouse,
        "manor" => DwellingStyle::Manor,
        "tavern" => DwellingStyle::Tavern,
        "shop" => DwellingStyle::Shop,
        _ => DwellingStyle::Cottage,
    };
    let seed = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(7);
    let floors = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(2);
    let basement = a.get(3).is_some_and(|s| s == "1");
    let params = match a.first().filter(|s| s.ends_with(".json")) {
        Some(f) => serde_json::from_str(&std::fs::read_to_string(f).expect("read")).expect("json"),
        None => DwellingParams { seed, style, floors, basement, ..Default::default() },
    };
    let d = mde_dwelling::generate(&params);
    for p in mde_dwelling::validate(&d) {
        println!("{p}");
    }
    if let Ok(l) = std::env::var("MDE_WALK") {
        let level: i32 = l.parse().unwrap_or(0);
        let cl: f32 = std::env::var("MDE_CL").ok().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        println!("{}", mde_dwelling::validate::walk_map(&d, level, 0.25, cl));
    }
    if std::env::var("MDE_PROPS").is_ok() {
        for f in &d.floors {
            for (k, p) in f.props.iter().enumerate() {
                println!("L{} prop {k} {:?} pos {} size {} rot {:.4}", f.level, p.kind, p.pos, p.size, p.rotation);
            }
            for w in &f.walls {
                println!("L{} wall {} -> {} th {}", f.level, w.a, w.b, w.thickness);
            }
        }
    }
    std::fs::create_dir_all("out/mde_dwelling").ok();
    std::fs::write("out/mde_dwelling/debug.svg", mde_dwelling::render_svg_all(&d)).ok();
}
