# Village houses (French Houses GN)

50 ready-made medieval town houses as glTF binaries (`house_00.glb` …
`house_49.glb`), described by `manifest.json`. They are exterior shells: whitewashed
or ochre plaster between dark timber framing, a stone ground floor, steep slate or
clay-tile roofs with dormers and chimneys, flower boxes and ivy on some of them.

![All 50 houses](../../docs/images/french_houses_contact.jpg)

## Provenance

Generated from the Geometry-Nodes project by **IRCSS / Shahriar Shahrabi**,
<https://github.com/IRCSS/Blender-Geometry-Node-French-Houses>, MIT licence
(see `LICENSE` here). Source file `GeometryNodesFrenchHous.blend`, upstream commit
`b49b3c1dca15a57f3696e23c6f3ba8badf1fa6a3` (sha256 of the .blend
`f44b4be9…3d5ae5`), evaluated with Blender 5.2.2 (`bpy`). The GLBs are derived works
and carry the upstream copyright notice; the .blend itself is not stored here.

Regenerate (deterministic for a given seed and source file, about 75 s):

```sh
python -I tools/blender/export_french_houses.py -- \
    --blend /path/to/GeometryNodesFrenchHous.blend --out assets/houses --count 50 --seed 1
python -I tools/blender/render_contact_sheet.py -- \
    --glb-dir assets/houses --out docs/images/french_houses_contact.jpg --cols 10 --ground
python3 tools/blender/validate_assets.py
```

See `tools/blender/README.md` for how the variants are made and what was tuned.

## Conventions

| | |
|---|---|
| Format | glTF 2.0 binary, one mesh node per file, no textures, no extensions |
| Axes / units | glTF: **Y up**, metres, right-handed (same as Bevy world space) |
| Origin | centre of the ground-floor footprint, **on the ground** (y = 0). The stone base sinks up to 0.25 m below y = 0 so houses sit on uneven ground without gaps. |
| Front | **+X**: the entrance door is always on the +X wall (`door_facing` in the manifest). |
| Size | real metres; do not scale non-uniformly (timber and tiles would distort). |

To make a house face the street of a town building whose `front` is the plan-space
vector `(fx, fy)`: world = `(plan.x, h, -plan.y)`, so rotate the model about Y by
`atan2(fy, fx)` (Bevy: `Quat::from_rotation_y(front.y.atan2(front.x))`) and put its
origin on the footprint centre. Pick a variant whose `footprint` fits the lot; the
footprint is `[x, z]` = [front-to-back depth, length of the front wall].

## Materials (recolourable)

Every primitive uses one material named after an `mde_core::MaterialSlot`:

| Material | What |
|---|---|
| `Plaster` | painted infill of the timber frame (`Appearance::wall`) |
| `Timber` | framing, beams, braces, gable boards (`Appearance::timber`) |
| `Stone` | ground floor, wall stones, chimneys |
| `Slate` or `RoofTile` | roof and dormer tiles (`Appearance::roof`; slate-grey roofs are `Slate`, clay roofs `RoofTile`) |
| `Door` | entrance door leaf and its ledges (`Appearance::door`) |
| `WindowFrame` | window frames, glazing bars, flower boxes (`Appearance::window`) |
| `Glass` | panes (dark, low roughness) |
| `Foliage` | ivy, window-box plants (double-sided) |

* `baseColorFactor` is the house's default colour (linear, as glTF requires). The
  same colours are in the manifest as **sRGB** (`colors`, like `mde_core::Rgba` /
  `Appearance`). To recolour, replace a slot's base colour (convert sRGB → linear).
* `COLOR_0` (normalised 8-bit RGBA, linear) multiplies the base colour as the glTF
  spec says. For recolourable slots it only holds a soft per-piece light/dark
  variation (every tile, beam and wall stone slightly different), so any base
  colour you set keeps that texture. `Foliage` has a **white** base colour and its
  real leaf / blossom colours in `COLOR_0`.
* Only `POSITION`, `NORMAL` (flat shading) and `COLOR_0` are stored; no UVs.

## manifest.json

An array, one object per house:

| Field | Meaning |
|---|---|
| `file` | GLB file name in this folder |
| `seed` | GenerateHouse seed used |
| `floors` | storeys above ground including the ground floor (as `Building::floors`) |
| `footprint` | `[x, z]` size of the ground-floor footprint in metres (without roof overhang or upper-floor jetties) |
| `bounds_min`, `bounds_max` | model-space AABB `[x, y, z]` of everything (roof overhangs, chimneys, ivy) |
| `height` | `bounds_max[1]`: ground to the highest point |
| `triangles` | triangle count |
| `front` | always `"+X"` |
| `door` | `[x, y, z]` centre of the entrance door (leaf and frame), model space, measured from the mesh |
| `door_facing` | axis the door leaf faces (always `"+X"`) |
| `door_size` | `[width, height]` of the door leaf and frame, metres |
| `materials` | material names present in the file |
| `archetype` | `cottage`, `townhouse`, `merchant` (narrow, tall), `hall` (long), `manor` (large) |
| `ivy`, `flower_boxes` | whether the house has ivy / window boxes |
| `wing` | `null`, or `[width, depth]` of a centred front wing (porch bay) on the +X side |
| `colors` | default sRGB colour per recolourable material |
| `bytes` | file size |

An NPC enters through `door`: walk to `door + (0.6, 0, 0)` (in front of it, rotated
with the house), then through the door's x position into the building.

## Numbers

* 50 houses, **39.2 MB** in total (0.43–1.24 MB each).
* Triangles: min 10 827, mean 19 652, max 28 563. 43 houses are within the
  25 k budget, 7 are between 25 k and 28.6 k; none needed decimation (hard cap 60 k).
* 21 houses with 2 storeys, 15 with 3, 11 with 4, 3 with 5; 30 slate and 20 clay
  roofs; 8 with ivy, 30 with flower boxes, 5 with a front wing.

## Limitations

* Exterior shells: there are no interiors or floors inside, and the door is a closed
  leaf. Walkable interiors come from the dwelling generator (`mde_dwelling`); use
  these models as the outside look and the manifest `door` as the entry point.
* The windows and the door are baked into each house; the part catalogue
  (`assets/parts`) is for procedurally built buildings.
* Footprints are rectangles (plus an optional front wing). Trapezoid foundations were
  tried and dropped: upstream's roof builder creases the ridge on non-rectangular
  foundations.
* Some wing houses show a row of upturned tiles along the ridge (an upstream roof
  artefact).
