"""Analyze user-created BIOS TTY logs offline. Never connects to an emulator.

An unmatched BEGIN identifies the last observed operation, not a proven crash
instruction: log buffering, a stopped emulator or missing TTY output can truncate
the trace. The native malloc error accumulator is reported without interpreting
its value as a successful allocation or a particular failure code.
"""
import argparse
import json
import re
from pathlib import Path

EVENT = re.compile(r"LCK (LOAD|GPU) (BEGIN|DONE) ([0-9a-fA-F]{8}) ([0-9a-fA-F]{8}) err=([0-9a-fA-F]{8})")
HEAP = re.compile(r"LCK (HEAP|FREE) ([0-9a-fA-F]{8}) ([0-9a-fA-F]{8}) RA=([0-9a-fA-F]{8})")


def analyze(text):
    events, pending, anomalies = [], [], []
    heap_events, active, failed_requests = [], {}, []
    for line_number, line in enumerate(text.splitlines(), 1):
        for match in HEAP.finditer(line):
            action, first, second, caller = match.groups()
            first, second, caller = int(first, 16), int(second, 16), int(caller, 16)
            row = dict(line=line_number, action=action, first_word=first,
                       second_word=second, caller_return_address=caller,
                       call_site=caller-8)
            if action == "HEAP":
                row.update(requested_bytes=first, returned_pointer=second)
                if second:
                    if second in active:
                        anomalies.append(f"Pointer reused without observed free at line {line_number}; heap reset/bulk-free/trace gap possible")
                    active[second] = row
                else:
                    row["observed_outstanding_requested_bytes"] = sum(r["requested_bytes"] for r in active.values())
                    row["largest_observed_outstanding_requests"] = sorted(active.values(), key=lambda r: r["requested_bytes"], reverse=True)[:12]
                    failed_requests.append(row)
            else:
                row["freed_pointer"] = first
                allocation = active.pop(first, None)
                row["matched_allocation_line"] = allocation["line"] if allocation else None
            heap_events.append(row)
        for match in EVENT.finditer(line):
            kind, phase, first, second, error = match.groups()
            event = dict(line=line_number, kind=kind, phase=phase,
                         first_word=int(first, 16), second_word=int(second, 16),
                         malloc_error_accumulator=int(error, 16))
            if phase == "BEGIN":
                event["meaning"] = ("type_size, source_pointer" if kind == "LOAD"
                                    else "TIM_flags, TIM_pointer")
                if kind == "LOAD":
                    event["native_type"] = int(first, 16) >> 24
                    event["declared_bytes"] = int(first, 16) & 0xffffff
                pending.append(event)
            else:
                event["meaning"] = ("type_size, return_value" if kind == "LOAD"
                                    else "original_A1, return_value")
                if pending and pending[-1]["kind"] == kind:
                    begin = pending.pop()
                    event["matching_begin_line"] = begin["line"]
                    if kind == "LOAD" and event["first_word"] != begin["first_word"]:
                        anomalies.append(f"LOAD type_size differs at line {line_number}")
                else:
                    anomalies.append(f"Unpaired/out-of-order {kind} DONE at line {line_number}")
            events.append(event)
    allocation_failures = [int(m.group(1)) for m in re.finditer(r"malloc err size (\d+)", text)]
    invalid_fetches = [int(m.group(1), 16) for m in re.finditer(r"Instruction read failed at PC=0x([0-9a-fA-F]+)", text)]
    return dict(schema=1, evidence_class="user-supplied native BIOS TTY observations",
                event_count=len(events), events=events,
                unfinished_operations=pending, anomalies=anomalies,
                last_observed_event=events[-1] if events else None,
                execution_observed=bool(events),
                native_allocation_failures_bytes=allocation_failures,
                invalid_instruction_fetch_addresses=invalid_fetches,
                allocation_failure_observed=bool(allocation_failures),
                heap_events=heap_events, failed_heap_requests=failed_requests,
                heap_accounting_scope="Observed wrapper calls only; pool resets/bulk frees, native headers and fragmentation are not reconstructed",
                successful_scene_load_verified=False, render_verified=False,
                collision_verified=False, root_cause_proven=False,
                limitation="Missing DONE localizes last logged work only; obtain RAM/PC or an engine error to prove the freeze mechanism.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = analyze(args.log.read_text(encoding="utf-8-sig", errors="replace"))
    result["source_log"] = str(args.log.resolve())
    encoded = json.dumps(result, indent=2)
    if args.output:
        with args.output.open("x", encoding="utf-8") as output:
            output.write(encoded + "\n")
    else:
        print(encoded)


if __name__ == "__main__":
    main()
