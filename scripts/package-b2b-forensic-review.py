"""Read-only A/B/C/M forensic packaging; no exporter/game/emulator invocation.
Usage: python scripts/package-b2b-forensic-review.py NEW_LOCAL_FOLDER NEW_REPO_FOLDER
Uses preserved 2026-10-31 fixture manifests; creates reports only, never IMGs.
"""
import csv
import hashlib
import json
from pathlib import Path
import runpy
import shutil
import struct
import sys

workspace = Path('D:/AstraRimElmProject')
old = workspace/'ManualTests/LCK_B2B_Loading_20261031_01'
combined = workspace/'ManualTests/LCK_B2B_Combined_20261031_01'
local, publish = map(Path, sys.argv[1:3])
assert not local.exists() and not publish.exists(), 'Create-only output folders required'
local.mkdir(parents=True)
audit = runpy.run_path(str(Path(__file__).with_name('audit-native-scene.py')))
Disc, inspect, decode, word = [audit[n] for n in ('Disc', 'inspect', 'decode', 'word')]
sha = lambda b: hashlib.sha256(b).hexdigest()


def load(path):
    return json.loads(Path(path).read_text(encoding='utf-8-sig'))


def file_sha(path):
    with open(path, 'rb') as f:
        h = hashlib.sha256()
        while b := f.read(1024*1024):
            h.update(b)
    return h.hexdigest()


isolation = load(old/'ISOLATION_REPORT.json')
matrix = load(combined/'Variants_Final/COMBINED_REPORT.json')
m = next(c for c in matrix['cases'] if c['case'].startswith('M_'))
sources = {c:isolation['cases'][i] for c,i in [('A',0),('B',1),('C',2)]}
sources['M'] = m
retail = Disc(sources['A']['path'])
valid_ids = [i for i in range(len(retail.starts)-1)
             if 0 < retail.starts[i] < retail.starts[i+1] <= retail.total]
baseline = {i:(len(b:=retail.entry(i)),sha(b)) for i in valid_ids}
baseline_prefix = retail.sectors(retail.prot, retail.starts[0])
rows, comparisons, detailed = [], {}, {}


def row(case, kind, identity, prot='', relative='', absolute='', offset='', size='', digest='', native='', detail=''):
    rows.append([case,kind,identity,prot,relative,absolute,offset,size,digest,native,detail or '-'])


def ranges(items):
    result = []
    for index, delta in items:
        if result and result[-1][1]+1 == index and result[-1][2] == delta:
            result[-1][1] = index
        else:
            result.append([index,index,delta])
    return result


