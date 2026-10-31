"""Explain all B1 survivors and all original raw candidates without promoting guesses."""
import argparse, collections, csv, hashlib, json
from pathlib import Path
csv.field_size_limit(64*1024*1024)
p=argparse.ArgumentParser()
for name in ('baseline','b1','investigation','matrix','output'):p.add_argument('--'+name,type=Path,required=True)
a=p.parse_args();a.output.mkdir(exist_ok=False)
def read(path):return json.loads(path.read_text(encoding='utf-8-sig'))
def write(name,value):(a.output/name).write_text(json.dumps(value,indent=2),encoding='utf-8')
roots={r['identity']:json.loads(r['fields'])for r in csv.DictReader((a.baseline/'ALL_ASSETS.tsv').open(encoding='utf-8'),delimiter='\t')if r['family']=='PROT entry'}
source=read(a.investigation/'MISSING_WORD_RETAIL_SEARCH.json');raw=read(a.investigation/'RAW_360_SOURCE_READBACK.json')
consumer_rows=read(a.investigation/'NON_GEOMETRY_CONSUMERS.json');consumers={r['sha256']:r for r in consumer_rows};assert len(consumers)==80
assert source['source_unchanged'];assert raw['unique_payloads']==360 and raw['occurrences']==680
failures=read(a.b1/'REMAINING_FAILURE_EVIDENCE.json')['objects'];assert len(failures)==45
word_candidates=collections.defaultdict(list)
for candidate in source['coordinate_candidates']:
 for w in candidate['covered_words']:word_candidates[tuple(w['word'])].append(candidate['identity'])
groups=collections.defaultdict(list);details=[]
for o in failures:
 points={(x+i,y)for c in o['components']for x,y,w in c['missing_word_spans']for i in range(w)}
 same_scene={i for point in points for i in word_candidates[point]if roots[i.split('/')[0]]['scene']==o['source_scene']}
 assert not same_scene,('A donor-scene static witness needs investigation',o['identity'],same_scene)
 first=tuple(o['components'][0]['missing_word_spans'][0][:2]);groups[str(first)].append(o['identity'])
 details.append({**o,'root_cause':'No supplying upload in verified boot/scene/stream sequence; exact static source or runtime producer not established',
  'native_missing_words':sorted(points),'retail_coordinate_candidates':{str(pt):word_candidates[pt]for pt in sorted(points)},
  'same_scene_static_TIM_witnesses':sorted(same_scene),
  'dynamic_producer_status':'Unknown. VRAM capture/clear address overlap and CLUT-walk support are not proof these primitives use those producers.',
  'not_a_native_engine_limit':True,'substitute_pixels_written':False})
write('ALL_45_DEPENDENCY_FINDINGS.json',{'objects':details,'shared_first_missing_word_groups':groups,
 'native_TIM_hashes_searched':source['TIM_payloads_searched'],'unresolved_TIM_tag_or_carrier_candidates':source['unresolved_carriers'],
 'search_limits':'Search covers native TIM candidates in the completed bounded audit, not proof of absence of opaque/raw runtime-generated pixels elsewhere in the IMG.'})
