"""Shared helpers for the Midearth Blender export scripts.

Imported by ``export_french_houses.py``, ``export_parts.py`` and
``render_contact_sheet.py``. Everything that touches ``bpy`` is imported lazily
so the pure-Python GLB helpers (``read_glb``, ``glb_summary``, ...) can also be
used from a plain Python interpreter, e.g. to validate assets in CI.

Conventions shared by every exported asset (see ``assets/houses/README.md`` and
``assets/parts/README.md``):

* glTF 2.0 binary (``.glb``), Y up, metres, no textures, no extensions.
* Materials carry the canonical names of ``mde_core::MaterialSlot``
  (``Plaster``, ``Timber``, ``Stone``, ``RoofTile``, ``Slate``, ``Door``,
  ``WindowFrame``, ``Glass``, ``Iron``, ``Foliage``, ``Planks`` ...) so a viewer
  can recolour a slot by replacing its ``baseColorFactor``.
* ``COLOR_0`` (linear RGBA) multiplies the base colour as the glTF spec says.
  For recolourable slots it only holds a soft light/dark variation; for
  ``Foliage`` the base colour is white and ``COLOR_0`` holds the real colours.
"""

from __future__ import annotations

import json
import math
import os
import struct
import sys

# ---------------------------------------------------------------------------
# Canonical materials
# ---------------------------------------------------------------------------

#: Canonical slot name -> (default sRGB colour, roughness, metallic, double sided).
#: Names match ``mde_core::MaterialSlot`` variant names exactly.
CANONICAL_MATERIALS: dict[str, tuple[tuple[float, float, float, float], float, float, bool]] = {
    "Plaster": ((0.90, 0.86, 0.77, 1.0), 0.95, 0.0, False),
    "Timber": ((0.27, 0.19, 0.13, 1.0), 0.85, 0.0, False),
    "Stone": ((0.60, 0.57, 0.52, 1.0), 0.95, 0.0, False),
    "RoofTile": ((0.58, 0.32, 0.22, 1.0), 0.85, 0.0, False),
    "Slate": ((0.31, 0.33, 0.37, 1.0), 0.70, 0.0, False),
    "Thatch": ((0.62, 0.52, 0.33, 1.0), 1.00, 0.0, False),
    "Planks": ((0.45, 0.33, 0.22, 1.0), 0.90, 0.0, False),
    "Door": ((0.36, 0.23, 0.14, 1.0), 0.80, 0.0, False),
    "WindowFrame": ((0.26, 0.18, 0.12, 1.0), 0.80, 0.0, False),
    "Glass": ((0.09, 0.11, 0.13, 1.0), 0.15, 0.0, False),
    "Iron": ((0.13, 0.13, 0.14, 1.0), 0.55, 0.8, False),
    "Foliage": ((1.0, 1.0, 1.0, 1.0), 0.90, 0.0, True),
}

#: Prefix given to every material that came from the source .blend, so the
#: canonical names (some of which, like "Stone", already exist there) are free.
SOURCE_MATERIAL_PREFIX = "src:"


def srgb_to_linear(c: float) -> float:
    """Convert one sRGB channel in ``[0, 1]`` to linear."""
    c = min(max(c, 0.0), 1.0)
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def srgba_to_linear(rgba) -> tuple[float, float, float, float]:
    """Convert an sRGB(A) colour to linear RGBA (alpha is passed through)."""
    a = rgba[3] if len(rgba) > 3 else 1.0
    return (srgb_to_linear(rgba[0]), srgb_to_linear(rgba[1]), srgb_to_linear(rgba[2]), a)


def script_args(argv: list[str] | None = None) -> list[str]:
    """Arguments after ``--`` (what Blender / bpy leave for the script)."""
    argv = sys.argv if argv is None else argv
    return argv[argv.index("--") + 1 :] if "--" in argv else argv[1:]


# ---------------------------------------------------------------------------
# bpy helpers
# ---------------------------------------------------------------------------


def open_source_blend(path: str) -> None:
    """Open the source .blend and empty its scene without deleting any data.

    Node groups reference helper objects and collections (plants, stones), so
    those stay in ``bpy.data``; they are only unlinked from the scene so renders
    and exports see nothing but what the script adds.
    """
    import bpy

    if not os.path.isfile(path):
        raise SystemExit(f"source .blend not found: {path}")
    bpy.ops.wm.open_mainfile(filepath=path)
    scene = bpy.context.scene
    for child in list(scene.collection.children):
        scene.collection.children.unlink(child)
    for obj in list(scene.collection.objects):
        scene.collection.objects.unlink(obj)
    for mat in bpy.data.materials:
        if not mat.name.startswith(SOURCE_MATERIAL_PREFIX):
            mat.name = SOURCE_MATERIAL_PREFIX + mat.name