for case, source in sources.items():
    path = source['path']
    assert file_sha(path) == source['sha256'], f'Preserved image changed: {case}'
    disc = Disc(path)
    assert len(disc.starts) == len(retail.starts)
    manifest = inspect(path, 1)
    changed, all_entries = [], []
    for i in valid_ids:
        payload = disc.entry(i)
        record = dict(entry=i,old_relative_lba=retail.starts[i],new_relative_lba=disc.starts[i],
                      old_disc_lba=retail.prot+retail.starts[i],new_disc_lba=disc.prot+disc.starts[i],
                      old_bytes=baseline[i][0],new_bytes=len(payload),old_sha256=baseline[i][1],new_sha256=sha(payload))
        all_entries.append(record)
        if (len(payload),record['new_sha256']) != baseline[i]:
            changed.append(record)
            row(case,'CHANGED_PROT',i,i,disc.starts[i],disc.prot+disc.starts[i],0,len(payload),record['new_sha256'],detail='retail_sha256='+baseline[i][1])
    bundle, pac = disc.entry(4), disc.entry(5)
    base = next(i for i in (0,4) if 0 < word(bundle,i) <= 32 and word(bundle,i+12) == 8+8*word(bundle,i))
    raw, events, persistent, peak = {}, [], 0, 0
    for d in manifest['descriptors']:
        kind, size = d['type'], d['size']
        row(case,'DESCRIPTOR',d['slot'],4,disc.starts[4],disc.prot+disc.starts[4],d['source_offset'],size,d.get('decoded_sha256',''),kind,'order='+str(d['slot']))
        if kind not in (10,15,20):
            raw[kind] = decode(bundle[d['source_offset']:],size)[0]
        destination = (size+3)//4*4 if 2 <= kind <= 9 else 0
        assert kind in (2,3,5,6,7,20), 'New descriptor adapter needed; do not guess allocation semantics'
        scratch = 4116 if destination else 0
        peak = max(peak,persistent+destination+scratch)
        events.append(dict(carrier=f'bundle descriptor {d["slot"]}',type=kind,decoded_bytes=size if destination else 0,
                           persistent_destination_bytes=destination,lzs_temporary_scratch_bytes=scratch,
                           persistent_before=persistent,accounted_peak=persistent+destination+scratch))
        persistent += destination
    at, streams = 0, []
    while (tag:=word(pac,at)) & 0xffffff:
        kind, size = tag >> 24, tag & 0xffffff
        assert kind == 1, 'New PAC lifetime adapter needed'
        body = pac[at+4:at+4+size]
        streams.append(dict(offset=at,type=kind,bytes=size,sha256=sha(body),tim_members=word(body,0),
                            allocation_bytes=0,lifetime='copy-only native mode 0x14: source arena; GPU completion unverified'))
        row(case,'PAC_CHUNK',len(streams)-1,5,disc.starts[5],disc.prot+disc.starts[5],at,size,sha(body),kind,'TIM members='+str(word(body,0)))
        at += 4+(size & ~3)
    model_count = word(raw[2],0)
    model_offsets = [word(raw[2],4+4*i)*4 for i in range(model_count)]+[len(raw[2])]
    models = []
    for i in range(model_count):
        b = raw[2][model_offsets[i]:model_offsets[i+1]]
        slot = i+5
        models.append(dict(scene_selector=i,global_slot=slot,source_offset=model_offsets[i],bytes=len(b),sha256=sha(b),objects=word(b,8)))
        row(case,'MODEL',i,4,disc.starts[4],disc.prot+disc.starts[4],model_offsets[i],len(b),sha(b),slot,'decoded type2 member; pointer_table='+hex(0x8007c018+4*slot)+'; objects='+str(word(b,8)))
    count = word(raw[5],0)
    offsets = [word(raw[5],4+4*i) for i in range(count)]+[len(raw[5])]
    clips = []
    for i in range(count):
        b = raw[5][offsets[i]:offsets[i+1]]
        bones,gate,frames,marker,rate = struct.unpack_from('<BBHHH',b)
        clips.append(dict(id=i+1,offset=offsets[i],bytes=len(b),sha256=sha(b),bones=bones,gate=gate,frames=frames,marker=marker,rate_word=rate))
        row(case,'ANM_CLIP',i,4,disc.starts[4],disc.prot+disc.starts[4],offsets[i],len(b),sha(b),i+1,f'decoded bank-relative offset; bones={bones}; frames={frames}; marker={marker:#x}; rate_word={rate}')
    ledger = source['native_allocations'] if case != 'M' else source['native']['resource_allocations']
    assert [a['size'] for a in ledger if a['action']=='allocate'] == [e['persistent_destination_bytes'] for e in events if e['persistent_destination_bytes']]
    if case in ('A','B','C'):
        assert persistent == source['runtime_profile']['persistent_destination_requests'] and peak == source['runtime_profile']['peak_accounted_requests']
    if case in ('B','C','M'):
        native = next(c['native'] for c in matrix['cases'] if c['case'] == source['case'])
        assert native['models'] == [[a['global_slot'],a['sha256']] for a in models]
    else:
        native = dict(scope='A independently parsed here; original SCUS allocation trace retained in isolation report')
    row(case,'IMG',Path(path).name,size=Path(path).stat().st_size,digest=source['sha256'],detail='user result: '+('FREEZES' if case=='M' else 'LOADS'))
    for index,name in [(4,'BUNDLE'),(5,'PAC')]:
        b = bundle if index == 4 else pac
        row(case,name,index,index,disc.starts[index],disc.prot+disc.starts[index],0,len(b),sha(b))
    prefix = disc.sectors(disc.prot,disc.starts[0])
    prefix_words = [(i//4-4,word(prefix,i)-word(baseline_prefix,i)) for i in range(0,len(prefix),4) if prefix[i:i+4] != baseline_prefix[i:i+4]]
    assert all(0 <= i < len(disc.starts) for i,_ in prefix_words), 'Non-TOC boot/prefix bytes changed'
    prefix_ranges = ranges(prefix_words)
    shifts = ranges([(i,disc.starts[i]-retail.starts[i]) for i in valid_ids if disc.starts[i] != retail.starts[i]])
    detailed[case] = all_entries
    comparisons[case] = dict(image=path,image_bytes=Path(path).stat().st_size,image_sha256=source['sha256'],
        user_reported_loading='FREEZES' if case=='M' else 'LOADS',prot_disc_lba=disc.prot,prot_sectors=disc.total,
        changed_prot_entries=changed,shifted_prot_entry_ranges=shifts,
        prot_prefix_bytes=len(prefix),prot_prefix_sha256=sha(prefix),retail_prot_prefix_sha256=sha(baseline_prefix),
        prefix_changes_only_TOC_start_or_bound_words=True,prefix_changed_TOC_ranges=prefix_ranges,
        old_post_PROT_disc_sector_range=[retail.prot+retail.total,Path(sources['A']['path']).stat().st_size//2352-1],
        downstream_disc_sector_delta=(Path(path).stat().st_size-Path(sources['A']['path']).stat().st_size)//2352,
        bundle_bytes=len(bundle),bundle_sha256=sha(bundle),pac_bytes=len(pac),pac_sha256=sha(pac),descriptors=manifest['descriptors'],
        man_counts=manifest['man_partition_counts'],models=models,anm_bank_sha256=sha(raw[5]),clips=clips,pac_chunks=streams,
        native_request_trace=ledger,allocation_events=events,persistent_bytes=persistent,peak_accounted_requests=peak,
        fixed_encoded_arena_bytes=404480,encoded_max_actual_bytes=max(len(bundle),len(pac)),
        peak_with_fixed_arena=peak+404480,retail_heap_arena_bytes=1263616,
        remainder_before_unknown_costs=1263616-peak-404480,
        new_import_actor_allocations=native.get('construction_allocations',[]),native_scope=native['scope'],
        allocation_limitations=['Native trace substitutes heap/LZS/GPU: decoder scratch is modelled separately, not observed in that trace',
            'Existing retail actors still allocate in M; imported-actor allocation count zero is not total actor cost zero',
            'Resident resources, scene cleanup, sidecars, GPU packets, audio, heap headers, fragmentation and future allocations omitted',
            'No current live allocation failure proven; source arena is reused, not a sum of bundle plus PAC'],live_heap_failure_proven=False)

details = local/'ALL_PROT_ENTRY_LAYOUT_AND_HASHES.json'
details.write_text(json.dumps(detailed,indent=2),encoding='utf-8')
evidence_paths = [old/'ISOLATION_REPORT.json',combined/'Variants_Final/COMBINED_REPORT.json',combined/'UserCapture/STATE_REPORT.json',
    combined/'CAPTURE_HEAP_LAYOUT.json',combined/'EXACT_SHARED_RESOURCE_COMPARE.json',combined/'ORDERED_TIM_TAILS.json',
    combined/'FINAL_SECTOR_VALIDATION.json',workspace/'ManualTests/LCK_B2B_Heap_20261031_01/HISTORICAL_HEAP_SUMMARY.json',
    workspace/'ManualTests/LCK_B2B_Combined_20261031_Regression_01/REGRESSION_RESULTS.json',
    workspace/'b2b-combined-final-matrix.log']
inventory = [dict(path=str(p),bytes=p.stat().st_size,sha256=file_sha(p)) for p in evidence_paths]
runtime_log = Path('D:/Emulators/Ps1/Duckstation/duckstation.log')
try:
    log = runtime_log.read_bytes()
    (local/'duckstation-log-readonly-snapshot.log').write_bytes(log)
    runtime = dict(path=str(runtime_log),bytes=len(log),sha256=sha(log),
                   scope='read-only file snapshot; not emulator interaction',
                   snapshot_path=str(local/'duckstation-log-readonly-snapshot.log'),
                   extract=[s for s in log.decode('utf-8',errors='replace').splitlines() if any(t in s.lower() for t in ('i/system: version:','boot filename:','loading cd image','map name town01','map read','map patch','decode','out read','tmds ','camera set','anm set','man set','town mode','game mode','malloc err','instruction read failed','failed to read block'))][-32:])
except (OSError,PermissionError) as exc:
    runtime = dict(path=str(runtime_log),available=False,reason=type(exc).__name__+': '+str(exc))
capture = load(combined/'UserCapture/STATE_REPORT.json')
data = dict(schema=1,source_checkpoint='e59873c30dccc1d3d84be9a2f5b4ae76a6640799',scope='Read-only packaging only; no fix, IMG generation or new gameplay claim',
            cases=comparisons,evidence_inventory=inventory,runtime_log=runtime,
            detailed_entry_inventory=dict(path=str(details),bytes=details.stat().st_size,sha256=file_sha(details)),
            saved_state_compact={k:capture[k] for k in ('source_sha256','save_version','media','pc','npc','resident_scene_display_name','man_counts','native_model_count','failed_actor_spawns','transition_actors','native_read','cdrom','cached_TOC_first_4096_bytes_match_target','pending_buffer_first_32_bytes_match_target')})
(local/'NATIVE_ALLOCATION_COMPARISON.json').write_text(json.dumps(data,indent=2),encoding='utf-8')
with (local/'A_B_C_M_RESOURCE_COMPARISON.tsv').open('w',encoding='utf-8',newline='') as f:
    writer=csv.writer(f,delimiter='\t');writer.writerow(['case','record_kind','identity','prot_entry','relative_lba','disc_lba','byte_offset','bytes','sha256','native_slot_or_id','detail']);writer.writerows(rows)

table = '| Case | User result | Scene/global models | ANM clips | P1 actors | Bundle/PAC bytes | Persistent requests | Peak + fixed arena |\n|---|---|---|---|---|---|---|---|\n'
for c,v in comparisons.items():
    table += f'| {c} | {v["user_reported_loading"]} | {len(v["models"])}/{len(v["models"])+5} | {len(v["clips"])} | {v["man_counts"][1]} | {v["bundle_bytes"]}/{v["pac_bytes"]} | {v["persistent_bytes"]} | {v["peak_with_fixed_arena"]} |\n'
summary = '''# B2B forensic review — A/B/C load; M freezes

This package contains source/evidence synchronization only. No exporter/runtime fix,
replacement IMG, GUI/emulator operation, production merge or Latest Build installation.
User confirms all combined-resource variants freeze, including M with no imported actors.
That rules out imported actor instantiation as a necessary trigger, not all existing actor or renderer activity.

Source checkpoint: `e59873c30dccc1d3d84be9a2f5b4ae76a6640799`.
History: `41ec9d8` -> `a29b784` -> `34bb350` -> `e5eefff` -> `e59873c`.
Full resource identities/decoded offsets/hashes are in the TSV and allocation JSON.

'''+table+'''
The first differing native allocation in M is still the FIRST type-2 model-pack
allocation: 320284 bytes versus B310900/C313500. There is no new sixth malloc;
the five persistent dispatcher calls retain the same order. M's first additional
model registration is global slot120 (scene selector115), after Balden at119.
M retains retail MAN/P1 count53; B/C each have54. M has both added clips70/71
and both TIM packs, without either new actor record. The ANM allocation grows
inside the existing type-5 call. These are structural/instruction differences,
NOT identified runtime failure points.

M has 477076 accounted persistent request bytes, only 88 fewer than E because
its MAN is retail. M freezing materially strengthens investigation of resource
loading, cumulative registration/material handling, transition/CD and live heap
lifetimes. It does not establish which subsystem failed or an allocator failure.

The prior cumulative native simulation passed model registration, pointer fixups
and imported construction on one CPU, with substituted heap/LZS/GPU/GTE/RNG.
The saved E snapshot shows an earlier pending read, not a reproduced hang site.
No combined-resource runtime success or fixed-freeze claim exists.

## Reviewer entry points

- `crates/studio/src/app/native_animation.rs`: closure, pack merge, MAN/ANM writer.
- `crates/studio/src/app/scene_registration.rs`: native resource registration/lifetimes.
- `crates/studio/src/app/native_loader_trace_tests.rs`: original loader/registrar simulation.
- `crates/studio/src/app/native_animation_combined_tests.rs`: M/F/G/H/I construction and generation provenance.
- `crates/studio/src/app/native_load_budget.rs`: exact accounting rules and omissions.
- `scripts/replay-native-heap.rs`: original allocator on historical snapshots, not current live proof.
- `scripts/inspect-duckstation-state77.py`: offline capture decoder, explicitly v77 only.

Existing automated checks: 324 Studio tests, 89 code-hook tests, 28 opt-in regressions,
combined instruction matrix and five diagnostic decoder/sector checks passed.
Those results are preserved historical automated evidence; this packaging task
does not rerun gameplay or certify native playback. See runtime limitations.
'''
(local/'B2B_FORENSIC_SUMMARY.md').write_text(summary,encoding='utf-8')
f_allocation_error = any('F_REVERSE_MODELS.img' in s for s in runtime.get('extract',[])) and any('malloc err size 8368' in s for s in runtime.get('extract',[]))
if f_allocation_error:
    with (local/'B2B_FORENSIC_SUMMARY.md').open('a',encoding='utf-8') as f:
        f.write('''
## Additional observed runtime evidence: F log, not M

The readable DuckStation log identifies `F_REVERSE_MODELS.img`. It reports
`tmds 121`, `anm set`, `man set`, then `malloc err size 8368` at26.6251 seconds,
followed by instruction-read failures at `0x000000A0` and `0x80000080`.
This is a direct runtime allocation-error message from F, not an inference from
static totals. It does not identify the allocation caller, prove the same failed
request in M, or prove the complete causal chain to the application hang.
The full log remains local; its exact path, hash and scoped extract are in the
allocation JSON and runtime report. No fix was implemented in this task.
''')
layout = '# PROT and disc layout differences\n\nExtraction-space entry identities are retained. Sizes are true next-start footprints, not the historical overread window.\n\n'
for c,v in comparisons.items():
    layout += f'## {c}\n\nIMG SHA256 `{v["image_sha256"]}`; bytes {v["image_bytes"]}. PROT starts at disc LBA {v["prot_disc_lba"]}, sectors {v["prot_sectors"]}.\n\n'
    layout += f'Changed decoded-user-data entry identities: {[e["entry"] for e in v["changed_prot_entries"]]}. PROT prefix (TOC/boot region) SHA256 `{v["prot_prefix_sha256"]}`; retail `{v["retail_prot_prefix_sha256"]}`.\n\n'
    for e in v['changed_prot_entries']:
        layout += f'- Entry{e["entry"]}: relative LBA {e["old_relative_lba"]}->{e["new_relative_lba"]}, disc LBA {e["old_disc_lba"]}->{e["new_disc_lba"]}, bytes {e["old_bytes"]}->{e["new_bytes"]}; old SHA `{e["old_sha256"]}`, new `{e["new_sha256"]}`.\n'
    layout += '\nExact shifted entry ranges (inclusive; constant relative/disc LBA delta; unchanged entries retain exact user-data hashes):\n\n'
    layout += '\n'.join(f'- Entries {lo}..{hi}: {delta:+d} sectors.' for lo,hi,delta in v['shifted_prot_entry_ranges']) or '- None.'
    layout += '\n\nPrefix differences are ONLY TOC start/bound words at byte `16 + 4*i`; all other boot/prefix bytes are exact retail. Lossless changed TOC ranges:\n\n'
    layout += '\n'.join(f'- TOC extraction indices {lo}..{hi}: {delta:+d}; index1233 is the terminal end bound.' for lo,hi,delta in v['prefix_changed_TOC_ranges']) or '- None.'
    layout += f'\n\nOriginal post-PROT disc sectors {v["old_post_PROT_disc_sector_range"]} move by {v["downstream_disc_sector_delta"]:+d}; ISO extents/volume size and raw MSF headers follow that same whole-sector growth.\n'
    layout += f'\n\nBundle SHA `{v["bundle_sha256"]}`; PAC SHA `{v["pac_sha256"]}`. Descriptor order `{[d["type"] for d in v["descriptors"]]}`.\n\n'
layout += f'''## Detailed inventory and physical-sector caveat

All {len(valid_ids)} valid extraction entries were independently compared for each case.
Full exact old/new entry LBAs, sizes and hashes remain local at `{details}`,
SHA256 `{file_sha(details)}` ({details.stat().st_size} bytes). The compact JSON
contains every changed entry and lossless constant-delta ranges for moved entries.
PROT growth also cascades downstream ISO extents/MSF headers; large physical raw
sector differences do not mean those resource payloads changed. Existing sector
validation manifests are hash-indexed in the JSON; no IMG is committed.
The bundle starts at disc LBA485 in all four cases. Its first raw sector header
is BCD `00:08:35`, mode2. Native pending E seek uses that same target.
'''
(local/'PROT_AND_DISC_LAYOUT_DIFF.md').write_text(layout,encoding='utf-8')
runtime_md = '''# Runtime evidence and limitations

## User-reported results

A retail control LOADS; B Balden alone LOADS; C Bubu1 alone LOADS.
M both resource sets with no imported actors FREEZES. User also confirms every
other combined-resource variant freezes. DuckStation itself becomes unresponsive;
another capture could not be made. Loading alone is not verified animation,
dialogue, behavior preservation, or broad gameplay compatibility.

## Saved-state observation (E, not M)

The preserved user-created v77 state identifies E as its media, SHA256
`4e58c04fbda95bac7b7498bf2bb0a2ee652903eb6bf2d237642f846a67b79513`.
Decoded offline: PC `0x8005FD40`, RA `0x8005FC1C` in the wait/VSync path;
resident MAN `Mt. Dhini`, counts `[3,14,19]`, 77 models, failed-spawn counter0;
transition actor `0x8008077C`, handler `0x80021934`, state3. It polls CD-sync.
Cached first4KiB of PROT TOC match E. Pending raw TOC6/extraction4 read requests
113 sectors with113 remaining; CD state `SeekingLogical`, source internal LBA28890,
target635, setloc binary `[0,8,35]`. The target buffer is not yet the new bundle.
The actual arena is404480 bytes and the pending bundle231424, so that read fits.
Old-scene free heap214920 bytes in two blocks is PRE-CLEANUP, not a target heap test.

This snapshot may precede the actual application hang. It cannot prove a persistent
seek failure, a native allocation failure, or the final frozen instruction. There
is no M runtime capture. No new capture was requested after the user reported
that the application itself was unresponsive. No assistant host interaction occurred.

## Allocation and lifetime boundaries

The JSON retains the original-instruction request chronology (synthetic pointers
are NOT actual PS1 addresses), plus independent descriptor accounting. Five
persistent calls load type2/3/5/6/7. Each compressed destination additionally uses
4116 bytes of temporary decoder scratch, freed on decoder return; that scratch
is omitted by the simulator's substituted decompressor and therefore added only
in the accounting model. Copy-only PAC TIM lists use the source arena; their
zero destination-allocation entry does not certify GPU completion or residency.
The fixed encoded arena is404480 bytes, retail heap1263616. Native heap headers,
fragmentation, existing actors, cleanup, resident resources, sidecars, rendering,
audio, DMA/GPU packets and future allocations are not fully accounted.
Do not label remaining static arithmetic as available live memory.

Historical heap replay succeeded for A/B/C/E on one older arena-return snapshot;
all failed the first model allocation on another older preload snapshot, INCLUDING
retail A. M has not been replayed on a live M heap. This neither identifies nor
rules out a current cumulative-memory/lifetime defect. Source provenance remains
distinct from user gameplay results and from bounded original-instruction tests.

## Retained detailed evidence

No retail binary, IMG, save state, RAM, local settings or full logs are committed.
The source files below remain unchanged; JSON contains compact native identities,
allocation traces and the saved-state extract needed to interpret this package.

'''
for e in inventory:
    runtime_md += f'- `{e["path"]}` — {e["bytes"]} bytes; SHA256 `{e["sha256"]}`.\n'
runtime_md += '\n## Existing emulator log\n\n'+json.dumps(runtime,indent=2)+'\n'
if f_allocation_error:
    runtime_md += '\nThis log is explicitly F, not M or the earlier E saved state. The logged 8368-byte allocation error is observed runtime evidence; its caller and M equivalence remain unproven. It supersedes treating allocation failure as purely hypothetical for F, but does not certify a fix or explain every combined-resource failure.\n'
(local/'RUNTIME_EVIDENCE_AND_LIMITATIONS.md').write_text(runtime_md,encoding='utf-8')
names = ['B2B_FORENSIC_SUMMARY.md','A_B_C_M_RESOURCE_COMPARISON.tsv','NATIVE_ALLOCATION_COMPARISON.json','PROT_AND_DISC_LAYOUT_DIFF.md','RUNTIME_EVIDENCE_AND_LIMITATIONS.md']
publish.mkdir(parents=True)
for name in names:
    shutil.copyfile(local/name,publish/name)
print(json.dumps(dict(files=[dict(name=n,bytes=(publish/n).stat().st_size,sha256=file_sha(publish/n)) for n in names],changed_entries={c:[e['entry'] for e in v['changed_prot_entries']] for c,v in comparisons.items()},totals={c:(v['persistent_bytes'],v['peak_accounted_requests']) for c,v in comparisons.items()}),indent=2))
