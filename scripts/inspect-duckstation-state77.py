"""Offline capture decoder. NEVER launches or attaches to DuckStation.
Usage: python inspect-duckstation-state77.py CAPTURE.sav NEW_OUTPUT_DIRECTORY [TARGET.img]
Requires zstandard on PYTHONPATH; accepts only the explicitly checked v77 layout.
Layout sources (read-only primary references): DuckStation src/core/
save_state_version.h, cpu_core.cpp::DoState, bus.cpp::DoState, cdrom.cpp::DoState.
https://github.com/stenzek/duckstation/tree/master/src/core
"""
import hashlib
import json
from pathlib import Path
import runpy
import struct
import sys
import zstandard


def word(data, at):
    return struct.unpack_from('<I', data, at)[0]


source = Path(sys.argv[1])
output = Path(sys.argv[2])
output.mkdir(exist_ok=False, parents=True)
encoded = source.read_bytes()
assert encoded[:4] == b'DUCC' and word(encoded, 4) == 77, 'Only verified state version 77 supported'
kind, size, decoded_size, at = struct.unpack_from('<4I', encoded, 200)
assert kind == 2 and at+size <= len(encoded) and decoded_size <= 32*1024*1024
data = zstandard.ZstdDecompressor().decompress(encoded[at:at+size], max_output_size=decoded_size)
assert len(data) == decoded_size
cpu = data.index(b'\x03\x00\x00\x00CPU')+7
gpr = cpu+8  # v77 predates GTE/muldiv completion tick serialization.
names = 'zero at v0 v1 a0 a1 a2 a3 t0 t1 t2 t3 t4 t5 t6 t7 s0 s1 s2 s3 s4 s5 s6 s7 t8 t9 k0 k1 gp sp fp ra hi lo'.split()
registers = dict(zip(names, struct.unpack_from('<34I', data, gpr)))
assert registers['zero'] == 0
pc, npc = struct.unpack_from('<2I', data, gpr+136)
bus = data.index(b'\x03\x00\x00\x00Bus')+7
ram_size = word(data, bus)
assert ram_size == 0x200000
ram_offset = bus+4+5*3*4  # five arrays of three memory-access timings.
ram = data[ram_offset:ram_offset+ram_size]


def read(address):
    offset = address & 0x1fffffff
    assert offset+4 <= len(ram)
    return word(ram, offset)


man = read(0x8007b898) & 0x1fffffff
counts = struct.unpack_from('<3H', ram, man+34)
section = man+43+3*sum(counts)+int.from_bytes(ram[man+40:man+43], 'little')
sections = []
for _ in range(6):
    size = int.from_bytes(ram[section:section+3], 'little')
    assert section+3+size <= len(ram)
    sections.append((section+3, size))
    section += 3+size
name_at, name_size = sections[2]
scene_name = ram[name_at:name_at+name_size].split(b'\x00')[0].decode('ascii', errors='replace')
actors = []
head = read(0x8007c34c)
p = read(head)
seen = set()
while p:
    assert p not in seen and len(seen) < 143
    seen.add(p)
    handler = read(p+12)
    if handler == 0x80021934:
        actors.append(dict(pointer=p, handler=handler, state=read(p+24) >> 16))
    p = read(p)

cd = data.index(b'\x05\x00\x00\x00CDROM')+9
command, second = struct.unpack_from('<2H', data, cd)
drive, status, secondary, mode, request, double_speed, interrupt_enable, interrupt_flag = data[cd+4:cd+12]
cursor = cd+12+8  # last interrupt tick, 64 bits since v71.
pending = data[cursor]
position = list(data[cursor+1:cursor+4])
current, seek_start, seek_end, subq = struct.unpack_from('<4I', data, cursor+4)
media_length, media_offset = struct.unpack_from('<2I', encoded, 168)
report = dict(source=str(source), source_sha256=hashlib.sha256(encoded).hexdigest(),
              save_version=77, media=encoded[media_offset:media_offset+media_length].decode('utf-8'),
              registers=registers, pc=pc, npc=npc, ram_offset=ram_offset,
              resident_scene_display_name=scene_name, man_counts=counts,
              native_model_count=read(0x8007b774), failed_actor_spawns=read(0x8007b8a0),
              actor_pool_free_top=read(0x8007c348), transition_actors=actors,
              native_read=dict(raw_toc_index=read(0x8007bc24), relative_lba=read(0x8007bc08),
                               sector_count=read(0x8007bc80), remaining=read(0x8007bc94),
                               phase=read(0x8007bc98), active=read(0x8007bca0),
                               destination=read(0x8007bc58)),
              cdrom=dict(command=command, second_response=second, drive_state=drive,
                         drive_state_name={0:'Idle',3:'SeekingPhysical',4:'SeekingLogical',7:'Reading',8:'Playing'}.get(drive,'Other'),
                         status=status, secondary_status=secondary, mode=mode,
                         interrupt_enable=interrupt_enable, interrupt_flag=interrupt_flag,
                         pending_interrupt=pending, setloc_position_binary=position,
                         current_lba=current, seek_start_lba=seek_start, seek_end_lba=seek_end,
                         current_subq_lba=subq),
              scope='User-created runtime snapshot, decoded offline; one snapshot does not prove an enduring stall or its cause')
if len(sys.argv) > 3:
    audit = runpy.run_path(str(Path(__file__).with_name('audit-native-scene.py')))
    disc = audit['Disc'](sys.argv[3])
    toc = disc.sectors(disc.prot, 2)
    report['cached_TOC_first_4096_bytes_match_target'] = toc == ram[0x1c70f0:0x1c80f0]
    entry = report['native_read']['raw_toc_index']-2
    bundle = disc.entry(entry)
    report['pending_target_entry'] = entry
    address = report['native_read']['destination'] & 0x1fffffff
    report['pending_buffer_first_32_bytes_match_target'] = ram[address:address+32] == bundle[:32]
    report['target_entry_sectors'] = len(bundle)//2048
(output/'capture.sav').write_bytes(encoded)
(output/'RAM.bin').write_bytes(ram)
(output/'STATE_REPORT.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
print(json.dumps({k: v for k, v in report.items() if k != 'registers'}, indent=2))
