import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("loader_log", Path(__file__).with_name("analyze-native-loader-log.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class LoaderLogTests(unittest.TestCase):
    def test_heap_trace_attributes_failure_without_claiming_available_memory(self):
        result = module.analyze("LCK HEAP 00001000 80100000 RA=80020008\n"
                                "LCK HEAP 000020b0 00000000 RA=80021008\n"
                                "LCK FREE 80100000 00000000 RA=80022008\n")
        failure = result["failed_heap_requests"][0]
        self.assertEqual(failure["requested_bytes"], 8368)
        self.assertEqual(failure["call_site"], 0x80021000)
        self.assertEqual(failure["observed_outstanding_requested_bytes"], 4096)
        self.assertEqual(result["heap_events"][-1]["matched_allocation_line"], 1)
    def test_native_allocation_failure_is_reported_after_completed_loader(self):
        result = module.analyze("I/TTY: malloc err size 8368\n"
                                "Instruction read failed at PC=0x000000A0, truncating block.\n")
        self.assertEqual(result["native_allocation_failures_bytes"], [8368])
        self.assertEqual(result["invalid_instruction_fetch_addresses"], [160])
        self.assertTrue(result["allocation_failure_observed"])
        self.assertFalse(result["root_cause_proven"])
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
