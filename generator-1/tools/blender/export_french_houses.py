"""Export village house variants from IRCSS's "French Houses" Geometry-Nodes project.

Source: https://github.com/IRCSS/Blender-Geometry-Node-French-Houses (MIT,
(c) 2025 Shahriar Shahrabi), file ``GeometryNodesFrenchHous.blend``. The .blend is
NOT stored in this repository; see ``tools/blender/README.md`` for how to fetch it.

For every variant the script builds an X-symmetric foundation quad (local +X is
the front / door side, as the upstream README requires), drives the
``GenerateHouse`` node group with a seed, storey count, roof curvature and ivy
switch, realises all instances, maps the procedural source materials onto the
canonical ``mde_core::MaterialSlot`` names (``Plaster``, ``Timber``, ``Stone``,
``RoofTile`` / ``Slate``, ``Door``, ``WindowFrame``, ``Glass``, ``Foliage``),
bakes a soft per-piece colour variation into ``COLOR_0`` and exports a GLB:
glTF Y up, metres, origin at the centre of the ground-floor footprint on the
ground, front facing +X. ``manifest.json`` describes every file.

Run with a Python that has the ``bpy`` module (Blender 5.x):

    python -I tools/blender/export_french_houses.py -- \\
        --blend /path/to/GeometryNodesFrenchHous.blend \\
        --out assets/houses --count 50 --seed 1

The output is deterministic for a given ``--seed`` and source file.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import random
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import bpy  # noqa: E402
import mde_blender_common as common  # noqa: E402
import numpy as np  # noqa: E402

UPSTREAM = "IRCSS/Blender-Geometry-Node-French-Houses (MIT, (c) 2025 Shahriar Shahrabi)"

#: Triangle budget: variants above SOFT_MAX are reported, above HARD_MAX decimated.
SOFT_MAX_TRIS = 25_000
HARD_MAX_TRIS = 60_000

# ---------------------------------------------------------------------------
# Variant planning
# ---------------------------------------------------------------------------

#: Footprint archetypes. ``facade`` is the length of the front (+X) wall along
#: Blender Y, ``depth`` the front-to-back size along Blender X (metres);
#: ``upper`` the range of upper storeys (GenerateHouse "NumberFloor").
ARCHETYPES: dict[str, dict] = {
    "cottage": {"weight": 11, "facade": (5.0, 7.5), "depth": (4.0, 5.2), "upper": (1, 1)},
    "townhouse": {"weight": 13, "facade": (6.0, 9.0), "depth": (4.5, 6.0), "upper": (1, 2)},
    # Narrow, deep and tall: 3 upper storeys, occasionally 4.
    "merchant": {"weight": 10, "facade": (4.6, 5.8), "depth": (5.5, 7.5), "upper": (3, 3), "p_extra": 0.2},
    "hall": {"weight": 8, "facade": (9.0, 11.0), "depth": (5.0, 6.5), "upper": (1, 2)},
    "manor": {"weight": 7, "facade": (8.0, 10.0), "depth": (6.0, 7.2), "upper": (2, 2), "p_extra": 0.25},
}

# Grounded, weathered medieval palettes (sRGB). Weights bias the common choices.
PLASTER = [
    ((0.92, 0.89, 0.81), 5),  # limewash white
    ((0.89, 0.84, 0.72), 5),  # cream
    ((0.85, 0.78, 0.63), 3),  # pale ochre
    ((0.80, 0.71, 0.55), 2),  # ochre
    ((0.84, 0.75, 0.66), 2),  # warm buff / faded pink
    ((0.78, 0.77, 0.72), 2),  # grey limewash
    ((0.72, 0.60, 0.47), 1),  # dark ochre
]
TIMBER = [
    ((0.22, 0.16, 0.11), 5),  # dark oak
    ((0.29, 0.20, 0.13), 4),  # brown oak
    ((0.17, 0.13, 0.11), 3),  # tarred, near black
    ((0.34, 0.25, 0.17), 2),  # mid brown
    ((0.30, 0.28, 0.25), 2),  # silvered, weathered oak
]
SLATE = [
    ((0.29, 0.31, 0.35), 4),
    ((0.25, 0.27, 0.30), 3),
    ((0.35, 0.36, 0.39), 2),
    ((0.32, 0.30, 0.34), 2),  # purple-grey slate
]
TILES = [
    ((0.55, 0.31, 0.22), 4),  # terracotta
    ((0.50, 0.29, 0.22), 3),  # aged terracotta
    ((0.60, 0.37, 0.27), 2),  # light clay
    ((0.44, 0.26, 0.20), 2),  # dark, mossy clay
    ((0.44, 0.33, 0.26), 1),  # brown weathered tiles
]
DOOR = [
    ((0.36, 0.23, 0.14), 5),  # oak
    ((0.28, 0.19, 0.12), 4),  # dark oak
    ((0.27, 0.31, 0.24), 2),  # muted green
    ((0.40, 0.19, 0.15), 2),  # oxblood
    ((0.27, 0.29, 0.33), 1),  # dull blue-grey
]
WINDOW = [
    ((0.24, 0.17, 0.11), 5),  # dark wood
    ((0.30, 0.35, 0.29), 2),  # green-grey
    ((0.36, 0.20, 0.16), 2),  # oxblood
    ((0.80, 0.77, 0.69), 1),  # off-white
    ((0.28, 0.30, 0.34), 1),  # blue-grey
]
STONE = [
    ((0.62, 0.59, 0.53), 4),  # limestone
    ((0.56, 0.54, 0.50), 3),  # grey stone
    ((0.66, 0.59, 0.49), 2),  # sandstone
    ((0.50, 0.48, 0.45), 1),  # dark rubble
]

#: Source material (original name in the .blend) -> canonical slot. "Roof" is
#: resolved per house to "Slate" or "RoofTile".
SOURCE_TO_SLOT = {
    "Roof": "ROOF",
    "FakeRoof": "ROOF",
    "UpperFloorsPaint": "Plaster",
    "GroundFloorWalls": "Stone",
    "Stone": "Stone",
    "Stone.001": "Stone",
    "Stone.002": "Stone",
    "Beam": "Timber",
    "BeamFakeTexture": "Timber",
    "BeamFakeTextureVertical": "Timber",
    "mde_window_frame": "WindowFrame",
    "mde_door": "Door",
    "mde_chimney_pot": "Stone",
    "WindowGlass": "Glass",
    "RoundWindwos": "Glass",
    "Copper": "Iron",
    "Greenery": "Foliage",
    "VertexColor": "Foliage",
    "GreenSta.001": "Foliage",
    "Material.001": "Foliage",
    "Material.002": "Foliage",
    "Material.003": "Foliage",
    "Material.005": "Foliage",
}

#: Order in which canonical slots appear in the exported material list.
SLOT_ORDER = [
    "Plaster",
    "Timber",
    "Stone",
    "RoofTile",
    "Slate",
    "Thatch",
    "Planks",
    "Door",
    "WindowFrame",
    "Glass",
    "Iron",
    "Foliage",
]


def pick(rng: random.Random, weighted: list) -> tuple:
    """Weighted choice from ``[(value, weight), ...]``."""
    total = sum(w for _, w in weighted)
    x = rng.uniform(0.0, total)
    for value, w in weighted:
        x -= w
        if x <= 0.0:
            return value
    return weighted[-1][0]


def plan_wing(rng: random.Random, archetype: str, facade: float) -> list[float] | None:
    """Sometimes a centred front wing (porch / entrance bay): ``[width, depth]`` in metres.

    Upstream builds a straight ridge only over rectangles (trapezoid foundations
    crease the roof), so a wing is the way to vary the footprint outline.
    """
    if archetype not in ("cottage", "townhouse") or facade < 6.5 or rng.random() >= 0.25:
        return None
    width = rng.uniform(2.0, min(3.0, facade - 3.5))
    return [round(width, 2), round(rng.uniform(1.0, 1.4), 2)]


def plan_variants(count: int, seed: int) -> list[dict]:
    """Deterministically choose footprint, storeys, roof, ivy and colours per variant."""
    rng = random.Random(f"mde-french-houses:{seed}")
    names = list(ARCHETYPES)
    weights = [ARCHETYPES[n]["weight"] for n in names]
    # Stratified archetype list: proportional to the weights, then shuffled.
    seq: list[str] = []
    total = sum(weights)
    for n, w in zip(names, weights, strict=True):
        seq += [n] * max(1, round(count * w / total))
    while len(seq) < count:
        seq.append(rng.choices(names, weights)[0])
    rng.shuffle(seq)
    seq = seq[:count]
    width = max(2, len(str(max(count - 1, 0))))
    specs = []
    for i, arch in enumerate(seq):
        a = ARCHETYPES[arch]
        upper = rng.randint(*a["upper"])
        if rng.random() < a.get("p_extra", 0.0):
            upper += 1
        slate = rng.random() < 0.55
        facade = round(rng.uniform(*a["facade"]), 2)
        colors = {
            "Plaster": pick(rng, PLASTER),
            "Timber": pick(rng, TIMBER),
            ("Slate" if slate else "RoofTile"): pick(rng, SLATE if slate else TILES),
            "Door": pick(rng, DOOR),
            "WindowFrame": pick(rng, WINDOW),
            "Stone": pick(rng, STONE),
        }
        specs.append(
            {
                "index": i,
                "file": f"house_{i:0{width}d}.glb",
                "archetype": arch,
                "gn_seed": rng.randint(0, 99_999),
                "upper_floors": upper,
                "facade": facade,
                "depth": round(rng.uniform(*a["depth"]), 2),
                "wing": plan_wing(rng, arch, facade),
                "roof_curvature": round(rng.uniform(0.7, 1.6), 2),
                "roof_slot": "Slate" if slate else "RoofTile",
                "ivy": rng.random() < 0.25,
                "flower_boxes": rng.random() < 0.5,
                "no_roof_extrude_front": rng.random() < 0.15,
                "no_roof_extrude_back": rng.random() < 0.25,
                "colors": {k: [round(c, 3) for c in v] + [1.0] for k, v in colors.items()},
            }
        )
    return specs


# ---------------------------------------------------------------------------
# Node-group preparation (in memory only; the .blend is never saved)
# ---------------------------------------------------------------------------


def _node(group, name: str, node_type: str | None = None):
    """Find a node by name (and optionally type); warn and return None if missing."""
    node = group.nodes.get(name)
    if node is None or (node_type and node.type != node_type):
        print(f"WARNING: node {group.name!r}/{name!r} not found (upstream file changed?) - skipped")
        return None
    return node


def _marker_material(name: str):
    """A source-side marker material used to tag geometry for slot mapping."""
    full = common.SOURCE_MATERIAL_PREFIX + name
    return bpy.data.materials.get(full) or bpy.data.materials.new(full)


#: In-memory tweaks of the upstream scatter settings (upstream value in comments).
TUNING = {
    "wall_stone_density": 1.2,  # stones per m^2 of ground-floor wall (10)
    "ivy_leaf_density": 60.0,  # ivy leaves per m^2 (126)
    "window_plant_density": 7.0,  # window-box plants per m^2 of soil (103)
    "roof_tile_spacing": 0.30,  # Poisson spacing of roof tiles, m (0.20)
    "dormer_tile_spacing": 0.24,  # ... of dormer-roof tiles, m (0.17)
    "tile_scale": 1.3,  # tile size factor (1.0 = 0.5 x 0.25 x 0.05 m)
    "mullion_spacing": 0.36,  # glazing-bar spacing in windows, m (0.30)
    "stone_decimate": 0.3,  # decimate ratio of the wall-stone meshes (1.0)
    "no_flower_box_probability": 0.3,  # per window, for houses with flower boxes (0.22)
}


def tune_node_groups(detail: float = 1.0) -> None:
    """Retag window frames, doors and chimneys and trim the most expensive details.

    * Window frames and glazing bars (``Generate Windows``), the door leaf and
      its planks and the chimney pots (``GenerateHouse``) use the generic
      "Beam" material upstream; they get marker materials so they export as
      ``WindowFrame`` / ``Door`` / ``Stone``.
    * Ivy leaves are 5-sided cylinders (16 triangles); a flat pentagon (3
      triangles, double-sided material) looks the same from a street.
    * Roof tiles lose their hidden bottom face and get soft sides (12 instead
      of 24 vertices each); tiles, wall stones, ivy and window-box plants are
      scattered more sparsely (see :data:`TUNING`) to fit the size budget.

    ``detail`` scales the scatter densities (1.0 = the shipped settings).
    """
    gh = bpy.data.node_groups.get("GenerateHouse")
    gw = bpy.data.node_groups.get("Generate Windows")
    if gh is None or gw is None:
        raise SystemExit("GenerateHouse / Generate Windows node groups not found in the .blend")

    retag = [
        (gw, ("Set Material.001", "Set Material.002"), "mde_window_frame"),
        (gh, ("Set Material.014", "Set Material.015"), "mde_door"),
        (gh, ("Set Material.013",), "mde_chimney_pot"),
    ]
    for group, names, marker in retag:
        mat = _marker_material(marker)
        for name in names:
            n = _node(group, name, "SET_MATERIAL")
            if n is not None:
                n.inputs["Material"].default_value = mat

    def set_input(group, node_name: str, socket: str, value: float) -> None:
        n = _node(group, node_name)
        if n is None or socket not in n.inputs:
            print(f"WARNING: {group.name!r}/{node_name!r} has no input {socket!r} - skipped")
            return
        n.inputs[socket].default_value = value

    density = max(detail, 0.25)
    set_input(gh, "Distribute Points on Faces.004", "Density Max", TUNING["wall_stone_density"] * detail)
    set_input(gh, "Distribute Points on Faces.005", "Density", TUNING["ivy_leaf_density"] * detail)
    set_input(gw, "Distribute Points on Faces", "Density Max", TUNING["window_plant_density"] * detail)
    set_input(gh, "Distribute Points on Faces", "Distance Min", TUNING["roof_tile_spacing"] / density**0.5)
    set_input(gh, "Distribute Points on Faces.001", "Distance Min", TUNING["dormer_tile_spacing"] / density**0.5)
    for name in ("Resample Curve", "Resample Curve.001"):
        set_input(gw, name, "Length", TUNING["mullion_spacing"])
    for value_name in ("Value.001", "Value.004"):
        n = _node(gh, value_name, "VALUE")
        if n is not None:
            n.outputs[0].default_value *= TUNING["tile_scale"]
    for cube_name in ("Cube", "Cube.002"):
        cube = _node(gh, cube_name, "MESH_PRIMITIVE_CUBE")
        if cube is not None:
            _optimise_tile(gh, cube)
    # Flat pentagon ivy leaves instead of thin cylinders.
    cyl = _node(gh, "Cylinder", "MESH_PRIMITIVE_CYLINDER")
    if cyl is not None:
        for link in list(cyl.outputs["Mesh"].links):
            circle = gh.nodes.new("GeometryNodeMeshCircle")
            circle.fill_type = "NGON"
            circle.inputs["Vertices"].default_value = cyl.inputs["Vertices"].default_value
            circle.inputs["Radius"].default_value = cyl.inputs["Radius"].default_value
            gh.links.new(circle.outputs["Mesh"], link.to_socket)
    # Wall stones (collection "Set") and window-box plants ("Plants Low Poly")
    # are organic blobs: decimate the stones and shade both smooth, which lets
    # the exporter share vertices between faces.
    stones = bpy.data.collections.get("Set")
    if stones is not None:
        for obj in stones.objects:
            if obj.type == "MESH" and not any(m.type == "DECIMATE" for m in obj.modifiers):
                mod = obj.modifiers.new("mde_decimate", "DECIMATE")
                mod.ratio = TUNING["stone_decimate"]
                obj.data.shade_smooth()
    plants = bpy.data.collections.get("Plants Low Poly")
    if plants is not None:
        for obj in plants.objects:
            if obj.type == "MESH":
                obj.data.shade_smooth()


def _optimise_tile(group, cube) -> None:
    """Insert nodes after a tile cube: drop the bottom face, flat top, smooth sides."""
    targets = [link.to_socket for link in cube.outputs["Mesh"].links]
    normal = group.nodes.new("GeometryNodeInputNormal")
    sep = group.nodes.new("ShaderNodeSeparateXYZ")
    group.links.new(normal.outputs["Normal"], sep.inputs["Vector"])
    is_bottom = group.nodes.new("FunctionNodeCompare")
    is_bottom.data_type = "FLOAT"
    is_bottom.operation = "LESS_THAN"
    is_bottom.inputs["B"].default_value = -0.5
    group.links.new(sep.outputs["Z"], is_bottom.inputs["A"])
    is_side = group.nodes.new("FunctionNodeCompare")
    is_side.data_type = "FLOAT"
    is_side.operation = "LESS_THAN"
    is_side.inputs["B"].default_value = 0.5
    group.links.new(sep.outputs["Z"], is_side.inputs["A"])
    delete = group.nodes.new("GeometryNodeDeleteGeometry")
    delete.domain = "FACE"
    group.links.new(cube.outputs["Mesh"], delete.inputs["Geometry"])
    group.links.new(is_bottom.outputs["Result"], delete.inputs["Selection"])
    smooth = group.nodes.new("GeometryNodeSetShadeSmooth")
    smooth.domain = "FACE"
    group.links.new(delete.outputs["Geometry"], smooth.inputs["Geometry"])
    group.links.new(is_side.outputs["Result"], smooth.inputs["Shade Smooth"])
    for sock in targets:
        group.links.new(smooth.outputs["Geometry"], sock)


def make_realize_group():
    """Node group: realise all instances and store each vertex's mesh-island index.

    The island index ("mde_island") drives the per-piece colour variation.
    """
    g = bpy.data.node_groups.new("MDE_RealizeIslands", "GeometryNodeTree")
    g.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
    g.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
    gi = g.nodes.new("NodeGroupInput")
    go = g.nodes.new("NodeGroupOutput")
    realize = g.nodes.new("GeometryNodeRealizeInstances")
    island = g.nodes.new("GeometryNodeInputMeshIsland")
    store = g.nodes.new("GeometryNodeStoreNamedAttribute")
    store.data_type = "INT"
    store.domain = "POINT"
    store.inputs["Name"].default_value = "mde_island"
    g.links.new(gi.outputs[0], realize.inputs["Geometry"])
    g.links.new(realize.outputs["Geometry"], store.inputs["Geometry"])
    g.links.new(island.outputs["Island Index"], store.inputs["Value"])
    g.links.new(store.outputs["Geometry"], go.inputs[0])
    return g


def set_modifier_input(mod, name: str, value) -> None:
    """Set a Geometry-Nodes modifier input by its interface name (Blender 4.x and 5.x)."""
    item = next(
        (
            it
            for it in mod.node_group.interface.items_tree
            if it.item_type == "SOCKET" and it.in_out == "INPUT" and it.name == name
        ),
        None,
    )
    if item is None:
        print(f"WARNING: {mod.node_group.name} has no input {name!r} - skipped")
        return
    props = getattr(mod, "properties", None)
    if props is not None and hasattr(props, "inputs"):  # Blender 5.x
        getattr(props.inputs, item.identifier).value = value
    else:  # Blender 4.x ID-property style
        mod[item.identifier] = value


# ---------------------------------------------------------------------------
# Per-house build
# ---------------------------------------------------------------------------


def foundation_mesh(spec: dict) -> tuple[list[tuple[float, float]], list[tuple[int, ...]]]:
    """Foundation in Blender XY: vertices and faces, mirror-symmetric about the X axis.

    A plain rectangle, or - when ``spec["wing"] = [width, depth]`` - a rectangle
    split into three strips with a centred front wing (porch / bay) attached to
    the middle strip, as in the upstream "HouseThree" example. The footprint
    bounds are centred on the origin.
    """
    hx = spec["depth"] / 2.0
    hy = spec["facade"] / 2.0
    wing = spec.get("wing")
    if not wing:
        return [(-hx, -hy), (hx, -hy), (hx, hy), (-hx, hy)], [(0, 1, 2, 3)]
    a, wd = wing[0] / 2.0, wing[1]
    sx = -wd / 2.0  # centre the bounds
    verts = [
        (-hx, -hy),
        (hx, -hy),
        (hx, -a),
        (-hx, -a),
        (hx, a),
        (-hx, a),
        (hx, hy),
        (-hx, hy),
        (hx + wd, -a),
        (hx + wd, a),
    ]
    faces = [(0, 1, 2, 3), (3, 2, 4, 5), (5, 4, 6, 7), (2, 8, 9, 4)]
    return [(x + sx, y) for x, y in verts], faces


def set_flower_boxes(enabled: bool) -> None:
    """Window boxes on most windows, or none (GenerateHouse has no input for it).

    Sets "NoFlowerProbability" (upstream 0.22) on every ``Generate Windows``
    node inside ``GenerateHouse``; affects the next evaluation.
    """
    gh = bpy.data.node_groups["GenerateHouse"]
    for node in gh.nodes:
        if node.type == "GROUP" and node.node_tree is not None and node.node_tree.name == "Generate Windows":
            sock = node.inputs.get("NoFlowerProbability")
            if sock is not None and not sock.is_linked:
                sock.default_value = TUNING["no_flower_box_probability"] if enabled else 1.0


def build_generator(spec: dict, realize_group):
    """Create the foundation object with the GenerateHouse + realise modifiers."""
    set_flower_boxes(spec["flower_boxes"])
    me = bpy.data.meshes.new(f"foundation_{spec['index']}")
    verts, faces = foundation_mesh(spec)
    me.from_pydata([(x, y, 0.0) for x, y in verts], [], faces)
    obj = bpy.data.objects.new(f"generator_{spec['index']}", me)
    bpy.context.scene.collection.objects.link(obj)
    # GenerateHouse derives some per-floor seeds from the object position; give
    # each variant its own spot (the evaluated mesh is in local space anyway).
    obj.location = (spec["index"] * 61.0 + 13.0, (spec["index"] % 7) * 47.0, 0.0)
    mod = obj.modifiers.new("house", "NODES")
    mod.node_group = bpy.data.node_groups["GenerateHouse"]
    set_modifier_input(mod, "Object", obj)
    set_modifier_input(mod, "Seed", spec["gn_seed"])
    set_modifier_input(mod, "NumberFloor", spec["upper_floors"])
    set_modifier_input(mod, "RoofCurvature", spec["roof_curvature"])
    set_modifier_input(mod, "SpawnIVY", spec["ivy"])
    set_modifier_input(mod, "High Polz Plants", False)
    set_modifier_input(mod, "DisableRoofExrudeFront", spec["no_roof_extrude_front"])
    set_modifier_input(mod, "DisableRoofExrudeBack", spec["no_roof_extrude_back"])
    lin = common.srgba_to_linear
    set_modifier_input(mod, "RoofColor", lin(spec["colors"][spec["roof_slot"]]))
    set_modifier_input(mod, "PaintColor", lin(spec["colors"]["Plaster"]))
    set_modifier_input(mod, "BeamCollor", lin(spec["colors"]["Timber"]))
    realize = obj.modifiers.new("realize", "NODES")
    realize.node_group = realize_group
    return obj


def _hash01(values: np.ndarray, salt: int) -> np.ndarray:
    """Deterministic integer hash -> uniform floats in [0, 1)."""
    offset = np.uint64((salt * 0x9E3779B9) & 0xFFFFFFFF)
    x = (values.astype(np.uint64) + offset) & np.uint64(0xFFFFFFFF)
    x = (x ^ (x >> np.uint64(16))) * np.uint64(0x45D9F3B) & np.uint64(0xFFFFFFFF)
    x = (x ^ (x >> np.uint64(16))) * np.uint64(0x45D9F3B) & np.uint64(0xFFFFFFFF)
    x = x ^ (x >> np.uint64(16))
    return x.astype(np.float64) / 4294967296.0


def _point_attr(mesh, name: str, width: int, default: float = 0.0) -> np.ndarray:
    """Point-domain attribute as an ``(n, width)`` array, or ``default`` if absent."""
    n = len(mesh.vertices)
    attr = mesh.attributes.get(name)
    if attr is None or attr.domain != "POINT":
        return np.full((n, width), default, dtype=np.float64)
    key = {
        "FLOAT": "value",
        "INT": "value",
        "FLOAT_VECTOR": "vector",
        "FLOAT_COLOR": "color",
        "BYTE_COLOR": "color",
    }.get(attr.data_type)
    if key is None:
        return np.full((n, width), default, dtype=np.float64)
    buf = np.empty(n * width, dtype=np.float64 if attr.data_type != "INT" else np.int64)
    attr.data.foreach_get(key, buf)
    return buf.reshape(n, width).astype(np.float64)


def bake_house(gen_obj, spec: dict, overrides: dict | None = None):
    """Evaluate the generator and return ``(object, slots)``: a plain mesh with canonical materials.

    ``spec`` needs ``file`` (object name), ``roof_slot`` and ``gn_seed`` (colour
    salt); ``overrides`` replaces entries of :data:`SOURCE_TO_SLOT` (used by the
    part exporter, where e.g. "Beam" means a window frame or a door leaf).
    """
    mapping = dict(SOURCE_TO_SLOT, **(overrides or {}))
    dg = bpy.context.evaluated_depsgraph_get()
    dg.update()
    ev = gen_obj.evaluated_get(dg)
    mesh = bpy.data.meshes.new_from_object(ev, preserve_all_data_layers=True, depsgraph=dg)
    mesh.name = spec["file"].removesuffix(".glb")
    obj = bpy.data.objects.new(mesh.name, mesh)
    bpy.context.scene.collection.objects.link(obj)
    bpy.data.objects.remove(gen_obj)

    npoly = len(mesh.polygons)
    if npoly == 0:
        raise RuntimeError(f"{spec['file']}: the generator produced no geometry")
    # Source material name per polygon.
    mat_index = np.empty(npoly, dtype=np.int64)
    mesh.polygons.foreach_get("material_index", mat_index)
    src_names = [common.source_material_name(m) for m in mesh.materials] or [None]
    unknown = sorted({n for n in src_names if n not in mapping and n is not None})
    if unknown:
        print(f"WARNING: unmapped source materials {unknown} -> Timber")

    def slot_of(src: str | None) -> str:
        slot = mapping.get(src, "Plaster" if src is None else "Timber")
        return spec["roof_slot"] if slot == "ROOF" else slot

    src_slots = [slot_of(n) for n in src_names]
    present = [s for s in SLOT_ORDER if s in src_slots]
    present += sorted(set(src_slots) - set(present))
    slot_index = {s: i for i, s in enumerate(present)}
    remap = np.array([slot_index[s] for s in src_slots], dtype=np.int64)
    mat_index = np.clip(mat_index, 0, len(src_names) - 1)
    poly_src = mat_index.copy()  # source material index per polygon
    new_index = remap[mat_index]

    paint_vertex_colors(mesh, spec, poly_src, src_names, src_slots)

    mesh.materials.clear()
    for s in present:
        mat = common.canonical_material(s)
        mesh.materials.append(mat)
    mesh.polygons.foreach_set("material_index", new_index.astype(np.int32))
    mesh.update()
    return obj, present


def paint_vertex_colors(mesh, spec: dict, poly_src: np.ndarray, src_names: list, src_slots: list) -> None:
    """Bake the ``Col`` corner colour attribute (exported as glTF ``COLOR_0``).

    Recolourable slots get a soft per-piece multiplier (each roof tile, beam and
    wall stone slightly lighter or darker); ``Foliage`` gets real leaf / blossom
    colours over a white base colour. All values are linear.
    """
    npoly = len(mesh.polygons)
    loop_total = np.empty(npoly, dtype=np.int64)
    mesh.polygons.foreach_get("loop_total", loop_total)
    corner_poly = np.repeat(np.arange(npoly), loop_total)
    corner_vert = np.empty(len(mesh.loops), dtype=np.int64)
    mesh.loops.foreach_get("vertex_index", corner_vert)

    island = _point_attr(mesh, "mde_island", 1)[:, 0].astype(np.int64)
    tile = _point_attr(mesh, "TileRandom", 1, -1.0)[:, 0]
    plant = _point_attr(mesh, "Color", 4, -1.0)

    salt = spec["gn_seed"]
    r = _hash01(island, salt)[corner_vert]
    r2 = _hash01(island, salt + 1)[corner_vert]
    t = tile[corner_vert]
    src = poly_src[corner_poly]
    slot = np.array(src_slots, dtype=object)[src]
    src_name = np.array([n or "" for n in src_names], dtype=object)[src]

    ncorner = len(corner_vert)
    col = np.ones((ncorner, 4), dtype=np.float64)

    def grey(mask, lo, hi, values):
        v = lo + (hi - lo) * values[mask]
        col[mask, 0] = v
        col[mask, 1] = v
        col[mask, 2] = v

    roof = (slot == "Slate") | (slot == "RoofTile")
    roof_r = np.where(t >= 0.0, t, r)
    grey(roof, 0.55, 1.0, roof_r)
    # A touch of hue drift between tiles (warmer / cooler).
    drift = (r2[roof] - 0.5) * 0.10
    col[roof, 0] *= 1.0 + drift
    col[roof, 2] *= 1.0 - drift
    grey(slot == "Timber", 0.72, 1.0, r)
    grey((slot == "Door") | (slot == "WindowFrame"), 0.85, 1.0, r)
    grey(slot == "Plaster", 0.90, 1.0, r)
    grey(slot == "Stone", 0.62, 1.0, r)

    foliage = slot == "Foliage"
    ivy = foliage & (src_name == "Greenery")
    dark = np.array([0.030, 0.062, 0.014])
    light = np.array([0.085, 0.140, 0.030])
    if ivy.any():
        col[ivy, :3] = dark + (light - dark) * r[ivy, None]
    flowers = foliage & ~ivy
    if flowers.any():
        pc = plant[corner_vert][flowers]
        has = pc[:, 0] >= 0.0
        base = np.where(has[:, None], pc[:, :3], (dark + light) / 2.0)
        # Slightly mute the saturated upstream blossoms for the grounded style.
        lum = base @ np.array([0.2126, 0.7152, 0.0722])
        col[flowers, :3] = lum[:, None] + (base - lum[:, None]) * 0.8
    np.clip(col, 0.0, 1.0, out=col)

    attr = mesh.color_attributes.new("Col", "FLOAT_COLOR", "CORNER")
    attr.data.foreach_set("color", col.astype(np.float32).ravel())
    mesh.color_attributes.active_color = attr
    mesh.color_attributes.render_color_index = mesh.color_attributes.find("Col")


def measure(obj, spec: dict) -> dict:
    """Footprint and door centre in model space (glTF axes: x, y up, z = -Blender y)."""
    mesh = obj.data
    n = len(mesh.vertices)
    co = np.empty(n * 3)
    mesh.vertices.foreach_get("co", co)
    co = co.reshape(n, 3)
    mi = np.empty(len(mesh.polygons), dtype=np.int64)
    mesh.polygons.foreach_get("material_index", mi)
    names = [m.name for m in mesh.materials]

    def verts_of(slot: str) -> np.ndarray:
        if slot not in names:
            return np.zeros((0, 3))
        k = names.index(slot)
        idx = set()
        for p, m in zip(mesh.polygons, mi, strict=True):
            if m == k:
                idx.update(p.vertices)
        return co[sorted(idx)] if idx else np.zeros((0, 3))

    verts, _ = foundation_mesh(spec)
    fx = [p[0] for p in verts]
    fy = [p[1] for p in verts]
    footprint = [round(max(fx) - min(fx), 3), round(max(fy) - min(fy), 3)]

    door = verts_of("Door")
    measured = len(door) > 0
    facing = None
    if measured:
        lo, hi = door.min(0), door.max(0)
        c = (lo + hi) / 2.0
        door_glb = [round(float(c[0]), 3), round(float(c[2]), 3), round(float(-c[1]), 3)]
        door_size = [round(float(hi[1] - lo[1]), 3), round(float(hi[2] - lo[2]), 3)]
        # Which way the door leaf faces: area-weighted normal of the Door faces.
        npoly = len(mesh.polygons)
        normals = np.empty(npoly * 3)
        mesh.polygons.foreach_get("normal", normals)
        areas = np.empty(npoly)
        mesh.polygons.foreach_get("area", areas)
        sel = mi == names.index("Door")
        nsum = (normals.reshape(npoly, 3)[sel] * areas[sel, None]).sum(0)
        axis = int(np.argmax(np.abs(nsum[:2])))
        sign = "+" if nsum[axis] > 0 else "-"
        # Blender +X = glTF +X, Blender +Y = glTF -Z.
        facing = sign + "X" if axis == 0 else ("-" if sign == "+" else "+") + "Z"
    else:  # fallback: middle of the front wall
        door_glb = [round(max(fx), 3), 1.0, 0.0]
        door_size = [0.0, 0.0]
    return {
        "footprint": footprint,
        "door": door_glb,
        "door_size": door_size,
        "door_facing": facing,
        "door_measured": measured,
    }


def _face_samples(face, spacing: float, max_steps: int = 16) -> list:
    """Points covering a face: its centre, then barycentric grids over its fan
    triangles at roughly ``spacing`` metres, pulled 3 % towards the centre so
    they do not sit exactly on shared edges."""
    c = face.calc_center_median()
    pts = [c]
    co = [v.co for v in face.verts]
    for i in range(1, len(co) - 1):
        a, b, d = co[0], co[i], co[i + 1]
        longest = max((a - b).length, (b - d).length, (d - a).length)
        k = max(1, min(max_steps, math.ceil(longest / spacing)))
        for u in range(k + 1):
            for v in range(k + 1 - u):
                p = a + (b - a) * (u / k) + (d - a) * (v / k)
                pts.append(p + (c - p) * 0.03)
    return pts


def merge_coplanar_faces(obj, angle_deg: float = 0.5) -> None:
    """Dissolve edges between coplanar faces of the same material.

    Curve-to-mesh beams come out as chains of short collinear segments and the
    walls as grids of quads; merging coplanar neighbours (limited dissolve)
    keeps the exact shape and shading with far fewer triangles. Faces only
    merge within one connected piece, so the per-piece colours are unaffected.
    """
    import bmesh

    mesh = obj.data
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.dissolve_limit(
        bm,
        angle_limit=math.radians(angle_deg),
        use_dissolve_boundaries=False,
        verts=list(bm.verts),
        edges=list(bm.edges),
        delimit={"MATERIAL", "SHARP"},
    )
    bm.to_mesh(mesh)
    bm.free()
    mesh.update()


def remove_hidden_faces(obj, reach: float = 0.04, inside_reach: float = 0.35, spacing: float = 0.05) -> int:
    """Delete faces that can never be seen from outside the house.

    The generator stacks closed boxes against and into each other: beams on and
    in walls, wall stones half sunk into plaster, tiles overlapping tiles,
    planks on the door. Rays are cast along the face normal from sample points
    covering the whole face (every ``spacing`` metres). A face is dropped only
    if *every* sample is blocked, where a sample is blocked when

    * any surface lies within ``reach`` metres in front of it (the face is
      pressed against something), or
    * the rays along both the normal and its opposite hit the *back* of a
      surface within ``inside_reach`` metres (the point is buried inside a
      closed volume, e.g. the half of a beam that sits inside a wall).

    Faces of double-sided materials (foliage) are always kept. Returns the
    number of deleted faces.
    """
    import bmesh
    from mathutils.bvhtree import BVHTree

    mesh = obj.data
    keep_slots = {i for i, m in enumerate(mesh.materials) if m is not None and not m.use_backface_culling}
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bm.faces.ensure_lookup_table()
    tree = BVHTree.FromBMesh(bm)

    def blocked(p, n, own: int) -> bool:
        loc, hn, index, dist = tree.ray_cast(p + n * 1e-4, n, inside_reach)
        if loc is None or index == own:
            return False
        if dist <= reach:
            return True
        if hn.dot(n) <= 0.0:
            return False  # a front face some way off: there is a visible gap
        back = -n
        loc, hn, index, _ = tree.ray_cast(p + back * 1e-4, back, inside_reach)
        return loc is not None and index != own and hn.dot(back) > 0.0

    hidden = []
    for face in bm.faces:
        if face.material_index in keep_slots:
            continue
        n = face.normal.copy()
        if n.length_squared < 0.5:
            continue
        if all(blocked(p, n, face.index) for p in _face_samples(face, spacing)):
            hidden.append(face)
    if hidden:
        bmesh.ops.delete(bm, geom=hidden, context="FACES_ONLY")
        bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context="VERTS")
    bm.to_mesh(mesh)
    bm.free()
    mesh.update()
    return len(hidden)


def decimate_if_needed(obj, tris: int) -> bool:
    """Add a collapse Decimate modifier when ``tris`` exceeds the hard budget."""
    if tris <= HARD_MAX_TRIS:
        return False
    mod = obj.modifiers.new("mde_budget", "DECIMATE")
    mod.ratio = max(0.1, HARD_MAX_TRIS * 0.95 / tris)
    print(f"  decimating {tris} tris with ratio {mod.ratio:.2f}")
    return True


def triangle_count(mesh) -> int:
    """Triangles after fan triangulation of every polygon."""
    totals = np.empty(len(mesh.polygons), dtype=np.int64)
    mesh.polygons.foreach_get("loop_total", totals)
    return int((totals - 2).clip(min=0).sum())


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


#: How often a variant is regenerated with another GenerateHouse seed when the
#: generator put the door on a side wall instead of the front (+X).
DOOR_RETRIES = 12


def produce_house(spec: dict, realize_group):
    """Generate, clean up and measure one variant; returns ``(obj, slots, info, decimated)``.

    GenerateHouse occasionally places the entrance on a gable (side) wall. The
    variant is then regenerated with the next seed (``spec["gn_seed"]`` is
    updated) so every exported house has its door on the +X front.
    """
    base_seed = spec["gn_seed"]
    for attempt in range(DOOR_RETRIES):
        spec["gn_seed"] = (base_seed + attempt * 7919) % 100_000
        gen = build_generator(spec, realize_group)
        obj, slots = bake_house(gen, spec)
        info = measure(obj, spec)
        if info["door_facing"] == "+X" or attempt == DOOR_RETRIES - 1:
            break
        print(f"  door faces {info['door_facing']} with seed {spec['gn_seed']}: retrying")
        mesh = obj.data
        bpy.data.objects.remove(obj)
        bpy.data.meshes.remove(mesh)
    if info["door_facing"] != "+X":
        print(f"WARNING: {spec['file']}: door faces {info['door_facing']} after {DOOR_RETRIES} seeds")
    before = triangle_count(obj.data)
    removed = remove_hidden_faces(obj)
    hidden_tris = triangle_count(obj.data)
    merge_coplanar_faces(obj)
    after = triangle_count(obj.data)
    print(f"  {before} tris -> {hidden_tris} ({removed} hidden faces removed) -> {after} (coplanar merged)")
    for s, c in spec["colors"].items():
        if s in slots:
            common.set_material_color(common.canonical_material(s), c)
    decimated = decimate_if_needed(obj, triangle_count(obj.data))
    info = measure(obj, spec)
    return obj, slots, info, decimated


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--blend", required=True, help="path to GeometryNodesFrenchHous.blend")
    p.add_argument("--out", default="assets/houses", help="output directory (default assets/houses)")
    p.add_argument("--count", type=int, default=50, help="number of variants (default 50)")
    p.add_argument("--seed", type=int, default=1, help="variant planning seed (default 1)")
    p.add_argument("--only", type=str, default="", help="comma-separated variant indices to (re)export")
    p.add_argument("--detail", type=float, default=1.0, help="scatter density factor (default 1.0)")
    return p.parse_args(common.script_args())


def main() -> None:
    args = parse_args()
    if args.count < 1:
        raise SystemExit("--count must be >= 1")
    out_dir = os.path.abspath(args.out)
    os.makedirs(out_dir, exist_ok=True)
    common.open_source_blend(os.path.abspath(args.blend))
    tune_node_groups(args.detail)
    realize_group = make_realize_group()
    specs = plan_variants(args.count, args.seed)
    only = {int(x) for x in args.only.split(",") if x.strip()} if args.only else None

    manifest_path = os.path.join(out_dir, "manifest.json")
    previous = {}
    if only is not None and os.path.isfile(manifest_path):
        with open(manifest_path) as f:
            previous = {e["file"]: e for e in json.load(f)}

    entries = []
    t_start = time.time()
    for spec in specs:
        path = os.path.join(out_dir, spec["file"])
        if only is not None and spec["index"] not in only:
            if spec["file"] in previous:
                entries.append(previous[spec["file"]])
            continue
        t0 = time.time()
        obj, slots, info, decimated = produce_house(spec, realize_group)
        common.export_glb(obj, path, copyright_note=f"Derived from {UPSTREAM}")
        saved = common.compact_vertex_colors(path)
        summary = common.glb_summary(path)
        entry = {
            "file": spec["file"],
            "seed": spec["gn_seed"],
            "floors": spec["upper_floors"] + 1,
            "footprint": info["footprint"],
            "bounds_min": summary["bounds_min"],
            "bounds_max": summary["bounds_max"],
            "height": summary["bounds_max"][1],
            "triangles": summary["triangles"],
            "front": "+X",
            "door": info["door"],
            "door_facing": info["door_facing"],
            "materials": summary["materials"],
            "archetype": spec["archetype"],
            "door_size": info["door_size"],
            "ivy": spec["ivy"],
            "flower_boxes": spec["flower_boxes"],
            "wing": spec["wing"],
            "colors": {s: c for s, c in spec["colors"].items() if s in summary["materials"]},
            "bytes": summary["bytes"],
        }
        if not info["door_measured"]:
            entry["door_estimated"] = True
        if decimated:
            entry["decimated"] = True
        entries.append(entry)
        flag = " (over soft budget)" if summary["triangles"] > SOFT_MAX_TRIS else ""
        print(
            f"[{spec['index'] + 1}/{len(specs)}] {spec['file']} {spec['archetype']:<9} "
            f"floors={entry['floors']} tris={summary['triangles']}{flag} "
            f"size={summary['bytes'] / 1e6:.2f}MB (-{saved / 1e6:.2f}) door={entry['door']} "
            f"t={time.time() - t0:.1f}s",
            flush=True,
        )
        # Free the baked mesh before the next variant.
        mesh = obj.data
        bpy.data.objects.remove(obj)
        bpy.data.meshes.remove(mesh)

    entries.sort(key=lambda e: e["file"])
    with open(manifest_path, "w") as f:
        json.dump(entries, f, indent=1)
        f.write("\n")
    total = sum(e["bytes"] for e in entries)
    tris = [e["triangles"] for e in entries]
    print(
        f"wrote {len(entries)} houses to {out_dir}: {total / 1e6:.1f} MB, "
        f"triangles min/avg/max {min(tris)}/{sum(tris) // len(tris)}/{max(tris)}, "
        f"{time.time() - t_start:.0f}s"
    )


if __name__ == "__main__":
    main()
