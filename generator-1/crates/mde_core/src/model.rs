//! The data contract shared by all generators and consumers.
//!
//! Everything here is plain data with `serde` derives. The JSON form of [`Town`],
//! [`Dwelling`] and [`Model`] is the interchange format for the CLI and other tools,
//! so changes must bump [`SCHEMA_VERSION`].
//!
//! All 2D coordinates are plan space (metres, x east, y north, CCW polygons).

use glam::Vec2;
use serde::{Deserialize, Serialize};

/// Bump on any breaking change to the serialised shape of this module.
pub const SCHEMA_VERSION: u32 = 1;

/// Closed polygon: CCW, metres, no repeated closing vertex.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Polygon(pub Vec<Vec2>);

/// Open polyline, metres.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Polyline(pub Vec<Vec2>);

/// sRGB colour with alpha, components in `[0, 1]` — the values a colour picker
/// shows. Convert to linear before using as a shader / vertex colour.
pub type Rgba = [f32; 4];

// ---------------------------------------------------------------------------
// Appearance (user-customisable look of a building)
// ---------------------------------------------------------------------------

/// Procedural window designs. [`Appearance::custom_window`] overrides these with
/// a model from the part catalogue.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WindowKind {
    /// Simple rectangular casement with a cross bar.
    #[default]
    Casement,
    /// Stone mullion and transom dividing it into four lights.
    Mullioned,
    /// Round-headed arch.
    Arched,
    /// Pointed (gothic) arch.
    Gothic,
    /// Small round oculus.
    Round,
    /// Diamond-leaded lattice panes.
    Lattice,
}

/// Procedural door designs. [`Appearance::custom_door`] overrides these with a
/// model from the part catalogue.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DoorKind {
    /// Vertical planks with horizontal ledges.
    #[default]
    Plank,
    /// Plank door under a round arch.
    Arched,
    /// Two leaves (wider openings, shops, taverns).
    Double,
    /// Split horizontally into a top and bottom half.
    Dutch,
    /// Iron-studded heavy door (manors, keeps, temples).
    Studded,
    /// Door with a small glazed panel.
    Glazed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RoofMaterial {
    /// Clay tiles.
    #[default]
    Tiles,
    Slate,
    Thatch,
    /// Wooden shingles.
    Shingles,
}

/// Colours and part choices of one building. Generators pick a pleasant
/// default per building; the viewer lets the user override it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// Plastered / painted wall colour.
    pub wall: Rgba,
    /// Exposed timber framing and beams.
    pub timber: Rgba,
    pub roof: Rgba,
    pub roof_material: RoofMaterial,
    pub door: Rgba,
    pub door_kind: DoorKind,
    /// Window frames and shutters.
    pub window: Rgba,
    pub window_kind: WindowKind,
    pub shutters: bool,
    pub flower_boxes: bool,
    /// Part-catalogue id (e.g. `"window/gothic_01"`) of a custom window model
    /// replacing the procedural one. See `assets/parts/manifest.json`.
    pub custom_window: Option<String>,
    /// Part-catalogue id of a custom door model.
    pub custom_door: Option<String>,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            wall: [0.93, 0.89, 0.80, 1.0],
            timber: [0.36, 0.25, 0.17, 1.0],
            roof: [0.66, 0.33, 0.22, 1.0],
            roof_material: RoofMaterial::Tiles,
            door: [0.45, 0.28, 0.16, 1.0],
            door_kind: DoorKind::Plank,
            window: [0.30, 0.40, 0.33, 1.0],
            window_kind: WindowKind::Casement,
            shutters: false,
            flower_boxes: false,
            custom_window: None,
            custom_door: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Town
// ---------------------------------------------------------------------------

/// Inputs to the town generator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TownParams {
    pub seed: u64,
    /// Number of inner-city patches (wards). ~6 = hamlet, ~15 = village,
    /// ~30 = town, 60+ = city.
    pub patches: u32,
    /// Surround the city with a wall, towers and gates.
    pub walls: bool,
    /// Add a walled castle (citadel) ward.
    pub citadel: bool,
    /// Add a central market plaza.
    pub plaza: bool,
    /// Add a temple / cathedral ward.
    pub temple: bool,
    /// Run a river through the town (optional feature; generators may ignore it).
    pub river: bool,
    /// Average building lot frontage in metres (smaller = denser).
    pub lot_size: f32,
    /// Construction of bridges where streets cross the river.
    pub bridge_kind: BridgeKind,
}

