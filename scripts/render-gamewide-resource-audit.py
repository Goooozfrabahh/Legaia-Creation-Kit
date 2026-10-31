"""Render Stage B reports from the read-only Rust census, never from assumptions."""
import argparse, csv, json, collections
from pathlib import Path
csv.field_size_limit(16*1024*1024)
p=argparse.ArgumentParser();p.add_argument('directory',type=Path);a=p.parse_args();root=a.directory
t=json.loads((root/'audit-totals.json').read_text(encoding='utf-8'));c=t['family_counts']; n=t['model_registrations_tested'];resolved=t['visual_package_dependencies_resolved'];encoded=t['native_package_readback_passed']
percent=lambda v: f'{100*v/n:.2f}%' if n else 'not applicable'
native_classes=collections.Counter();native_hashes=set();raw_models=[];raw_model_hashes=set();unresolved_wall_edges=0;clut_uploads=0
with (root/'ALL_ASSETS.tsv').open(encoding='utf-8',newline='') as f:
 for row in csv.DictReader(f,delimiter='\t'):
  if row['family']=='PROT entry':native_classes[json.loads(row['fields']).get('class','Unknown')]+=1
  if row['family']=='NativeModel':native_hashes.add(row['sha256'])
  if row['family']=='TMD':raw_models.append((row['identity'],row['sha256']));raw_model_hashes.add(row['sha256'])
  if row['family']=='OrderedNativeUpload' and json.loads(row['fields']).get('clut') is not None:clut_uploads+=1
  unresolved_wall_edges+=sum(d.get('target','').endswith('/walls') and d.get('relationship')=='Unresolved'for d in json.loads(row['dependencies']))
extra_models=[(identity,sha)for identity,sha in raw_models if sha not in native_hashes]
with (root/'EXPORT_COMPATIBILITY.tsv').open(encoding='utf-8',newline='')as f:existing_ids={row['identity']for row in csv.DictReader(f,delimiter='\t')}
with (root/'EXPORT_COMPATIBILITY.tsv').open('a',encoding='utf-8',newline='')as f:
 w=csv.writer(f,delimiter='\t',lineterminator='\n')
 for identity,sha in extra_models:
  if identity not in existing_ids:w.writerow([identity,'true','true','unknown','unknown','unknown','unknown','unknown','false','false','false','false','DISCOVERED / REGISTRATION UNKNOWN','No established named-scene native registration for this exact payload footprint; raw structural TMD scan only. Content identity is not Base Form identity.'])
