"""Export the starter part catalogue (windows, doors, landmarks) from IRCSS's
"French Houses" Geometry-Nodes project.

Source: https://github.com/IRCSS/Blender-Geometry-Node-French-Houses (MIT,
(c) 2025 Shahriar Shahrabi), file ``GeometryNodesFrenchHous.blend`` (not stored
in this repository; see ``tools/blender/README.md``).

Every window / door part is produced by one of the upstream window or door node
groups driven with a simple point, edge or quad input, then normalised to the
part convention of ``assets/parts/README.md``:

* glTF Y up, metres; the wall plane is glTF XY (z = 0), the part faces +Z;
* the centre of the opening (the glass of a window, the leaf of a door) is at
  the origin; ``width`` / ``height`` in the manifest are that opening's size;
* ``mount: "flush"`` parts fill a ``width x height`` hole cut into the wall
  (anything inside that rectangle that is not glass or door - arch spandrels,
  reveals - belongs to the part); ``mount: "surface"`` parts (oriels, porches)
  stand on the wall with their back at z = 0;
* canonical material names (``WindowFrame``, ``Door``, ``Glass``, ``Stone``,
  ``RoofTile`` ...), so a viewer recolours them like the houses.

Landmarks are placed like houses instead: origin at the centre of the footprint
on the ground (no preferred front).

    python -I tools/blender/export_parts.py -- \\
        --blend /path/to/GeometryNodesFrenchHous.blend --out assets/parts
"""

from __future__ import annotations

import argparse
import json
import math
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import bpy  # noqa: E402  (bpy must be imported before bmesh / mathutils)

# isort: split
import bmesh  # noqa: E402
import export_french_houses as fh  # noqa: E402
import mde_blender_common as common  # noqa: E402
import numpy as np  # noqa: E402
from mathutils import Matrix, Vector  # noqa: E402

SOURCE = "IRCSS French Houses (MIT)"

#: Per-category reinterpretation of upstream materials ("Beam" is used for
#: everything wooden upstream).
CATEGORY_OVERRIDES = {
    "window": {"Beam": "WindowFrame", "FakeWoodTextureWithUV": "WindowFrame", "Roof": "RoofTile"},
    "door": {"Beam": "Door", "FakeWoodTextureWithUV": "Door", "Roof": "RoofTile"},
    "landmark": {"FakeWoodTextureWithUV": "Door", "Sail": "Plaster", "Copper": "Iron"},
}

#: Slots whose faces define the opening of a window / door part.
OPENING_SLOTS = {"window": ("Glass",), "door": ("Door",)}

