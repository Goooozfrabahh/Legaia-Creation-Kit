#!/usr/bin/env python3
"""Offline capture analysis and bounded user-run DuckStation GDB observations.

Never starts an emulator, edits its installation/settings, writes guest memory
or registers, or produces a game image. 'trace' intentionally pauses/resumes the
user's running emulator and uses metadata-only hardware execution breakpoints.
"""
import argparse
import hashlib
import json
import re
import socket
import struct
import time
import zipfile
from pathlib import Path

RAM_BASE, RAM_SIZE = 0x80000000, 0x200000
HEAP_START, HEAP_END = 0x80091800, 0x801C6000
REGISTRY, COUNT, ARENA, HEAP = 0x8007C018, 0x8007B774, 0x8007B85C, 0x8007BB58
HOOKS = {0x80017888: 'malloc', 0x80017B94: 'free', 0x800583C8: 'upload',
         0x80058490: 'move', 0x80026B4C: 'register_model', 0x8001E1B4: 'stage_init'}
REGIONS = [(880, 64, 16, 64), (832, 0, 16, 1), (880, 128, 16, 64), (848, 0, 16, 1)]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_digest(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def ram_reader(data):
    if len(data) != RAM_SIZE:
        raise ValueError('Expected exactly 2 MiB of main RAM')

    def read(address, size):
        offset = address - RAM_BASE
        if offset < 0 or size < 0 or offset + size > len(data):
            raise ValueError('RAM read out of bounds')
        return data[offset:offset + size]
    return read


def word(read, address):
    return struct.unpack('<I', read(address, 4))[0]


def heap_state(read):
    """Validate both allocated bucket lists and the free list, not byte scans."""
    descriptor = word(read, HEAP)
    if not HEAP_START <= descriptor < HEAP_END - 8:
        raise ValueError('Heap descriptor outside known retail arena')
    head, buckets = struct.unpack('<II', read(descriptor, 8))
    if not 1 <= buckets <= 16 or not HEAP_START <= head - (buckets - 1) * 12 < head + 24 <= HEAP_END:
        raise ValueError('Invalid heap sentinel layout')
    seen = set()

    def walk(sentinel):
        node, tail, zero = struct.unpack('<III', read(sentinel, 12))
        if zero != 0:
            raise ValueError('Nonzero heap sentinel size')
        previous, result = sentinel, []
        while node != sentinel:
            if node in seen or len(seen) >= 4096 or not HEAP_START <= node < HEAP_END - 12 or node % 4:
                raise ValueError('Cyclic/out-of-range heap list')
            seen.add(node)
            nxt, prev, size = struct.unpack('<III', read(node, 12))
            if prev != previous or size % 4 or size == 0 or node + 12 + size > head - (buckets - 1) * 12:
                raise ValueError('Inconsistent heap header/backlink/extent')
            result.append({'header': node, 'payload': node + 12, 'bytes': size})
            previous, node = node, nxt
        if previous != tail:
            raise ValueError('Heap sentinel backlink mismatch')
        return result

    free = walk(head + 12)
    allocated = []
    for bucket in range(buckets):
        for block in walk(head - bucket * 12):
            allocated.append(dict(block, bucket=bucket))
    intervals = sorted((b['header'], b['payload'] + b['bytes']) for b in free + allocated)
    if any(a[1] > b[0] for a, b in zip(intervals, intervals[1:])):
        raise ValueError('Overlapping heap blocks')
    return {'descriptor': descriptor, 'buckets': buckets, 'free': free, 'allocated': allocated,
            'free_bytes': sum(b['bytes'] for b in free),
            'largest_free': max((b['bytes'] for b in free), default=0),
            'allocated_bytes': sum(b['bytes'] for b in allocated),
            'scope': 'one snapshot; not allocation-time headroom or temporal high-water proof'}


def ram_state(read):
    count = word(read, COUNT)
    result = {'scene': word(read, 0x80084540) & 0xFFFF, 'model_count': count,
              'last_registered': word(read, 0x8007BB38), 'prefix': word(read, 0x8007B6F8),
              'watermark': word(read, 0x8007B824), 'arena': word(read, ARENA),
              'actor_free_counter': word(read, 0x8007C348), 'stp_latch': word(read, 0x8007B998),
              'known_index_200_conflict': count > 200, 'production_approved': False}
    result['models'] = [word(read, REGISTRY + i * 4) for i in range(min(count, 200))]
    result['active_registry_sha256'] = digest(read(REGISTRY, min(count, 200) * 4))
    try:
        result['heap'] = heap_state(read)
        blocks = result['heap']['allocated']
        result['arena_allocation'] = next((b for b in blocks if b['payload'] == result['arena']), None)
        result['model_allocation_owners'] = sorted({b['header'] for p in result['models'] for b in blocks
                                                  if b['payload'] <= p < b['payload'] + b['bytes']})
    except ValueError as e:
        result['heap_error'] = str(e)
    return result


def iso_file(path, name, entry=None):
    """Bounded read-only ISO/PROT extraction for evidence comparison only."""
    with open(path, 'rb') as f:
        def read(lba, size):
            if size < 0 or size > 2 * RAM_SIZE:
                raise ValueError('Evidence file read exceeds bound')
            out = bytearray()
            for i in range((size + 2047) // 2048):
                f.seek((lba + i) * 2352 + 24)
                block = f.read(2048)
                if len(block) != 2048:
                    raise ValueError('Truncated raw ISO sector')
                out.extend(block)
            return bytes(out[:size])
        pvd = read(16, 2048)
        if pvd[1:6] != b'CD001':
            raise ValueError('Expected raw 2352-byte ISO image')
        root = pvd[156:190]
        data = read(struct.unpack_from('<I', root, 2)[0], struct.unpack_from('<I', root, 10)[0])
        offset = 0
        while offset < len(data):
            length = data[offset]
            if not length:
                offset = (offset // 2048 + 1) * 2048
                continue
            record = data[offset:offset + length]
            if len(record) < 34:
                raise ValueError('Invalid ISO directory record')
            offset += length
            if record[33:33 + record[32]].split(b';')[0].decode('ascii', errors='replace') != name:
                continue
            lba, size = struct.unpack_from('<I', record, 2)[0], struct.unpack_from('<I', record, 10)[0]
            if entry is None:
                return read(lba, size)
            header = read(lba, 2048)
            count, sectors = struct.unpack_from('<II', header, 4)
            if not 0 <= entry < count - 1 or not 0 < sectors <= 128:
                raise ValueError('PROT entry/header out of bounds')
            header = read(lba, sectors * 2048)
            start, end = struct.unpack_from('<II', header, 8 + (entry + 2) * 4)
            if start < sectors or end <= start or end * 2048 > size:
                raise ValueError('Invalid PROT entry footprint')
            return read(lba + start, (end - start) * 2048)
    raise ValueError('ISO file not found: ' + name)


def region_bytes(raw, region):
    x, y, w, h = region
    if len(raw) != 1024 * 512 * 2 or w <= 0 or h <= 0 or not (0 <= x <= 1024 - w and 0 <= y <= 512 - h):
        raise ValueError('Invalid raw VRAM/region')
    return b''.join(raw[(row * 1024 + x) * 2:(row * 1024 + x + w) * 2] for row in range(y, y + h))


def analyze(args):
    report = {'schema':'lck-native-runtime-v1','tool_sha256':file_digest(__file__), 'zip_sha256': file_digest(args.archive), 'production_approved': False, 'ram': {}, 'vram': {}, 'save_disc_paths': {}}
    source = iso_file(args.source_image, 'PROT.DAT', args.map_entry) if args.source_image else None
    comparison = iso_file(args.comparison_image, 'PROT.DAT', args.map_entry) if args.comparison_image else None
    for label, path in [('declared_source', args.source_image), ('comparison_image', args.comparison_image)]:
        if path:
            report[label] = {'path': str(path), 'sha256': file_digest(path)}
    with zipfile.ZipFile(args.archive) as archive:
        if sum(e.file_size for e in archive.infolist()) > 64 * 1024 * 1024 or archive.testzip() is not None:
            raise ValueError('Oversized or corrupt archive')
        names = [e.filename for e in archive.infolist() if not e.is_dir()]
        if len(names) > 64 or len({Path(n).name for n in names}) != len(names):
            raise ValueError('Duplicate/too many capture names')
        if args.fixtures:
            args.fixtures.mkdir()  # exclusive; never overwrite an existing capture directory
        vrams = []
        for name in names:
            raw = archive.read(name)
            if name.endswith('_ram.bin'):
                summary = ram_state(ram_reader(raw))
                summary['sha256'] = digest(raw)
                if source:
                    needle = source[0x10000:0x12000]
                    pos = raw.find(needle) if len(needle) == 8192 and any(needle) else -1
                    if pos >= 0x10000 and raw.find(needle, pos + 1) == -1:
                        base = pos - 0x10000
                        loaded = raw[base:base + len(source)]
                        differences = [i for i, (a, b) in enumerate(zip(source, loaded)) if a != b]
                        summary['loaded_map'] = {'base': RAM_BASE + base, 'different_bytes_from_source': len(differences),
                                                 'changed_descriptors': sorted({i // 32 for i in differences if i < 0x4000}),
                                                 'matches_comparison_map': loaded == comparison if comparison else None,
                                                 'descriptor_array_matches_comparison':loaded[:0x4000]==comparison[:0x4000] if comparison else None,
                                                 'walls_floors_match_comparison':loaded[0x4000:0x8000]==comparison[0x4000:0x8000] if comparison else None,
                                                 'different_bytes_from_comparison':sum(a!=b for a,b in zip(loaded,comparison)) if comparison else None}
                report['ram'][name] = summary
            elif name.endswith('_vram.bin'):
                if len(raw) != 1024 * 512 * 2:
                    raise ValueError('Invalid VRAM snapshot length')
                vrams.append(raw)
                report['vram'][name] = {'sha256': digest(raw), 'regions': []}
                for region in REGIONS:
                    data = region_bytes(raw, region)
                    report['vram'][name]['regions'].append({'rect': region, 'sha256': digest(data),
                        'nonzero_words': sum(data[i:i + 2] != b'\0\0' for i in range(0, len(data), 2))})
            elif name.endswith('.sav'):
                paths = re.findall(rb'[A-Za-z]:[\\/][\x20-\x7e]*?\.(?:img|bin|cue)', raw[:1024], re.I)
                report['save_disc_paths'][name] = [p.decode().replace('\\\\', '\\') for p in paths]
            if args.fixtures and (name.endswith('_ram.bin') or name.endswith('_vram.bin')):
                with open(args.fixtures / Path(name).name, 'xb') as f:
                    f.write(raw)
        if vrams:
            report['vram_activity_words'] = sum(any(r[i:i + 2] != b'\0\0' for r in vrams) for i in range(0, len(vrams[0]), 2))
            report['vram_changed_words'] = sum(any(r[i:i + 2] != vrams[0][i:i + 2] for r in vrams[1:]) for i in range(0, len(vrams[0]), 2))
        report['limitations'] = ['No zero/unseen VRAM cell is certified free', 'Snapshots cannot establish temporal peak or upload ownership',
                                'Save-header image paths are provenance observations, not cryptographic runtime attestation']
    text = json.dumps(report, indent=2)
    if args.out:
        with open(args.out, 'x', encoding='utf-8') as f:
            f.write(text + '\n')
    else:
        print(text)
    for label, state in report['ram'].items():
        heap = state.get('heap', {})
        print(f"{label}: scene={state['scene']:#06x}, models={state['model_count']}, free={heap.get('free_bytes')}, largest={heap.get('largest_free')}")


def frame(command):
    data = command.encode('ascii')
    return b'$' + data + b'#' + f'{sum(data) & 255:02x}'.encode()


class RSP:
    """Small legacy-compatible allowlisted RSP client; no write/PC-setting API."""
    def __init__(self, sock):
        self.sock = sock

    def send(self, command):
        allowed = command in ('?', 'g', 'c', 'qSupported') or re.fullmatch(r'm[0-9a-f]+,[0-9a-f]+|[Zz]1,[0-9a-f]+,4', command)
        if not allowed:
            raise ValueError('Forbidden debugger command: ' + command)
        self.sock.sendall(frame(command))

    def packet(self, deadline=None):
        def byte():
            if deadline is not None:
                remaining=deadline-time.monotonic()
                if remaining<=0:
                    raise socket.timeout('Capture interval ended')
                self.sock.settimeout(remaining)
            value = self.sock.recv(1)
            if not value:
                raise EOFError('GDB disconnected')
            return value
        while byte() != b'$':
            pass  # ACKs are not response packets
        data = bytearray()
        while True:
            value = byte()
            if value == b'#':
                break
            data.extend(value)
            if len(data) > 65536:
                raise ValueError('Oversized debugger reply')
        checksum = byte() + byte()
        if sum(data) & 255 != int(checksum, 16):
            raise ValueError('Debugger reply checksum mismatch')
        self.sock.sendall(b'+')
        return bytes(data).decode('ascii')

    def request(self, command):
        self.send(command)
        return self.packet()

    def read(self, address, size):
        physical = address & 0x1FFFFFFF
        if size < 0 or not (physical + size <= RAM_SIZE or 0x1F800000 <= physical <= 0x1F800400 - size):
            raise ValueError('Only RAM/scratchpad reads allowed')
        output = bytearray()
        for offset in range(0, size, 512):
            length = min(512, size - offset)
            reply = self.request(f'm{address + offset:x},{length:x}')
            if len(reply) != length * 2:
                raise ValueError('Incomplete/unsupported memory response')
            output.extend(bytes.fromhex(reply))
        return bytes(output)

    def registers(self):
        reply = self.request('g')
        if len(reply) < 38 * 8:
            raise ValueError('Unsupported MIPS register layout')
        return struct.unpack('<38I', bytes.fromhex(reply[:38 * 8]))

    def breakpoint(self, address, add=True):
        if address % 4 or not 0x80010000 <= address < 0x80200000:
            raise ValueError('Invalid execution breakpoint')
        if self.request(f'{"Z" if add else "z"}1,{address:x},4') != 'OK':
            raise ValueError('Hardware execution breakpoints unavailable')

    def interrupt(self):
        self.sock.sendall(b'\x03')


class Output:
    def __init__(self, path, max_bytes):
        path.mkdir()  # exclusive new artifact directory
        self.path, self.limit, self.used = path, max_bytes, 0
        self.finalizing = False
        self.log = open(path / 'events.jsonl', 'x', encoding='utf-8')

    def claim(self, size):
        reserve = 0 if self.finalizing else RAM_SIZE + 65536
        if self.used + size > self.limit - reserve:
            raise ValueError('Capture output bound reached')
        self.used += size

    def emit(self, event):
        text = json.dumps(event, separators=(',', ':')) + '\n'
        self.claim(len(text.encode()))
        self.log.write(text)
        self.log.flush()

    def write(self, name, data):
        self.claim(len(data))
        with open(self.path / name, 'xb') as f:
            f.write(data)


def straight_line(instruction):
    op = instruction >> 26
    return op not in (1, 2, 3, 4, 5, 6, 7, 16, 17, 18, 19) and not (op == 0 and instruction & 63 in (8, 9, 12, 13))


def trace(args):
    if not 1 <= args.seconds <= 300 or not 1 <= args.max_events <= 10000 or not 8 <= args.max_mib <= 64:
        raise ValueError('Capture bounds outside supported range')
    exe = iso_file(args.source_image, 'SCUS_942.54')
    if exe[:8] != b'PS-X EXE':
        raise ValueError('Expected original US executable')
    base = struct.unpack_from('<I', exe, 24)[0]
    output = Output(args.out_dir, args.max_mib * 1024 * 1024)
    output.emit({'type': 'manifest', 'schema':'lck-native-runtime-v1', 'emulator_version_user_declared':args.emulator_version,
                 'source_path': str(args.source_image), 'source_sha256': file_digest(args.source_image),
                 'provenance': 'user-declared source; GDB cannot authenticate the mounted image',
                 'scope': 'bounded known native hooks, no instruction trace, no production approval'})
    owned, pending, gateway = set(), [], None
    paused, started, sequence = True, time.monotonic(), 0
    failure = None
    stats={'heap_samples':0,'max_allocated_payload_observed':0,'min_largest_free_observed':None}
    def observe(heap):
        if heap is None:return
        stats['heap_samples']+=1
        stats['max_allocated_payload_observed']=max(stats['max_allocated_payload_observed'],heap['allocated_bytes'])
        old=stats['min_largest_free_observed']
        stats['min_largest_free_observed']=heap['largest_free'] if old is None else min(old,heap['largest_free'])
    with socket.create_connection(('127.0.0.1', args.port), timeout=5) as sock:
        sock.settimeout(5)
        rsp = RSP(sock)
        try:
            rsp.request('?')
            for address in HOOKS:
                expected = exe[2048 + address - base:2048 + address - base + 4]
                if rsp.read(address, 4) != expected or not straight_line(struct.unpack('<I', expected)[0]):
                    raise ValueError(f'Unverified/branching hook prologue at {address:#x}')
            output.write('initial_ram.bin', rsp.read(RAM_BASE, RAM_SIZE))
            initial=ram_state(rsp.read);observe(initial.get('heap'));output.emit({'type': 'initial', 'state': initial})
            for address in HOOKS:
                rsp.breakpoint(address); owned.add(address)
            while sequence < args.max_events and time.monotonic() - started < args.seconds:
                rsp.send('c'); paused = False
                try:
                    rsp.packet(started + args.seconds)
                except socket.timeout:
                    rsp.interrupt(); sock.settimeout(5); rsp.packet()
                    paused = True
                    break
                paused = True; sock.settimeout(5)
                regs = rsp.registers(); pc, sp, ra = regs[37], regs[29], regs[31]
                if gateway and pc == gateway[0]:
                    rsp.breakpoint(pc, False); owned.remove(pc)
                    old = gateway[1]; rsp.breakpoint(old); owned.add(old); gateway = None
                    continue
                returned = next((p for p in reversed(pending) if p['ra'] == pc and p['sp'] == sp), None)
                if returned:
                    pending.remove(returned)
                    event = {'type': returned['hook'] + '_return', 'pair': returned['pair'], 'pc': pc, 'sp': sp,
                             'host_elapsed_s':round(time.monotonic()-started,6),
                             'v0': regs[2], 'request': returned['args'], 'scene': word(rsp.read, 0x80084540) & 65535,
                             'model_count': word(rsp.read, COUNT), 'arena': word(rsp.read, ARENA)}
                    if returned['hook'] in ('malloc', 'free', 'stage_init'):
                        try:
                            event['heap'] = heap_state(rsp.read)
                            observe(event['heap'])
                        except ValueError as e:
                            event['heap_error'] = str(e)
                    if returned['hook'] == 'register_model':
                        index=returned['index']
                        event['issued_index']=index
                        if index<200:event['registered_pointer']=word(rsp.read, REGISTRY+index*4)
                    output.emit(event); sequence += 1
                    if not any(p['ra'] == pc for p in pending) and pc not in HOOKS:
                        rsp.breakpoint(pc, False); owned.remove(pc)
                        continue
                    raise ValueError('Shared recursive return breakpoint requires unsupported stepping; capture stopped without a gap')
                if pc not in HOOKS:
                    raise ValueError(f'Unexpected stop at {pc:#x}; debugger/user intervention or unknown stop')
                hook = HOOKS[pc]; event = {'type': hook, 'pair': sequence, 'pc': pc, 'sp': sp, 'ra': ra,
                                         'host_elapsed_s':round(time.monotonic()-started,6),
                                         'gp':regs[28],
                                         'args': list(regs[4:8]), 'scene': word(rsp.read, 0x80084540) & 65535,
                                         'model_count': word(rsp.read, COUNT)}
                if hook in ('malloc', 'free', 'register_model', 'stage_init'):
                    try:
                        event['heap_before'] = heap_state(rsp.read)
                        observe(event['heap_before'])
                    except ValueError as e:
                        event['heap_error'] = str(e)
                    if ra in owned and ra not in HOOKS and not any(p['ra'] == ra for p in pending):
                        raise ValueError('Breakpoint ownership collision')
                    pending.append({'hook': hook, 'pair': sequence, 'ra': ra, 'sp': sp, 'args': list(regs[4:8]),'index':event['model_count']})
                    if ra not in owned:
                        rsp.breakpoint(ra); owned.add(ra)
                if hook in ('upload', 'move'):
                    rect = struct.unpack('<4h', rsp.read(regs[4], 8)); event['rect'] = rect
                    event['stp_latch']=word(rsp.read,0x8007B998)
                    event['order_scope']='CPU wrapper submission order, not DMA completion or unknown direct GPU commands'
                    x, y, w, h = rect
                    if not (0 <= x <= 1024 - w and 0 <= y <= 512 - h and w > 0 and h > 0):
                        raise ValueError('Unsupported native upload/copy rectangle')
                    if hook == 'upload':
                        data = rsp.read(regs[5], w * h * 2)
                        name = f'upload_{sequence:05d}.bin'; output.write(name, data)
                        event.update(data_file=name, sha256=digest(data))
                    else:event['destination']=[regs[5],regs[6]]
                if hook == 'register_model':
                    event['tmd_header'] = rsp.read(regs[4], 12).hex()
                output.emit(event); sequence += 1
                # 8153 has no RSP single-step. Cross only a verified nonbranch
                # prologue with an ephemeral +4 breakpoint, then re-arm entry.
                rsp.breakpoint(pc, False); owned.remove(pc)
                if pc + 4 in owned:
                    raise ValueError('Prologue gateway conflicts with another breakpoint')
                rsp.breakpoint(pc + 4); owned.add(pc + 4); gateway = (pc + 4, pc)
        except (Exception, KeyboardInterrupt) as e:
            failure = f'{type(e).__name__}: {e}'
        finally:
            cleanup_errors=[]
            output.finalizing = True
            cleanup_deadline=time.monotonic()+15
            if not paused:
                try:
                    rsp.interrupt(); sock.settimeout(5); rsp.packet(); paused=True
                except Exception as e:cleanup_errors.append(str(e))
            for address in list(owned):
                if time.monotonic()>=cleanup_deadline:
                    cleanup_errors.append('Breakpoint cleanup deadline exceeded');break
                try:
                    sock.settimeout(max(0.1,min(2,cleanup_deadline-time.monotonic())))
                    rsp.breakpoint(address,False);owned.remove(address)
                except Exception as e:cleanup_errors.append(f'{address:#x}: {e}')
            if cleanup_errors:failure=failure or 'Incomplete breakpoint cleanup: '+'; '.join(cleanup_errors)
            try:
                sock.settimeout(5)
                if paused:output.write('final_ram.bin', rsp.read(RAM_BASE, RAM_SIZE))
                output.emit({'type': 'final', 'state': ram_state(rsp.read) if paused else None, 'events': sequence,
                             'unmatched_returns': pending, 'failure': failure, 'production_approved': False,
                             'breakpoints_remaining':sorted(owned),'paused':paused,
                             'sampled_statistics':stats,
                             'coverage': 'known hooks within bounded interval only; not runtime acceptance'})
            except Exception as e:
                print('Cleanup/capture incomplete:', e)
                failure = failure or f'Cleanup/capture incomplete: {e}'
            output.log.close()
    if failure:
        raise ValueError(failure)
    print(f'Captured {sequence} bounded events; emulator left paused. No guest writes or image output.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='mode', required=True)
    p = commands.add_parser('analyze')
    p.add_argument('archive', type=Path); p.add_argument('--source-image', type=Path)
    p.add_argument('--comparison-image', type=Path); p.add_argument('--map-entry', type=int, default=1)
    p.add_argument('--out', type=Path); p.add_argument('--fixtures', type=Path)
    p = commands.add_parser('trace')
    p.add_argument('--source-image', type=Path, required=True); p.add_argument('--out-dir', type=Path, required=True)
    p.add_argument('--port', type=int, default=1234); p.add_argument('--seconds', type=int, default=120)
    p.add_argument('--max-events', type=int, default=4000); p.add_argument('--max-mib', type=int, default=16)
    p.add_argument('--emulator-version', default='not independently attested')
    args = parser.parse_args()
    try:
        (analyze if args.mode == 'analyze' else trace)(args)
    except (OSError, ValueError, zipfile.BadZipFile) as e:
        parser.exit(1, str(e) + '\n')


if __name__ == '__main__':
    main()