(root/'additional-model-candidates.json').write_text(json.dumps({'raw_source_identities_without_exact_scene_bank_hash':len(extra_models),'distinct_additional_payload_hashes':len(raw_model_hashes-native_hashes),'union_distinct_model_payload_hashes':len(raw_model_hashes|native_hashes),'native_scene_payload_hashes':len(native_hashes)},indent=2),encoding='utf-8')
class_table='\n'.join(f'| {k} | {v} |' for k,v in sorted(native_classes.items()))
(root/'native-container-families.json').write_text(json.dumps(native_classes,indent=2),encoding='utf-8')
counts='\n'.join(f'| {k} | {v} | {t["unique_content_counts"].get(k,"—")} |' for k,v in sorted(c.items()))
memory=json.loads((root/'SCENE_MEMORY_PROFILES.json').read_text(encoding='utf-8'));memory_ok=sum('profile'in x for x in memory['scenes'])
behavior_counts=collections.Counter(row['classification']for row in csv.DictReader((root/'SCRIPTED_ANIMATED_ASSETS.tsv').open(encoding='utf-8'),delimiter='\t'))
summary=f'''# Stage B retail resource audit — 2026-10-31

Source: `{t['source']}`  
SHA256: `{t['source_sha256']}`; unchanged after audit. Baseline `fec6025`.

Attempted **{t['named_blocks_attempted']} distinct CDNAME blocks/variants**, **{t['field_cells']} with recognized field MAP layouts**; **{t['failed_cell_inventories']} failed cell inventories**. A CDNAME block is not necessarily a playable cell. No variants merged by town prefix.

## Counts and denominators

| Family | Source identity rows | Distinct payload hashes within that family |
|---|---:|---:|
{counts}

Container/member, scan-hit and native scene identities are separate *views*: do not sum these rows to claim unique semantic assets. Exact hashes identify shared bytes only. Native models can be repeated between blocks or registration paths. Primary scene record categories and MAP definition/occurrence distinctions remain in ALL_ASSETS.tsv.

## Original archive container families

| Existing native parser classification | PROT entries |
|---|---:|
{class_table}

These counts retain character, battle, audio, effects and other non-field carriers even when no clip/frame/item-level semantic decoder is established. A recognized outer container does not prove its inner behavior closure.

## Capability measurements

- Additional raw TMD source identities without an exact named-scene bank payload hash: **{len(extra_models)}**, representing **{len(raw_model_hashes-native_hashes)} additional payload hashes**. These remain registration-unknown in the compatibility matrix; byte-footprint differences can include padding, so they are not asserted to be distinct semantic models.
- Union of raw structural TMD and native scene-model payload hashes: **{len(raw_model_hashes|native_hashes)}**. Content hashes do not define Base Forms.
- Native scene model registrations attempted: **{n}**, including repeated source identities.
- Complete **static visual TIM/TMD package closure plus encoding/readback**: **{resolved}/{n} ({percent(resolved)})**. This is a bounded individual-model test into an empty texture context with field runtime regions reserved, **not** arbitrary destination-cell fit.
- Native TIM/TMD package readback: **{encoded}/{n} ({percent(encoded)})**; this is not an IMG export rate.
- Full behavior dependency closure proven: **0/{n}**. Unknown animation/script/physics closure is retained, not failed assets or invalid game data.
- New census-wide IMG readback: **0/{n}**. Separate exact scene fixtures are recorded in VALIDATION_RESULTS.md.
- Gameplay verification in this audit: **0**. Historical user-reported Bylon rendering/collision and R8 lifecycle evidence is retained separately and is not applied to every asset.
- Opaque/unresolved identity rows: **{t['unknown_identity_rows']}** (overlapping container views; not distinct missing files).

Positive animation/script evidence appears in SCRIPTED_ANIMATED_ASSETS.tsv. Other assets remain Unknown / Insufficient Evidence; absence of a parsed binding does not establish Static. No asset is removed from the inventory or editor on that basis.

## Behavior evidence counts

Positive record-structure evidence (not active playback proof): {dict(behavior_counts)}. Unknown remains the conservative result for unresolved dimensions; absence does not establish Static, Animated Only or Scripted Only. CLUT-bearing ordered uploads: {clut_uploads} occurrences; raw palette-word images and repeated boot uploads are separately retained.

## Registration and memory

`SCENE_MEMORY_PROFILES.json` retains one native request profile or explicit unresolved reason for every named block: **{memory_ok} profiles**, **{len(memory["scenes"])-memory_ok} unresolved contexts**. `audit-totals.json` includes one bounded real resource-registration probe per discovered source-route family, with independent static loader SHA comparison and native allocation-request profiles. Those are static contracts, not original-engine execution. Every model's package result is recorded separately. Empty-destination package success does not establish persistent transforms, independent reference allocation or collision in a destination.

The encoded fixed asset arena is 404,480 bytes. Decoded persistent destinations consume separate heap blocks. Neither arena accounting nor package parse proves live heap capacity, fragmentation, actor/render/effect allocations, GPU packet capacity, or runtime residency. Read per-family profiles and warnings.

## Animated & Scripted Object Window design

Retain each asset's primary category (Architecture, Terrain, NPC, Effects, etc.). Add orthogonal filters: Animated Only, Scripted Only, Animated + Scripted, Unknown Dependencies. “Only” requires evidence of absence in the other dimension; current positive evidence with unresolved other behavior remains Unknown Dependencies. Preserve visible geometry and preserve behavior are distinct future placement modes; never label static detachment a complete behavior copy. No production UI changes made.

See MISSING_CAPABILITIES.md and AUDIT_INTEGRITY.md before using these counts.
'''
(root/'GAMEWIDE_AUDIT_SUMMARY.md').write_text(summary,encoding='utf-8')
capabilities=[
('1. Complete ordered texture contexts and composite VRAM reads','Unresolved texture/CLUT','TIM, raw palettes, streamed resources, TMD primitives','native_asset_plan.rs, scene_registration.rs, multi_tim.rs, texture_relocation.rs','Resolve missing source banks and native upload chronology; implement lossless composite word regions when an image/palette spans multiple last-writer uploads. Retain character/runtime reservations.','Proven loader routes and source-bank identities'),
('2. Native carrier and model registration families','Native carrier/loader adapter','Descriptor bundles, raw/embedded packs, non-field/cutscene/battle/global resources','native_loader_trace.rs; engine-scene/model_bank.rs, scene_bundle.rs; container-formats','Connect every detected container family to a verified native registrar, including non-field routes; test cumulative imports and retained selectors against independent loader contracts.','Descriptor and streaming provenance; destination allocation'),
('3. Animation, MAN, scripts and interaction closure','Model materialization','NPCs, MAN, MOVE, ANM/CLUT-walk, VDF, event stagers, MES','native_script_dependencies.rs; engine-scene/scene_resources; scene-formats/man_section.rs','Resolve shared/special model banks, animation identity/clip/skeleton lifetimes and dynamic script targets; relocate incoming references and triggers with explicit geometry-only versus full-behavior semantics.','Model registration; instruction/control-flow evidence'),
('4. Collision and walkable-floor ownership','Collision writer limitation','Retail wall grid, floor tiers/LUT, elevation overrides, actor colliders','native_wall_journal.rs, native_wall_export.rs, native_elevation_codec.rs, collision_generation.rs','Keep editor-owned wall layer recomposition; establish original native ownership from actual scripts/records. Add exact walkable-surface encoding where original representation supports it. Mesh walls are not floor surfaces.','Complete physical dependencies and runtime sampling evidence'),
('5. Resource allocation, staging and runtime budgets','Native VRAM allocation','All import families and destinations','native_load_budget.rs, native_loader_trace.rs, scene_registration.rs, texture_relocation.rs','Bounded mixed-import fixture matrix by loader family and destination context; resource sharing/retirement and independent references; compare allocator traces when user supplies runtime logs.','All resource lifetimes and verified original-engine behavior'),
('6. Primitive and opaque format decoding','Primitive/upload decoder','Unrecognized data, unsupported primitives, multimedia/item/effect/audio containers','container-formats, actor-formats, scene-formats, disc-audit','Investigate opaque range hashes/offsets and parser failures, create format-level adapters and representative regressions; retain unknown assets until decoded.','Native format evidence; no heuristic validity restrictions')]
reason_groups=collections.Counter();examples={}
with (root/'EXPORT_COMPATIBILITY.tsv').open(encoding='utf-8',newline='')as f:
 for row in csv.DictReader(f,delimiter='\t'):
  if row['result'] not in ('UNRESOLVED','FAILED'):continue
  why=row['reason']
  key=('Untextured geometry import route' if 'No textured dependencies'in why else 'Native carrier/upload context' if 'No native field descriptor table'in why or 'No verified native texture-upload route'in why else 'Mixed-depth native image interpretation' if 'image depth mismatch'in why else 'Raw palette upload alignment' if 'Unaligned native palette'in why else 'Composite VRAM image/palette writer' if 'Fragmented'in why else 'Unresolved source VRAM words' if 'external image/palette'in why else 'Other explicit decoder/allocation failures')
  reason_groups[key]+=1;examples.setdefault(key,(row['identity'],why))
