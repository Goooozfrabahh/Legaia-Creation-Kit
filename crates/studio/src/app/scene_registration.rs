//! Native descriptor-bundle and streamed PAC resource serialization.
//! Resource bytes are not permission to export unresolved behavior or budgets.
// Compatibility/preflight checks become diagnostics only inside the explicit
// unrestricted writer scope. Normal mode retains the original behavior.
macro_rules! export_check {
    ($condition:expr, $($message:tt)+) => {
        if !$condition {
            if !crate::app::ps1_export::unrestricted::warn_check(format!($($message)+)) {
                anyhow::bail!($($message)+);
            }
        }
    };
    ($condition:expr) => { export_check!($condition, "Native check failed: {}", stringify!($condition)); };
}
use std::collections::{BTreeMap,BTreeSet};
use anyhow::{Result,Context,ensure};
use legaia_scene_formats::{scene_asset_table as table,field_objects};
use legaia_tim::Vram;
use super::{NativeAssetPlan,ResourceId};
use super::super::collision_recipes;
use super::texture_relocation::multi_tim::MultiPlan;

// Existing verified field-loader allocations (scene_asset_table / asset-type).
const BUNDLE_ARENA:usize=0x62c00;
const TIM_LIST_BUFFER:usize=0x70800;
pub(in crate::app) fn validate_stored_extent(size:usize,limits:Limits)->Result<()> {
    export_check!(size<=limits.max_encoded.min(BUNDLE_ARENA),"Sector-rounded native resource exceeds arena/explicit encoded budget");Ok(())
}

/// 0x8007C018 + 200*4 = 0x8007C338, the native SFX countdown array.
/// This is a proven conflict boundary, NOT a certified safe table capacity.
pub(in crate::app) const FIRST_KNOWN_MODEL_CONFLICT:usize=200;
#[derive(Debug)]
pub(in crate::app) struct ModelRegistrationDiagnostic {pub existing:usize,pub additional:usize,total:usize}
impl ModelRegistrationDiagnostic {
    /// Counts are GLOBAL registrations, including residents/other contributors.
    pub fn assess(existing:usize,additional:usize)->Result<Self> {
        let total=existing.checked_add(additional).context("Global model registration count overflow")?;
        Ok(Self{existing,additional,total})
    }
    pub fn summary(&self)->String {
        let first=if self.additional==0 {"none".into()}else{self.existing.to_string()};
        format!("Global model occupancy: {} existing; {} proposed additional; {} resulting; first proposed index {}; first known conflicting index {} at 0x8007C338 (SFX countdown array). Indices below 200 are not certified safe; stricter resource/runtime limits still apply.",self.existing,self.additional,self.total,first,FIRST_KNOWN_MODEL_CONFLICT)
    }
    pub fn reject_known_collision(&self)->Result<()> {
        export_check!(self.total<=FIRST_KNOWN_MODEL_CONFLICT,"Model registration reaches index {} or above. {}",FIRST_KNOWN_MODEL_CONFLICT,self.summary());Ok(())
    }
}

