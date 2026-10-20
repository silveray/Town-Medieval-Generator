"""Validate ``assets/houses`` and ``assets/parts`` against their manifests.

Plain Python 3 (no Blender needed), so it can run in CI:

    python3 tools/blender/validate_assets.py            # repo root as cwd
    python3 tools/blender/validate_assets.py --root /path/to/repo

Checks, per manifest entry: the GLB exists and parses, its triangle count and
bounds match the manifest, every material name is a canonical
``mde_core::MaterialSlot`` name, houses stand on the ground with their footprint
centred and the door on the +X front, window / door parts have their opening
centred on the origin, and the size / triangle budgets hold. Also lists GLBs
dropped into the part folders without a manifest entry (allowed: the viewer
auto-scans them) and reports their bounds. Exit code 1 on any error.
"""

from __future__ import annotations

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import mde_blender_common as common  # noqa: E402

#: Every ``mde_core::MaterialSlot`` variant name.
MATERIAL_SLOTS = {
    "Plaster",
    "Timber",
    "Stone",
    "RoofTile",
    "Thatch",
    "Slate",
    "Planks",
    "WoodFloor",
    "Door",
    "WindowFrame",
    "Iron",
    "Glass",
    "Ground",
    "Grass",
    "Street",
    "Field",
    "Water",
    "Foliage",
    "Bark",
}
HOUSE_KEYS = ("file", "seed", "floors", "footprint", "bounds_min", "bounds_max", "height", "triangles", "front", "door")
PART_KEYS = ("id", "category", "file", "width", "height", "depth", "triangles", "source")
HOUSES_MAX_BYTES = 45_000_000
HARD_MAX_TRIS = 60_000
TOL = 0.02


class Report:
    """Collects errors and warnings."""

    def __init__(self) -> None:
        self.errors: list[str] = []
        self.warnings: list[str] = []

    def error(self, msg: str) -> None:
        self.errors.append(msg)

    def warn(self, msg: str) -> None:
        self.warnings.append(msg)


def _close(a, b, tol: float = TOL) -> bool:
    return all(abs(x - y) <= tol for x, y in zip(a, b, strict=True))


def check_glb(path: str, entry: dict, label: str, rep: Report) -> dict | None:
    if not os.path.isfile(path):
        rep.error(f"{label}: missing file {path}")
        return None
    try:
        s = common.glb_summary(path)
    except (ValueError, KeyError, IndexError, OSError) as exc:
        rep.error(f"{label}: unreadable GLB ({exc})")
        return None
    if entry.get("triangles") != s["triangles"]:
        rep.error(f"{label}: manifest says {entry.get('triangles')} triangles, file has {s['triangles']}")
    if s["triangles"] > HARD_MAX_TRIS:
        rep.error(f"{label}: {s['triangles']} triangles exceeds the hard budget {HARD_MAX_TRIS}")
    bad = [m for m in s["materials"] if m not in MATERIAL_SLOTS]
    if bad:
        rep.error(f"{label}: non-canonical material names {bad}")
    if "bounds_min" in entry and not _close(entry["bounds_min"], s["bounds_min"]):
        rep.error(f"{label}: bounds_min {entry['bounds_min']} != file {s['bounds_min']}")
    if "bounds_max" in entry and not _close(entry["bounds_max"], s["bounds_max"]):
        rep.error(f"{label}: bounds_max {entry['bounds_max']} != file {s['bounds_max']}")
    return s