# Each part: id, category, how to build it (``kind``), generator inputs and
# the mount type. ``kind``:
#   points  - wrapper feeding one point into a group with a "Points" input
#   modifier- the group as a Geometry-Nodes modifier on an input mesh
#             (``input``: "quad" 2x2 m horizontal, "edge" along X of ``w`` m)
#   house_door - the entrance door cut out of a GenerateHouse house
PARTS: list[dict] = [
    {
        "id": "window/casement_01",
        "category": "window",
        "kind": "points",
        "group": "Generate Windows",
        "inputs": {"Normal": (-1.0, 0.0, 0.0), "Seed": 3, "NoFlowerProbability": 1.0, "High Polz Plants": False},
        "tweaks": [
            ("Generate Windows", "Resample Curve", "Length", 0.30),
            ("Generate Windows", "Resample Curve.001", "Length", 0.30),
        ],
        "mount": "flush",
        "label": "Six-light timber casement",
    },
    {
        "id": "window/casement_wide_01",
        "category": "window",
        "kind": "points",
        "group": "Generate Windows",
        "inputs": {
            "Normal": (-1.0, 0.0, 0.0),
            "Seed": 8,
            "WidthCalculationType": "HardCoded",
            "WidthHardcodedValue": 4.0,
            "NoFlowerProbability": 1.0,
            "High Polz Plants": False,
        },
        "tweaks": [
            ("Generate Windows", "Resample Curve", "Length", 0.30),
            ("Generate Windows", "Resample Curve.001", "Length", 0.30),
        ],
        "mount": "flush",
        "label": "Wide multi-light timber window (shop / hall front)",
    },
    {
        "id": "window/lattice_01",
        "category": "window",
        "kind": "points",
        "group": "Generate Windows",
        "inputs": {"Normal": (-1.0, 0.0, 0.0), "Seed": 2, "NoFlowerProbability": 1.0, "High Polz Plants": False},
        "tweaks": [
            ("Generate Windows", "Resample Curve", "Length", 0.16),
            ("Generate Windows", "Resample Curve.001", "Length", 0.16),
        ],
        "mount": "flush",
        "label": "Small-paned (lattice) timber window",
    },
    {
        "id": "window/casement_flowers_01",
        "category": "window",
        "kind": "points",
        "group": "Generate Windows",
        "inputs": {"Normal": (-1.0, 0.0, 0.0), "Seed": 5, "NoFlowerProbability": 0.0, "High Polz Plants": False},
        "tweaks": [
            ("Generate Windows", "Resample Curve", "Length", 0.30),
            ("Generate Windows", "Resample Curve.001", "Length", 0.30),
        ],
        "mount": "flush",
        "label": "Timber casement with flower box",
    },
    {
        "id": "window/oriel_01",
        "category": "window",
        "kind": "modifier",
        "input": "quad",
        "group": "GenerateAlcoveWindows",
        "inputs": {
            "Seed": -106,
            "AlcoveMaterial": "UpperFloorsPaint",
            "WifthType": "HardCodedWidth",
            "HardcodedWidth": 2.74,
            "Dpeth": 1.23,
            "BackLift": 0.42,
            "High Polz Plants": False,
        },
        "mount": "surface",
        "label": "Wide oriel (bay) window with lean-to roof and flower box",
    },
    {
        "id": "window/oriel_02",
        "category": "window",
        "kind": "modifier",
        "input": "quad",
        "group": "GenerateAlcoveWindows",
        "inputs": {
            "Seed": -128,
            "AlcoveMaterial": "Stone.001",
            "WifthType": "Automatic",
            "Dpeth": 1.0,
            "BackLift": 0.2,
            "High Polz Plants": False,
        },
        "mount": "surface",
        "label": "Narrow stone oriel window",
    },
    {
        "id": "window/lancet_01",
        "category": "window",
        "kind": "modifier",
        "input": "edge",
        "w": 1.2,
        "group": "GenerateWindowsFromEdges",
        "inputs": {"TotalWallHeight": 3.0},
        "mount": "flush",
        "crop": True,
        "label": "Gothic lancet window (pointed arch)",
    },
    {
        "id": "door/plank_01",
        "category": "door",
        "kind": "house_door",
        "seed": 18559,
        "mount": "flush",
        "label": "Ledged plank door (the house entrance door)",
    },
    {
        "id": "door/porch_01",
        "category": "door",
        "kind": "modifier",
        "input": "quad",
        "group": "GenerateDoorsWithAlcove",
        "inputs": {"Seed": -138, "AlcoveMaterial": "UpperFloorsPaint", "DoorWidth": 1.0, "DoorHeight": 1.35},
        "mount": "surface",
        "label": "Plank door in a stone porch with a tiled canopy",
    },
    {
        "id": "door/porch_02",
        "category": "door",
        "kind": "modifier",
        "input": "quad",
        "group": "GenerateDoorsWithAlcove",
        "inputs": {"Seed": -100, "AlcoveMaterial": "UpperFloorsPaint", "DoorWidth": 1.31, "DoorHeight": 1.4},
        "mount": "surface",
        "label": "Wide plank door in a stone porch with a tiled canopy",
    },
    {
        "id": "door/arched_01",
        "category": "door",
        "kind": "modifier",
        "input": "edge",
        "w": 2.0,
        "group": "CurvedDoorFromEdges",
        "inputs": {
            "Seed": 722,
            "Reverse": False,
            "Side Padding Normalized": 0.64,
            "TileHeight": 3.0,
            "FlattenTop": 0.0,
            "DoorDepth": -0.22,
            "DoorHeightNormalized": 0.8,
            "Create Door Mat Geometry": False,
        },
        "mount": "flush",
        "crop": True,
        "dress_door": True,
        "label": "Round-arched plank door in a stone reveal",
    },
    {
        "id": "door/arched_wide_01",
        "category": "door",
        "kind": "modifier",
        "input": "edge",
        "w": 2.6,
        "group": "CurvedDoorFromEdges",
        "inputs": {
            "Seed": 913,
            "Reverse": False,
            "Side Padding Normalized": 0.62,
            "TileHeight": 3.2,
            "FlattenTop": 0.5,
            "DoorDepth": -0.25,
            "DoorHeightNormalized": 0.78,
            "Create Door Mat Geometry": False,
        },
        "mount": "flush",
        "crop": True,
        "dress_door": True,
        "label": "Wide, flatter-arched plank door (barn, tavern, workshop)",
    },
    {
        "id": "landmark/tower_round_01",
        "category": "landmark",
        "kind": "modifier",
        "input": "quad",
        "group": "Generate Tower",
        "inputs": {
            "Seed": 2265,
            "High Poly Plants": False,
            "Upper Floor Extrude": 0.3,
            "Upper Floor Tilt": -5.82,
            "Upper Floor Window Offset": 0.1,
            "Upper Floor Height": 1.16,
            "Number Of Floors": 1,
            "RoofHeight": 3.97,
            "BaseRadius": 1.4,
            "FirstFloorHeight": 6.33,
            "SpawnGroundFloor": True,
            "Spawn Stone On wall": True,
            "SpawnIvy": False,
            "Spawn Light Shaft": True,
            "Low Point Offset": 1.55,
        },
        "roof_slot": "Slate",
        "colors": {
            "Plaster": (0.89, 0.84, 0.72, 1.0),
            "Timber": (0.22, 0.16, 0.11, 1.0),
            "Slate": (0.29, 0.31, 0.35, 1.0),
        },
        "label": "Round watchtower with a timber-framed upper storey",
    },
    {
        "id": "landmark/tower_square_01",
        "category": "landmark",
        "kind": "modifier",
        "input": "quad",
        "group": "Generate Square Tower",
        "inputs": {
            "Number Of Floors": 2,
            "Seed": 2521,
            "BaseDepth": 4.48,
            "BaseWidth": 4.11,
            "RoofHeight": 3.78,
            "FirstFloorHeight": 3.6,
            "SpawnIvy": False,
            "SpawnGroundFloor": True,
            "Spawn Stones On Walls": True,
            "High Polz Plants": False,
        },
        "roof_slot": "RoofTile",
        "colors": {
            "Plaster": (0.92, 0.89, 0.81, 1.0),
            "Timber": (0.22, 0.16, 0.11, 1.0),
            "RoofTile": (0.55, 0.31, 0.22, 1.0),
        },
        "label": "Square tower house",
    },
    {
        "id": "landmark/windmill_01",
        "category": "landmark",
        "kind": "modifier",
        "input": "quad",
        "group": "WindMill",
        "inputs": {"Seed": 288, "Roof geo type": "Real Geometry ", "High Poly Plants": False},
        "copy_inputs_from": "WindMillTower",
        "roof_slot": "Thatch",
        "colors": {
            "Thatch": (0.36, 0.30, 0.22, 1.0),
            "Plaster": (0.82, 0.78, 0.68, 1.0),
            "Timber": (0.30, 0.21, 0.14, 1.0),
        },
        "label": "Stone tower windmill with sails",
    },
]


