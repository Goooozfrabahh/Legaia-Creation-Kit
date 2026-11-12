#!/usr/bin/env python3
"""Read-only native grid audit. Does not patch a game or run an emulator.

Uses exact retail image fingerprints, historical allocator free lists, and
saved LCK ground requests. Pointer-load candidates are NOT a complete dataflow
analysis: offsets can belong to other structures and pointers can be aliased.
Output is evidence for integration, never a native-export success receipt.
"""
import argparse
import hashlib
import json
import math
import struct
from pathlib import Path

PROFILES = {
    "scus": "292256e2e66db42727f613406785e444254d3f699569e611f65fcf1c6d2f3482",
    "overlay": "0b39d892d783ffcc89dac06ee395540be0a7392c5cbc79fb9ba3442daccc09f6",
}


def image(path, kind, base, offset):
    raw = path.read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    if digest != PROFILES[kind]:
        raise ValueError(f"{kind} does not match the audited retail profile: {digest}")
    data = raw[offset:]
    candidates = []
    for at in range(0, len(data) - 3, 4):
        word = struct.unpack_from("<I", data, at)[0]
        if word >> 26 in (35, 43) and word & 0xffff in (0x3ec, 0xd8):
            candidates.append({"address": f"0x{base + at:08x}", "word": f"0x{word:08x}"})
    return {"path": str(path), "sha256": digest, "bytes": len(raw),
            "map_pointer_offset_candidates_not_exhaustive": candidates}


def heap(path):
    ram = path.read_bytes()
    if len(ram) != 0x200000:
        raise ValueError("Expected 2 MiB historical RAM")

    def word(address):
        if not 0x80000000 <= address <= 0x801ffffc or address & 3:
            raise ValueError(f"Invalid allocator pointer {address:#x}")
        return struct.unpack_from("<I", ram, address - 0x80000000)[0]

    descriptor = word(0x8007bb58)
    top = word(descriptor)
    sentinel = top + 12
    cursor = word(top + 16)
    seen = set()
    blocks = []
    while cursor != sentinel:
        if cursor in seen or len(seen) >= 4096:
            raise ValueError("Cyclic or oversized free list")
        seen.add(cursor)
        size = word(cursor + 8)
        if cursor + size > 0x80200000:
            raise ValueError("Free block extends beyond retail RAM")
        blocks.append({"address": f"0x{cursor:08x}", "bytes": size})
        cursor = word(cursor + 4)
    return {"path": str(path), "sha256": hashlib.sha256(ram).hexdigest(),
            "historical_only": True, "free_bytes": sum(b["bytes"] for b in blocks),
            "largest_free_block": max((b["bytes"] for b in blocks), default=0),
            "blocks": blocks}


def project(path):
    raw = path.read_bytes()
    data = json.loads(raw.decode("utf-8-sig"))
    result = []
    for name, scene in data.get("cell_snapshots", {}).items():
        for line in scene.splitlines():
            if not line.startswith("GROUND_EDIT\t"):
                continue
            ground = json.loads(line.split("\t", 1)[1])
            deleted = {tuple(v) for v in ground.get("deleted", [])}
            requests = [(s, t) for s, t in ground.get("originals", []) if tuple(s) not in deleted]
            requests += [(c["source"], c["transform"]) for c in ground.get("copies", [])]
            cells, pages = set(), set()
            clipped = invalid = 0
            for source, t in requests:
                sx, sz = abs(t["scale"][0]), abs(t["scale"][2])
                if not all(math.isfinite(v) for v in [sx, sz, *t["offset"]]) or min(sx, sz) <= 0:
                    invalid += 1
                    continue
                if t["turns"] % 2:
                    sx, sz = sz, sx
                cx = source[0] * 128 + 64 + t["offset"][0]
                cz = source[1] * 128 + 64 + t["offset"][2]
                x0, x1 = math.floor((cx - 64 * sx) / 128), math.ceil((cx + 64 * sx) / 128)
                z0, z1 = math.floor((cz - 64 * sz) / 128), math.ceil((cz + 64 * sz) / 128)
                clipped += int(x0 < -64 or z0 < -64 or x1 > 192 or z1 > 192)
                for z in range(max(-64, z0), min(192, z1)):
                    for x in range(max(-64, x0), min(192, x1)):
                        if 0 <= x < 128 and 0 <= z < 128:
                            continue
                        cells.add((x, z))
                        pages.add(((x + 64) // 16, (z + 64) // 16))
            result.append({"scene": name, "ground_requests": len(requests), "external_grid_tiles": len(cells),
                           "external_pages": len(pages), "sparse_grid_bytes": 528 + len(pages) * 768,
                           "requests_crossing_256_bounds": clipped, "invalid_requests": invalid,
                           "excludes": ["geometry", "textures", "runtime code", "allocator overhead", "new descriptors", "slope data"]})
    return {"path": str(path), "sha256": hashlib.sha256(raw).hexdigest(), "cells": result}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--scus", type=Path, required=True)
    p.add_argument("--overlay", type=Path, required=True)
    p.add_argument("--ram", type=Path, action="append", default=[])
    p.add_argument("--project", type=Path)
    p.add_argument("--output", type=Path, required=True)
    args = p.parse_args()
    report = {
        "format": "LCK_NATIVE_GRID_AUDIT", "version": 1,
        "runtime_patch_installed": False, "gameplay_verified": False,
        "scus": image(args.scus, "scus", 0x80010000, 0x800),
        "field_overlay": image(args.overlay, "overlay", 0x801ce818, 0),
        "historical_heaps": [heap(path) for path in args.ram],
        "dense_256_extra_bytes": 0x38000 - 0x14000,
        "integration_required": [
            "per-scene payload selection, allocation, loading, freeing and reload after battle",
            "all native grid readers and MAN grid writers, with unchanged legacy-scene behavior",
            "height interpolation and wall half-tile quadrant/bias semantics",
            "object creation plus region-window visibility and camera boundaries",
            "floor geometry/textures, shared descriptor capacity, and resource budgets",
            "trigger/warp/region signed coordinates without byte overflow",
            "existing Stage E and extended-floor hook composition",
            "native runtime and user gameplay verification",
        ],
    }
    if args.project:
        report["project"] = project(args.project)
    # Evidence files are new artifacts; do not replace an earlier audit.
    with args.output.open("x", encoding="utf-8") as out:
        json.dump(report, out, indent=2)
        out.write("\n")
    print(json.dumps({"output": str(args.output), "historical_heaps": report["historical_heaps"],
                      "project": report.get("project")}, indent=2))


if __name__ == "__main__":
    main()