def check_houses(root: str, rep: Report) -> None:
    folder = os.path.join(root, "assets", "houses")
    path = os.path.join(folder, "manifest.json")
    if not os.path.isfile(path):
        rep.error("assets/houses/manifest.json missing")
        return
    with open(path) as f:
        entries = json.load(f)
    total = 0
    for e in entries:
        label = f"houses/{e.get('file')}"
        missing = [k for k in HOUSE_KEYS if k not in e]
        if missing:
            rep.error(f"{label}: missing keys {missing}")
            continue
        s = check_glb(os.path.join(folder, e["file"]), e, label, rep)
        if s is None:
            continue
        total += s["bytes"]
        fx, fz = e["footprint"]
        lo, hi = s["bounds_min"], s["bounds_max"]
        if not -0.5 <= lo[1] <= 0.05:
            rep.error(f"{label}: lowest point y={lo[1]} (ground must be y=0)")
        if abs(e["height"] - hi[1]) > TOL:
            rep.error(f"{label}: height {e['height']} != bounds_max.y {hi[1]}")
        if e["front"] != "+X" or e.get("door_facing", "+X") != "+X":
            rep.error(f"{label}: front / door must face +X")
        dx, dy, dz = e["door"]
        if not (fx / 2 - 1.6 <= dx <= fx / 2 + 0.3 and 0.3 <= dy <= 2.0 and abs(dz) <= fz / 2 + TOL):
            rep.error(f"{label}: door {e['door']} is not on the +X front of footprint {e['footprint']}")
        if not (lo[0] <= -fx / 2 + TOL and hi[0] >= fx / 2 - TOL and lo[2] <= -fz / 2 + TOL and hi[2] >= fz / 2 - TOL):
            rep.error(f"{label}: footprint {e['footprint']} not centred inside bounds {lo}..{hi}")
    files = {f for f in os.listdir(folder) if f.endswith(".glb")}
    listed = {e.get("file") for e in entries}
    for extra in sorted(files - listed):
        rep.warn(f"houses/{extra}: GLB not in manifest")
    if total > HOUSES_MAX_BYTES:
        rep.error(f"assets/houses: {total / 1e6:.1f} MB exceeds {HOUSES_MAX_BYTES / 1e6:.0f} MB")
    print(f"houses: {len(entries)} entries, {total / 1e6:.1f} MB")


def check_parts(root: str, rep: Report) -> None:
    folder = os.path.join(root, "assets", "parts")
    path = os.path.join(folder, "manifest.json")
    if not os.path.isfile(path):
        rep.error("assets/parts/manifest.json missing")
        return
    with open(path) as f:
        manifest = json.load(f)
    if manifest.get("schema") != 1:
        rep.error(f"assets/parts/manifest.json: schema {manifest.get('schema')} (expected 1)")
    entries = manifest.get("parts", [])
    seen: set[str] = set()
    for e in entries:
        label = f"parts/{e.get('id')}"
        missing = [k for k in PART_KEYS if k not in e]
        if missing:
            rep.error(f"{label}: missing keys {missing}")
            continue
        if e["id"] in seen:
            rep.error(f"{label}: duplicate id")
        seen.add(e["id"])
        if e["category"] not in ("window", "door", "landmark"):
            rep.error(f"{label}: unknown category {e['category']}")
        if not e["id"].startswith(e["category"] + "/") or not e["file"].startswith(e["category"] + "/"):
            rep.error(f"{label}: id and file must live under '{e['category']}/'")
        s = check_glb(os.path.join(folder, e["file"]), e, label, rep)
        if s is None or e["category"] == "landmark":
            continue
        lo, hi = s["bounds_min"], s["bounds_max"]
        # The opening is centred: the part spans at least the opening around the origin.
        if lo[0] > -e["width"] / 2 + TOL or hi[0] < e["width"] / 2 - TOL:
            rep.error(f"{label}: x bounds {lo[0]}..{hi[0]} do not cover the centred opening width {e['width']}")
        if lo[1] > -e["height"] / 2 + TOL or hi[1] < e["height"] / 2 - TOL:
            rep.error(f"{label}: y bounds {lo[1]}..{hi[1]} do not cover the centred opening height {e['height']}")
        if e.get("mount") == "surface" and not -0.1 <= lo[2] <= 0.0:
            rep.error(f"{label}: surface part should start at the wall (z_min {lo[2]})")
        if abs((hi[2] - lo[2]) - e["depth"]) > TOL:
            rep.error(f"{label}: depth {e['depth']} != z extent {hi[2] - lo[2]:.3f}")
    listed = {e.get("file") for e in entries}
    for cat in ("window", "door", "landmark"):
        sub = os.path.join(folder, cat)
        if not os.path.isdir(sub):
            continue
        for name in sorted(os.listdir(sub)):
            rel = f"{cat}/{name}"
            if name.endswith(".glb") and rel not in listed:
                s = common.glb_summary(os.path.join(sub, name))
                rep.warn(f"parts/{rel}: not in manifest (auto-scanned; bounds {s['bounds_min']}..{s['bounds_max']})")
    print(f"parts: {len(entries)} entries")


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--root", default=".", help="repository root (default: cwd)")
    args = p.parse_args()
    rep = Report()
    check_houses(os.path.abspath(args.root), rep)
    check_parts(os.path.abspath(args.root), rep)
    for w in rep.warnings:
        print(f"warning: {w}")
    for e in rep.errors:
        print(f"error: {e}")
    print("OK" if not rep.errors else f"{len(rep.errors)} error(s)")
    return 1 if rep.errors else 0


if __name__ == "__main__":
    sys.exit(main())