impl Default for TownParams {
    fn default() -> Self {
        Self {
            seed: 1,
            patches: 15,
            walls: true,
            citadel: true,
            plaza: true,
            temple: true,
            river: false,
            lot_size: 8.0,
            bridge_kind: BridgeKind::Wood,
        }
    }
}

/// Character of a patch of town; drives lot sizes, building types and density.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WardKind {
    Castle,
    Market,
    Cathedral,
    Craftsmen,
    Merchant,
    Administration,
    Military,
    Patriciate,
    Slum,
    Farm,
    Park,
    /// Open land just outside a gate.
    Gate,
    /// Unbuilt land (outskirts, countryside).
    Empty,
}

/// One cell of the town's Voronoi partition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Patch {
    pub id: u32,
    pub shape: Polygon,
    pub ward: WardKind,
    /// Inside the city wall (or the inner city if there is no wall).
    pub within_walls: bool,
    /// Part of the settlement at all (as opposed to surrounding countryside).
    pub within_city: bool,
}

/// A closed fortification: city wall or citadel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Wall {
    pub shape: Polygon,
    /// Tower positions on the wall line.
    pub towers: Vec<Vec2>,
    /// Gate positions on the wall line.
    pub gates: Vec<Vec2>,
    pub height: f32,
    pub thickness: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StreetKind {
    /// Main road through a gate to the centre.
    Artery,
    Street,
    Alley,
    /// Road outside the walls.
    Road,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Street {
    pub kind: StreetKind,
    pub path: Polyline,
    pub width: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BuildingKind {
    House,
    Shop,
    Workshop,
    Tavern,
    Mansion,
    Temple,
    Keep,
    Tower,
    Barracks,
    Farmhouse,
    Barn,
    Mill,
}

/// A building lot produced by the town generator. Its footprint is what the
/// dwelling generator (or an asset placer) fills in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Building {
    pub id: u32,
    /// [`Patch::id`] this building belongs to.
    pub patch: u32,
    pub kind: BuildingKind,
    pub footprint: Polygon,
    /// Storeys above ground, ≥ 1.
    pub floors: u8,
    /// Unit vector pointing from the building towards the street it faces
    /// (where the main entrance goes).
    pub front: Vec2,
    /// Seed for anything generated per building (dwelling layout, asset choice).
    pub seed: u64,
    /// Colours and window / door / roof choices.
    #[serde(default)]
    pub appearance: Appearance,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tree {
    pub pos: Vec2,
    pub radius: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct River {
    pub path: Polyline,
    pub width: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BridgeKind {
    /// Plank deck on timber trestles with railings.
    #[default]
    Wood,
    /// Masonry arch bridge with parapets.
    Stone,
}

/// A bridge, generated over a river crossing or placed freely by the user. The
/// deck runs from end `a` to end `b`; both ends sit on the ground.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bridge {
    pub kind: BridgeKind,
    pub a: Vec2,
    pub b: Vec2,
    /// Deck width in metres.
    pub width: f32,
    /// Rise of the deck at mid-span above the ends, metres (0 = flat).
    #[serde(default)]
    pub height: f32,
}

/// Fenced yard on a large house lot: the house stands at the street end and
/// the rest of the lot is a garden or backyard behind a wooden fence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Yard {
    /// Id of the building on the lot.
    pub building: u32,
    /// The whole lot; the building's footprint is part of it.
    pub lot: Polygon,
    /// Wooden fence runs along the lot edges the building does not close.
    pub fences: Vec<Polyline>,
    /// Gate openings (`a`, `b`), each in a gap between fence runs.
    pub gates: Vec<(Vec2, Vec2)>,
}

/// Complete output of the town generator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Town {
    pub schema: u32,
    pub params: TownParams,
    pub center: Vec2,
    /// Radius of the settled area (useful for camera framing and ground size).
    pub radius: f32,
    pub patches: Vec<Patch>,
    pub wall: Option<Wall>,
    pub citadel: Option<Wall>,
    pub plaza: Option<Polygon>,
    pub streets: Vec<Street>,
    pub buildings: Vec<Building>,
    /// Farmland plots outside the walls.
    pub fields: Vec<Polygon>,
    pub trees: Vec<Tree>,
    pub river: Option<River>,
    #[serde(default)]
    pub bridges: Vec<Bridge>,
    /// Fenced yards of the houses on large lots.
    #[serde(default)]
    pub yards: Vec<Yard>,
}

// ---------------------------------------------------------------------------
// Dwelling (building interior + exterior shell)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DwellingStyle {
    Cottage,
    Townhouse,
    Farmhouse,
    Manor,
    Tavern,
    Shop,
}

