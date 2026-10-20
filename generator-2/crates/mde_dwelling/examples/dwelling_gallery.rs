//! Contact sheets for eyeballing many seeds at once:
//! `out/mde_dwelling/gallery_<style>.svg`, 12 seeds per style (all floors).
//!
//! ```text
//! cargo run -p mde_dwelling --example dwelling_gallery -- [first_seed] [floors]
//! ```

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use mde_core::{DwellingParams, DwellingStyle};

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let first: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let floors: u8 = args.next().and_then(|s| s.parse().ok()).unwrap_or(2);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/mde_dwelling");
    fs::create_dir_all(&dir)?;
    for (name, style) in [
        ("cottage", DwellingStyle::Cottage),
        ("townhouse", DwellingStyle::Townhouse),
        ("farmhouse", DwellingStyle::Farmhouse),
        ("manor", DwellingStyle::Manor),
        ("tavern", DwellingStyle::Tavern),
        ("shop", DwellingStyle::Shop),
    ] {
        let mut sheets = Vec::new();
        for seed in first..first + 12 {
            let d = mde_dwelling::generate(&DwellingParams { seed, style, floors, basement: seed % 3 == 0, ..Default::default() });
            sheets.push(mde_dwelling::render_svg_all(&d));
        }
        // Nest the sheets in a 3-column grid.
        let size = |s: &str, attr: &str| -> f32 {
            s.split(&format!("{attr}=\"")).nth(1).and_then(|r| r.split('"').next()).and_then(|v| v.parse().ok()).unwrap_or(400.0)
        };
        let cw = sheets.iter().map(|s| size(s, "width")).fold(0.0, f32::max);
        let ch = sheets.iter().map(|s| size(s, "height")).fold(0.0, f32::max);
        let mut out = String::new();
        let _ = writeln!(out, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{:.0}\" height=\"{:.0}\">", cw * 3.0, ch * 4.0);
        for (k, s) in sheets.iter().enumerate() {
            let inner = s.replacen("<svg ", &format!("<svg x=\"{:.0}\" y=\"{:.0}\" ", (k % 3) as f32 * cw, (k / 3) as f32 * ch), 1);
            out.push_str(&inner);
        }
        out.push_str("</svg>\n");
        let path = dir.join(format!("gallery_{name}.svg"));
        fs::write(&path, out)?;
        println!("{}", path.display());
    }
    Ok(())
}
