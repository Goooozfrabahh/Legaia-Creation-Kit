//! Native additive wall serialization for explicitly authored imported copies.
//! Authorship is not recovered retail physics or a walkable-surface contract.
use super::{Assignment,Evidence,ObjectId,NativeObjectId,SourceProvenance,Contribution,WallAdditionPlan,digest};
use std::collections::BTreeSet;
use legaia_scene_formats::field_objects::ObjectRecord;

pub(in crate::app) struct ImportContribution {
    pub assignment:Assignment,
    pub source_map:Vec<u8>,pub source_man:Option<Vec<u8>>,
    pub destination_descriptor:u16,pub destination_cell:[u8;2],
    /// Original donor mesh center, versus its final serialized native pose.
    pub source_center:[f32;3],pub final_center:[f32;3],
}

#[derive(Debug)]
pub(in crate::app) struct ImportWallPlan {
    wall:WallAdditionPlan,
    pub owners:Vec<u64>,pub added_bits:u32,pub changed_bytes:usize,
    /// Original donor provenance, never replaced in saved recipes.
    pub sources:Vec<SourceProvenance>,
}
impl ImportWallPlan {
    pub fn apply(&self,map:&[u8])->Result<Vec<u8>,String>{self.wall.apply(map)}
}

/// Rebase validated donor authoring into one exact destination revision, then
/// invoke the existing deterministic union/conflict compiler once for all UIDs.
pub(in crate::app) fn compile(scene:&str,map_entry:u32,map:&[u8],inputs:&[ImportContribution])->Result<ImportWallPlan,String> {
    if map.len()!=0x12000 {return Err("Expected complete destination MAP".into());}
    let mut rebased=Vec::new();let mut owners=BTreeSet::new();let mut sources=Vec::new();
    for input in inputs {
        let a=&input.assignment;a.recipe.validate()?;
        let ObjectId::Created(uid)=a.owner else{return Err("Original-object collision removal requires verified ownership; this writer only adds imported copies".into());};
        if uid==0||!owners.insert(uid){return Err("Duplicate/zero imported collision UID".into());}
        if !matches!(a.recipe.evidence,Evidence::UserAuthored{..}) {
            return Err(format!("UID {uid}: collision is an unreviewed mesh proposal/native observation, not explicit authored wall intent; physical collision remains unknown"));
        }
        let source=&a.recipe.source;let id=&source.object;
        if input.source_map.len()!=0x12000||digest(&input.source_map)!=source.map_sha256 {return Err(format!("UID {uid}: donor collision MAP changed"));}
        let off=id.descriptor_id as usize*32;let grid=0x8000+(id.cell[1] as usize*128+id.cell[0] as usize)*2;
        if input.source_map[off..off+32]!=source.descriptor||u16::from_le_bytes(input.source_map[grid..grid+2].try_into().unwrap())&511!=id.descriptor_id||!ObjectRecord::parse(&input.source_map,id.descriptor_id as usize).is_some_and(|r|r.is_placed()) {
            return Err(format!("UID {uid}: donor collision descriptor/reference changed"));
        }
        if let Some(hash)=&source.man_sha256 {
            if input.source_man.as_ref().is_none_or(|man|digest(man)!=*hash){return Err(format!("UID {uid}: donor MAN provenance changed or missing"));}
        }
        if input.destination_descriptor>=512||input.destination_cell.iter().any(|&v|v>=128) {return Err("Imported native collision destination out of range".into());}
        let record=input.destination_descriptor as usize*32;let cell=input.destination_cell;
        let grid=0x8000+(cell[1] as usize*128+cell[0] as usize)*2;
        if u16::from_le_bytes(map[grid..grid+2].try_into().unwrap())&511!=input.destination_descriptor||!ObjectRecord::parse(map,input.destination_descriptor as usize).is_some_and(|r|r.is_placed()) {
            return Err(format!("UID {uid}: collision destination has no allocated native instance"));
        }
        if input.source_center.iter().chain(&input.final_center).any(|v|!v.is_finite()) {return Err("Nonfinite collision pose".into());}
        // Generated geometry is saved in its actual rest-pose coordinate frame;
        // manual grid authoring refers to the original source native grid.
        let mut walls=if let Some(g)=&a.recipe.generated {
            g.wall_cells([0,1,2].map(|axis|input.final_center[axis]-g.pivot[axis]),[0.0;3],[1.0;3])?
        }else{BTreeSet::new()};
        let step=|v:f32|->Result<i32,String>{if !v.is_finite()||v.abs()>32768.0||v%64.0!=0.0 {Err("Manual imported walls require exact 64-unit native translation".into())}else{Ok((v/64.0) as i32)}};
        let mut open=BTreeSet::new();
        if !a.recipe.walls.is_empty()||!a.recipe.nonblocking.is_empty() {
            if input.final_center[1]!=input.source_center[1]{return Err("Manual imported walls cannot encode a vertical change".into());}
            let dx=step(input.final_center[0]-input.source_center[0])?;let dz=step(input.final_center[2]-input.source_center[2])?;
            for &[x,z] in &a.recipe.nonblocking {let cell=[x+dx,z+dz];walls.remove(&cell);open.insert(cell);}
            for &[x,z] in &a.recipe.walls {walls.insert([x+dx,z+dz]);}
        }
        if walls.iter().chain(&open).flatten().any(|v|!(0..256).contains(v)){return Err("Imported collision crosses native MAP boundary".into());}
        let mut recipe=a.recipe.clone();recipe.walls=walls.into_iter().collect();recipe.nonblocking=open.into_iter().collect();recipe.generated=None;
        recipe.source=SourceProvenance{object:NativeObjectId{scene:scene.into(),map_entry,cell,descriptor_id:input.destination_descriptor},map_sha256:digest(map),man_sha256:None,descriptor:map[record..record+32].try_into().unwrap()};
        rebased.push(Assignment{owner:ObjectId::Created(uid),recipe});sources.push(source.clone());
    }
    let contributions=rebased.iter().map(|assignment|Contribution{assignment,offset:[0.0;3],rotation:[0.0;3],scale:[1.0;3]}).collect::<Vec<_>>();
    let wall=WallAdditionPlan::compile(scene,map_entry,map,&contributions)?;let output=wall.apply(map)?;
    let changed_bytes=map.iter().zip(&output).filter(|(a,b)|a!=b).count();
    let added_bits=map.iter().zip(&output).map(|(&a,&b)|(b&!a).count_ones()).sum();
    Ok(ImportWallPlan{wall,owners:owners.into_iter().collect(),added_bits,changed_bytes,sources})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(uid:u64)->(Vec<u8>,ImportContribution) {
        let mut source=vec![0u8;0x12000];source[4*32+18]=4;let grid=0x8000+(4*128+4)*2;source[grid..grid+2].copy_from_slice(&4u16.to_le_bytes());
        let provenance=SourceProvenance{object:NativeObjectId{scene:"donor".into(),map_entry:9,cell:[4,4],descriptor_id:4},map_sha256:digest(&source),man_sha256:Some(digest(b"native-man")),descriptor:source[128..160].try_into().unwrap()};
        let mut destination=vec![0u8;0x12000];destination[8*32+18]=4;destination[9*32+18]=4;
        for (cell,id) in [([5,5],8u16),([6,5],9)]{let grid=0x8000+(cell[1]*128+cell[0])*2;destination[grid..grid+2].copy_from_slice(&id.to_le_bytes());}
        destination[0x4000]=0x1a; // unrelated original wall + floor tier 10
        (destination,ImportContribution{assignment:Assignment{owner:ObjectId::Created(uid),recipe:super::super::Recipe{version:1,source:provenance,evidence:Evidence::UserAuthored{note:"Explicit synthetic wall authoring, not recovered native physics".into()},walls:vec![[8,8]],nonblocking:vec![],generated:None}},source_map:source,source_man:Some(b"native-man".to_vec()),destination_descriptor:8,destination_cell:[5,5],source_center:[576.0,0.0,576.0],final_center:[704.0,0.0,576.0]})
    }
    #[test]
    fn imported_walls_follow_pose_preserve_provenance_and_shared_baseline() {
        let (map,input)=fixture(1);let original=input.assignment.clone();
        let plan=compile("target",2,&map,&[input]).unwrap();let output=plan.apply(&map).unwrap();
        assert_eq!(plan.owners,vec![1]);assert_eq!(plan.sources,vec![original.recipe.source]);assert_eq!(plan.added_bits,1);
        assert_ne!(output[0x4000+4*128+5]&0x10,0);assert_eq!(output[0x4000],0x1a);
        for i in 0..map.len(){assert!(output[i]==map[i]||i==0x4000+4*128+5);}
        let (_,mut moved)=fixture(1);moved.final_center[0]+=64.0;let next=compile("target",2,&map,&[moved]).unwrap().apply(&map).unwrap();assert_ne!(next,output);assert_eq!(next[0x4000+4*128+5]&0x10,0);
        assert!(plan.apply(&output).is_err());
    }
    #[test]
    fn imported_overlap_is_a_union_but_open_requirements_never_clear_walls() {
        let (map,a)=fixture(1);let (_,mut b)=fixture(2);b.destination_descriptor=9;b.destination_cell=[6,5];
        let plan=compile("target",2,&map,&[a,b]).unwrap();assert_eq!(plan.added_bits,1);assert_eq!(plan.owners,vec![1,2]);
        let (_,mut open)=fixture(2);open.assignment.recipe.nonblocking=std::mem::take(&mut open.assignment.recipe.walls);open.destination_descriptor=9;open.destination_cell=[6,5];
        let (_,a)=fixture(1);assert!(compile("target",2,&map,&[a,open]).unwrap_err().contains("conflicts"));
        let (_,mut open)=fixture(1);open.assignment.recipe.walls.clear();open.assignment.recipe.nonblocking=vec![[0,0]];open.final_center=open.source_center;
        assert!(compile("target",2,&map,&[open]).unwrap_err().contains("protected native wall"));
    }
    #[test]
    fn native_observation_empty_proposal_and_stale_sources_cannot_prove_nonblocking() {
        let (map,mut input)=fixture(1);input.assignment.recipe.walls.clear();input.assignment.recipe.evidence=Evidence::MeshDerived{note:"Empty mesh proposal is still unknown physical behavior".into()};
        assert!(compile("target",2,&map,&[input]).unwrap_err().contains("unreviewed"));
        let (_,mut input)=fixture(1);input.assignment.recipe.evidence=Evidence::NativeObserved{evidence:"Nearby wall is not ownership".into()};assert!(compile("target",2,&map,&[input]).is_err());
        let (_,mut input)=fixture(1);input.source_map[0x4000]^=0x10;assert!(compile("target",2,&map,&[input]).unwrap_err().contains("MAP changed"));
        let (_,mut input)=fixture(1);input.source_man=None;assert!(compile("target",2,&map,&[input]).unwrap_err().contains("MAN provenance"));
        let (_,mut input)=fixture(1);input.final_center[0]+=1.0;assert!(compile("target",2,&map,&[input]).is_err());
    }
    #[test]
    fn reviewed_generated_pose_and_manual_source_grid_use_their_own_frames() {
        let (map,mut input)=fixture(1);
        let triangles=vec![[[320.0,0.0,320.0],[320.0,120.0,320.0],[320.0,0.0,448.0]]];
        input.assignment.recipe.generated=Some(super::super::super::collision_generation::GeneratedCollision{version:1,mesh_sha256:digest(&serde_json::to_vec(&triangles).unwrap()),triangles,pivot:[384.0,0.0,384.0],band_height:128.0,min_face_area:0.0,native_inspection:"Reviewed synthetic mesh proposal, not retail physics".into()});
        input.assignment.recipe.walls.clear();input.assignment.recipe.nonblocking=vec![[8,8]];
        let output=compile("target",2,&map,&[input]).unwrap().apply(&map).unwrap();
        assert_eq!(output[0x4000+4*128+5]&0x10,0,"source-grid correction must open the final transformed mesh cell");
        assert_eq!(output[0x4000],map[0x4000]);
    }
}
