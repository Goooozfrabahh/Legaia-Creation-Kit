"""Verify changed raw Mode-2 Form-1 sectors independently (EDC + P/Q ECC).
Usage: python scripts/verify-img-changes.py RETAIL NEW_REPORT.json CANDIDATE...
Unchanged sectors inherit the source; no game or emulator is run.
"""
import hashlib
import json
import struct
import sys

edc_lut = []
forward, backward = [0]*256, [0]*256
for i in range(256):
    value = i
    for _ in range(8):
        value = (value >> 1) ^ (0xd8018001 if value & 1 else 0)
    edc_lut.append(value)
    value = (i << 1) ^ (0x11d if i & 128 else 0)
    forward[i] = value
    backward[i ^ value] = i


def parity(sector, majors, minors, mult, inc, dest):
    size = majors*minors
    for major in range(majors):
        index = (major >> 1)*mult+(major & 1)
        a = b = 0
        for _ in range(minors):
            value = sector[12+index]
            index = (index+inc) % size
            a = forward[a ^ value]
            b ^= value
        a = backward[forward[a] ^ b]
        sector[dest+major] = a
        sector[dest+major+majors] = a ^ b


def valid(raw):
    assert len(raw) == 2352 and raw[15] == 2
    assert raw[16:20] == raw[20:24]
    if raw[18] & 32:
        if raw[2348:] == bytes(4):
            return True  # XA Form-2 EDC is optional.
        edc = 0
        for value in raw[16:2348]:
            edc = (edc >> 8) ^ edc_lut[(edc ^ value) & 255]
        return struct.pack('<I', edc) == raw[2348:]
    sector = bytearray(raw)
    edc = 0
    for value in raw[16:2072]:
        edc = (edc >> 8) ^ edc_lut[(edc ^ value) & 255]
    sector[2072:2076] = struct.pack('<I', edc)
    sector[12:16] = bytes(4)
    parity(sector, 86, 24, 2, 86, 2076)
    parity(sector, 52, 43, 86, 88, 2248)
    return sector[2072:] == raw[2072:]


def inspect(source, candidate, inherited):
    changed = []
    reencoded = []
    with open(source, 'rb') as a, open(candidate, 'rb') as b:
        lba = 0
        while True:
            original, new = a.read(2352), b.read(2352)
            if not original and not new:
                break
            assert len(new) == 2352, 'Truncated candidate or missing source sectors'
            if new != original:
                changed.append(lba)
                # Relayout moves original sectors and updates MSF headers. EDC/ECC
                # ignore that header; exact source payload+parity needs no re-encode.
                if hashlib.sha256(new[16:]).digest() not in inherited:
                    assert valid(new), 'Bad changed sector %d in %s' % (lba, candidate)
                    reencoded.append(lba)
            lba += 1
    return dict(image=candidate, total_sectors=lba, changed_sectors=len(changed),
                all_new_payload_edc_ecc_valid=True, reencoded_sectors=len(reencoded),
                reencoded_lbas=reencoded,
                moved_payload_rule='Exact original bytes 16..2352 including stored parity; raw MSF header excluded')


if __name__ == '__main__':
    inherited = set()
    with open(sys.argv[1], 'rb') as source:
        while raw := source.read(2352):
            assert len(raw) == 2352
            inherited.add(hashlib.sha256(raw[16:]).digest())
    rows = [inspect(sys.argv[1], p, inherited) for p in sys.argv[3:]]
    with open(sys.argv[2], 'x', encoding='utf-8') as out:
        json.dump(dict(source=sys.argv[1], cases=rows, gameplay_verified=False), out, indent=2)
