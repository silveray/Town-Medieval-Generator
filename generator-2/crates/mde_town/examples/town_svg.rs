//! Write sample town maps to `out/mde_town/`.
//!
//! ```sh
//! cargo run -p mde_town --example town_svg --release            # the three presets
//! cargo run -p mde_town --example town_svg --release -- 42 30 walls,river,stone
//! ```
//!
//! Presets: an open village without walls, a walled town, and a walled city
//! with a river. With arguments: `seed patches [flags]` where flags is a comma
//! list of `walls,citadel,plaza,temple,river,stone`. `bench` times 60-patch
//! cities over 20 seeds (no files written).

use std::path::PathBuf;
use std::time::Instant;

use mde_core::{BridgeKind, TownParams};

fn write(name: &str, params: &TownParams) -> std::io::Result<()> {
    let t = Instant::now();
    let town = mde_town::generate(params);
    let dt = t.elapsed();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/mde_town");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{name}.svg"));
    std::fs::write(&path, mde_town::render_svg(&town))?;
    let json = serde_json::to_string(&town).map_err(std::io::Error::other)?;
    std::fs::write(dir.join(format!("{name}.json")), json)?;
    println!(
        "{}: {} patches, {} buildings, {} streets, {} fields, {} trees, {} bridges in {:.1} ms",
        path.display(),
        town.patches.len(),
        town.buildings.len(),
        town.streets.len(),
        town.fields.len(),
        town.trees.len(),
        town.bridges.len(),
        dt.as_secs_f64() * 1e3
    );
    Ok(())
}

fn bench() {
    for river in [false, true] {
        let mut times = Vec::new();
        let mut buildings = 0;
        for seed in 0..20 {
            let p = TownParams { seed, patches: 60, river, ..Default::default() };
            let t = Instant::now();
            let town = mde_town::generate(&p);
            times.push(t.elapsed().as_secs_f64() * 1e3);
            buildings += town.buildings.len();
        }
        times.sort_by(f64::total_cmp);
        println!(
            "patches=60 river={river}: median {:.1} ms, max {:.1} ms, {} buildings/town",
            times[times.len() / 2],
            times[times.len() - 1],
            buildings / 20
        );
    }
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "bench") {
        bench();
        return Ok(());
    }
    if !args.is_empty() {
        let seed = args[0].parse().unwrap_or(1);
        let patches = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
        let flags = args.get(2).cloned().unwrap_or_else(|| "walls,citadel,plaza,temple".into());
        let has = |f: &str| flags.split(',').any(|x| x == f);
        let params = TownParams {
            seed,
            patches,
            walls: has("walls"),
            citadel: has("citadel"),
            plaza: has("plaza"),
            temple: has("temple"),
            river: has("river"),
            bridge_kind: if has("stone") { BridgeKind::Stone } else { BridgeKind::Wood },
            ..Default::default()
        };
        return write(&format!("town_{seed}"), &params);
    }
    let village = TownParams {
        seed: 7,
        patches: 12,
        walls: false,
        citadel: false,
        plaza: true,
        temple: true,
        river: false,
        ..Default::default()
    };
    write(&format!("town_{}", village.seed), &village)?;
    let town = TownParams { seed: 21, patches: 30, ..Default::default() };
    write(&format!("town_{}", town.seed), &town)?;
    let city =
        TownParams { seed: 1234, patches: 60, river: true, bridge_kind: BridgeKind::Stone, ..Default::default() };
    write(&format!("town_{}", city.seed), &city)
}
