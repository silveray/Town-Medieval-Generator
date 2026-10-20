//! Write sample floor plans to `out/mde_dwelling/`: one sheet per style (all
//! floors side by side) plus houses on rotated, non-rectangular town lots.
//!
//! ```text
//! cargo run -p mde_dwelling --example dwelling_svg -- [seed] [floors]
//! ```

use std::fs;
use std::path::PathBuf;

use mde_core::{DwellingParams, DwellingStyle, Polygon, Vec2};

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let seed: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(7);
    let floors: u8 = args.next().and_then(|s| s.parse().ok()).unwrap_or(2);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/mde_dwelling");
    fs::create_dir_all(&dir)?;
    let styles = [
        ("cottage", DwellingStyle::Cottage),
        ("townhouse", DwellingStyle::Townhouse),
        ("farmhouse", DwellingStyle::Farmhouse),
        ("manor", DwellingStyle::Manor),
        ("tavern", DwellingStyle::Tavern),
        ("shop", DwellingStyle::Shop),
    ];
    for (name, style) in styles {
        let params = DwellingParams { seed, style, floors, basement: seed.is_multiple_of(2), ..Default::default() };
        let d = mde_dwelling::generate(&params);
        write(&dir, &format!("{name}_{seed}"), &d)?;
    }
    // A townhouse on a skewed, rotated street lot (front faces south-west).
    let lot = Polygon(vec![Vec2::new(0.0, 0.0), Vec2::new(7.5, -2.0), Vec2::new(10.5, 9.0), Vec2::new(1.5, 11.5)]);
    let lot = lot.rotated(Vec2::ZERO, 0.4);
    let front = Vec2::new(0.2, -1.0).rotate(Vec2::from_angle(0.4)).normalize();
    let params = DwellingParams { seed, style: DwellingStyle::Townhouse, floors: 3, basement: true, footprint: lot, front, ..Default::default() };
    let d = mde_dwelling::generate(&params);
    write(&dir, &format!("lot_townhouse_{seed}"), &d)?;
    // A tavern on a trapezoid lot.
    let lot = Polygon(vec![Vec2::new(0.0, 0.0), Vec2::new(15.0, 0.0), Vec2::new(13.0, 11.0), Vec2::new(1.0, 12.5)]).rotated(Vec2::ZERO, -0.7);
    let params = DwellingParams { seed, style: DwellingStyle::Tavern, floors: 2, footprint: lot, ..Default::default() };
    let d = mde_dwelling::generate(&params);
    write(&dir, &format!("lot_tavern_{seed}"), &d)?;
    Ok(())
}

fn write(dir: &std::path::Path, stem: &str, d: &mde_core::Dwelling) -> std::io::Result<()> {
    let problems = mde_dwelling::validate(d);
    let path = dir.join(format!("{stem}.svg"));
    fs::write(&path, mde_dwelling::render_svg_all(d))?;
    let rooms: usize = d.floors.iter().map(|f| f.rooms.len()).sum();
    let props: usize = d.floors.iter().map(|f| f.props.len()).sum();
    println!("{} — {} floors, {rooms} rooms, {props} props, {} problems", path.display(), d.floors.len(), problems.len());
    for p in problems.iter().take(8) {
        println!("    {p}");
    }
    Ok(())
}
