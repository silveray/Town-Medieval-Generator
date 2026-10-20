"""Preview window / door parts set into a test wall, exactly as the manifest says.

For every ``window`` / ``door`` entry of ``assets/parts/manifest.json`` a hole is
cut into a plastered test wall and the part is imported at the hole's centre:

* ``mount: "flush"`` parts get a ``width x height`` hole and sit in it,
* ``mount: "surface"`` parts stand on the wall face (a hole is still cut, as
  a viewer would do for a walk-through door).

Doors stand on the ground, windows are centred 1.6 m up. If a part is off
centre, too big for its hole, or faces the wrong way, it shows here. Useful to
check user-made parts before adding them to the catalogue:

    python -I tools/blender/preview_parts_wall.py -- \\
        --manifest assets/parts/manifest.json --out out/parts_wall.png
"""

from __future__ import annotations

import argparse
import json
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import bpy  # noqa: E402
import mathutils  # noqa: E402
import mde_blender_common as common  # noqa: E402
import render_contact_sheet as sheet  # noqa: E402

WALL_THICKNESS = 0.3
WINDOW_CENTRE = 1.6
GAP = 0.6


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Render catalogue parts set into a test wall.")
    p.add_argument("--manifest", default="assets/parts/manifest.json")
    p.add_argument("--out", required=True, help="output image (.png / .jpg)")
    p.add_argument("--samples", type=int, default=24)
    p.add_argument("--width", type=int, default=2400, help="image width in pixels")
    p.add_argument("--quality", type=int, default=85, help="JPEG quality")
    return p.parse_args(common.script_args())


def _box(name: str, lo, hi):
    me = bpy.data.meshes.new(name)
    (x0, y0, z0), (x1, y1, z1) = lo, hi
    verts = [(x, y, z) for x in (x0, x1) for y in (y0, y1) for z in (z0, z1)]
    faces = [(0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6), (0, 2, 6, 4), (1, 5, 7, 3)]
    me.from_pydata(verts, [], faces)
    me.update()
    obj = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(obj)
    return obj


def _flat_material(name: str, srgb) -> bpy.types.Material:
    mat = bpy.data.materials.new(name)
    if mat.node_tree is None:
        mat.use_nodes = True
    bsdf = next(n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    bsdf.inputs["Base Color"].default_value = common.srgba_to_linear(srgb)
    bsdf.inputs["Roughness"].default_value = 0.95
    return mat


def main() -> None:
    args = parse_args()
    manifest_path = os.path.abspath(args.manifest)
    with open(manifest_path) as f:
        parts = [p for p in json.load(f)["parts"] if p["category"] in ("window", "door")]
    if not parts:
        raise SystemExit("no window / door parts in the manifest")
    root = os.path.dirname(manifest_path)
    scene, cam = sheet.setup_scene(args.width, args.samples)

    # Lay the parts out left to right.
    slots, x = [], 0.0
    for p in parts:
        lo, hi = p.get("bounds_min", [-p["width"] / 2, 0, 0]), p.get("bounds_max", [p["width"] / 2, 0, 0])
        extent = max(p["width"], hi[0] - lo[0])
        cx = x + extent / 2.0
        cz = p["height"] / 2.0 if p["category"] == "door" else WINDOW_CENTRE
        slots.append((p, cx, cz))
        x += extent + GAP
    total = x - GAP
    top = max(3.4, max(cz + p["height"] / 2 + 1.0 for p, _, cz in slots))

    wall = _box("wall", (-0.8, 0.0, 0.0), (total + 0.8, WALL_THICKNESS, top))
    wall.data.materials.append(_flat_material("wall", (0.86, 0.82, 0.73, 1.0)))
    for p, cx, cz in slots:
        cutter = _box(
            f"hole_{p['id']}",
            (cx - p["width"] / 2, -1.0, cz - p["height"] / 2),
            (cx + p["width"] / 2, 1.0, cz + p["height"] / 2),
        )
        mod = wall.modifiers.new(f"cut_{p['id']}", "BOOLEAN")
        mod.operation = "DIFFERENCE"
        mod.object = cutter
        cutter.hide_render = True
    ground = _box("ground", (-50.0, -50.0, -0.1), (total + 50.0, 50.0, 0.0))
    ground.data.materials.append(_flat_material("ground", (0.33, 0.35, 0.27, 1.0)))

    for p, cx, cz in slots:
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=os.path.join(root, p["file"]))
        new = [o for o in bpy.data.objects if o not in before]
        sheet.adapt_materials(new)
        for o in new:
            if o.parent is None:
                o.location = (cx, 0.0, cz)
        print(f"placed {p['id']} ({p['width']} x {p['height']} m, {p.get('mount', '?')})")

    # Camera: in front of the wall (-Y), slightly to the right and above.
    centre = mathutils.Vector((total / 2.0, 0.0, top / 2.0))
    aspect = 0.36
    scene.render.resolution_y = int(args.width * aspect)
    cam.data.lens = 50.0
    fov = 2.0 * math.atan(cam.data.sensor_width / (2.0 * cam.data.lens))
    dist = (total + 1.6) / 2.0 / math.tan(fov / 2.0) * 1.08
    direction = mathutils.Vector((0.1, -1.0, 0.16)).normalized()
    cam.location = centre + direction * dist
    cam.rotation_euler = (centre - cam.location).to_track_quat("-Z", "Y").to_euler()
    cam.data.clip_end = dist * 10.0
    out = os.path.abspath(args.out)
    os.makedirs(os.path.dirname(out), exist_ok=True)
    jpeg = out.lower().endswith((".jpg", ".jpeg"))
    scene.render.image_settings.file_format = "JPEG" if jpeg else "PNG"
    if jpeg:
        scene.render.image_settings.quality = args.quality
    scene.render.filepath = out
    bpy.ops.render.render(write_still=True)
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
