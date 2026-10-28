"""User-run read-only observer. Never starts/pauses/controls an emulator.

Live mode opens an EXISTING exported DuckStation mapping with FILE_MAP_READ.
No process handle, GDB connection, RAM write, input or desktop API is used.
"""
import argparse
import ctypes
import hashlib
import datetime
import json
import pathlib
import struct
import time
import uuid

TRACE = 0x77728
PLAYER = 0x7C364
RAM_SIZE = 0x200000


def decode(read, source_mode):
    for _ in range(20):
        data = read(TRACE, 256)
        if data[:4] != b"FXD1" or struct.unpack_from("<I", data, 4)[0] != 1:
            return {"source_mode": source_mode, "status": "NO_OBSERVABLE_PROFILE", "executed": False}
        busy, calls, pairs = struct.unpack_from("<III", data, 8)
        if busy:
            continue
        player = struct.unpack("<I", read(PLAYER, 4))[0]
        live = None
        if 0x80000000 <= player <= 0x801FFFE0:
            first = read(player & 0x1FFFFF, 32)
            second = read(player & 0x1FFFFF, 32)
            if first[20:26] == second[20:26]:
                live = struct.unpack_from("<hhh", first, 20)
        check = read(TRACE, 256)
        if check != data or struct.unpack_from("<I", check, 8)[0]:
            continue
        probes = []
        for i in range(8):
            x, z, retail, stage, rflags, sflags, seq = struct.unpack_from("<hhiiIII", data, 64 + i*24)
            probes.append(dict(slot=i, x=x, z=z, retail_ground=retail, stage_ground=stage,
                               retail_flags=rflags, stage_flags=sflags, sequence=seq, sampled=seq != 0))
        arg = struct.unpack_from("<I", data, 32)[0]
        query = struct.unpack_from("<hhh", data, 36)
        # The stored pair for live Vahn has its own coordinates and sequence;
        # never conflate it with a newer moving actor position.
        return dict(source_mode=source_mode,
                    status=("WRONG_CELL_OR_NO_EXTENSION" if calls and not struct.unpack_from("<I", data, 56)[0] else "PAIRS_AVAILABLE" if pairs else "HOOK_EXECUTED_NO_PAIRS" if calls else "LOADED_NOT_EXECUTED"),
                    executed=calls != 0, outer_calls=calls, controlled_pairs=pairs,
                    player_pointer=hex(player), live_vahn_xyz=live,
                    natural_query_actor=hex(arg), natural_query_xyz=query,
                    natural_query_is_vahn=arg == player and player != 0,
                    natural_retail_ground=struct.unpack_from("<i", data, 44)[0],
                    natural_stage_ground=struct.unpack_from("<i", data, 48)[0],
                    natural_matched_mode=struct.unpack_from("<I", data, 52)[0] & 0xFFFF,
                    profile_tag=struct.unpack_from("<I", data, 52)[0] >> 16,
                    extension_active=struct.unpack_from("<I", data, 56)[0] != 0,
                    probes=probes, correctness="Compare recorded pairs against the declared test oracle; execution counters alone do not prove correct heights.")
    return {"source_mode": source_mode, "status": "BUSY_OR_INCOHERENT_RETRY", "executed": None}


def judge(packet, oracle):
    if not packet.get("executed"):
        return "NOT_EXECUTED"
    if not packet.get("extension_active"):
        return "NO_ACTIVE_EXTENSION_IN_CURRENT_CELL"
    if not packet.get("code_matches_expected"):
        return "WRONG_OR_UNVERIFIED_CODE"
    profile = next((p for p in oracle.get("probe_profiles", []) if p["tag"] == packet.get("profile_tag")), None)
    if not profile:
        return "UNKNOWN_CELL_PROFILE"
    packet["native_cell"] = profile["scene"]
    fresh = packet.get("fresh_probe_slots", [])
    for expected in profile["oracle"]:
        if expected["slot"] not in fresh:
            continue
        got = packet["probes"][expected["slot"]]
        if (got["x"], got["z"], got["retail_ground"], got["stage_ground"]) != (expected["x"], expected["z"], expected["retail"], expected["stage"]):
            return "FAIL_HEIGHT_COMPARISON_CAPTURE_LOG"
    if not packet.get("all_probes_fresh_this_session"):
        return "WAIT_FOR_EIGHT_FRESH_NATIVE_PAIRS"
    return "PASS_FIXED_NATIVE_PAIRS" if packet["source_mode"] == "USER_RUN_READ_ONLY_SHARED_RAM" else "OFFLINE_PAIRS_MATCH_NOT_LIVE_PROOF"


class Session:
    def __init__(self):
        self.previous_calls=None
        self.previous_active=False
        self.previous_profile=None
        self.session_pairs=None

    def update(self,packet):
        if packet.get("status")=="BUSY_OR_INCOHERENT_RETRY":
            return
        current_calls=packet.get("outer_calls")
        counter_reset=isinstance(current_calls,int) and isinstance(self.previous_calls,int) and current_calls < self.previous_calls
        packet["counter_advanced_since_last_read"] = isinstance(current_calls,int) and isinstance(self.previous_calls,int) and current_calls > self.previous_calls
        if isinstance(current_calls,int):
            self.previous_calls=current_calls
        active = packet.get("extension_active", False)
        pairs = packet.get("controlled_pairs", 0)
        if self.session_pairs is None or not active or not self.previous_active or pairs < self.session_pairs or packet.get("profile_tag") != self.previous_profile or counter_reset:
            self.session_pairs = pairs
        packet["fresh_probe_slots"] = [p["slot"] for p in packet.get("probes", []) if active and p["sequence"] > self.session_pairs]
        packet["all_probes_fresh_this_session"] = len(packet["fresh_probe_slots"]) == 8
        self.previous_active = active
        self.previous_profile = packet.get("profile_tag")


