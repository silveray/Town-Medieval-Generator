//! Generate a whole town and save it as a 3D file you can open in Blender
//! (or any glTF viewer), plus a flat SVG map and the town data as JSON.
//!
//! ```sh
//! cargo run -p mde_mesh --example town_glb --release                 # seed 1, 30 patches
//! cargo run -p mde_mesh --example town_glb --release -- 42 20 walls,river,stone,storybook
//! ```
//!
//! Arguments: `seed patches flags`. Flags is a comma list of
//! `walls,citadel,plaza,temple,river,stone` and `storybook` (cream walls, dark timber, teal or terracotta roofs).
//! Files are written to `out/town/`.

use std::path::PathBuf;

use mde_core::{BridgeKind, TownParams};
use mde_mesh::{build_town, gltf, TownMeshOptions};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seed = args.first().and_then(|s| s.parse().ok()).unwrap_or(1);
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
    let region = if has("storybook") { Some(mde_town::Region::Storybook) } else { None };
    let town = mde_town::generate_with(&params, region);
    let models = build_town(&town, &TownMeshOptions::default());

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/town");
    std::fs::create_dir_all(&dir)?;
    let name = format!("town_{seed}");
    std::fs::write(dir.join(format!("{name}.glb")), gltf::write_glb(&models))?;
    std::fs::write(dir.join(format!("{name}.svg")), mde_town::render_svg(&town))?;
    std::fs::write(dir.join(format!("{name}.json")), serde_json::to_string(&town).map_err(std::io::Error::other)?)?;
    println!("{} buildings -> {}", town.buildings.len(), dir.join(format!("{name}.glb")).display());
    Ok(())
}