/// Required caller-supplied fixture budgets, NOT inferred retail RAM limits.
#[derive(Clone,Copy)]
pub(in crate::app) struct Limits {pub max_models:usize,pub max_decoded:usize,pub max_encoded:usize}
pub(in crate::app) struct SceneRegistration {
    bytes:Vec<u8>,models:BTreeMap<ResourceId,u16>,source_sha256:String,
    diagnostic:ModelRegistrationDiagnostic,
    pac:Option<(String,Vec<u8>)>,
}
impl SceneRegistration {
    pub fn bytes(&self)->&[u8]{&self.bytes}
    pub fn model_diagnostic(&self)->String{self.diagnostic.summary()}
    pub fn pac_bytes(&self)->Option<&[u8]>{self.pac.as_ref().map(|(_,bytes)|bytes.as_slice())}
    pub fn validate_pac_source(&self,source:&[u8])->Result<()> {
        export_check!(self.pac.as_ref().is_some_and(|(hash,_)|*hash==collision_recipes::digest(source)),"Destination PAC changed or was not planned");Ok(())
    }
    /// Bundle-relative registration order. MAP use requires a single-mesh-
    /// bundle field context; other CDNAME contributors must be resolved first.
    pub fn selector(&self,id:&ResourceId)->Result<u16>{self.models.get(id).copied().context("Model not registered")}
    pub fn validate_source(&self,source:&[u8])->Result<()> {export_check!(collision_recipes::digest(source)==self.source_sha256,"Destination bundle changed");Ok(())}
    /// Return a copy of a placed record; never mutate an original descriptor.
    /// This changes only the verified mesh selector, not collision/scripts.
    pub fn bind_record(&self,id:&ResourceId,source:&[u8;32])->Result<[u8;32]> {
        let record=field_objects::ObjectRecord::parse(source,0).context("Invalid native record")?;
        export_check!(record.is_placed(),"Registration requires a placed MAP template");
        let selector=self.selector(id)?;let mut out=*source;out[16..18].copy_from_slice(&selector.to_le_bytes());
        let parsed=field_objects::ObjectRecord::parse(&out,0).context("Registered record readback")?;
        export_check!(field_objects::pack_mesh_index(4,&parsed)==Some(selector),"MAP selector readback mismatch");Ok(out)
    }
}
struct Slot {kind:u8,decoded:Vec<u8>,stored:Vec<u8>,suffix:Vec<u8>}
fn slots(source:&[u8],limits:Limits)->Result<(table::SceneAssetTable,Vec<Slot>)> {
    slots_for_scene(source,limits,false)
}
fn slots_for_scene(source:&[u8],limits:Limits,streamed:bool)->Result<(table::SceneAssetTable,Vec<Slot>)> {
    export_check!(limits.max_models>0 && limits.max_models<=u16::MAX as usize-field_objects::FIELD_ACTOR_PACK_BIAS as usize && limits.max_decoded>0 && limits.max_encoded>0,"Invalid fixture resource budgets");
    let table=table::detect(source).context("Only a verified plain descriptor table at offset zero is supported")?;
    export_check!(table.total_size_is_consistent(),"Inconsistent native decompressed-size total");
    export_check!(table.descriptor_size_sum() as usize<=limits.max_decoded && source.len()<=limits.max_encoded.min(BUNDLE_ARENA),"Source bundle exceeds native arena/explicit fixture budgets");
    let mut kinds=BTreeSet::new();let mut out=Vec::new();
    for (i,d) in table.used().iter().enumerate() {
        export_check!(((1..=7).contains(&d.type_byte)||(streamed&&d.type_byte==0x14)) && kinds.insert(d.type_byte),"Duplicate/unsupported descriptor: bare TMD2, PAC streaming and alternate registrations require another adapter");
        let start=d.data_offset as usize;let end=table.used().get(i+1).map(|d|d.data_offset as usize).unwrap_or(source.len());
        if d.type_byte==1 {export_check!(d.size as usize<=TIM_LIST_BUFFER,"Native TIM-list buffer capacity exceeded");}
        export_check!(start>=8+table.count*8 && start<end && end<=source.len() && d.size>0,"Invalid native payload boundaries");
        let stored=&source[start..end];
        // Flag(0x14) returns the loader mode; its bytes are not LZS data.
        // Preserve its declared size, opaque payload and padding verbatim.
        let (decoded,consumed)=if d.type_byte==0x14 {
            export_check!(d.size as usize<=stored.len(),"Flag payload exceeds native extent");
            (stored[..d.size as usize].to_vec(),d.size as usize)
        }else{legaia_lzs::decompress_tracked(stored,d.size as usize)?};
        export_check!(decoded.len()==d.size as usize && consumed<=stored.len(),"Invalid compressed payload extent");
        if d.type_byte==6 {export_check!(decoded==[0,0,0,0],"Active native CLUT walker requires destination runtime-write closure");}
        out.push(Slot{kind:d.type_byte,decoded,stored:stored.to_vec(),suffix:stored[consumed..].to_vec()});
    }
    export_check!(kinds.contains(&2) && if streamed{kinds.contains(&0x14)}else{kinds.contains(&1)},"Bundle needs existing TMD-pack and native texture route");Ok((table,out))
}
fn members(raw:&[u8])->Result<Vec<Vec<u8>>> {
    let entries=legaia_container_formats::pack::parse_pack(raw)?;
    let header=4+entries.len()*4;let mut end=header;
    for e in &entries {export_check!(e.byte_offset>=end && e.byte_offset%4==0 && e.size>0,"Overlapping/empty native pack member");end=e.byte_offset.checked_add(e.size).context("Pack extent overflow")?;export_check!(end<=raw.len(),"Pack extent out of bounds");}
    Ok(entries.iter().map(|e|raw[e.byte_offset..e.byte_offset+e.size].to_vec()).collect())
}
fn pack(items:&[Vec<u8>])->Result<Vec<u8>> {
    export_check!(items.len()<=65536,"Native pack capacity exceeded");let mut out=vec![0;4+items.len()*4];out[..4].copy_from_slice(&(items.len() as u32).to_le_bytes());
    for (i,item) in items.iter().enumerate(){export_check!(out.len()/4<=u32::MAX as usize,"Pack offset overflow");let word=(out.len()/4) as u32;out[4+i*4..8+i*4].copy_from_slice(&word.to_le_bytes());out.extend_from_slice(item);while out.len()%4!=0{out.push(0);}}
    export_check!(members(&out)?==items,"Native pack rewrite changed existing member bytes");Ok(out)
}
fn upload_pack(raw:&[u8],vram:&mut Vram)->Result<()> {
    // These are descriptor-declared retail TIMs, not arbitrary scanner hits.
    // The native decoder accepts their upper flag metadata; preserve it raw.
    // Newly generated package TIMs are validated strictly before reaching here.
    for member in members(raw)? {let tim=legaia_tim::parse(&member)?;
        export_check!(matches!(tim.mode,legaia_tim::PixelMode::Bpp4|legaia_tim::PixelMode::Bpp8|legaia_tim::PixelMode::Bpp16),"Unsupported destination TIM mode");
        export_check!(member[tim.byte_extent()..].iter().all(|b|*b==0),"Unknown bytes after native TIM member");vram.upload_tim(&tim);
    }Ok(())
}