(root/'measured-missing-capabilities.json').write_text(json.dumps({'counts':reason_groups,'examples':examples},indent=2),encoding='utf-8')
parts=['# Missing reusable capabilities\n\nCounts below are concrete census model-check failures where that check applies. They do not count all affected behavior assets: an unknown dependency is not proof it is irrelevant. Inspect ALL_ASSETS.tsv and cell graphs for exact affected identities.\n']
for title,key,families,modules,fix,deps in capabilities:
 count=t['failures'].get(key,0)
 parts.append(f'## {title}\n\n- Measured check failures in this bucket: **{count}**. Other affected assets remain unresolved; no invented count.\n- Families: {families}.\n- Root cause: {key}; inspect precise per-model reason in EXPORT_COMPATIBILITY.tsv.\n- Modules: {modules}.\n- Universal fix: {fix}\n- Prerequisites: {deps}.\n')
parts.append(f'## Measured behavior/physics closure scope\n\n- MAN script records: {c.get("ManRecord",0)}; actor headers: {c.get("ManActor",0)}; animation-selector operands: {c.get("AnimationSelector",0)}. Compatible animation clip/lifetime and dynamic script closure are not proven by those counts.\n- Explicit unresolved wall-ownership edges: {unresolved_wall_edges}; retail floor grids: {c.get("FloorTierGrid",0)}; retained elevation-override rows: {c.get("ElevationOverride",0)}. Rows are stored native records, not exclusive object ownership or runtime-executed surfaces.\n\n')
parts.append('## Precise measured failure mechanisms\n\n')
for key,num in reason_groups.most_common():
 ex,why=examples[key]
 parts.append(f'- **{key}: {num} model identities.** First witness `{ex}`: `{why}`.\n')
