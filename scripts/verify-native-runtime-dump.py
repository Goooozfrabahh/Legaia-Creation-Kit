"""Read a USER-SUPPLIED 2 MiB PS1 RAM dump; never start/connect to an emulator.

Compare live DAT_8007C018 pointers to object-verification.json model witnesses.
Original FUN_800268DC changes TMD flag 0 -> 1 and three relative pointers per
object to absolute pointers. Reverse only those documented fixups before hashing.
This verifies sampled live registry identity, never rendering or interaction.
"""
import argparse
import hashlib
import json
import struct
from pathlib import Path


def inspect(ram, report):
    if len(ram) != 0x200000:
        raise ValueError("Expected exactly 2 MiB raw main RAM; no savestate/header/compression")

    def offset(va, size=4):
        if not 0x80000000 <= va <= 0x80200000 - size:
            raise ValueError(f"Pointer {va:#x} + {size:#x} outside main RAM")
        return va - 0x80000000

    def word(va):
        return struct.unpack_from("<I", ram, offset(va))[0]

    scene = ram[offset(0x80084548):offset(0x80084548) + 8].split(b"\0")[0].decode("ascii", "replace")
    bank = word(0x8007B6F8)
    counter = word(0x8007B774)
    rows = []
    for obj in report["objects"]:
        row = {"uid": obj["uid"], "render_visibility_verified": False,
               "collision_interaction_verified": False, "native_loader_identity_verified": False}
        try:
            witness = obj["native_loader_identity"]
            selector = witness["scene_selector"]
            slot = bank + selector
            if slot >= counter or slot >= 200:
                raise ValueError(f"Live slot {slot} outside registered count {counter} / known table conflict")
            pointer = word(0x8007C018 + slot * 4)
            size = witness["relocated_tmd_bytes"]
            start = offset(pointer, size)
            raw = bytearray(ram[start:start + size])
            magic, flags, count = struct.unpack_from("<III", raw)
            if magic != 0x80000002 or flags not in (0, 1) or 12 + count * 28 > size:
                raise ValueError("Live pointer is not a supported TMD header")
            if flags == 1:
                struct.pack_into("<I", raw, 4, 0)
                for group in range(count):
                    for field in (0, 8, 16):
                        at = 12 + group * 28 + field
                        absolute = struct.unpack_from("<I", raw, at)[0]
                        relative = absolute - pointer - 12
                        if not 0 <= relative < size:
                            raise ValueError("Live group pointer outside the expected TMD")
                        struct.pack_into("<I", raw, at, relative)
            actual = hashlib.sha256(raw).hexdigest()
            row.update(global_slot=slot, pointer=hex(pointer), actual_normalized_sha256=actual,
                       expected_sha256=witness["relocated_tmd_sha256"],
                       native_loader_identity_verified=actual == witness["relocated_tmd_sha256"])
            if not row["native_loader_identity_verified"]:
                row["gap"] = "Live bytes differ: wrong resource, mutation, stale capture, or corruption; not render proof"
        except (KeyError, ValueError, struct.error) as error:
            row["gap"] = str(error)
        rows.append(row)
    return {"schema": 1, "scene_in_dump": scene, "scene_bank_base": bank,
            "global_model_count": counter, "ram_sha256": hashlib.sha256(ram).hexdigest(),
            "evidence_class": "user-supplied live main-RAM snapshot; sampled registry identity only",
            "objects": rows, "render_verified": False, "collision_interaction_verified": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("ram", type=Path)
    parser.add_argument("verification", type=Path)
    parser.add_argument("output", type=Path, help="NEW JSON path; never overwrites")
    args = parser.parse_args()
    result = inspect(args.ram.read_bytes(), json.loads(args.verification.read_text(encoding="utf-8")))
    with args.output.open("x", encoding="utf-8") as file:
        json.dump(result, file, indent=2)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