/// Descriptor order first, then the explicitly selected native PAC stream.
/// No scanner order, guessed shared palettes or active CLUT animation.
pub(in crate::app) fn native_texture_sequence(bundle:&[u8],pac:Option<&[u8]>,limits:Limits)->Result<(Vec<Vec<u8>>,Vram)> {
    let (_,slots)=slots_for_scene(bundle,limits,pac.is_some())?;let mut textures=Vec::new();let mut vram=Vram::new();
    for slot in slots {if slot.kind==1 {upload_pack(&slot.decoded,&mut vram)?;textures.extend(members(&slot.decoded)?);}}
    if let Some(pac)=pac {textures.extend(pac_uploads(pac,&mut vram)?.1);}
    export_check!(!textures.is_empty(),"No verified native texture-upload route");Ok((textures,vram))
}

/// Strict mode-0x14 texture stream. Follow native chunk order, not scanner
/// order; preserve every original chunk and nonzero trailer bytes.
fn pac_uploads(source:&[u8],vram:&mut Vram)->Result<(usize,Vec<Vec<u8>>)> {
    let mut offset=0usize;let mut textures=Vec::new();
    loop {
        let header=u32::from_le_bytes(source.get(offset..offset+4).context("PAC lacks a complete terminator")?.try_into()?);
        let size=(header&0xffffff) as usize;
        if size==0 {return Ok((offset,textures));}
        export_check!(size%4==0,"Unaligned PAC payload would overlap native chunk stepping");
        let body=source.get(offset+4..offset+4+size).context("PAC chunk exceeds payload boundary")?;
        match header>>24 {
            1=>{export_check!(size<=TIM_LIST_BUFFER,"Native PAC TIM-list buffer capacity exceeded");upload_pack(body,vram)?;textures.extend(members(body)?);}
            0=>{let tim=legaia_tim::parse(body)?;export_check!(matches!(tim.mode,legaia_tim::PixelMode::Bpp4|legaia_tim::PixelMode::Bpp8|legaia_tim::PixelMode::Bpp16),"Unsupported PAC TIM mode");export_check!(body[tim.byte_extent()..].iter().all(|b|*b==0),"Unknown bytes after PAC TIM");vram.upload_tim(&tim);textures.push(body.to_vec());}
            kind=>anyhow::bail!("PAC type {kind:#x} requires model-selector/behavior contributor mapping; cannot insert bundle models safely"),
        }
        offset+=4+size;
    }
}
fn append_pac_uploads(source:&[u8],incoming:&[u8],vram:&mut Vram,budget:usize)->Result<Vec<u8>> {
    export_check!(source.len()<=budget.min(BUNDLE_ARENA),"Source PAC exceeds native arena/explicit budget");
    let before=vram.clone();let (end,existing)=pac_uploads(source,vram)?;
    let mut added=Vec::new();
    for raw in members(incoming)? {
        legaia_tim::parse_native_upload(&raw)?;
        if !existing.contains(&raw)&&!added.contains(&raw) {added.push(raw);}
    }
    let mut output=source[..end].to_vec();
    if !added.is_empty() {
        let payload=pack(&added)?;
        export_check!(payload.len()<=TIM_LIST_BUFFER&&payload.len()<=0xffffff,"Native PAC texture buffer/descriptor capacity exceeded");
        output.extend_from_slice(&table::encode_size_word(1,payload.len() as u32).to_le_bytes());output.extend_from_slice(&payload);
    }
    output.extend_from_slice(&source[end..]);
    // Native walking stops at the size-zero terminator. Consume only verified
    // zero sector padding, never opaque trailer residue, to avoid archive growth.
    if output.len()>source.len() && source[end..].iter().all(|b|*b==0) {
        let used=output.len()-(source.len()-end);
        if used+4<=source.len(){output.resize(source.len(),0);}
    }
    export_check!(output.len()<=budget.min(BUNDLE_ARENA),"Compiled PAC exceeds native arena/explicit budget");
    let mut readback=before;pac_uploads(&output,&mut readback)?;
    *vram=readback;Ok(output)
}