def source_material_name(mat) -> str | None:
    """Original (pre-prefix) name of a source material, or its name otherwise."""
    if mat is None:
        return None
    name = mat.name
    return name[len(SOURCE_MATERIAL_PREFIX) :] if name.startswith(SOURCE_MATERIAL_PREFIX) else name


def canonical_material(name: str):
    """Get or create the Blender material for canonical slot ``name``.

    The material is a plain Principled BSDF so the glTF exporter writes
    ``baseColorFactor``, ``roughnessFactor``, ``metallicFactor`` and
    ``doubleSided`` from it. There is exactly one datablock per slot (so the
    exported name is exactly the slot name); call :func:`set_material_color`
    before each export to give the asset its own default colour.
    """
    import bpy

    if name not in CANONICAL_MATERIALS:
        raise KeyError(f"not a canonical material slot: {name}")
    mat = bpy.data.materials.get(name)
    if mat is not None:
        return mat
    default, roughness, metallic, double_sided = CANONICAL_MATERIALS[name]
    mat = bpy.data.materials.new(name)
    if mat.name != name:  # a stray datablock already owns the name
        raise RuntimeError(f"material name {name!r} is taken; rename source materials first")
    if mat.node_tree is None:  # older Blender versions create materials without nodes
        mat.use_nodes = True
    nodes = mat.node_tree.nodes
    bsdf = next((n for n in nodes if n.type == "BSDF_PRINCIPLED"), None)
    if bsdf is None:
        for n in list(nodes):
            nodes.remove(n)
        bsdf = nodes.new("ShaderNodeBsdfPrincipled")
        out = nodes.new("ShaderNodeOutputMaterial")
        mat.node_tree.links.new(bsdf.outputs[0], out.inputs[0])
    bsdf.inputs["Roughness"].default_value = roughness
    bsdf.inputs["Metallic"].default_value = metallic
    mat.roughness = roughness
    mat.metallic = metallic
    mat.use_backface_culling = not double_sided
    set_material_color(mat, default)
    return mat