by=collections.defaultdict(list)
for occurrence in raw['identities']:by[occurrence['payload_sha256']].append(occurrence)
assert len(by)==360
counts=collections.Counter();inventory=[]
for sha,occurrences in sorted(by.items()):
 r=occurrences[0];e=r['payload_evidence'];kind=e['kind'];counts[kind]+=1
 if kind=='BareModel':
  family=r['root']['fields']['class']
  form={'SceneTmdStream':'BattleBackdropComponent','SummonReadef':'SummonComponent','Pack':'EffectGeometryCandidate','LzsContainer':'CharacterGeometryCandidate'}.get(family,'UnresolvedGeometry')
  work='Existing native TMD registrar accepts this format; exact texture, actor/draw recipe, animation, references and budgets still need closure. No placement/export certification.'
 elif kind=='ModelPack':
  form='ModelResourceContainer';work='Resolve members individually through native type-2 pack dispatch; container is not a placed object. Keep each occurrence and its member address.'
 else:
  c=consumers[sha]
  form='Animation'if c['ANM_payload_parsed']else'AudioCarrierComponent'if c['audio_carrier_detected']else'UnresolvedTaggedResource'
  work=c['native_consumer_evidence']+'; no bare TMD/model-pack geometry was established. Retain original identity without assigning false placeability.'
 inventory.append({'payload_sha256':sha,'structural_kind':kind,'provisional_base_form_classification':form,
  'classification_confidence':'Structural format and source-family evidence; semantic independent object identity not certified',
  'native_support_and_missing_work':work,'occurrences':occurrences,'retain_in_universal_object_inventory':True,'placeability_verified':False})
write('RAW_360_CLASSIFIED.json',{'payloads':inventory,'counts':counts,'source_occurrences':680,'hash_equality_does_not_merge_FormKeys':True})
pack_aliases=sum(all(m['scene_sources']for m in r['occurrences'][0]['geometry_extent_matches'])for r in inventory if r['structural_kind']=='ModelPack')
with (a.output/'RAW_360_INDEX.tsv').open('w',encoding='utf-8',newline='')as f:
 w=csv.writer(f,delimiter='\t');w.writerow(['payload_sha256','structural_kind','provisional_form_class','all_source_identities','independent_placeability_verified'])
 for r in inventory:w.writerow([r['payload_sha256'],r['structural_kind'],r['provisional_base_form_classification'],';'.join(x['identity']for x in r['occurrences']),False])
with (a.output/'ALL_45_INDEX.tsv').open('w',encoding='utf-8',newline='')as f:
 w=csv.writer(f,delimiter='\t');w.writerow(['identity','TMD_sha256','registration','missing_word_spans_by_primitive','same_scene_static_witnesses','root_cause'])
 for r in details:w.writerow([r['identity'],r['source_tmd_sha256'],json.dumps(r['registration']),json.dumps([{'primitive':c['primitive_offset'],'role':c['role'],'TSB':c['tsb'],'CBA':c['cba'],'missing_spans':c['missing_word_spans']}for c in r['components']]),len(r['same_scene_static_TIM_witnesses']),r['root_cause']])
rows=lambda root:{r['identity']:r for r in csv.DictReader((root/'EXPORT_COMPATIBILITY.tsv').open(encoding='utf-8'),delimiter='\t')if r['identity'].startswith('scene:')}
old=rows(Path(str(a.b1).replace('_Results','_03_WordClosure')));new=rows(a.matrix)
assert len(old)==8546 and old.keys()==new.keys();regressions=[i for i in old if old[i]['automated_package_readback']=='true'and new[i]['automated_package_readback']!='true'];assert not regressions
passed=sum(r['automated_package_readback']=='true'for r in new.values());initial=sum(r['automated_package_readback']=='true'for r in old.values())
totals=read(a.matrix/'audit-totals.json');assert totals['source_unchanged'] and totals['native_package_readback_passed']==passed
summary={'baseline':'3ab839b','initial_passed':initial,'final_passed':passed,'cohort':8546,'remaining':8546-passed,'regressions':regressions,
 'raw_payload_classification':counts,'all_360_retained':True,'all_680_source_occurrences_readback_verified':True,
 'packs_whose_every_geometry_member_matches_existing_scene_sources':pack_aliases,'non_geometry_native_consumers':dict(collections.Counter(r['native_consumer_evidence']for r in consumer_rows)),
 'TIM_payloads_searched':source['TIM_payloads_searched'],'unresolved_TIM_tag_candidates':len(source['unresolved_carriers']),
 'static_pixel_fix_justified':False,'GUI_verified':False,'gameplay_verified':False,'source_sha256':source['source_sha256']}
