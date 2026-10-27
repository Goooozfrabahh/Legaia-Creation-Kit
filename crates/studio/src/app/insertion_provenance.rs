//! Verify recorded native additions against BOTH actual image revisions.
//! Receipt assertions alone cannot establish collision ownership or safety.
use super::*;
use std::collections::BTreeMap;
use super::super::collision_recipes::digest;

#[derive(Clone,Debug)]
pub(super) struct VerifiedInsertions {
    pub map_entry:usize,
    map_hash:String,
    cells:BTreeMap<(u8,u8),usize>,
    receipt:serde_json::Value,
    floor_lut:[i16;16],
}
impl VerifiedInsertions {
    pub fn contains(&self,cell:(u8,u8))->bool {self.cells.contains_key(&cell)}
    pub fn applies_to(&self,map_entry:usize,cell:(u8,u8))->bool {self.map_entry==map_entry&&self.contains(cell)}
    pub fn covers(&self,map:&[u8],edit:&MoveEdit)->bool {
        self.cells.get(&edit.cell).is_some_and(|&id|
            digest(map)==self.map_hash&&record_at(map,edit.cell,edit.source_position,edit.env_slot,edit.source_rotation).is_ok_and(|(actual,_)|actual==id))
    }
    pub fn output_receipt(&self,image:&[u8])->Result<serde_json::Value> {
        let disc=DiscPatcher::open(image.to_vec())?;let map=disc.read_entry_footprint(self.map_entry)?;
        let mut out=self.receipt.clone();let mut alive=Vec::new();
        for mut entry in out["instances"].as_array().context("Receipt instances missing")?.clone() {
            let [col,row]:[u8;2]=serde_json::from_value(entry["spawn_cell"].clone())?;
            let id=entry["descriptor"].as_u64().context("Receipt descriptor missing")? as usize;
            let off=GRID_OFF+(row as usize*128+col as usize)*2;
            if u16::from_le_bytes(map[off..off+2].try_into()?)&511!=id as u16 {continue;}
            let record=ObjectRecord::parse(&map,id).context("Output insertion record missing")?;
            let tier=(map[WALL_OFF+row as usize*128+col as usize]&15) as usize;
            entry["psx_world_position"]=serde_json::json!([field_objects::world_x(col,record.x_off),-(self.floor_lut[tier] as i32)+record.y_off as i32,field_objects::world_z(row,record.z_off)]);
            alive.push(entry);
        }
        out["pending_collision_recipes"]=serde_json::json!(alive.len());
        out["instances"]=serde_json::json!(alive);
        out["output_sha256"]=serde_json::json!(digest(image));
        out["production_export_approved"]=serde_json::json!(false);
        out["playability_verified"]=serde_json::json!(false);
        out["continuation_scope"]=serde_json::json!("Existing verified native additions moved/removed; original wall/floor/trigger/MAN data preserved. This output retains experimental-source status, not a new production approval.");
        Ok(out)
    }
}

pub(super) fn context(source:&Path)->Result<Option<(String,usize)>> {
    let path=source.with_extension("json");if !path.is_file(){return Ok(None);}
    ensure!(std::fs::metadata(&path)?.len()<=1024*1024,"Insertion receipt exceeds bounded metadata size");
    let value:serde_json::Value=serde_json::from_slice(&std::fs::read(path)?)?;
    if value["schema"]!="lck-experimental-visual-v1" {return Ok(None);}
    Ok(Some((value["destination"].as_str().context("Receipt scene missing")?.to_owned(),
        value["map_entry"].as_u64().and_then(|v|usize::try_from(v).ok()).context("Receipt MAP index missing")?)))
}

