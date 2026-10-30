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


def analyze(text):
    events, pending, anomalies = [], [], []
    for line_number, line in enumerate(text.splitlines(), 1):
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
    return dict(schema=1, evidence_class="user-supplied native BIOS TTY observations",
                event_count=len(events), events=events,
                unfinished_operations=pending, anomalies=anomalies,
                last_observed_event=events[-1] if events else None,
                execution_observed=bool(events),
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
