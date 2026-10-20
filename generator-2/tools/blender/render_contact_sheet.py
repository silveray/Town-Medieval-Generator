"""Render a contact sheet of GLB assets (round-trip check of the exports).

Every GLB is imported into a fresh, empty scene (so only what the file itself
contains is rendered), lit by a sun and a sky, framed from the front-right
(the assets' front is +X) and rendered with Cycles. The tiles are composed into
one grid image, row-major in file-name order.

    python -I tools/blender/render_contact_sheet.py -- \\
        --glb-dir assets/houses --out docs/images/french_houses_contact.jpg --cols 10

Use ``--files a.glb b.glb`` instead of ``--glb-dir`` for a subset. The material
of each slot is rendered exactly as glTF defines it: base colour x COLOR_0.
"""

from __future__ import annotations

import argparse
import glob
import math
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import bpy  # noqa: E402
import mathutils  # noqa: E402
import mde_blender_common as common  # noqa: E402
import numpy as np  # noqa: E402


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Render a grid of GLB previews.")
    src = p.add_mutually_exclusive_group(required=True)
    src.add_argument("--glb-dir", help="directory whose *.glb files are rendered (recursively)")
    src.add_argument("--files", nargs="+", help="explicit GLB files")
    p.add_argument("--out", required=True, help="output image (.jpg or .png)")
    p.add_argument("--cols", type=int, default=10, help="tiles per row (default 10)")
    p.add_argument("--tile", type=int, default=240, help="tile size in pixels (default 240)")
    p.add_argument("--samples", type=int, default=24, help="Cycles samples per tile (default 24)")
    p.add_argument("--azimuth", type=float, default=38.0, help="camera azimuth from +X towards +Y, degrees")
    p.add_argument("--elevation", type=float, default=20.0, help="camera elevation, degrees")
    p.add_argument("--quality", type=int, default=82, help="JPEG quality (default 82)")
    p.add_argument("--ground", action="store_true", help="add a ground plane under each asset")
    p.add_argument("--part", action="store_true", help="part mode: front is +Z (glTF), i.e. -Y in Blender")
    return p.parse_args(common.script_args())


def setup_scene(tile: int, samples: int):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = samples
    scene.cycles.use_denoising = True
    scene.render.resolution_x = tile
    scene.render.resolution_y = tile
    scene.render.film_transparent = False
    scene.render.image_settings.file_format = "PNG"
    try:
        scene.view_settings.view_transform = "AgX"
        scene.view_settings.look = "AgX - Base Contrast"
    except TypeError:
        scene.view_settings.view_transform = "Standard"
    world = bpy.data.worlds.new("sky")
    scene.world = world
    if world.node_tree is None:
        world.use_nodes = True
    bg = world.node_tree.nodes["Background"]
    bg.inputs["Color"].default_value = (0.55, 0.62, 0.72, 1.0)
    bg.inputs["Strength"].default_value = 0.9
    sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN"))
    sun.data.energy = 3.2
    sun.data.angle = math.radians(4.0)
    sun.rotation_euler = (math.radians(50.0), math.radians(8.0), math.radians(25.0))
    scene.collection.objects.link(sun)
    cam = bpy.data.objects.new("camera", bpy.data.cameras.new("camera"))
    cam.data.lens = 50.0
    scene.collection.objects.link(cam)
    scene.camera = cam
    return scene, cam


def adapt_materials(objects) -> None:
    """Make imported materials render the way a glTF engine (e.g. Bevy) shows them.

    * base colour x ``COLOR_0`` (the glTF rule; Blender's importer leaves the
      colour attribute unused),
    * single-sided materials are see-through from behind (back-face culling,
      which Cycles does not do by itself), so missing faces would show up.
    """
    for obj in objects:
        if obj.type != "MESH":
            continue
        layer = obj.data.color_attributes[0].name if obj.data.color_attributes else None
        for slot in obj.material_slots:
            mat = slot.material
            if mat is None or mat.get("mde_adapted"):
                continue
            mat["mde_adapted"] = True
            nt = mat.node_tree
            bsdf = next((n for n in nt.nodes if n.type == "BSDF_PRINCIPLED"), None)
            if bsdf is None:
                continue
            base = bsdf.inputs["Base Color"]
            uses_vc = any(link.from_node.type in ("VERTEX_COLOR", "ATTRIBUTE") for link in base.links)
            if layer is not None and not uses_vc:
                vc = nt.nodes.new("ShaderNodeVertexColor")
                vc.layer_name = layer
                mix = nt.nodes.new("ShaderNodeMix")
                mix.data_type = "RGBA"
                mix.blend_type = "MULTIPLY"
                mix.inputs["Factor"].default_value = 1.0
                if base.links:
                    nt.links.new(base.links[0].from_socket, mix.inputs["A"])
                else:
                    mix.inputs["A"].default_value = base.default_value[:]
                nt.links.new(vc.outputs["Color"], mix.inputs["B"])
                nt.links.new(mix.outputs["Result"], base)
            if mat.use_backface_culling:
                out = next((n for n in nt.nodes if n.type == "OUTPUT_MATERIAL"), None)
                if out is None or not out.inputs["Surface"].links:
                    continue
                shader = out.inputs["Surface"].links[0].from_socket
                geo = nt.nodes.new("ShaderNodeNewGeometry")
                clear = nt.nodes.new("ShaderNodeBsdfTransparent")
                cull = nt.nodes.new("ShaderNodeMixShader")
                nt.links.new(geo.outputs["Backfacing"], cull.inputs["Fac"])
                nt.links.new(shader, cull.inputs[1])
                nt.links.new(clear.outputs[0], cull.inputs[2])
                nt.links.new(cull.outputs[0], out.inputs["Surface"])