# ---------------------------------------------------------------------------
# Building the generator objects
# ---------------------------------------------------------------------------


def _input_mesh(part: dict):
    me = bpy.data.meshes.new(f"in_{part['id']}")
    kind = part.get("input", "none")
    if kind == "quad":
        me.from_pydata([(-1, -1, 0), (1, -1, 0), (1, 1, 0), (-1, 1, 0)], [], [(0, 1, 2, 3)])
    elif kind == "edge":
        w = part.get("w", 1.0) / 2.0
        me.from_pydata([(-w, 0, 0), (w, 0, 0)], [(0, 1)], [])
    return me


def _resolve(value):
    """Turn material names in an input table into datablocks."""
    if isinstance(value, str):
        mat = bpy.data.materials.get(common.SOURCE_MATERIAL_PREFIX + value)
        if mat is not None:
            return mat
    return value


def _points_wrapper(part: dict):
    """Node group: a single point at the origin fed into ``part['group']``."""
    g = bpy.data.node_groups.new(f"MDE_part_{part['id']}", "GeometryNodeTree")
    g.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
    g.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
    go = g.nodes.new("NodeGroupOutput")
    pts = g.nodes.new("GeometryNodePoints")
    pts.inputs["Count"].default_value = 1
    node = g.nodes.new("GeometryNodeGroup")
    node.node_tree = bpy.data.node_groups[part["group"]]
    g.links.new(pts.outputs["Points"], node.inputs["Points"])
    for name, value in part["inputs"].items():
        sock = node.inputs.get(name)
        if sock is None:
            print(f"WARNING: {part['group']} has no input {name!r}")
            continue
        sock.default_value = _resolve(value)
    join = g.nodes.new("GeometryNodeJoinGeometry")
    for out in node.outputs:
        if out.type == "GEOMETRY":
            g.links.new(out, join.inputs[0])
    g.links.new(join.outputs[0], go.inputs[0])
    return g


