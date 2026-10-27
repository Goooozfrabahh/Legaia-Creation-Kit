"""No emulator connections: codec, bounds and heap verification only."""
import unittest
import struct
import json
import tempfile
import argparse
from pathlib import Path
from unittest.mock import patch
from lck_runtime_capture import RSP, frame, heap_state, ram_reader, straight_line, region_bytes


class FakeSocket:
    def __init__(self, reply=b''):
        self.reply=bytearray(reply);self.sent=[]
    def recv(self, n):
        value=bytes(self.reply[:n]);del self.reply[:n];return value
    def sendall(self, data):
        self.sent.append(data)
    def settimeout(self, value):
        pass


class CaptureTests(unittest.TestCase):
    def test_bounded_hook_pairs_gateways_and_cleanup_without_any_emulator(self):
        import lck_runtime_capture as module
        ram=bytearray(0x200000)
        root,head=0x80091800,0x801C5FE8
        def put(a,*v):struct.pack_into('<'+'I'*len(v),ram,a-0x80000000,*v)
        put(0x8007BB58,root);put(root,head,2);put(head,head,head,0);put(head-12,head-12,head-12,0)
        put(head+12,root+8,root+8,0);put(root+8,head+12,head+12,(head-12)-(root+20));put(0x80084540,3)
        exe=bytearray(0x70000);exe[:8]=b'PS-X EXE';struct.pack_into('<I',exe,24,0x80010000)
        for address in module.HOOKS:
            put(address,0x27BDFFD0);struct.pack_into('<I',exe,2048+address-0x80010000,0x27BDFFD0)
        struct.pack_into('<4h',ram,0x1000,880,64,4,4)
        stops=[0x80017888,0x8001788C,0x80020000,0x80017B94,0x80017B98,0x80020000,0x800583C8,0x800583CC,0x80026B4C]
        class Net:
            def __enter__(self):return self
            def __exit__(self,*a):pass
            def settimeout(self,v):pass
        class Device:
            instance=None
            def __init__(self,sock):self.pc=0x80010000;self.owned=set();self.commands=[];Device.instance=self
            def request(self,c):self.commands.append(c);assert c=='?';return 'S02'
            def read(self,a,n):return ram_reader(ram)(a,n)
            def registers(self):
                r=[0]*38;r[37]=self.pc;r[29]=0x801FF000;r[31]=0x80020000;r[2]=0x800A0000;r[28]=0x8007B318
                r[4]=0x80001000;r[5]=0x80002000
                return r
            def breakpoint(self,a,add=True):
                if add:self.owned.add(a)
                else:self.owned.remove(a)
            def send(self,c):
                self.commands.append(c);assert c=='c';self.pc=stops.pop(0)
            def packet(self,*args):return 'S02'
            def interrupt(self):pass
        target=(Path(__file__).resolve().parents[1]/'target').resolve();target.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=target,prefix='lck_capture_mock_') as tmp:
            self.assertTrue(Path(tmp).resolve().is_relative_to(target))
            source=Path(tmp)/'source.img';source.write_bytes(b'fixture, no disc image')
            args=argparse.Namespace(source_image=source,out_dir=Path(tmp)/'capture',max_mib=8,seconds=30,max_events=6,port=1234,emulator_version='mock')
            with patch.object(module,'iso_file',return_value=bytes(exe)),patch.object(module.socket,'create_connection',return_value=Net()),patch.object(module,'RSP',Device):
                module.trace(args)
            events=[json.loads(line) for line in (args.out_dir/'events.jsonl').read_text().splitlines()]
            self.assertEqual([e['type'] for e in events if e['type'].endswith('_return')],['malloc_return','free_return'])
            self.assertEqual(events[-1]['events'],6);self.assertFalse(events[-1]['production_approved']);self.assertEqual(Device.instance.owned,set())
            self.assertEqual({c for c in Device.instance.commands},{'?','c'})
            self.assertLess(sum(p.stat().st_size for p in args.out_dir.iterdir()),8*1024*1024)
            stops[:]=[0x80012340];args.out_dir=Path(tmp)/'failed';args.max_events=1
            with patch.object(module,'iso_file',return_value=bytes(exe)),patch.object(module.socket,'create_connection',return_value=Net()),patch.object(module,'RSP',Device):
                with self.assertRaisesRegex(ValueError,'Unexpected stop'):module.trace(args)
            self.assertEqual(Device.instance.owned,set())
            final=json.loads((args.out_dir/'events.jsonl').read_text().splitlines()[-1]);self.assertFalse(final['production_approved']);self.assertIn('Unexpected stop',final['failure'])
    def test_rsp_legacy_checksum_and_no_guest_write_or_step_commands(self):
        sock=FakeSocket(b'+'+frame('S02'));client=RSP(sock)
        self.assertEqual(client.request('?'),'S02')
        for command in ['M80010000,1:ff','G00000000','P1=00000000','s','c80010000','Z0,80017888,4','qDuckStation:input']:
            with self.assertRaises(ValueError):client.send(command)
        self.assertEqual(sock.sent,[frame('?'),b'+'])
        with self.assertRaises(ValueError):RSP(FakeSocket(b'$S02#00')).packet()

    def test_legacy_registers_and_bounded_reads(self):
        raw=struct.pack('<73I',*range(73)).hex()
        client=RSP(FakeSocket(frame(raw)))
        self.assertEqual(client.registers()[37],37)
        sock=FakeSocket(frame('00112233'));self.assertEqual(RSP(sock).read(0x80000000,4),b'\0\x11\x22\x33')
        with self.assertRaises(ValueError):RSP(sock).read(0x1F801810,4)
        with self.assertRaises(ValueError):RSP(sock).read(0x801FFFFF,2)

    def test_no_single_step_needed_only_straight_prologue_gateway(self):
        self.assertTrue(straight_line(0x27BDFFD0))  # addiu sp,sp,-0x30
        for instruction in [0x03E00008,0x0C000000,0x10000000,0x0000000C]:
            self.assertFalse(straight_line(instruction))

    def test_heap_backlinks_bounds_and_snapshot_scope(self):
        raw=bytearray(0x200000)
        def put(a,*v):struct.pack_into('<'+'I'*len(v),raw,a-0x80000000,*v)
        root,head=0x80091800,0x801C5FE8
        put(0x8007BB58,root);put(root,head,2)
        put(head,head,head,0);put(head-12,head-12,head-12,0)
        put(head+12,root+8,root+8,0)
        size=(head-12)-(root+8+12);put(root+8,head+12,head+12,size)
        state=heap_state(ram_reader(raw));self.assertEqual(state['largest_free'],size)
        self.assertEqual(len(state['free']),1);self.assertIn('one snapshot',state['scope'])
        put(root+12,0)
        with self.assertRaises(ValueError):heap_state(ram_reader(raw))

    def test_vram_regions_cannot_wrap_or_accept_wrong_size(self):
        raw=bytes(1024*512*2)
        self.assertEqual(len(region_bytes(raw,(880,64,16,64))),2048)
        with self.assertRaises(ValueError):region_bytes(raw,(1020,0,16,1))
        with self.assertRaises(ValueError):region_bytes(raw[:4],(0,0,1,1))


if __name__=='__main__':unittest.main()