def frame(cam, objects, azimuth: float, elevation: float) -> None:
    """Point the camera at the objects' bounding sphere so it fills the frame."""
    pts = []
    for obj in objects:
        if obj.type == "MESH":
            pts += [obj.matrix_world @ mathutils.Vector(c) for c in obj.bound_box]
    lo = mathutils.Vector([min(p[i] for p in pts) for i in range(3)])
    hi = mathutils.Vector([max(p[i] for p in pts) for i in range(3)])
    centre = (lo + hi) / 2.0
    radius = max((hi - lo).length / 2.0, 0.05)
    fov = 2.0 * math.atan(cam.data.sensor_width / (2.0 * cam.data.lens))
    dist = radius / math.sin(fov / 2.0) * 0.92
    az, el = math.radians(azimuth), math.radians(elevation)
    direction = mathutils.Vector((math.cos(az) * math.cos(el), math.sin(az) * math.cos(el), math.sin(el)))
    cam.location = centre + direction * dist
    cam.rotation_euler = (centre - cam.location).to_track_quat("-Z", "Y").to_euler()
    cam.data.clip_start = max(dist / 1000.0, 0.001)
    cam.data.clip_end = dist * 10.0


def render_tile(path: str, scene, cam, args, tmp_png: str) -> np.ndarray:
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=path)
    objects = [o for o in bpy.data.objects if o not in before]
    if args.part:
        # Parts face glTF +Z = Blender -Y; turn them to face the house camera (+X).
        for o in objects:
            if o.parent is None:
                o.rotation_mode = "XYZ"
                o.rotation_euler.z += math.radians(90.0)
        bpy.context.view_layer.update()
    adapt_materials(objects)
    ground = None
    if args.ground:
        lo_z = min((o.matrix_world @ mathutils.Vector(c)).z for o in objects if o.type == "MESH" for c in o.bound_box)
        bpy.ops.mesh.primitive_plane_add(size=200.0, location=(0.0, 0.0, min(lo_z, 0.0) + 0.001))
        ground = bpy.context.active_object
        mat = bpy.data.materials.get("ground") or bpy.data.materials.new("ground")
        mat.diffuse_color = (0.30, 0.33, 0.24, 1.0)
        if mat.node_tree is None:
            mat.use_nodes = True
        bsdf = next(n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
        bsdf.inputs["Base Color"].default_value = (0.16, 0.19, 0.11, 1.0)
        bsdf.inputs["Roughness"].default_value = 1.0
        ground.data.materials.append(mat)
    frame(cam, objects, args.azimuth, args.elevation)
    scene.render.filepath = tmp_png
    bpy.ops.render.render(write_still=True)
    img = bpy.data.images.load(tmp_png, check_existing=False)
    px = np.array(img.pixels[:], dtype=np.float32).reshape(img.size[1], img.size[0], 4)
    bpy.data.images.remove(img)
    for o in objects + ([ground] if ground else []):
        data = o.data
        bpy.data.objects.remove(o)
        if data is not None and data.users == 0 and isinstance(data, bpy.types.Mesh):
            bpy.data.meshes.remove(data)
    for m in list(bpy.data.materials):
        if m.users == 0:
            bpy.data.materials.remove(m)
    return px


def main() -> None:
    args = parse_args()
    files = args.files or sorted(glob.glob(os.path.join(args.glb_dir, "**", "*.glb"), recursive=True))
    if not files:
        raise SystemExit("no GLB files found")
    scene, cam = setup_scene(args.tile, args.samples)
    cols = max(1, min(args.cols, len(files)))
    rows = math.ceil(len(files) / cols)
    sheet = np.ones((rows * args.tile, cols * args.tile, 4), dtype=np.float32)
    with tempfile.TemporaryDirectory() as tmp:
        tmp_png = os.path.join(tmp, "tile.png")
        for i, path in enumerate(files):
            px = render_tile(os.path.abspath(path), scene, cam, args, tmp_png)
            r, c = divmod(i, cols)
            # Blender images are stored bottom-up; the sheet is filled bottom-up too.
            y0 = (rows - 1 - r) * args.tile
            sheet[y0 : y0 + args.tile, c * args.tile : (c + 1) * args.tile] = px
            print(f"[{i + 1}/{len(files)}] {os.path.basename(path)}", flush=True)
    # Tiles were read back from display-referred 8-bit PNGs: save the bytes as-is.
    out = bpy.data.images.new("sheet", cols * args.tile, rows * args.tile, alpha=False)
    out.colorspace_settings.name = "sRGB"
    out.pixels.foreach_set(sheet.ravel())
    out_path = os.path.abspath(args.out)
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    out.file_format = "JPEG" if out_path.lower().endswith((".jpg", ".jpeg")) else "PNG"
    out.save(filepath=out_path, quality=args.quality)
    print(f"wrote {out_path} ({os.path.getsize(out_path) / 1e3:.0f} KB, {cols}x{rows} tiles)")


if __name__ == "__main__":
    main()
