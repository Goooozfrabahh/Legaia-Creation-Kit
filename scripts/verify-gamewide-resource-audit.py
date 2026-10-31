"""Verify that census counts, graph identities and compatibility rows agree."""
import argparse,csv,json,collections,hashlib
from pathlib import Path
csv.field_size_limit(16*1024*1024)
p=argparse.ArgumentParser();p.add_argument('directory',type=Path);a=p.parse_args();r=a.directory
t=json.loads((r/'audit-totals.json').read_text());counts=collections.Counter();ids=set();hashes=collections.defaultdict(set);edges=[]
for line in (r/'asset-nodes.jsonl').open(encoding='utf-8'):
 n=json.loads(line);assert n['id'] not in ids,n['id'];ids.add(n['id']);counts[n['family']]+=1
 if n['sha256']:hashes[n['family']].add(n['sha256'])
 for d in n['dependencies']:edges.append((n['id'],d['target']))
assert counts==t['family_counts'],(counts,t['family_counts'])
assert counts['PROT entry']==1233,counts['PROT entry']
assert all(b in ids for a,b in edges),[(a,b)for a,b in edges if b not in ids][:10]
with (r/'ALL_ASSETS.tsv').open(encoding='utf-8',newline='')as f:
 inventory_ids={x['identity']for x in csv.DictReader(f,delimiter='\t')};assert inventory_ids==ids
with (r/'EXPORT_COMPATIBILITY.tsv').open(encoding='utf-8',newline='')as f:
 rows=list(csv.DictReader(f,delimiter='\t'))
assert sum(x['identity'].startswith('scene:')for x in rows)==t['model_registrations_tested'];assert all(x['identity']in ids for x in rows)
assert sum(x['automated_package_readback']=='true'for x in rows)==t['native_package_readback_passed']
assert all(x['gameplay']=='false'for x in rows)
graph_ids=set();decoder=json.JSONDecoder()
with (r/'RESOURCE_DEPENDENCIES.json').open(encoding='utf-8')as f:
 header=f.readline().strip();meta=json.loads(header+']}');assert meta['source_sha256']==t['source_sha256']
 for line in f:
  node,end=decoder.raw_decode(line);graph_ids.add(node['id']);tail=line[end:].strip()
  if tail!=',':
   trailer=json.loads('{"nodes":['+tail);assert trailer['gameplay_verified'] is False
assert graph_ids==ids
memory=json.loads((r/'SCENE_MEMORY_PROFILES.json').read_text(encoding='utf-8'));assert memory['source_sha256']==t['source_sha256'];assert {x['scene']for x in memory['scenes']}=={x['name']for x in t['scene_summaries']};assert all(x['runtime_verified']is False for x in memory['scenes'])
report={'identity_rows':len(ids),'counts_verified':True,'graph_edges_verified':len(edges),'dangling_edges':0,'compatibility_rows':len(rows),'source_unchanged':t['source_unchanged'],'gameplay_claims':0,'graph_json_valid':True}
(r/'audit-integrity-check.json').write_text(json.dumps(report,indent=2));print(json.dumps(report))