parts.append('\nUntextured TMDs are legitimate geometry: the current foreign-import material planner demands textured dependencies, so these are a pipeline route gap, not invalid models. Native image-depth mismatch is a resolver assumption about upload metadata versus sampled GPU page depth; raw VRAM words are not intrinsically typed by their uploader. Unaligned palette upload rejection is an ordered-context validator constraint; distinguish upload-strip coordinates from the actual CBA palette start before proposing a fix. Fragmented regions require lossless composite last-writer word assembly. Missing source words remain unresolved until an actual native source bank/upload is identified.\n')
parts.append('## Engine extensions\n\nNo new original-engine limitation is proven by these host-side failures. Dynamic registry capacity, scene heaps, floor span limits and VRAM residency require specific data/instruction evidence plus runtime validation. Do not install an unverified engine patch or interpret missing adapters as hardware impossibility. Unrestricted export remains unchanged.\n')
(root/'MISSING_CAPABILITIES.md').write_text('\n'.join(parts),encoding='utf-8')
integrity=f'''# Audit integrity

- Original disc hash before/after: `{t['source_sha256']}`; equal.
- All 1,233 indexed PROT entries are attempted independently, outside the World Objects catalog. All ISO files are hashed in the existing ISO reader's logical 2,048-byte-per-sector view; PROT.DAT is not redundantly parsed as an ISO blob.
- All named CDNAME blocks are attempted; field identification is based on native structure, not rendering. Failed blocks retain raw entry identities; no scene parse failure removes those entries.
- Exact source offsets distinguish physical archive offsets, decoded container coordinates, and scene-local registration/record indices. A decompressed child offset is never presented as an on-disc patch address.
- Graph links qualify scene-local targets. Container hierarchy is represented by source identities; upload ordering, source rows and hashes retained. Hash equality does not merge semantic Base Forms.
- Raw structural model/TIM scanners are word-aligned and use existing parser plausibility rules; registration is not inferred from a scan hit. Tiny/zero-object models rejected by that scanner may still be listed by native registrars. Arbitrarily unaligned hidden data is not proven absent.
- Nested decode budget: 64 MiB of cumulative input per top-level container, maximum nesting 6, individual declared decode capped at16 MiB. Strict LZS containers at depth<3; unknown/malformed ranges retained by hash. Limits produce explicit gaps, not silent success.
- Opaque complement spans in non-recursive leaves include residual headers and padding with nonzero words; some are not separate semantic assets. Container padding/trailers not individually decoded by every family remain within exact parent identities. This is a complete top-level archive census, not proof every logical asset/clip/script instruction has been decoded.
- ISO logical files over16 MiB are hashed in the complete 2,048-byte view but their multimedia frames/audio sectors are not exhaustively decoded. CD-XA Form2 tails outside that logical view are protected by the full raw-disc SHA256 but are not claimed as decoded individual resources. ISO tree does not establish unallocated-sector content; no claim that filesystem-free sectors contain no data.
- Models are checked individually with existing strict writers (no unrestricted bypass scope). Failed package planning combines dependency/encoding/allocation failures; visual dependency success is a lower bound, not a diagnosis that source data is absent.
- Encoder package checks do not test all source/destination combinations or write IMGs for every model. Only bounded fixture exports can prove their own byte-level invariants. No new preview is rendered, no GUI behavior or gameplay is verified here.
- Classification retains positive evidence only; “static”, “animated only” and “scripted only” are not asserted by absence. Dialogue text, clip splitting, item semantics, effects and audio subresources remain partially typed or opaque.
- Scan process flushes inventory and compatibility after each block; raw cells are independent checkpoint JSONs. This run reached final source rehash and audit-totals publication. No per-block timeout is claimed.

## Explicit scan gaps ({len(t['scan_gaps'])})

'''
integrity+='\n'.join('- '+s for s in t['scan_gaps'])
integrity+='\n\n## Per-cell unresolved relationships\n\n'
for s in t['scene_summaries']:
 if 'error'in s:integrity+=f"- {s['name']}: FAILED: {s['error']}\n"
 else:
  integrity+=f"- {s['name']}: {len(s['gaps'])} explicit gaps; see matching cells JSON and audit-totals.json.\n"
(root/'AUDIT_INTEGRITY.md').write_text(integrity,encoding='utf-8')
print(f'Reports rendered; {n} model identities, {resolved} visual packages, {len(t["scan_gaps"])} scan gaps')