def build_part_generator(part: dict, realize_group):
    """Object whose evaluated mesh is the raw (source-material) part."""
    obj = bpy.data.objects.new(f"gen_{part['id']}", _input_mesh(part))
    bpy.context.scene.collection.objects.link(obj)
    mod = obj.modifiers.new("part", "NODES")
    if part["kind"] == "points":
        mod.node_group = _points_wrapper(part)
    else:
        mod.node_group = bpy.data.node_groups[part["group"]]
        template = bpy.data.objects.get(part.get("copy_inputs_from", ""))
        if template is not None:  # start from the upstream example's settings
            src = template.modifiers[0]
            for item in mod.node_group.interface.items_tree:
                if item.item_type == "SOCKET" and item.in_out == "INPUT" and item.socket_type != "NodeSocketGeometry":
                    try:
                        value = getattr(src.properties.inputs, item.identifier).value
                        getattr(mod.properties.inputs, item.identifier).value = value
                    except (AttributeError, TypeError):
                        pass
        for name, value in part["inputs"].items():
            fh.set_modifier_input(mod, name, _resolve(value))
        if any(it.name == "Object" for it in mod.node_group.interface.items_tree if it.item_type == "SOCKET"):
            fh.set_modifier_input(mod, "Object", obj)
    realize = obj.modifiers.new("realize", "NODES")
    realize.node_group = realize_group
    return obj


def build_house_door(part: dict, realize_group):
    """Cut the entrance door out of a small GenerateHouse house.

    Returns the baked object (canonical materials, Door faces only) and the
    distance of the house's front wall from the origin along +X.
    """
    spec = {
        "index": 0,
        "file": "door_source.glb",
        "archetype": "cottage",
        "gn_seed": part["seed"],
        "upper_floors": 1,
        "facade": 6.0,
        "depth": 4.6,
        "wing": None,
        "roof_curvature": 1.3,
        "roof_slot": "RoofTile",
        "ivy": False,
        "flower_boxes": False,
        "no_roof_extrude_front": False,
        "no_roof_extrude_back": False,
        "colors": {s: common.CANONICAL_MATERIALS[s][0] for s in ("RoofTile", "Plaster", "Timber")},
    }
    gen = fh.build_generator(spec, realize_group)
    obj, _ = fh.bake_house(gen, spec)
    keep_only_slots(obj, {"Door"})
    return obj, spec["depth"] / 2.0


def keep_only_slots(obj, slots: set[str]) -> None:
    """Delete every face whose material is not in ``slots``."""
    mesh = obj.data
    keep = {i for i, m in enumerate(mesh.materials) if m is not None and m.name in slots}
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.delete(bm, geom=[f for f in bm.faces if f.material_index not in keep], context="FACES")
    bm.to_mesh(mesh)
    bm.free()
    mesh.update()


# ---------------------------------------------------------------------------
# Normalisation
# ---------------------------------------------------------------------------


def _slot_faces(mesh, slots) -> list:
    names = [m.name if m else "" for m in mesh.materials]
    wanted = {i for i, n in enumerate(names) if n in slots}
    return [p for p in mesh.polygons if p.material_index in wanted]


