import hashlib
import importlib.util
from pathlib import Path
import struct
import unittest

spec = importlib.util.spec_from_file_location("runtime_dump", Path(__file__).resolve().parents[1] / "verify-native-runtime-dump.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class RuntimeEvidenceTests(unittest.TestCase):
    def fixture(self):
        ram = bytearray(0x200000)
        pointer = 0x80100000
        model = struct.pack("<10I", 0x80000002, 0, 1, 28, 0, 28, 0, 28, 0, 0)
        normalized_hash = hashlib.sha256(model).hexdigest()
        live = bytearray(model)
        struct.pack_into("<I", live, 4, 1)
        for at in (12, 20, 28):
            struct.pack_into("<I", live, at, pointer + 40)
        ram[0x100000:0x100028] = live
        struct.pack_into("<I", ram, 0x7C018 + 119 * 4, pointer)
        struct.pack_into("<I", ram, 0x7B6F8, 5)
        struct.pack_into("<I", ram, 0x7B774, 122)
        ram[0x84548:0x8454F] = b"town01\0"
        report = {"objects": [{"uid": 1, "native_loader_identity": {"scene_selector": 114, "relocated_tmd_bytes": 40, "relocated_tmd_sha256": normalized_hash}}]}
        return ram, report

    def test_original_pointer_fixups_normalize_without_claiming_rendering(self):
        ram, report = self.fixture()
        result = module.inspect(ram, report)
        self.assertTrue(result["objects"][0]["native_loader_identity_verified"])
        self.assertFalse(result["render_verified"])
        self.assertFalse(result["collision_interaction_verified"])

    def test_unregistered_or_corrupted_live_resource_never_verifies(self):
        ram, report = self.fixture()
        ram[0x100024] = 1
        self.assertFalse(module.inspect(ram, report)["objects"][0]["native_loader_identity_verified"])
        struct.pack_into("<I", ram, 0x7B774, 119)
        self.assertFalse(module.inspect(ram, report)["objects"][0]["native_loader_identity_verified"])


if __name__ == "__main__":
    unittest.main()