/// Inputs to the dwelling generator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DwellingParams {
    pub seed: u64,
    /// Outer face of the exterior walls. If empty the generator picks its own
    /// footprint from `style` and `seed`.
    pub footprint: Polygon,
    /// Storeys above ground, ≥ 1.
    pub floors: u8,
    pub basement: bool,
    /// Unit vector towards the street / main entrance. Zero = generator decides.
    pub front: Vec2,
    pub style: DwellingStyle,
    /// Floor-to-floor height in metres.
    pub storey_height: f32,
    /// Planning grid size in metres (rooms snap to it).
    pub cell_size: f32,
    /// Look of the house. `None` = generator picks one from the seed and style.
    pub appearance: Option<Appearance>,
}

impl Default for DwellingParams {
    fn default() -> Self {
        Self {
            seed: 1,
            footprint: Polygon::default(),
            floors: 2,
            basement: false,
            front: Vec2::ZERO,
            style: DwellingStyle::Townhouse,
            storey_height: 3.0,
            cell_size: 1.0,
            appearance: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RoomKind {
    Hall,
    Corridor,
    Stairwell,
    Living,
    Kitchen,
    Dining,
    Bedroom,
    Study,
    Library,
    Storage,
    Pantry,
    Workshop,
    Shop,
    Taproom,
    Bath,
    Privy,
    Chapel,
    Nursery,
    Servants,
    Cellar,
    Attic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Room {
    pub id: u32,
    pub kind: RoomKind,
    /// Interior outline (wall centrelines), CCW.
    pub shape: Polygon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OpeningKind {
    /// Interior door.
    Door,
    /// Open archway without a door leaf.
    Doorway,
    /// Exterior entrance door.
    Entrance,
    Window,
}

/// A door or window: a segment along a wall centreline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Opening {
    pub kind: OpeningKind,
    pub a: Vec2,
    pub b: Vec2,
    /// Sill height above the floor (0 for doors).
    pub sill: f32,
    /// Opening height.
    pub height: f32,
    /// Room ids on either side; `None` = outside.
    pub rooms: [Option<u32>; 2],
}

/// Wall segment along its centreline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WallSegment {
    pub a: Vec2,
    pub b: Vec2,
    pub thickness: f32,
    pub exterior: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stair {
    pub footprint: Polygon,
    /// Unit vector in which the flight climbs.
    pub up: Vec2,
    pub from_level: i32,
    pub to_level: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PropKind {
    Bed,
    Table,
    Chair,
    Bench,
    Hearth,
    Stove,
    Counter,
    Shelf,
    Bookshelf,
    Wardrobe,
    Chest,
    Barrel,
    Crate,
    Desk,
    Bathtub,
    Altar,
    Anvil,
    Cot,
}

/// Furniture or fixture placed in a room.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prop {
    pub kind: PropKind,
    pub room: u32,
    pub pos: Vec2,
    /// Width (local x) and depth (local y).
    pub size: Vec2,
    /// Rotation around the vertical axis, radians CCW; local +y is the "front".
    pub rotation: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Floor {
    /// 0 = ground floor, -1 = basement, 1.. upper floors.
    pub level: i32,
    /// Height of this floor's surface above ground (metres).
    pub elevation: f32,
    /// Floor-to-ceiling height.
    pub height: f32,
    /// Outer face of the exterior walls on this floor.
    pub outline: Polygon,
    pub rooms: Vec<Room>,
    pub walls: Vec<WallSegment>,
    pub openings: Vec<Opening>,
    pub stairs: Vec<Stair>,
    pub props: Vec<Prop>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RoofKind {
    Gable,
    Hip,
    HalfHip,
    Gambrel,
    /// Pyramid / conical.
    Tent,
    Flat,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Roof {
    pub kind: RoofKind,
    /// Pitch from horizontal in degrees.
    pub pitch_deg: f32,
    /// Eave overhang beyond the top-floor outline, metres.
    pub overhang: f32,
    /// Unit vector along the ridge (ignored for Tent / Flat).
    pub ridge: Vec2,
}

/// Complete output of the dwelling generator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dwelling {
    pub schema: u32,
    pub params: DwellingParams,
    /// Ordered from lowest level to highest.
    pub floors: Vec<Floor>,
    pub roof: Roof,
    /// Resolved look (from `params.appearance` or picked by the generator).
    pub appearance: Appearance,
}

// ---------------------------------------------------------------------------
// Meshes (world space: Y up, metres)
// ---------------------------------------------------------------------------

/// Logical material of a mesh part. Consumers map these to their own materials
/// (Bevy `StandardMaterial`, other engines' materials, ...). Per-building
/// colours from [`Appearance`] travel as vertex colours, so one material per
/// slot is enough.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MaterialSlot {
    Plaster,
    Timber,
    Stone,
    RoofTile,
    Thatch,
    /// Slate roofing.
    Slate,
    /// Wooden shingles, bridge decks, planking.
    Planks,
    WoodFloor,
    Door,
    /// Window frames, mullions, shutters.
    WindowFrame,
    /// Iron fittings: door studs, hinges, lanterns.
    Iron,
    Glass,
    Ground,
    Grass,
    Street,
    Field,
    Water,
    Foliage,
    Bark,
}

/// Indexed triangle mesh, world space, CCW front faces.
/// `normals`, `uvs` and `colors` are either empty or the same length as `positions`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Linear RGBA vertex colours (tint multiplied with the slot material).
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

impl MeshData {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Append `other`, re-basing its indices. Attribute arrays that are empty on
    /// one side are padded with defaults so the result stays consistent.
    pub fn append(&mut self, other: &MeshData) {
        let base = self.positions.len() as u32;
        let n_self = self.positions.len();
        let n_other = other.positions.len();
        fn merge<T: Clone>(dst: &mut Vec<T>, src: &[T], n_dst: usize, n_src: usize, default: T) {
            if dst.is_empty() && src.is_empty() {
                return;
            }
            if dst.is_empty() {
                dst.resize(n_dst, default.clone());
            }
            if src.is_empty() {
                dst.extend(std::iter::repeat_n(default, n_src));
            } else {
                dst.extend_from_slice(src);
            }
        }
        merge(&mut self.normals, &other.normals, n_self, n_other, [0.0, 1.0, 0.0]);
        merge(&mut self.uvs, &other.uvs, n_self, n_other, [0.0, 0.0]);
        merge(&mut self.colors, &other.colors, n_self, n_other, [1.0, 1.0, 1.0, 1.0]);
        self.positions.extend_from_slice(&other.positions);
        self.indices.extend(other.indices.iter().map(|i| i + base));
    }
}

/// One material's worth of geometry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshPart {
    pub material: MaterialSlot,
    pub mesh: MeshData,
}

/// A named collection of mesh parts (one building, one town layer, ...).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub name: String,
    pub parts: Vec<MeshPart>,
}

impl Model {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), parts: Vec::new() }
    }