def facing_of(mesh, slots) -> Vector:
    """Horizontal unit vector the opening faces (area-weighted face normal)."""
    acc = Vector((0.0, 0.0, 0.0))
    for p in _slot_faces(mesh, slots):
        acc += p.normal * p.area
    acc.z = 0.0
    if acc.length < 1e-6:
        raise RuntimeError("cannot tell which way the part faces (no opening faces)")
    # Snap to the nearest axis: every generator builds axis-aligned openings.
    if abs(acc.x) >= abs(acc.y):
        return Vector((math.copysign(1.0, acc.x), 0.0, 0.0))
    return Vector((0.0, math.copysign(1.0, acc.y), 0.0))


def opening_bounds(mesh, slots) -> tuple[Vector, Vector]:
    verts = {v for p in _slot_faces(mesh, slots) for v in p.vertices}
    if not verts:
        raise RuntimeError("part has no opening faces")
    co = [mesh.vertices[i].co for i in verts]
    lo = Vector([min(c[k] for c in co) for k in range(3)])
    hi = Vector([max(c[k] for c in co) for k in range(3)])
    return lo, hi


def crop_to_box(obj, lo: Vector, hi: Vector) -> None:
    """Cut the mesh down to the axis-aligned box ``lo..hi`` (X and Z only)."""
    mesh = obj.data
    bm = bmesh.new()
    bm.from_mesh(mesh)
    planes = [
        ((lo.x, 0, 0), (-1, 0, 0)),
        ((hi.x, 0, 0), (1, 0, 0)),
        ((0, 0, lo.z), (0, 0, -1)),
        ((0, 0, hi.z), (0, 0, 1)),
    ]
    for co, no in planes:
        geom = list(bm.verts) + list(bm.edges) + list(bm.faces)
        bmesh.ops.bisect_plane(bm, geom=geom, dist=1e-5, plane_co=co, plane_no=no, clear_outer=True)
    bm.to_mesh(mesh)
    bm.free()
    mesh.update()


def normalise_opening_part(obj, part: dict, wall_offset: float = 0.0) -> dict:
    """Rotate / move a window or door so it faces glTF +Z with its opening at the origin.

    Blender -Y is glTF +Z, so the opening is turned to face -Y. ``wall_offset``
    is the distance of the wall surface from the generator origin along the
    original facing direction (flush parts only).
    """
    mesh = obj.data
    slots = OPENING_SLOTS[part["category"]]
    f = facing_of(mesh, slots)
    angle = math.atan2(f.y, f.x)
    rot = Matrix.Rotation(-math.pi / 2.0 - angle, 4, "Z")  # facing -> (0, -1, 0)
    mesh.transform(rot)
    lo, hi = opening_bounds(mesh, slots)
    cx, cz = (lo.x + hi.x) / 2.0, (lo.z + hi.z) / 2.0
    if part["mount"] == "surface":
        # Back of the part (largest Blender y, i.e. most negative glTF z) sits
        # 2 cm inside the wall so uneven walls show no gap.
        ys = [v.co.y for v in mesh.vertices]
        wall_y = max(ys) - 0.02
    else:
        wall_y = -wall_offset
    mesh.transform(Matrix.Translation((-cx, -wall_y, -cz)))
    mesh.update()
    if part.get("crop"):
        half_w, half_h = (hi.x - lo.x) / 2.0, (hi.z - lo.z) / 2.0
        crop_to_box(obj, Vector((-half_w, 0, -half_h)), Vector((half_w, 0, half_h)))
    return {"width": round(hi.x - lo.x, 3), "height": round(hi.z - lo.z, 3)}


