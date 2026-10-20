# Blender asset tools

Scripts that turn IRCSS's MIT-licensed Geometry-Nodes project
["French Houses"](https://github.com/IRCSS/Blender-Geometry-Node-French-Houses) into
the game's glTF assets: the 50 village houses in `assets/houses/` and the window /
door / landmark part catalogue in `assets/parts/`.

| Script | Needs | Does |
|---|---|---|
| `export_french_houses.py` | bpy | builds and exports N house variants + `assets/houses/manifest.json` |
| `export_parts.py` | bpy | exports the windows, doors and landmarks + `assets/parts/manifest.json` |
| `render_contact_sheet.py` | bpy | renders a grid of GLBs (re-imported into an empty scene) |
| `preview_parts_wall.py` | bpy | renders the window / door parts set into a test wall, per the manifest |
| `validate_assets.py` | Python 3 only | checks both manifests against the GLBs (CI friendly) |
| `mde_blender_common.py` | — | shared helpers: canonical materials, GLB export, pure-Python GLB reader / writer |

## Setup

1. **The source .blend** (≈ 9 MB, not stored in this repository):

   ```sh
   git clone https://github.com/IRCSS/Blender-Geometry-Node-French-Houses /tmp/fh
   git -C /tmp/fh checkout b49b3c1dca15a57f3696e23c6f3ba8badf1fa6a3   # version the assets were made from
   # file: /tmp/fh/GeometryNodesFrenchHous.blend (Blender 5.0 Geometry Nodes)
   ```

2. **Blender 5.x as a Python module** (headless, no GPU needed; Cycles CPU renders):

   ```sh
   python3.13 -m venv .venv-bpy && .venv-bpy/bin/pip install bpy==5.2.*
   ```

   The scripts also run inside a Blender install:
   `blender -b -P tools/blender/export_parts.py -- --blend …`.

Run every script with the bpy Python and pass arguments after `--`; use `-I`
(isolated mode) so nothing from the working directory is imported implicitly
(the scripts add their own folder to `sys.path`).

## Houses

```sh
.venv-bpy/bin/python -I tools/blender/export_french_houses.py -- \
    --blend /tmp/fh/GeometryNodesFrenchHous.blend --out assets/houses --count 50 --seed 1
```

Options: `--count`, `--seed` (variant planning; output is deterministic for a seed),
`--only 3,17` (re-export some variants, merging into the existing manifest),
`--detail 0.8` (scale the scatter densities). About 1.5 s per house.

Per variant the script

1. **plans** it from the seed: archetype (cottage, townhouse, narrow tall merchant
   house, long hall, manor) → foundation size, 1–3 upper storeys (occasionally 4),
   roof curvature, slate or clay roof, ivy, flower boxes, an optional centred front
   wing, and a grounded palette for plaster, timber, roof, door, window frames and
   stone;
2. builds an **X-symmetric foundation quad** (front / door side = local +X, as the
   upstream README requires) with a `GenerateHouse` modifier, then a realise-instances
   modifier that also stores each mesh island's index;
3. evaluates it; if upstream put the door on a gable wall, it retries with the next
   seed so every house has its entrance on +X;
4. maps upstream's procedural materials onto **canonical `MaterialSlot` names**
   (Beam → Timber, UpperFloorsPaint → Plaster, GroundFloorWalls / Stone → Stone,
   Roof → Slate / RoofTile, WindowGlass → Glass, plants / ivy → Foliage; window
   frames, the door and chimney pots are re-tagged inside the node groups first so
   they come out as WindowFrame / Door / Stone);
5. bakes `COLOR_0`: a soft per-island light/dark multiplier (each tile, beam, stone)
   and real leaf / blossom colours on white `Foliage`;
6. **removes hidden faces** (ray tests on a 5 cm sample grid: faces pressed against or
   buried inside other geometry, e.g. beam backs inside walls), then **merges
   coplanar faces** (limited dissolve; curve-to-mesh beams are chains of collinear
   segments) — together 10–30 % fewer triangles with an identical look;
7. exports a GLB (Y up, no UVs, flat normals), rewrites `COLOR_0` as 8-bit, measures
   footprint, bounds and the door centre, and writes the manifest entry.

In-memory tweaks of the upstream node groups (the .blend is never saved) keep the
houses within ~25 k triangles and the folder under 45 MB; they are in
`TUNING` at the top of `tune_node_groups()`:

| Tweak | Upstream | Here |
|---|---|---|
| wall stones per m² | 10 | 1.2 (meshes decimated to 30 %, smooth shaded) |
| ivy leaves per m², leaf shape | 126, 5-sided cylinder | 60, flat pentagon |
| window-box plants per m² | 103 (high-poly plants) | 7 (low-poly plants) |
| roof tile spacing / size | 0.20 m / 0.5 | 0.30 m / 0.65, no hidden bottom face, soft sides |
| glazing-bar spacing | 0.30 m | 0.36 m |
| flower boxes | 78 % of windows | 70 % on half the houses, none on the rest |

Then render and check:

```sh
.venv-bpy/bin/python -I tools/blender/render_contact_sheet.py -- \
    --glb-dir assets/houses --out docs/images/french_houses_contact.jpg --cols 10 --ground
python3 tools/blender/validate_assets.py
```

The contact sheet re-imports each GLB into an empty scene and renders it the way
a glTF engine would (base colour × `COLOR_0`, back faces culled), so missing faces
or wrong colours show up.

## Parts

```sh
.venv-bpy/bin/python -I tools/blender/export_parts.py -- \
    --blend /tmp/fh/GeometryNodesFrenchHous.blend --out assets/parts
.venv-bpy/bin/python -I tools/blender/preview_parts_wall.py -- \
    --out docs/images/french_houses_parts_wall.jpg
```

`PARTS` in `export_parts.py` lists each part: the upstream node group, how it is
driven (one point, an edge or a quad), its inputs, and how it mounts in a wall. The
script rotates every window / door to face glTF +Z, centres its opening (glass /
door leaf) on the origin with the wall surface at z = 0, crops edge-built wall slabs
to the opening rectangle, and writes `assets/parts/manifest.json`, keeping entries
that users added by hand. See `assets/parts/README.md` for the part conventions and
how to add parts.

## Licence

Upstream is MIT (© 2025 Shahriar Shahrabi); the exported GLBs are derived works and
ship with that licence (`assets/houses/LICENSE`, `assets/parts/LICENSE`). Credit:
IRCSS, *Blender Geometry Node French Houses*.
