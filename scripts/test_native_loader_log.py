import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("loader_log", Path(__file__).with_name("analyze-native-loader-log.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class LoaderLogTests(unittest.TestCase):
    def test_freeze_inside_gpu_retains_both_pending_operations(self):
        result = module.analyze("prefix LCK LOAD BEGIN 00000100 80091800 err=00000000\n"
                                "LCK GPU BEGIN 00000008 800a0000 err=00000001\n")
        self.assertEqual([e["kind"] for e in result["unfinished_operations"]], ["LOAD", "GPU"])
        self.assertEqual(result["events"][0]["declared_bytes"], 256)
        self.assertFalse(result["root_cause_proven"])

    def test_complete_nested_calls_are_paired_without_claiming_gameplay(self):
        result = module.analyze("LCK LOAD BEGIN 00000100 80091800 err=00000000\n"
                                "LCK GPU BEGIN 00000008 800a0000 err=00000000\n"
                                "LCK GPU DONE 00000000 00000000 err=00000000\n"
                                "LCK LOAD DONE 00000100 00000000 err=00000000\n")
        self.assertEqual(result["unfinished_operations"], [])
        self.assertEqual(result["anomalies"], [])
        self.assertTrue(result["execution_observed"])
        self.assertFalse(result["successful_scene_load_verified"])

    def test_missing_logging_is_not_success(self):
        result = module.analyze("unrelated emulator message")
        self.assertFalse(result["execution_observed"])
        self.assertIsNone(result["last_observed_event"])


if __name__ == "__main__":
    unittest.main()