/// Plain-bundle compatibility entry point. No MAP or behavior acceptance is
/// implied by successful resource serialization.
pub(in crate::app) fn register(inventory:&NativeAssetPlan,source:&[u8],before:&Vram,relocation:&MultiPlan,limits:Limits)->Result<SceneRegistration> {
    register_scene(inventory,source,None,before,relocation,limits)
}

/// Generic native route selected by the destination's Flag(0x14), never its
/// town name. PACs with model/script contributors require another adapter:
/// inserting bundle models would otherwise shift their global selectors.
pub(in crate::app) fn register_scene(inventory:&NativeAssetPlan,source:&[u8],pac:Option<&[u8]>,before:&Vram,relocation:&MultiPlan,limits:Limits)->Result<SceneRegistration> {
    for id in relocation.source_resources() {let raw=inventory.resources.get(&id).and_then(|r|r.payload.as_deref()).context("Registration source missing")?;export_check!(collision_recipes::digest(raw)==id.sha256,"Registration source fingerprint changed");}
    let package=relocation.package(before)?;
    let (header,mut decoded)=slots_for_scene(source,limits,pac.is_some())?;
    let ti=header.descriptor_index(1);let mi=header.descriptor_index(2).unwrap();
    let original_models=members(&decoded[mi].decoded)?;let mut models=original_models.clone();
    for raw in &models {legaia_tmd::parse(raw)?;}
    let original_pool=table::mesh_pool(source);export_check!(original_pool.len()==models.len(),"Native descriptor-walk pool differs from TMD pack");
    let incoming=members(&package.tmd_pack)?;let mut identities=BTreeSet::new();let mut mapping=BTreeMap::new();
    for (id,slot) in &package.model_slots {
        export_check!(identities.insert(*slot),"Conflicting incoming model identity/slot");let raw=incoming.get(*slot as usize).context("Incoming model slot out of range")?;legaia_tmd::parse(raw)?;
        let selector=if let Some(i)=models.iter().position(|m|m==raw){i}else{let i=models.len();models.push(raw.clone());i};
        mapping.insert(id.clone(),selector);
    }
    export_check!(mapping.len()==incoming.len(),"Unmapped incoming native model");
    // This adapter's existing single-bundle field contract has a five-model
    // resident head. Never compare the bundle-local selector with index 200.
    let existing=original_models.len().checked_add(field_objects::FIELD_ACTOR_PACK_BIAS as usize).context("Resident model occupancy overflow")?;
    let diagnostic=ModelRegistrationDiagnostic::assess(existing,models.len()-original_models.len())?;
    diagnostic.reject_known_collision()?;
    export_check!(models.len()<=limits.max_models,"Native model selector fixture capacity exhausted. {}",diagnostic.summary());
    let mapping=mapping.into_iter().map(|(id,selector)|Ok((id,u16::try_from(selector)?))).collect::<Result<BTreeMap<_,_>>>()?;
    let mut replay=before.clone();
    let pac_output=if let Some(pac)=pac {
        if let Some(ti)=ti {upload_pack(&decoded[ti].decoded,&mut replay)?;}
        Some((collision_recipes::digest(pac),append_pac_uploads(pac,&package.tim_pack,&mut replay,limits.max_encoded)?))
    }else{
        let ti=ti.context("Missing destination TIM-list")?;
        let mut tims=members(&decoded[ti].decoded)?;
        for tim in members(&package.tim_pack)? {legaia_tim::parse_native_upload(&tim)?;if !tims.contains(&tim){tims.push(tim);}}
        decoded[ti].decoded=pack(&tims)?;upload_pack(&decoded[ti].decoded,&mut replay)?;None
    };
    export_check!(super::super::naming::material_key(&replay)==super::super::naming::material_key(&relocation.apply(before)?),"Destination native TIM-list replay differs from relocated package; upload policy unresolved");
    decoded[mi].decoded=pack(&models)?;
    if let Some(ti)=ti {export_check!(decoded[ti].decoded.len()<=TIM_LIST_BUFFER,"Native TIM-list buffer capacity exhausted");}
    let total=decoded.iter().try_fold(0usize,|n,s|n.checked_add(s.decoded.len()).context("Decoded size overflow"))?;
    export_check!(total<=limits.max_decoded && total<=u32::MAX as usize,"Native decoded memory budget exceeded");
    let mut output=source[..8+header.count*8].to_vec();output[4..8].copy_from_slice(&(total as u32).to_le_bytes());
    for (i,slot) in decoded.iter().enumerate() {
        export_check!(slot.decoded.len()<=0xffffff && output.len()<=u32::MAX as usize,"Native descriptor width exceeded");
        let at=8+i*8;let offset=output.len() as u32;output[at..at+4].copy_from_slice(&table::encode_size_word(slot.kind,slot.decoded.len() as u32).to_le_bytes());output[at+4..at+8].copy_from_slice(&offset.to_le_bytes());
        if (pac.is_none()&&Some(i)==ti)||i==mi {let compressed=legaia_disc_patch::compress_within(&slot.decoded,limits.max_encoded).context("Native compressed payload budget exceeded")?;output.extend_from_slice(&compressed);output.extend_from_slice(&slot.suffix);}
        else{output.extend_from_slice(&slot.stored);}
    }
    if output.len()>source.len() {
        let extra=output.len()-source.len();let last=decoded.last().context("Empty native bundle")?;
        if extra<=last.suffix.len()&&last.suffix[last.suffix.len()-extra..].iter().all(|b|*b==0){output.truncate(source.len());}
    }
    export_check!(output.len()<=limits.max_encoded.min(BUNDLE_ARENA),"Native encoded bundle arena/budget exceeded");
    let (read_header,read_slots)=slots_for_scene(&output,limits,pac.is_some())?;export_check!(read_header.count==header.count,"Descriptor count changed");
    for (i,(before_slot,after)) in decoded.iter().zip(&read_slots).enumerate() {
        export_check!(before_slot.decoded==after.decoded,"Reparsed descriptor payload differs");
        if (pac.is_some()||Some(i)!=ti)&&i!=mi {
            export_check!(before_slot.stored.starts_with(&after.stored)&&before_slot.stored[after.stored.len()..].iter().all(|b|*b==0),"Untouched native payload or nonzero padding changed");
        }
    }
    let pool=table::mesh_pool(&output);export_check!(pool.len()==models.len(),"Native registration count readback failed");
    for (i,raw) in original_models.iter().enumerate(){export_check!(pool[i].bytes==*raw,"Unrelated original model selector shifted");}
    for (id,selector) in &mapping {export_check!(pool[*selector as usize].bytes==incoming[package.model_slots[id] as usize],"New model failed native descriptor lookup");}
    Ok(SceneRegistration{bytes:output,models:mapping,source_sha256:collision_recipes::digest(source),diagnostic,pac:pac_output})
}

#[cfg(test)]
#[path = "scene_registration_tests.rs"]
mod tests;