write('CLOSURE_SUMMARY.json',summary)
report=f'''# Stage B1.1 resource closure investigation

Initial package coverage: **{initial}/8546**. Final: **{passed}/8546**. Baseline regressions: **0**.
All 45 source identities retain exact TMD hash, page/palette references, ordered carriers and missing-word spans in ALL_45_DEPENDENCY_FINDINGS.json.

## Missing static inputs

There are 19 groups by first missing address; this groups a shared symptom, not proof of one runtime cause.
All 5,410 distinct missing word coordinates have coordinate candidates elsewhere among the retail TIM sources searched. None has a witness in its own donor scene's inventoried static TIMs. An unrelated scene's bytes at the same address are not the requested texture. No such substitution was made.
The search examined {source['TIM_payloads_searched']} unique TIM-tag/scanner payload hashes. {len(source['unresolved_carriers'])} candidates failed native TIM extraction/validation (including false typed-stream interpretations); these are retained explicitly. Opaque resources and runtime-produced data are not exhausted by a TIM search.
Known framebuffer/capture/clear addresses, CLUT-walk MoveImage behavior and ideal UV support identify follow-up domains. They do not establish the producer or the intended words for these models. Every unresolved model has some missing words intersecting ideal UV support; dropping bounding-box corners alone does not prove complete material closure.
No supported evidence justifies changing these 45 material packages yet. This is not a proven PS1 hardware limitation.

## Raw payload investigation

All 360 original payload hashes and 680 source occurrences were recovered from their exact original PROT, descriptor, LZS, pack or stream coordinates and hash-verified.
Final structural counts: **{dict(counts)}**. See RAW_360_CLASSIFIED.json for every source, member, geometry extent and provisional Base Form classification.
The old audit counted type-2 dispatch payloads as TMD-family entries even when they were containers or lacked valid geometry. The generic model-payload classifier now records bare model versus complete model pack versus unresolved tag. Whole-pack hashes and padding differences are preserved, not treated as new semantic object identities.
All {pack_aliases} model packs have every geometry member matched to established scene registrations after separately retaining stored-member padding and geometry extents. These whole-pack hashes do not add new bare models. Native audio/ANM consumer findings for the other 80 tags: {dict(collections.Counter(r['native_consumer_evidence']for r in consumer_rows))}.
Bare-model families include battle-stage shells, summon components, effect geometry and field character meshes. Battle shells require the native object mask and second draw transform; character/effect/summon geometry requires its appropriate animation, texture and actor/draw dependencies. Native type 9 and type 2 already provide bare/pack registration formats; this is not full cross-cell registration or gameplay verification.
All candidates remain in the universal investigation inventory. Unknown candidates are not removed or offered as falsely proven independent placeable objects. The current Object Window still needs a general unregistered-resource adapter to expose these source identities; this change does not claim that adapter is complete.

## Changes and Stage B2

Implemented reusable payload classification, geometry-extent versus stored-payload evidence, source readback and cross-disc missing-word investigation. No unsupported pixel repair, sampler bypass or new asset restriction was added. Unrestricted experimental export is preserved.
Stage B2 must bind animation/VDF, native actor/draw class, palette lifetime, scripts and resource budgets to each Base Form separately from static geometry. Static detachment cannot certify behavior or freeze an unknown runtime texture producer.

## Verification boundaries

These are automated structural/package tests. Original-engine rendering, visibility, collision behavior and gameplay compatibility have not been tested in this run. Latest Build and protected IMGs are not installation targets. Regression results and GitHub refs are recorded separately in the delivery manifest.
The encoded fixed asset arena remains 404,480 bytes; package closure does not establish decoded heap capacity, staging or actor/GPU allocations. The complete eight-TIM resident-character source was checked separately and supplies none of the missing words. Character and alternate-face protections remain unchanged.
'''
(a.output/'STAGE_B1_1_REPORT.md').write_text(report,encoding='utf-8');print(json.dumps(summary))
