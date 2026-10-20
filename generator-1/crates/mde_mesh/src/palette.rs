//! Material slot colours and the colour helpers the builders share.
//!
//! Colours that come from [`mde_core::Appearance`] are sRGB (what a colour picker
//! shows). Meshes carry **linear** vertex colours, so everything goes through
//! [`srgb_to_linear`] first.
//!
//! Recolourable slots ([`MaterialSlot::Plaster`], `Timber`, `RoofTile`, `Thatch`,
//! `Slate`, `Planks`, `Door`, `WindowFrame`) have a (near) white base colour, so the
//! vertex colour alone decides how they look and the viewer can recolour a
//! building by rebuilding its mesh. Every other slot has its real colour here and
//! its vertex colours are only a subtle variation around white. a game engine can then use
//! one material per slot multiplied by the vertex colour.

use mde_core::{MaterialSlot, Rgba};

/// Linear RGBA colour.
pub type Linear = [f32; 4];

/// Every material slot, in declaration order.
pub const ALL_SLOTS: [MaterialSlot; 19] = [
    MaterialSlot::Plaster,
    MaterialSlot::Timber,
    MaterialSlot::Stone,
    MaterialSlot::RoofTile,
    MaterialSlot::Thatch,
    MaterialSlot::Slate,
    MaterialSlot::Planks,
    MaterialSlot::WoodFloor,
    MaterialSlot::Door,
    MaterialSlot::WindowFrame,
    MaterialSlot::Iron,
    MaterialSlot::Glass,
    MaterialSlot::Ground,
    MaterialSlot::Grass,
    MaterialSlot::Street,
    MaterialSlot::Field,
    MaterialSlot::Water,
    MaterialSlot::Foliage,
    MaterialSlot::Bark,
];

/// One sRGB channel in `[0, 1]` → linear (IEC 61966-2-1). Non-finite input maps to 0.
#[inline]
pub fn srgb_channel_to_linear(v: f32) -> f32 {
    let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// sRGB colour (as stored in [`mde_core::Appearance`]) → linear RGBA. Alpha is
/// passed through (clamped to `[0, 1]`).
pub fn srgb_to_linear(c: Rgba) -> [f32; 4] {
    let a = if c[3].is_finite() { c[3].clamp(0.0, 1.0) } else { 1.0 };
    [srgb_channel_to_linear(c[0]), srgb_channel_to_linear(c[1]), srgb_channel_to_linear(c[2]), a]
}

/// Shorthand for an opaque sRGB colour → linear.
#[inline]
pub(crate) fn lin(r: f32, g: f32, b: f32) -> Linear {
    srgb_to_linear([r, g, b, 1.0])
}

/// Whether the slot's look is decided by vertex colours (base colour ≈ white).
pub fn is_recolourable(slot: MaterialSlot) -> bool {
    use MaterialSlot as S;
    matches!(
        slot,
        S::Plaster | S::Timber | S::RoofTile | S::Thatch | S::Slate | S::Planks | S::Door | S::WindowFrame
    )
}

/// Base colour of a material slot (linear RGBA). Near white for recolourable
/// slots (see [`is_recolourable`]), the real colour otherwise.
pub fn slot_color(slot: MaterialSlot) -> [f32; 4] {
    use MaterialSlot as S;
    match slot {
        S::Plaster | S::Timber | S::RoofTile | S::Thatch | S::Slate | S::Planks | S::Door | S::WindowFrame => {
            [1.0, 1.0, 1.0, 1.0]
        }
        S::Stone => lin(0.70, 0.66, 0.58),
        S::WoodFloor => lin(0.56, 0.41, 0.28),
        S::Iron => lin(0.17, 0.17, 0.18),
        S::Glass => lin(0.23, 0.27, 0.32),
        S::Ground => lin(0.50, 0.42, 0.31),
        S::Grass => lin(0.47, 0.56, 0.30),
        S::Street => lin(0.62, 0.59, 0.54),
        S::Field => lin(0.72, 0.64, 0.40),
        S::Water => lin(0.24, 0.36, 0.40),
        S::Foliage => lin(0.40, 0.52, 0.25),
        S::Bark => lin(0.38, 0.30, 0.23),
    }
}

/// Perceptual roughness of a material slot for PBR renderers (metallic is 0
/// everywhere).
pub fn slot_roughness(slot: MaterialSlot) -> f32 {
    use MaterialSlot as S;
    match slot {
        S::Plaster => 0.92,
        S::Timber => 0.82,
        S::Stone => 0.88,
        S::RoofTile => 0.78,
        S::Thatch => 0.97,
        S::Slate => 0.62,
        S::Planks => 0.86,
        S::WoodFloor => 0.7,
        S::Door => 0.72,
        S::WindowFrame => 0.7,
        S::Iron => 0.5,
        S::Glass => 0.12,
        S::Ground => 0.96,
        S::Grass => 0.95,
        S::Street => 0.88,
        S::Field => 0.96,
        S::Water => 0.08,
        S::Foliage => 0.88,
        S::Bark => 0.92,
    }
}

/// Canonical slot name (also the glTF material name), e.g. `"RoofTile"`.
pub fn slot_name(slot: MaterialSlot) -> &'static str {
    use MaterialSlot as S;
    match slot {
        S::Plaster => "Plaster",
        S::Timber => "Timber",
        S::Stone => "Stone",
        S::RoofTile => "RoofTile",
        S::Thatch => "Thatch",
        S::Slate => "Slate",
        S::Planks => "Planks",
        S::WoodFloor => "WoodFloor",
        S::Door => "Door",
        S::WindowFrame => "WindowFrame",
        S::Iron => "Iron",
        S::Glass => "Glass",
        S::Ground => "Ground",
        S::Grass => "Grass",
        S::Street => "Street",
        S::Field => "Field",
        S::Water => "Water",
        S::Foliage => "Foliage",
        S::Bark => "Bark",
    }
}