def set_material_color(mat, srgba) -> None:
    """Set a canonical material's base colour from an sRGB(A) colour."""
    lin = srgba_to_linear(srgba)
    bsdf = next(n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    bsdf.inputs["Base Color"].default_value = lin
    mat.diffuse_color = lin


def export_glb(obj, path: str, copyright_note: str = "") -> None:
    """Export one mesh object as a GLB (Y up, no UVs, active colour attribute as COLOR_0)."""
    import bpy

    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    for o in bpy.context.scene.objects:
        o.select_set(False)
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.export_scene.gltf(
        filepath=path,
        export_format="GLB",
        use_selection=True,
        export_yup=True,
        export_apply=True,
        export_texcoords=False,
        export_normals=True,
        export_tangents=False,
        export_vertex_color="ACTIVE",
        export_all_vertex_colors=False,
        export_attributes=False,
        export_materials="EXPORT",
        export_image_format="NONE",
        export_animations=False,
        export_morph=False,
        export_skins=False,
        export_cameras=False,
        export_lights=False,
        export_extras=False,
        export_copyright=copyright_note,
    )


# ---------------------------------------------------------------------------
# Pure-Python GLB helpers (no bpy, no numpy)
# ---------------------------------------------------------------------------

_GLB_MAGIC = 0x46546C67
_CHUNK_JSON = 0x4E4F534A
_CHUNK_BIN = 0x004E4942
_COMPONENT = {5120: ("b", 1), 5121: ("B", 1), 5122: ("h", 2), 5123: ("H", 2), 5125: ("I", 4), 5126: ("f", 4)}
_NCOMP = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT2": 4, "MAT3": 9, "MAT4": 16}


def read_glb(path: str) -> tuple[dict, bytes]:
    """Read a GLB file into ``(gltf_json, bin_chunk)``."""
    with open(path, "rb") as f:
        data = f.read()
    magic, version, length = struct.unpack_from("<III", data, 0)
    if magic != _GLB_MAGIC or version != 2:
        raise ValueError(f"{path}: not a glTF 2 binary")
    offset, gltf, binary = 12, None, b""
    while offset < min(length, len(data)):
        clen, ctype = struct.unpack_from("<II", data, offset)
        chunk = data[offset + 8 : offset + 8 + clen]
        if ctype == _CHUNK_JSON:
            gltf = json.loads(chunk.decode("utf-8"))
        elif ctype == _CHUNK_BIN:
            binary = bytes(chunk)
        offset += 8 + clen
    if gltf is None:
        raise ValueError(f"{path}: GLB without JSON chunk")
    return gltf, binary


def write_glb(path: str, gltf: dict, binary: bytes) -> None:
    """Write ``gltf`` + ``binary`` as a GLB (chunks padded to 4 bytes)."""
    js = json.dumps(gltf, separators=(",", ":")).encode("utf-8")
    js += b" " * ((4 - len(js) % 4) % 4)
    binary = binary + b"\0" * ((4 - len(binary) % 4) % 4)
    total = 12 + 8 + len(js) + (8 + len(binary) if binary else 0)
    with open(path, "wb") as f:
        f.write(struct.pack("<III", _GLB_MAGIC, 2, total))
        f.write(struct.pack("<II", len(js), _CHUNK_JSON))
        f.write(js)
        if binary:
            f.write(struct.pack("<II", len(binary), _CHUNK_BIN))
            f.write(binary)


def _accessor_values(gltf: dict, binary: bytes, index: int) -> list:
    """Decode accessor ``index`` into a flat list of numbers (normalisation not applied)."""
    acc = gltf["accessors"][index]
    fmt, size = _COMPONENT[acc["componentType"]]
    ncomp = _NCOMP[acc["type"]]
    count = acc["count"]
    if "bufferView" not in acc:
        return [0] * (count * ncomp)
    view = gltf["bufferViews"][acc["bufferView"]]
    base = view.get("byteOffset", 0) + acc.get("byteOffset", 0)
    stride = view.get("byteStride", size * ncomp)
    if stride == size * ncomp:
        return list(struct.unpack_from(f"<{count * ncomp}{fmt}", binary, base))
    out: list = []
    for i in range(count):
        out.extend(struct.unpack_from(f"<{ncomp}{fmt}", binary, base + i * stride))
    return out


def compact_vertex_colors(path: str) -> int:
    """Rewrite a GLB so every ``COLOR_*`` attribute is normalised ``UNSIGNED_BYTE``.

    Blender writes vertex colours as normalised ``UNSIGNED_SHORT`` (8 bytes per
    vertex); 8-bit is plenty for colour tints and halves that cost. The buffer is
    rebuilt with one tightly packed, 4-byte aligned view per accessor. Returns
    the number of bytes saved. Files without colour attributes are left as is.
    """
    gltf, binary = read_glb(path)
    color_accessors = set()
    for mesh in gltf.get("meshes", []):
        for prim in mesh.get("primitives", []):
            for name, idx in prim.get("attributes", {}).items():
                if name.startswith("COLOR_") and gltf["accessors"][idx]["componentType"] != 5121:
                    color_accessors.add(idx)
    if not color_accessors:
        return 0
    if any("sparse" in a for a in gltf["accessors"]):
        return 0  # not produced by our exports; leave untouched
    view_targets = {}
    for acc in gltf["accessors"]:
        if "bufferView" in acc:
            view_targets[acc["bufferView"]] = gltf["bufferViews"][acc["bufferView"]].get("target")
    new_views: list[dict] = []
    new_bin = bytearray()

    def push(data: bytes, target) -> int:
        while len(new_bin) % 4:
            new_bin.append(0)
        view = {"buffer": 0, "byteOffset": len(new_bin), "byteLength": len(data)}
        if target is not None:
            view["target"] = target
        new_bin.extend(data)
        new_views.append(view)
        return len(new_views) - 1

    # Non-accessor views (e.g. embedded images) are copied verbatim.
    remap_views: dict[int, int] = {}
    for vi, view in enumerate(gltf.get("bufferViews", [])):
        if vi not in view_targets:
            start = view.get("byteOffset", 0)
            remap_views[vi] = push(binary[start : start + view["byteLength"]], view.get("target"))
    for ai, acc in enumerate(gltf["accessors"]):
        if "bufferView" not in acc:
            continue
        values = _accessor_values(gltf, binary, ai)
        target = view_targets.get(acc["bufferView"])
        if ai in color_accessors:
            scale = 65535.0 if acc["componentType"] == 5123 else 1.0
            if acc["componentType"] == 5121:
                scale = 255.0
            values = [max(0, min(255, int(round(v / scale * 255.0)))) for v in values]
            acc["componentType"] = 5121
            acc["normalized"] = True
            ncomp = _NCOMP[acc["type"]]
            if ncomp == 3:  # VEC3 u8 would break 4-byte element alignment
                values = [x for i in range(0, len(values), 3) for x in (*values[i : i + 3], 255)]
                acc["type"] = "VEC4"
        fmt, _ = _COMPONENT[acc["componentType"]]
        data = struct.pack(f"<{len(values)}{fmt}", *values)
        acc["bufferView"] = push(data, target)
        acc.pop("byteOffset", None)
    for img in gltf.get("images", []):
        if "bufferView" in img:
            img["bufferView"] = remap_views[img["bufferView"]]
    old_len = len(binary)
    gltf["bufferViews"] = new_views
    gltf["buffers"] = [{"byteLength": len(new_bin)}]
    write_glb(path, gltf, bytes(new_bin))
    return old_len - len(new_bin)


def glb_summary(path: str) -> dict:
    """Triangle / vertex counts, material names and Y-up bounds of a GLB.

    Bounds are computed from the vertex data with node transforms applied
    (translation, rotation, scale and matrices, recursively).
    """
    gltf, binary = read_glb(path)
    nodes = gltf.get("nodes", [])

    def local_matrix(node: dict) -> list[list[float]]:
        if "matrix" in node:
            m = node["matrix"]
            return [[m[c * 4 + r] for c in range(4)] for r in range(4)]
        tx, ty, tz = node.get("translation", [0.0, 0.0, 0.0])
        qx, qy, qz, qw = node.get("rotation", [0.0, 0.0, 0.0, 1.0])
        sx, sy, sz = node.get("scale", [1.0, 1.0, 1.0])
        rot = [
            [1 - 2 * (qy * qy + qz * qz), 2 * (qx * qy - qz * qw), 2 * (qx * qz + qy * qw)],
            [2 * (qx * qy + qz * qw), 1 - 2 * (qx * qx + qz * qz), 2 * (qy * qz - qx * qw)],
            [2 * (qx * qz - qy * qw), 2 * (qy * qz + qx * qw), 1 - 2 * (qx * qx + qy * qy)],
        ]
        return [
            [rot[0][0] * sx, rot[0][1] * sy, rot[0][2] * sz, tx],
            [rot[1][0] * sx, rot[1][1] * sy, rot[1][2] * sz, ty],
            [rot[2][0] * sx, rot[2][1] * sy, rot[2][2] * sz, tz],
            [0.0, 0.0, 0.0, 1.0],
        ]

    def mul(a, b):
        return [[sum(a[r][k] * b[k][c] for k in range(4)) for c in range(4)] for r in range(4)]

    tris = verts = 0
    lo = [math.inf] * 3
    hi = [-math.inf] * 3
    used_materials: list[str] = []

    def visit(ni: int, parent):
        nonlocal tris, verts
        node = nodes[ni]
        world = mul(parent, local_matrix(node))
        if "mesh" in node:
            for prim in gltf["meshes"][node["mesh"]].get("primitives", []):
                if prim.get("mode", 4) != 4:
                    continue
                pos_idx = prim["attributes"]["POSITION"]
                count = gltf["accessors"][pos_idx]["count"]
                verts += count
                if "indices" in prim:
                    tris += gltf["accessors"][prim["indices"]]["count"] // 3
                else:
                    tris += count // 3
                if "material" in prim:
                    name = gltf["materials"][prim["material"]].get("name", "")
                    if name not in used_materials:
                        used_materials.append(name)
                p = _accessor_values(gltf, binary, pos_idx)
                for i in range(0, len(p), 3):
                    x, y, z = p[i], p[i + 1], p[i + 2]
                    w = [world[r][0] * x + world[r][1] * y + world[r][2] * z + world[r][3] for r in range(3)]
                    for k in range(3):
                        if w[k] < lo[k]:
                            lo[k] = w[k]
                        if w[k] > hi[k]:
                            hi[k] = w[k]
        for child in node.get("children", []):
            visit(child, world)

    identity = [[1.0 if r == c else 0.0 for c in range(4)] for r in range(4)]
    scene = gltf.get("scenes", [{}])[gltf.get("scene", 0)] if gltf.get("scenes") else {"nodes": list(range(len(nodes)))}
    for ni in scene.get("nodes", []):
        visit(ni, identity)
    if tris == 0:
        lo, hi = [0.0] * 3, [0.0] * 3
    return {
        "triangles": tris,
        "vertices": verts,
        "materials": used_materials,
        "bounds_min": [round(v, 3) for v in lo],
        "bounds_max": [round(v, 3) for v in hi],
        "bytes": os.path.getsize(path),
    }