    /// Append geometry to the part for `material`, creating it if needed.
    pub fn add(&mut self, material: MaterialSlot, mesh: &MeshData) {
        match self.parts.iter_mut().find(|p| p.material == material) {
            Some(part) => part.mesh.append(mesh),
            None => self.parts.push(MeshPart { material, mesh: mesh.clone() }),
        }
    }

    pub fn triangle_count(&self) -> usize {
        self.parts.iter().map(|p| p.mesh.triangle_count()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_round_trip_json() {
        let p = TownParams::default();
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<TownParams>(&s).unwrap(), p);
        // Missing fields fall back to defaults.
        let partial: DwellingParams = serde_json::from_str(r#"{"seed": 9}"#).unwrap();
        assert_eq!(partial.seed, 9);
        assert_eq!(partial.floors, DwellingParams::default().floors);
    }

    #[test]
    fn mesh_append_rebases_and_pads() {
        let mut a = MeshData {
            positions: vec![[0.0; 3]; 3],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        let b = MeshData {
            positions: vec![[1.0; 3]; 3],
            colors: vec![[0.5; 4]; 3],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        a.append(&b);
        assert_eq!(a.indices, vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(a.colors.len(), 6);
        assert_eq!(a.colors[0], [1.0; 4]);
        assert!(a.normals.is_empty());
    }
}