/// White, the neutral vertex colour.
pub(crate) const WHITE: Linear = [1.0, 1.0, 1.0, 1.0];

/// Multiply the RGB channels of a linear colour by `f`.
#[inline]
pub(crate) fn scale(c: Linear, f: f32) -> Linear {
    [c[0] * f, c[1] * f, c[2] * f, c[3]]
}

/// Component-wise product of two linear colours.
#[inline]
pub(crate) fn mul(a: Linear, b: Linear) -> Linear {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
}

/// Linear interpolation between two linear colours.
#[inline]
pub(crate) fn mix(a: Linear, b: Linear, t: f32) -> Linear {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

/// Clamp RGB to `[0, 1]` (vertex colours must stay in range for glTF).
#[inline]
pub(crate) fn clamp01(c: Linear) -> Linear {
    [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0), c[3].clamp(0.0, 1.0)]
}

/// Fixed colours used for parts that `Appearance` does not cover (linear).
pub(crate) mod fixed {
    use super::{lin, Linear};

    /// Interior whitewash.
    pub fn whitewash() -> Linear {
        lin(0.90, 0.87, 0.80)
    }
    /// Dark void behind bare openings (custom parts are dropped in front of it).
    pub fn void() -> Linear {
        lin(0.06, 0.055, 0.05)
    }
    /// Linen for beds.
    pub fn linen() -> Linear {
        lin(0.86, 0.82, 0.72)
    }
    /// Red wool blankets / altar cloth.
    pub fn wool() -> Linear {
        lin(0.55, 0.20, 0.16)
    }
    /// Brick red, as a tint over the Stone slot.
    pub fn brick() -> Linear {
        lin(0.92, 0.62, 0.52)
    }
    /// Warm sandstone tint for quoins and dressed stone.
    pub fn dressed_stone() -> Linear {
        lin(1.0, 0.95, 0.84)
    }
    /// Soot / fire-back.
    pub fn soot() -> Linear {
        lin(0.18, 0.16, 0.15)
    }
    /// Glowing embers / lantern glass.
    pub fn ember() -> Linear {
        lin(1.0, 0.62, 0.25)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_linear_endpoints() {
        assert_eq!(srgb_to_linear([0.0, 1.0, 0.5, 1.0])[..2], [0.0, 1.0]);
        let mid = srgb_to_linear([0.5, 0.5, 0.5, 0.25]);
        assert!((mid[0] - 0.214).abs() < 1e-3);
        assert_eq!(mid[3], 0.25);
        assert_eq!(srgb_to_linear([f32::NAN, 2.0, -1.0, f32::INFINITY]), [0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn recolourable_slots_are_white() {
        for s in ALL_SLOTS {
            let c = slot_color(s);
            if is_recolourable(s) {
                assert!(c[..3].iter().all(|&v| v > 0.9), "{s:?}");
            }
            assert!(c.iter().all(|v| (0.0..=1.0).contains(v)));
            assert!((0.0..=1.0).contains(&slot_roughness(s)));
            assert_eq!(slot_name(s), format!("{s:?}"));
        }
    }
}
