"""Compare the fixed retail cohort and explain every unresolved static read.
No textures, native bytes, editor state or compatibility gates are modified.
"""
import argparse,collections,csv,hashlib,json,math,re
from pathlib import Path
csv.field_size_limit(64*1024*1024)
p=argparse.ArgumentParser();p.add_argument('--baseline',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('phases',type=Path,nargs='+');a=p.parse_args();a.output.mkdir(exist_ok=False)
def load_rows(root):
 return {r['identity']:r for r in csv.DictReader((root/'EXPORT_COMPATIBILITY.tsv').open(encoding='utf-8'),delimiter='\t') if r['identity'].startswith('scene:')}
def ok(r):return r['automated_package_readback']=='true'
def family(reason):
 if 'No native field descriptor table'in reason or 'No verified native texture-upload route'in reason:return 'Native carrier/upload context'
 if 'No textured dependencies'in reason:return 'Untextured geometry'
 if 'image depth mismatch'in reason:return 'Upload depth versus GPU sampling depth'
 if 'Unaligned native palette'in reason:return 'Palette upload alignment'
 if 'Fragmented'in reason:return 'Composite VRAM ownership'
 if 'Unresolved native VRAM word'in reason or 'external image/palette'in reason:return 'Unobserved source words'
 return reason
base=load_rows(a.baseline);assert len(base)==8546
phases=[];previous=base
for root in a.phases:
 rows=load_rows(root);assert rows.keys()==base.keys(),'Cohort identity drift'
 totals=json.loads((root/'audit-totals.json').read_text(encoding='utf-8'));assert totals['source_unchanged']
 assert totals['source_sha256']=='750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6'
 passed=sum(ok(r)for r in rows.values());assert passed==totals['native_package_readback_passed']
 phases.append({'path':str(root),'tested':len(rows),'passed':passed,'percent':100*passed/len(rows),'failure_categories':dict(collections.Counter(family(r['reason'])for r in rows.values()if not ok(r))),'regressions_from_previous':[i for i in rows if ok(previous[i])and not ok(rows[i])],'regressions_from_baseline':[i for i in rows if ok(base[i])and not ok(rows[i])]})
 previous=rows
final=previous;assert not phases[-1]['regressions_from_baseline']
failed={i:r for i,r in final.items()if not ok(r)}
cohorts={}
for key in sorted({family(r['reason'])for r in base.values()if not ok(r)}):
 ids=[i for i,r in base.items()if not ok(r)and family(r['reason'])==key]
 cohorts[key]={'baseline_failed':len(ids),'now_package_verified':sum(ok(final[i])for i in ids),'remaining':sum(not ok(final[i])for i in ids)}
# Reconstruct written-word coverage from serialized native upload rectangles.
# This is an independent union calculation, not a default-black VRAM image.
def coverage(records):
 mask=bytearray(1024*512)
 for r in records:
  x,y,w,h=r['image']
  for yy in range(y,min(512,y+h)):
   if 0<=x<1024:mask[yy*1024+x:yy*1024+min(1024,x+w)]=b'\1'*min(w,1024-x)
  if r['clut']:
   x,y,w,h=r['clut'];w*=h
   if 0<=x<1024 and 0<=y<512:mask[y*1024+x:y*1024+min(1024,x+w)]=b'\1'*min(w,1024-x)
 return mask
runtime=[('field display front',(0,0,320,240)),('field display back',(0,240,320,240)),('field capture rows',(320,0,320,240)),('field capture/clear reservation',(320,256,320,256)),('CLUT-walk reservation',(640,256,16,256))]
def regions(x,y):return [n for n,(a,b,w,h)in runtime if a<=x<a+w and b<=y<b+h]
def overlaps_triangle(uv,cell):
 # Conservative mathematical UV-cell intersection, NOT a PS1 raster oracle.
 x,y=cell;box=[(x,y),(x+1,y),(x,y+1),(x+1,y+1)];axes=[(1,0),(0,1)]
 axes +=[(uv[(i+1)%3][1]-uv[i][1],uv[i][0]-uv[(i+1)%3][0])for i in range(3)]
 for ax,ay in axes:
  ta=[a*ax+b*ay for a,b in uv];tb=[a*ax+b*ay for a,b in box]
  if max(ta)<min(tb) or max(tb)<min(ta):return False
 return True
def in_uv_support(prim,x,y):
 if not prim['textured']:return False
 tsb=prim['tsb'];depth=prim['sampled_depth'];div=[4,2,1][depth];pagex=(tsb&15)*64;pagey=((tsb>>4)&1)*256;uv=prim['uvs'];tris=[uv[:3]] if len(uv)==3 else [[uv[0],uv[1],uv[2]],[uv[1],uv[3],uv[2]]]
 return any(overlaps_triangle(t,((x-pagex)*div+i,y-pagey))for t in tris for i in range(div))
records={};masks={}
for row in csv.DictReader((a.phases[-1]/'ALL_ASSETS.tsv').open(encoding='utf-8'),delimiter='\t'):
 suffix='/canonical-static-dependencies';identity=row['identity']
 if not identity.endswith(suffix)or identity[:-len(suffix)]not in failed:continue
 identity=identity[:-len(suffix)];d=json.loads(row['fields']);scene=d['scene'];mask=masks.setdefault(scene,coverage(d['ordered_uploads']));components=[]
 for prim in d['primitives']:
  if not prim['unresolved_words']:continue
  total=0
  for role,key in [('image','image_words'),('CLUT','palette_words')]:
   rect=prim[key]
   if rect is None:continue
   x,y,w,h=rect;spans=[];points=[]
   for yy in range(y,y+h):
    start=None
    for xx in range(x,x+w):
     missing=xx>=1024 or yy>=512 or not mask[yy*1024+xx]
     if missing:
      points.append((xx,yy));total+=1
      if start is None:start=xx
     elif start is not None:spans.append([start,yy,xx-start]);start=None
    if start is not None:spans.append([start,yy,x+w-start])
   if points:
    components.append({'primitive_offset':prim['offset'],'role':role,'required_rect':rect,'tsb':prim['tsb'],'cba':prim['cba'],'sampling_depth':prim['sampled_depth'],'uvs':prim['uvs'],'missing_words':len(points),'missing_word_spans':spans,'known_runtime_reservation_intersections':dict(collections.Counter(n for xx,yy in points for n in regions(xx,yy))),'missing_outside_all_known_reservations':sum(not regions(xx,yy)for xx,yy in points),'missing_word_cells_intersecting_ideal_uv_support':sum(in_uv_support(prim,xx,yy)for xx,yy in points) if role=='image'else None,'interpretation':'No static upload witness. Runtime-region overlap is evidence of an address domain, not proof of its producer. UV-support analysis is mathematical only; it does not establish PS1 raster, edge, texture-window or execution behavior. No missing words were invented, filled or discarded.'})
  assert total==prim['unresolved_words'],(identity,total,prim['unresolved_words'])
 assert components,identity
 records[identity]={'identity':identity,'source_scene':scene,'selector':d['selector'],'source_tmd_sha256':d['tmd_sha256'],'registration':d['registration'],'carriers':d['carriers'],'ordered_native_uploads':d['ordered_uploads'],'encoder_failure':failed[identity]['reason'],'components':components,'status':'EXPLICIT UNRESOLVED SOURCE INPUT / NOT EXPORTED BY THIS PACKAGE CHECK','runtime_verified':False}
assert records.keys()==failed.keys(),(records.keys()^failed.keys())
(a.output/'REMAINING_FAILURE_EVIDENCE.json').write_text(json.dumps({'schema':1,'count':len(records),'objects':list(records.values())},indent=2),encoding='utf-8')
with (a.output/'REMAINING_FAILURES.tsv').open('w',encoding='utf-8',newline='')as f:
 w=csv.writer(f,delimiter='\t');w.writerow(['identity','TMD_SHA256','primitive_offset','role','required_rect','missing_word_count','native_upload_sources','runtime_address_overlap','ideal_UV_support_word_count','exact_encoder_failure'])
 for i,r in records.items():
  for c in r['components']:w.writerow([i,r['source_tmd_sha256'],hex(c['primitive_offset']),c['role'],json.dumps(c['required_rect']),c['missing_words'],len(r['ordered_native_uploads']),json.dumps(c['known_runtime_reservation_intersections']),c['missing_word_cells_intersecting_ideal_uv_support'],r['encoder_failure']])
summary={'baseline':'03e23ab','baseline_passed':sum(ok(r)for r in base.values()),'cohort_count':len(base),'phases':phases,'original_failure_cohorts':cohorts,'remaining':len(records),'source_sha256':'750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6','GUI_verified':False,'gameplay_verified':False}
(a.output/'COVERAGE.json').write_text(json.dumps(summary,indent=2),encoding='utf-8')
# Keep raw candidates independent of the registered-model matrix.
raw=json.loads((a.baseline/'additional-model-candidates.json').read_text(encoding='utf-8'));assert raw['distinct_additional_payload_hashes']==360
(a.output/'RAW_REGISTRATION_INVESTIGATION.json').write_text(json.dumps({'baseline_inventory':str(a.baseline/'ALL_ASSETS.tsv'),'counts':raw,'status':'Separate native-registration investigation; never removed or promoted by material-package success'},indent=2),encoding='utf-8')
text='# Stage B1 universal static resource resolution\n\nBaseline `03e23ab`; unchanged original retail SHA256 `'+summary['source_sha256']+'`. Fixed cohort: 8,546 source model-registration identities. No source-town production exceptions, asset whitelist, synthesized textures, dropped identities or modified retail files.\n\n| Pass | Native TIM/TMD package readback | Coverage |\n|---|---:|---:|\n| Baseline | 5,597 / 8,546 | 65.49% |\n'
for phase in phases:text+=f"| {Path(phase['path']).name} | {phase['passed']} / 8,546 | {phase['percent']:.2f}% |\n"
text+='\nFinal regressions against baseline: **0**. Native package verification is not arbitrary destination fit, runtime heap, rendering, collision, animation or script verification.\n\n## Original failure cohorts\n\n'
for name,c in cohorts.items():text+=f"- {name}: {c['now_package_verified']}/{c['baseline_failed']} now package-verified; {c['remaining']} explicit remaining failures.\n"
text+='\n## Canonical representation\n\nBase Forms retain their existing FormKey; the new optional static resource record stores original carrier hashes/offsets, exact model registration source and selector/member, ordered native image/CLUT uploads, primitive sampling depth and UV/word requirements, and per-word last-writer sequence evidence. Shared payloads never merge Base Forms or placed References. Existing project files remain readable and are not rewritten. Full behavior coverage remains unverified. Native imports and per-Base-Form export diagnostics use the same resolver.\n\n## Remaining evidence\n\nEvery final failure is retained in REMAINING_FAILURE_EVIDENCE.json and REMAINING_FAILURES.tsv with its TMD hash, registration, primitive coordinates, complete missing-word spans and complete source upload chain. Coverage is independently reconstructed from serialized native upload rectangles and checked against the Rust canonical missing-word count. Runtime reservation overlap and ideal UV triangle support are reported as evidence, never as permission to invent, discard or alias missing inputs. Some conservative UV-bounding-box requirements may include words outside ideal triangle support; actual PS1 raster/edge/texture-window behavior needs further proof before narrowing that validation. Addresses inside known display/capture/clear regions may require live engine-produced input; overlap alone does not identify an actual producer. Missing static/shared/animated palette sources remain explicit unresolved evidence. No original-engine limitation or unverified engine patch is inferred.\n\nThe 360 raw model payload hashes without exact established scene-bank identity remain a separate investigation, preserving their 680 source discovery identities in the original inventory.\n\n## Runtime and memory boundaries\n\nEmpty-context package success does not establish fit in every destination. Production imports still protect native display/capture, boot and complete character/alternate-face VRAM. Shared-resource allocation, cumulative native selectors, staging/decoded heap budgets and owned collision lifecycle remain validated by their existing regressions. Dynamic image/CLUT inputs, animation, scripts, interaction and retail rendering remain unverified. Unrestricted experimental export remains enabled; technical encoding failures are explicit. Latest Build and protected images are unchanged.\n'
(a.output/'STAGE_B1_RESULTS.md').write_text(text,encoding='utf-8')
print(json.dumps({'phases':[(p['passed'],p['percent'])for p in phases],'remaining_evidence_records':len(records),'regressions':len(phases[-1]['regressions_from_baseline'])}))