pub(super) fn load(source:&Path,disc:&DiscPatcher,map_entry:usize,scene:&str,floor_lut:[i16;16])->Result<Option<VerifiedInsertions>> {
    let file=source.with_extension("json");
    if !file.exists(){return Ok(None);}
    ensure!(std::fs::metadata(&file)?.len()<=1024*1024,"Insertion receipt exceeds bounded metadata size");
    let receipt:serde_json::Value=serde_json::from_slice(&std::fs::read(&file)?)?;
    if receipt["schema"]!="lck-experimental-visual-v1" {return Ok(None);}
    ensure!(receipt["visual_only"]==true&&receipt["production_export_approved"]==false,"Unrecognized insertion receipt approval policy");
    ensure!(receipt["output_sha256"].as_str()==Some(digest(disc.image()).as_str()),"Insertion receipt source IMG hash mismatch");
    ensure!(receipt["map_entry"].as_u64()==Some(map_entry as u64)&&receipt["destination"].as_str()==Some(scene),"Insertion receipt belongs to another native scene; no guessed cross-scene scope");
    let baseline=PathBuf::from(receipt["source_image"].as_str().context("Receipt baseline path missing")?);
    ensure!(baseline.extension().and_then(|x|x.to_str()).is_some_and(|x|x.eq_ignore_ascii_case("img")||x.eq_ignore_ascii_case("bin")),"Receipt baseline must be a raw disc image");
    let baseline=baseline.canonicalize().context("Resolve read-only insertion baseline")?;
    ensure!(baseline!=source.canonicalize()?,"Insertion receipt cannot prove itself against the same IMG");
    let size=std::fs::metadata(&baseline)?.len();ensure!(size>0&&size%2352==0&&size<=2*1024*1024*1024,"Invalid baseline raw-image size");
    let bytes=std::fs::read(&baseline)?;
    ensure!(receipt["source_sha256"].as_str()==Some(digest(&bytes).as_str()),"Insertion baseline IMG hash mismatch");
    let old=DiscPatcher::open(bytes)?;
    verify_revisions(receipt,&old,disc,map_entry,floor_lut).map(Some)
}