def normalise_landmark(obj) -> None:
    """Centre the footprint (bounds of everything below 0.5 m) on the origin."""
    mesh = obj.data
    co = [v.co for v in mesh.vertices]
    base = [c for c in co if c.z < 0.5] or co
    cx = (min(c.x for c in base) + max(c.x for c in base)) / 2.0
    cy = (min(c.y for c in base) + max(c.y for c in base)) / 2.0
    mesh.transform(Matrix.Translation((-cx, -cy, 0.0)))
    mesh.update()


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def export_part(part: dict, out_dir: str, realize_group) -> dict:
    t0 = time.time()
    cat = part["category"]
    file_rel = part["id"] + ".glb"
    name = part["id"].replace("/", "_")
    wall_offset = 0.0
    restore = apply_tweaks(part.get("tweaks", []))
    try:
        if part["kind"] == "house_door":
            obj, wall_offset = build_house_door(part, realize_group)
        else:
            gen = build_part_generator(part, realize_group)
            spec = {
                "file": name + ".glb",
                "roof_slot": part.get("roof_slot", "RoofTile"),
                "gn_seed": hash_id(part["id"]),
            }
            obj, _ = fh.bake_house(gen, spec, overrides=CATEGORY_OVERRIDES[cat])
    finally:
        apply_tweaks(restore)
    obj.name = name
    size = {}
    if cat == "landmark":
        normalise_landmark(obj)
    else:
        size = normalise_opening_part(obj, part, wall_offset)
    if part.get("dress_door"):
        dress_flat_door(obj, hash_id(part["id"]))
    fh.remove_hidden_faces(obj)
    fh.merge_coplanar_faces(obj)
    slots = [m.name for m in obj.data.materials]
    for slot in slots:
        colour = part.get("colors", {}).get(slot, common.CANONICAL_MATERIALS[slot][0])
        common.set_material_color(common.canonical_material(slot), colour)
    path = os.path.join(out_dir, file_rel)
    common.export_glb(obj, path, copyright_note=f"Derived from {fh.UPSTREAM}")
    common.compact_vertex_colors(path)
    summary = common.glb_summary(path)
    lo, hi = summary["bounds_min"], summary["bounds_max"]
    entry = {
        "id": part["id"],
        "category": cat,
        "file": file_rel,
        "width": size.get("width", round(hi[0] - lo[0], 3)),
        "height": size.get("height", round(hi[1] - lo[1], 3)),
        "depth": round(hi[2] - lo[2], 3),
        "triangles": summary["triangles"],
        "source": SOURCE,
        "label": part["label"],
        "materials": summary["materials"],
        "bounds_min": lo,
        "bounds_max": hi,
    }
    if cat != "landmark":
        entry["mount"] = part["mount"]
    mesh = obj.data
    bpy.data.objects.remove(obj)
    bpy.data.meshes.remove(mesh)
    print(
        f"{part['id']:<28} {entry['width']:5.2f} x {entry['height']:5.2f} x {entry['depth']:5.2f} m "
        f"tris={entry['triangles']:6d} {summary['bytes'] / 1e3:7.0f} KB  t={time.time() - t0:.1f}s",
        flush=True,
    )
    return entry


def apply_tweaks(tweaks: list) -> list:
    """Set ``(group, node, input, value)`` node inputs; returns the tweaks that undo them."""
    undo = []
    for group_name, node_name, socket, value in tweaks:
        group = bpy.data.node_groups.get(group_name)
        node = group.nodes.get(node_name) if group is not None else None
        sock = node.inputs.get(socket) if node is not None else None
        if sock is None:
            print(f"WARNING: cannot tweak {group_name}/{node_name}/{socket}")
            continue
        undo.append((group_name, node_name, socket, sock.default_value))
        sock.default_value = value
    return undo


