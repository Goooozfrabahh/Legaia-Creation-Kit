"""Read-only raw IMG scene audit; never launches game code.
Usage: python scripts/audit-native-scene.py MAP_ENTRY NEW_REPORT.json IMAGE...
Structural evidence only, not a native execution or gameplay proof.
"""
import hashlib
import json
import struct
import sys


def word(data, at=0):
    return struct.unpack_from('<I', data, at)[0]


def digest(data):
    return hashlib.sha256(data).hexdigest()


class Disc:
    def __init__(self, path):
        self.file = open(path, 'rb')
        pvd = self.sectors(16, 1)
        assert pvd[1:6] == b'CD001'
        root = self.sectors(word(pvd, 158), (word(pvd, 166)+2047)//2048)
        at = 0
        while at < len(root):
            n = root[at]
            if not n:
                at = (at//2048+1)*2048
                continue
            r = root[at:at+n]
            if r[33:33+r[32]].split(b';')[0] == b'PROT.DAT':
                self.prot = word(r, 2)
                self.total = word(r, 10)//2048
                break
            at += n
        else:
            raise ValueError('No PROT.DAT')
        header = self.sectors(self.prot, 1)
        count, sectors = word(header, 4)+1, word(header, 8)
        assert 1 < count < 10000 and 0 < sectors < 64
        self.starts = list(struct.unpack_from('<%dI' % count, self.sectors(self.prot, sectors), 16))

    def sectors(self, lba, count):
        out = bytearray()
        for i in range(count):
            self.file.seek((lba+i)*2352+24)
            b = self.file.read(2048)
            assert len(b) == 2048
            out.extend(b)
        return bytes(out)

    def entry(self, index):
        start = self.starts[index]
        end = self.starts[index+1] if index+1 < len(self.starts) else self.total
        assert start < end and end-start < 65536
        return self.sectors(self.prot+start, end-start)


def decode(data, size):
    ring = bytearray(4096)
    pos, at, bits = 0xfee, 0, 0
    out = bytearray()
    while len(out) < size:
        if not bits & 0x100:
            bits = data[at] | 0xff00
            at += 1
        if bits & 1:
            value = data[at]
            at += 1
            out.append(value)
            ring[pos] = value
            pos = (pos+1) & 4095
        else:
            a, b = data[at:at+2]
            at += 2
            source = a | ((b & 0xf0) << 4)
            for i in range((b & 15)+3):
                value = ring[(source+i) & 4095]
                out.append(value)
                ring[pos] = value
                pos = (pos+1) & 4095
        bits >>= 1
    assert len(out) == size, 'Whole native token exceeds declared output'
    return bytes(out), at


def inspect(path, entry):
    disc = Disc(path)
    bundle = disc.entry(entry+3)
    candidates = [i for i in (0, 4) if 0 < word(bundle, i) <= 32 and
                  word(bundle, i+12) == 8+8*word(bundle, i)]
    assert len(candidates) == 1, candidates
    base = candidates[0]
    slots, payloads = [], {}
    for i in range(word(bundle, base)):
        tag, offset = struct.unpack_from('<II', bundle, base+8+i*8)
        kind, size = tag >> 24, tag & 0xffffff
        row = dict(slot=i, type=kind, size=size, source_offset=base+offset)
        if kind not in (10, 15, 20):
            raw, used = decode(bundle[base+offset:], size)
            row.update(decoded_sha256=digest(raw), compressed_bytes_consumed=used)
            payloads[kind] = raw
        slots.append(row)
    man = payloads[3]
    counts = struct.unpack_from('<3H', man, 0x22)
    start = 43+3*sum(counts)
    section = start+int.from_bytes(man[40:43], 'little')
    sections = []
    for _ in range(6):
        size = int.from_bytes(man[section:section+3], 'little')
        assert section+3+size <= len(man)
        sections.append(dict(offset=section, size=size, sha256=digest(man[section+3:section+3+size])))
        section += 3+size
    bank = payloads[5]
    count = word(bank)
    offsets = [word(bank, 4+i*4) for i in range(count)]+[len(bank)]
    return dict(image=path, map_sha256=digest(disc.entry(entry)), bundle_bytes=len(bundle),
                bundle_sha256=digest(bundle), pac_bytes=len(disc.entry(entry+4)),
                pac_sha256=digest(disc.entry(entry+4)), descriptors=slots,
                man_partition_counts=counts, man_sections=sections,
                anm_clip_sha256=[digest(bank[offsets[i]:offsets[i+1]]) for i in range(count)],
                gameplay_verified=False)


if __name__ == '__main__':
    rows = [inspect(p, int(sys.argv[1])) for p in sys.argv[3:]]
    with open(sys.argv[2], 'x', encoding='utf-8') as out:
        json.dump(dict(scope='independent structural comparison; not runtime verification',
                       map_entry=int(sys.argv[1]), cases=rows), out, indent=2)
