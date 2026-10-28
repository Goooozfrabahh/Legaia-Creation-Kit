import struct
import unittest
from lck_floor_monitor import decode, judge, Session, TRACE, PLAYER, RAM_SIZE


class MonitorTests(unittest.TestCase):
    def fixture(self, calls=0, pairs=0):
        ram=bytearray(RAM_SIZE)
        ram[TRACE:TRACE+4]=b"FXD1"
        struct.pack_into("<I",ram,TRACE+4,1)
        struct.pack_into("<II",ram,TRACE+12,calls,pairs)
        struct.pack_into("<I",ram,TRACE+56,0x801104f8)
        struct.pack_into("<I",ram,PLAYER,0x801b0000)
        struct.pack_into("<hhh",ram,0x1b0014,4448,-64,2368)
        return ram

    def test_presence_is_not_execution_and_execution_is_not_correctness(self):
        ram=self.fixture()
        packet=decode(lambda a,n:ram[a:a+n],"TEST_FIXTURE")
        self.assertEqual(packet["status"],"LOADED_NOT_EXECUTED")
        self.assertFalse(packet["executed"])
        struct.pack_into("<I",ram,TRACE+12,10)
        packet=decode(lambda a,n:ram[a:a+n],"TEST_FIXTURE")
        self.assertEqual(packet["status"],"HOOK_EXECUTED_NO_PAIRS")
        self.assertFalse(any(p["sampled"] for p in packet["probes"]))

    def test_actual_player_elevation_is_distinct_from_ground_return(self):
        ram=self.fixture(256,8)
        struct.pack_into("<I",ram,TRACE+32,0x801b0000)
        struct.pack_into("<ii",ram,TRACE+44,0,-112)
        struct.pack_into("<hhiiIII",ram,TRACE+64,4448,2368,0,-64,0,0x800000,1)
        packet=decode(lambda a,n:ram[a:a+n],"TEST_FIXTURE")
        self.assertEqual(packet["live_vahn_xyz"],(4448,-64,2368))
        self.assertEqual(packet["natural_stage_ground"],-112)
        self.assertTrue(packet["natural_query_is_vahn"])
        self.assertEqual(packet["probes"][0]["stage_ground"],-64)
        struct.pack_into("<I",ram,TRACE+56,0)
        self.assertEqual(decode(lambda a,n:ram[a:a+n],"TEST_FIXTURE")["status"],"WRONG_CELL_OR_NO_EXTENSION")

    def test_busy_and_unknown_profiles_fail_without_inventing_results(self):
        ram=self.fixture(5,1);struct.pack_into("<I",ram,TRACE+8,1)
        self.assertEqual(decode(lambda a,n:ram[a:a+n],"TEST_FIXTURE")["status"],"BUSY_OR_INCOHERENT_RETRY")
        ram[TRACE]=0
        self.assertEqual(decode(lambda a,n:ram[a:a+n],"TEST_FIXTURE")["status"],"NO_OBSERVABLE_PROFILE")

    def test_pass_requires_fresh_correct_pairs_and_expected_code(self):
        probes=[dict(slot=i,x=10+i,z=20,retail_ground=0,stage_ground=-64,sequence=i+1) for i in range(8)]
        oracle=dict(probe_profiles=[dict(tag=1,scene="town01",oracle=[dict(slot=i,x=10+i,z=20,retail=0,stage=-64) for i in range(1,8)])])
        packet=dict(executed=True,extension_active=True,code_matches_expected=True,profile_tag=1,probes=probes,source_mode="USER_RUN_READ_ONLY_SHARED_RAM",fresh_probe_slots=[],all_probes_fresh_this_session=False)
        self.assertEqual(judge(packet,oracle),"WAIT_FOR_EIGHT_FRESH_NATIVE_PAIRS")
        packet.update(fresh_probe_slots=list(range(8)),all_probes_fresh_this_session=True)
        self.assertEqual(judge(packet,oracle),"PASS_FIXED_NATIVE_PAIRS")
        probes[3]["stage_ground"]=999
        self.assertEqual(judge(packet,oracle),"FAIL_HEIGHT_COMPARISON_CAPTURE_LOG")
        probes[3]["stage_ground"]=-64;packet["code_matches_expected"]=False
        self.assertEqual(judge(packet,oracle),"WRONG_OR_UNVERIFIED_CODE")
        packet["code_matches_expected"]=True;packet["source_mode"]="OFFLINE_DUMP_NOT_LIVE"
        self.assertEqual(judge(packet,oracle),"OFFLINE_PAIRS_MATCH_NOT_LIVE_PROOF")

    def test_saved_counters_and_cell_switches_do_not_count_as_fresh_execution(self):
        session=Session()
        def packet(calls,pairs,tag=1):
            return dict(outer_calls=calls,controlled_pairs=pairs,profile_tag=tag,extension_active=True,
                        probes=[dict(slot=i,sequence=pairs-i) for i in range(8)])
        first=packet(1000,50);session.update(first)
        self.assertFalse(first["counter_advanced_since_last_read"])
        self.assertEqual(first["fresh_probe_slots"],[])
        current=packet(1256,58);session.update(current)
        self.assertTrue(current["counter_advanced_since_last_read"])
        self.assertTrue(current["all_probes_fresh_this_session"])
        old_state=packet(200,10);session.update(old_state)
        self.assertFalse(old_state["counter_advanced_since_last_read"])
        self.assertFalse(old_state["all_probes_fresh_this_session"])
        switched=packet(500,18,2);session.update(switched)
        self.assertEqual(switched["fresh_probe_slots"],[])

    def test_busy_snapshot_preserves_session_epoch(self):
        session=Session();first=dict(outer_calls=10,controlled_pairs=0,profile_tag=1,extension_active=True,probes=[])
        session.update(first);session.update(dict(status="BUSY_OR_INCOHERENT_RETRY"))
        self.assertEqual(session.previous_calls,10)
        self.assertEqual(session.session_pairs,0)


if __name__=="__main__":
    unittest.main()