class ExistingMapping:
    def __init__(self, name):
        if not name.startswith("duckstation_") or not name.removeprefix("duckstation_").isdigit():
            raise ValueError("Use the exact duckstation_PID exported mapping name from your DuckStation log")
        self.k = ctypes.WinDLL("kernel32", use_last_error=True)
        self.k.OpenFileMappingW.argtypes = [ctypes.c_uint32, ctypes.c_int, ctypes.c_wchar_p]
        self.k.OpenFileMappingW.restype = ctypes.c_void_p
        self.k.MapViewOfFile.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_size_t]
        self.k.MapViewOfFile.restype = ctypes.c_void_p
        self.k.UnmapViewOfFile.argtypes = [ctypes.c_void_p]
        self.k.CloseHandle.argtypes = [ctypes.c_void_p]
        self.handle = self.k.OpenFileMappingW(4, False, name)  # FILE_MAP_READ; no creation
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        self.base = self.k.MapViewOfFile(self.handle, 4, 0, 0, RAM_SIZE)
        if not self.base:
            self.k.CloseHandle(self.handle)
            raise ctypes.WinError(ctypes.get_last_error())

    def read(self, offset, size):
        if offset < 0 or offset + size > RAM_SIZE:
            raise ValueError("Read outside PS1 main RAM")
        return ctypes.string_at(self.base + offset, size)

    def close(self):
        self.k.UnmapViewOfFile(self.base)
        self.k.CloseHandle(self.handle)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--mapping", help="Existing DuckStation exported RAM mapping name; USER runs this")
    mode.add_argument("--prompt", action="store_true", help="User types the existing exported mapping name; no process discovery")
    mode.add_argument("--dump", type=pathlib.Path, help="Offline flat 2 MiB RAM dump; never labeled live evidence")
    parser.add_argument("--jsonl", type=pathlib.Path, help="NEW evidence file; never overwritten")
    parser.add_argument("--seconds", type=float, default=30)
    parser.add_argument("--oracle", type=pathlib.Path, help="JSON receipt accompanying this observable IMG; required for a PASS verdict")
    args = parser.parse_args()
    if args.jsonl is None:
        args.jsonl=pathlib.Path.cwd()/("floor-capture-"+uuid.uuid4().hex+".jsonl")
    oracle=json.loads(args.oracle.read_text()) if args.oracle else {}
    mapping = None
    if args.dump:
        ram = args.dump.read_bytes()
        if len(ram) != RAM_SIZE:
            parser.error("Expected a flat 2 MiB RAM dump")
        read = lambda offset, size: ram[offset:offset+size]
        source_mode = "OFFLINE_DUMP_NOT_LIVE"
    else:
        name=input("Existing exported RAM mapping name (duckstation_PID): ").strip() if args.prompt else args.mapping
        mapping = ExistingMapping(name)
        read = mapping.read
        source_mode = "USER_RUN_READ_ONLY_SHARED_RAM"
    try:
        with args.jsonl.open("x", encoding="utf-8") as log:
            deadline = time.monotonic() + args.seconds
            session=Session()
            while True:
                packet = decode(read, source_mode)
                packet["utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
                if packet.get("status")=="BUSY_OR_INCOHERENT_RETRY":
                    log.write(json.dumps(packet)+"\n");log.flush()
                    print("Native telemetry busy; retrying without using a torn snapshot",flush=True)
                    if args.dump or time.monotonic() >= deadline:break
                    time.sleep(0.25);continue
                session.update(packet)
                active=packet.get("extension_active",False)
                pairs=packet.get("controlled_pairs",0)
                code_va = oracle.get("code_va", 0x8007AB80) & 0x1FFFFF
                code_size = oracle.get("code_bytes", 0)
                packet["code_matches_expected"] = bool(code_size and hashlib.sha256(read(code_va,code_size)).hexdigest() == oracle.get("probe_code_sha256"))
                packet["verdict"] = judge(packet, oracle)
                packet["execution_verdict"] = ("CONTROLLED_STAGE_E_EXECUTED" if packet["counter_advanced_since_last_read"] and pairs > session.session_pairs and packet["code_matches_expected"] and active else "NATIVE_HOOK_EXECUTED_WAITING_FOR_PAIR" if packet["counter_advanced_since_last_read"] and packet["code_matches_expected"] else "HISTORICAL_COUNTER_ONLY" if packet.get("outer_calls",0) else "NOT_EXECUTED")
                if packet.get("live_vahn_xyz") and active and oracle.get("request"):
                    request=oracle["request"];x0=request["source_tile"][0]*128+request["delta"][0];z0=request["source_tile"][1]*128+request["delta"][1]
                    x,_,z=packet["live_vahn_xyz"]
                    packet["vahn_observed_inside_relocated_floor"] = x0 <= x < x0+128 and z0 <= z < z0+128
                log.write(json.dumps(packet) + "\n")
                log.flush()
                print(f"{packet['execution_verdict']} | {packet['verdict']} | calls={packet.get('outer_calls')} pairs={pairs} cell={packet.get('native_cell','unknown')} Vahn XYZ={packet.get('live_vahn_xyz')} | full data: {args.jsonl}",flush=True)
                if args.dump or time.monotonic() >= deadline:
                    break
                time.sleep(0.25)
    finally:
        if mapping:
            mapping.close()


if __name__ == "__main__":
    main()