fn verify_revisions(receipt:serde_json::Value,old:&DiscPatcher,disc:&DiscPatcher,map_entry:usize,floor_lut:[i16;16])->Result<VerifiedInsertions> {
    ensure!(receipt["output_sha256"].as_str()==Some(digest(disc.image()).as_str()),"Insertion receipt source IMG hash mismatch");
    let map=disc.read_entry_footprint(map_entry)?;let before=old.read_entry_footprint(map_entry)?;
    ensure!(map.len()==MAP_LEN&&before.len()==MAP_LEN,"Incomplete insertion MAP revision");
    ensure!(before[WALL_OFF..GRID_OFF]==map[WALL_OFF..GRID_OFF],"Insertion receipt cannot account for changed MAP wall/floor contributions; explicit per-owner baseline reconstruction required");
    let load=|d:&DiscPatcher|->Result<Vec<u8>>{d.read_disc_sectors(d.entry_disc_lba(map_entry).context("Missing source MAP address")?,0x28)};
    ensure!(load(old)?[0x10000..0x14000]==load(disc)?[0x10000..0x14000],"Insertion trigger/binding/fallback data differs from baseline");
    let prior_bundle=old.read_entry_footprint(map_entry+3)?;let bundle=disc.read_entry_footprint(map_entry+3)?;
    ensure!(scene_asset_table::detect(&prior_bundle).is_some()&&scene_asset_table::detect(&bundle).is_some(),"Unrecognized insertion resource bundle");
    let man=|b:&[u8]|->Result<Option<Vec<u8>>>{scene_asset_table::decode_slot_by_type(b,3).transpose().map_err(anyhow::Error::msg)};
    ensure!(man(&prior_bundle)?==man(&bundle)?,"Insertion MAN scripts changed; script-independent deletion/movement not established");
    let prior_pool=scene_asset_table::mesh_pool(&prior_bundle);let pool=scene_asset_table::mesh_pool(&bundle);
    ensure!(pool.len()>=prior_pool.len()&&prior_pool.iter().zip(&pool).all(|(a,b)|a.bytes==b.bytes),"Original model selectors/resources changed since insertion baseline");
    let instances=receipt["instances"].as_array().context("Receipt instance array missing")?;
    ensure!(instances.len()<=512,"Too many insertion provenance entries");
    let mut cells=BTreeMap::new();let mut ids=HashSet::new();
    for entry in instances {
        let id:usize=serde_json::from_value(entry["descriptor"].clone())?;
        let [col,row]:[u8;2]=serde_json::from_value(entry["spawn_cell"].clone())?;
        let selector:u16=serde_json::from_value(entry["selector"].clone())?;
        ensure!(id>3&&id<512&&col<128&&row<128&&ids.insert(id)&&cells.insert((col,row),id).is_none(),"Invalid/duplicate insertion identity");
        let off=GRID_OFF+(row as usize*128+col as usize)*2;
        ensure!(before[id*32..id*32+32].iter().all(|b|*b==0)&&before[off..off+2]==[0,0],"Claimed insertion replaced an existing native record or reference");
        let refs=|m:&[u8]|m[GRID_OFF..0x10000].chunks_exact(2).filter(|b|u16::from_le_bytes([b[0],b[1]])&511==id as u16).count();
        ensure!(refs(&before)==0&&refs(&map)==1&&u16::from_le_bytes(map[off..off+2].try_into()?)&511==id as u16,"Claimed insertion is shared, missing or previously referenced");
        let record=ObjectRecord::parse(&map,id).context("Inserted descriptor missing")?;
        ensure!(record.is_placed()&&field_objects::pack_mesh_index(id as u16,&record)==Some(selector)&&pool.get(selector as usize).is_some(),"Insertion model reference does not resolve");
    }
    Ok(VerifiedInsertions{map_entry,map_hash:digest(&map),cells,receipt,floor_lut})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn disc(map:&[u8],man_byte:u8)->DiscPatcher {
        let mut tmd=vec![0;64];tmd[..4].copy_from_slice(&0x80000002u32.to_le_bytes());tmd[8..12].copy_from_slice(&1u32.to_le_bytes());
        let mut pack=vec![1,0,0,0,1,0,0,0];pack.extend_from_slice(&tmd);
        let payloads=[vec![0;4],pack,vec![man_byte;8],vec![0;4],vec![0;4],vec![0;4],vec![0;4]];
        let mut bundle=vec![0;64];bundle[..4].copy_from_slice(&7u32.to_le_bytes());bundle[4..8].copy_from_slice(&(payloads.iter().map(|p|p.len() as u32).sum::<u32>()).to_le_bytes());
        for (i,p) in payloads.iter().enumerate() {let at=8+i*8;let offset=bundle.len() as u32;
            bundle[at..at+4].copy_from_slice(&scene_asset_table::encode_size_word(i as u8+1,p.len() as u32).to_le_bytes());
            bundle[at+4..at+8].copy_from_slice(&offset.to_le_bytes());bundle.extend_from_slice(&legaia_lzs::compress(p));}
        let entries=[map.to_vec(),vec![0;0x2000],vec![0;2048],bundle,vec![0;2048]];
        let mut prot=vec![0;2048];prot[4..8].copy_from_slice(&(entries.len() as u32).to_le_bytes());prot[8..12].copy_from_slice(&1u32.to_le_bytes());
        for (i,p) in entries.iter().enumerate(){let lba=(prot.len()/2048) as u32;prot[16+i*4..20+i*4].copy_from_slice(&lba.to_le_bytes());prot.extend_from_slice(p);prot.resize(prot.len().div_ceil(2048)*2048,0);}
        let end=(prot.len()/2048) as u32;prot[16+entries.len()*4..20+entries.len()*4].copy_from_slice(&end.to_le_bytes());
        DiscPatcher::open(legaia_disc_patch::disc::synth::synth_disc(&prot)).unwrap()
    }
    fn fixture()->(Vec<u8>,Vec<u8>,MoveEdit,serde_json::Value) {
        let mut old=vec![0;MAP_LEN];old[20*32+0x12..20*32+0x14].copy_from_slice(&0x16u16.to_le_bytes());
        old[GRID_OFF+(10*128+10)*2..GRID_OFF+(10*128+10)*2+2].copy_from_slice(&20u16.to_le_bytes());
        let (off,bit)=wall_offset(40,42).unwrap();old[off]|=bit; // near the future copy, not contributed by it
        let mut new=old.clone();new[511*32..512*32].copy_from_slice(&old[20*32..21*32]);
        new[GRID_OFF+(20*128+20)*2..GRID_OFF+(20*128+20)*2+2].copy_from_slice(&511u16.to_le_bytes());
        let edit=MoveEdit{cell:(20,20),source_position:(2624,2624),env_slot:0,source_rotation:[0;3],delta:[64.0,0.0,0.0],collision_bounds:Some([2560,2688,2560,2688])};
        let receipt=serde_json::json!({"schema":"lck-experimental-visual-v1","visual_only":true,"production_export_approved":false,"destination":"synthetic","map_entry":0,"instances":[{"uid":1,"descriptor":511,"selector":0,"spawn_cell":[20,20]}]});
        (old,new,edit,receipt)
    }
    fn sealed(old:&[u8],new:&[u8],mut receipt:serde_json::Value)->Result<VerifiedInsertions> {
        let old=disc(old,7);let new=disc(new,7);receipt["output_sha256"]=serde_json::json!(digest(new.image()));
        verify_revisions(receipt,&old,&new,0,[0;16])
    }
    #[test]
    fn added_instance_does_not_own_unchanged_nearby_walls_and_proof_is_revision_bound() {
        let (old,new,edit,receipt)=fixture();let proof=sealed(&old,&new,receipt).unwrap();
        let bindings=NativeBindings::default();
        assert!(native_deletion::analyze(&new,&edit,&bindings,None).unwrap().validate().is_err());
        native_deletion::analyze(&new,&edit,&bindings,Some(&proof)).unwrap().validate().unwrap();
        let edits=ExportEdits{deletes:vec![edit.clone()],insertion_provenance:Some(proof.clone()),..Default::default()};
        let patches=map_patches_with_bindings(&new,&edits,&bindings).unwrap();assert!(patches.iter().all(|(o,_)|*o>=GRID_OFF));
        let mut stale=new.clone();stale[0x4000]^=1;assert!(!proof.covers(&stale,&edit));
    }
    #[test]
    fn changed_walls_scripts_shared_or_original_records_cannot_forge_addition_provenance() {
        let (old,new,_,receipt)=fixture();let mut changed=new.clone();changed[WALL_OFF]|=0x10;
        assert!(sealed(&old,&changed,receipt.clone()).is_err());
        changed=new.clone();changed[GRID_OFF+(21*128+20)*2..GRID_OFF+(21*128+20)*2+2].copy_from_slice(&511u16.to_le_bytes());
        assert!(sealed(&old,&changed,receipt.clone()).is_err());
        let mut claim=receipt.clone();claim["instances"][0]["descriptor"]=serde_json::json!(20);assert!(sealed(&old,&new,claim).is_err());
        let a=disc(&old,7);let b=disc(&new,8);let mut claim=receipt;claim["output_sha256"]=serde_json::json!(digest(b.image()));
        assert!(verify_revisions(claim,&a,&b,0,[0;16]).is_err());
    }
    #[test]
    fn actual_receipt_hashes_and_baseline_are_required_and_continuations_remain_experimental() {
        let (old,new,_,mut receipt)=fixture();let a=disc(&old,7);let b=disc(&new,7);
        let dir=std::env::current_dir().unwrap().join("target").join(format!("m11-proof-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));std::fs::create_dir_all(&dir).unwrap();
        let baseline=dir.join("synthetic-baseline.img");let source=dir.join("synthetic-insertion.img");
        std::fs::write(&baseline,a.image()).unwrap();std::fs::write(&source,b.image()).unwrap();
        receipt["source_image"]=serde_json::json!(baseline);receipt["source_sha256"]=serde_json::json!(digest(a.image()));receipt["output_sha256"]=serde_json::json!(digest(b.image()));
        std::fs::write(source.with_extension("json"),serde_json::to_vec(&receipt).unwrap()).unwrap();
        let proof=load(&source,&b,0,"synthetic",[0;16]).unwrap().unwrap();
        assert_eq!(context(&source).unwrap(),Some(("synthetic".into(),0)));
        assert!(proof.applies_to(0,(20,20)));assert!(!proof.applies_to(1,(20,20)));
        let out=proof.output_receipt(b.image()).unwrap();assert_eq!(out["production_export_approved"],false);assert_eq!(out["playability_verified"],false);
        assert!(load(&source,&b,1,"other",[0;16]).is_err());
        receipt["output_sha256"]=serde_json::json!("0".repeat(64));std::fs::write(source.with_extension("json"),serde_json::to_vec(&receipt).unwrap()).unwrap();
        assert!(load(&source,&b,0,"synthetic",[0;16]).is_err());
    }
}