def dress_flat_door(obj, salt: int, plank: float = 0.15) -> None:
    """Give a flat (texture-only upstream) door leaf planks, ledges and iron straps.

    Expects the normalised part: leaf in the XZ plane facing -Y (glTF +Z) with
    the opening centred on the origin. The leaf is split into vertical planks
    of slightly different shade (via the ``Col`` attribute), and two ledges with
    iron strap hinges are added below the springing of the arch.
    """
    mesh = obj.data
    names = [m.name for m in mesh.materials]
    if "Door" not in names:
        return
    if "Iron" not in names:
        mesh.materials.append(common.canonical_material("Iron"))
        names.append("Iron")
    door_i, iron_i = names.index("Door"), names.index("Iron")
    bm = bmesh.new()
    bm.from_mesh(mesh)
    col = bm.loops.layers.float_color.get("Col")
    leaf = [f for f in bm.faces if f.material_index == door_i and f.normal.y < -0.9]
    if not leaf or col is None:
        bm.free()
        return
    xs = [v.co.x for f in leaf for v in f.verts]
    zs = [v.co.z for f in leaf for v in f.verts]
    x0, x1, z0, z1 = min(xs), max(xs), min(zs), max(zs)
    y_leaf = min(v.co.y for f in leaf for v in f.verts)
    # Planks: cut the leaf into vertical strips.
    n = max(2, round((x1 - x0) / plank))
    for k in range(1, n):
        x = x0 + (x1 - x0) * k / n
        faces = [f for f in bm.faces if f.material_index == door_i and f.normal.y < -0.9]
        geom = faces + list({e for f in faces for e in f.edges}) + list({v for f in faces for v in f.verts})
        bmesh.ops.bisect_plane(bm, geom=geom, dist=1e-5, plane_co=(x, 0.0, 0.0), plane_no=(1.0, 0.0, 0.0))
    for f in bm.faces:
        if f.material_index == door_i and f.normal.y < -0.9:
            for e in f.edges:
                e.smooth = False  # sharp: keeps the plank joints through merge_coplanar_faces
            k = int((f.calc_center_median().x - x0) / (x1 - x0) * n)
            shade = 0.78 + 0.22 * fh._hash01(np.array([k]), salt)[0]
            for loop in f.loops:
                loop[col] = (shade, shade, shade, 1.0)
    # Ledges and strap hinges, kept below the springing of a round arch.
    width = x1 - x0
    spring = z1 - width / 2.0
    for frac in (0.18, 0.62):
        zc = z0 + (spring - z0) * frac / 0.62 if spring > z0 else z0 + (z1 - z0) * frac
        _box(bm, (x0 + 0.05, x1 - 0.05), (y_leaf - 0.035, y_leaf), (zc - 0.06, zc + 0.06), door_i, col, 0.7)
        _box(
            bm,
            (x0 + 0.02, x0 + width * 0.6),
            (y_leaf - 0.045, y_leaf - 0.035),
            (zc - 0.03, zc + 0.03),
            iron_i,
            col,
            1.0,
        )
    bm.to_mesh(mesh)
    bm.free()
    mesh.update()


def _box(bm, xr, yr, zr, material: int, col_layer, shade: float) -> None:
    """Add an axis-aligned box to ``bm`` with one material and a grey colour."""
    verts = [bm.verts.new((x, y, z)) for x in xr for y in yr for z in zr]
    # verts index = ix*4 + iy*2 + iz
    quads = [(0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6), (0, 2, 6, 4), (1, 5, 7, 3)]
    for q in quads:
        face = bm.faces.new([verts[i] for i in q])
        face.material_index = material
        face.smooth = False
        for loop in face.loops:
            loop[col_layer] = (shade, shade, shade, 1.0)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[-6:])


def hash_id(text: str) -> int:
    """Stable small integer from a string (Python's ``hash`` is salted per run)."""
    h = 2166136261
    for ch in text.encode("utf-8"):
        h = ((h ^ ch) * 16777619) & 0xFFFFFFFF
    return h % 100_000


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Export the window / door / landmark part catalogue.")
    p.add_argument("--blend", required=True, help="path to GeometryNodesFrenchHous.blend")
    p.add_argument("--out", default="assets/parts", help="output directory (default assets/parts)")
    p.add_argument("--only", default="", help="comma-separated part ids to (re)export")
    return p.parse_args(common.script_args())


def main() -> None:
    args = parse_args()
    out_dir = os.path.abspath(args.out)
    common.open_source_blend(os.path.abspath(args.blend))
    fh.tune_node_groups(1.0)
    realize_group = fh.make_realize_group()
    only = {s.strip() for s in args.only.split(",") if s.strip()}
    manifest_path = os.path.join(out_dir, "manifest.json")
    previous: dict[str, dict] = {}
    if os.path.isfile(manifest_path):
        with open(manifest_path) as f:
            previous = {p["id"]: p for p in json.load(f).get("parts", [])}
    entries = []
    for part in PARTS:
        if only and part["id"] not in only:
            if part["id"] in previous:
                entries.append(previous[part["id"]])
            continue
        entries.append(export_part(part, out_dir, realize_group))
    # Keep hand-added catalogue entries (user parts) that this script does not own.
    ours = {p["id"] for p in PARTS}
    entries += [p for pid, p in previous.items() if pid not in ours]
    entries.sort(key=lambda p: (p["category"], p["id"]))
    with open(manifest_path, "w") as f:
        json.dump({"schema": 1, "parts": entries}, f, indent=1)
        f.write("\n")
    print(f"wrote {len(entries)} parts to {manifest_path}")


if __name__ == "__main__":
    main()
