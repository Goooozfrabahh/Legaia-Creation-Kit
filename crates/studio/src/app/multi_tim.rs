//! Ordered upload replay and multi-TIM closure, isolated from IMG export.
use super::*;

#[derive(Clone,Debug)]
pub(in crate::app) struct Upload {
    pub sequence:Option<u32>,pub resource:ResourceId,
    pub image:bool,pub clut:bool,pub merge_clut_zeros:bool,
}
#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord)]
struct Owner {event:usize,clut:bool}
struct SourceBlock {rect:Rect,bytes:Vec<u8>,depth:u8}
pub(in crate::app) struct UploadTrace {
    events:Vec<Upload>,blocks:BTreeMap<Owner,SourceBlock>,
    owners:Vec<Option<Owner>>,vram:Vram,pub replay_evidence:String,
    fingerprints:BTreeMap<ResourceId,String>,
    tim_flags:BTreeMap<ResourceId,u32>,
}
fn payload<'a>(inventory:&'a NativeAssetPlan,id:&ResourceId,kind:ResourceKind)->Result<&'a [u8]> {
    export_check!(id.kind==kind,"Wrong resource kind in dependency closure");
    let raw=inventory.resources.get(id).and_then(|r|r.payload.as_deref()).context("Upload resource missing")?;
    export_check!(super::super::digest(raw)==id.sha256,"Stale upload resource");Ok(raw)
}
impl UploadTrace {
    /// A scanner's resource order is not evidence. Supply explicit sequence
    /// numbers and a complete independently obtained reference VRAM state.
    /// Matching replay proves this fixture, not chronology of arbitrary retail.
    pub fn verify_replay(inventory:&NativeAssetPlan,events:&[Upload],expected:&Vram,evidence:&str)->Result<Self> {
        Self::verify(inventory,events,expected,evidence,false)
    }
    /// Explicit native descriptor-upload path; never use for scanner hits.
    pub fn verify_native_replay(inventory:&NativeAssetPlan,events:&[Upload],expected:&Vram,evidence:&str)->Result<Self> {
        Self::verify(inventory,events,expected,evidence,true)
    }
    fn verify(inventory:&NativeAssetPlan,events:&[Upload],expected:&Vram,evidence:&str,native:bool)->Result<Self> {
        export_check!(!events.is_empty() && !evidence.trim().is_empty(),"Upload-order evidence missing");
        let mut trace=Self{events:events.to_vec(),blocks:BTreeMap::new(),owners:vec![None;1024*512],vram:Vram::new(),replay_evidence:evidence.into(),fingerprints:BTreeMap::new(),tim_flags:BTreeMap::new()};
        for (i,event) in events.iter().enumerate() {
            export_check!(event.sequence==Some(i as u32),"Ambiguous or incomplete upload ordering");
            export_check!(event.image||event.clut,"Empty upload operation");
            let raw=payload(inventory,&event.resource,ResourceKind::Tim)?;let tim=if native{legaia_tim::parse_native_upload(raw)?}else{legaia_tim::parse_strict(raw)?};
            trace.tim_flags.insert(event.resource.clone(),tim.flags);
            export_check!(tim.byte_extent()==raw.len(),"Trailing TIM data in upload trace");
            let depth=match tim.mode {PixelMode::Bpp4=>0,PixelMode::Bpp8=>1,PixelMode::Bpp16=>2,_=>bail!("Unsupported ordered TIM mode")};
            trace.fingerprints.insert(event.resource.clone(),super::super::digest(raw));
            if event.clut {
                let c=tim.clut.as_ref().context("CLUT upload has no palette block")?;
                let rect=Rect{x:c.fb_x,y:c.fb_y,w:c.w.checked_mul(c.h).context("Linear CLUT overflow")?,h:1}.valid()?;
                export_check!(rect.x%16==0,"Unaligned native palette");
                trace.write(Owner{event:i,clut:true},SourceBlock{rect,bytes:c.entries.iter().flat_map(|v|v.to_le_bytes()).collect(),depth:3},event.merge_clut_zeros);
            }
            if event.image {
                let rect=Rect{x:tim.image.fb_x,y:tim.image.fb_y,w:tim.image.fb_w,h:tim.image.h}.valid()?;
                trace.write(Owner{event:i,clut:false},SourceBlock{rect,bytes:tim.image.data,depth},false);
            }
        }
        export_check!(super::super::super::naming::material_key(&trace.vram)==super::super::super::naming::material_key(expected),"Ordered upload replay differs from reference pixels/coverage");
        Ok(trace)
    }
    fn write(&mut self,owner:Owner,block:SourceBlock,merge:bool) {
        for y in 0..block.rect.h as usize {for x in 0..block.rect.w as usize {
            let i=(y*block.rect.w as usize+x)*2;let bytes=&block.bytes[i..i+2];
            if merge && bytes==[0,0]{continue;}
            let xx=block.rect.x as usize+x;let yy=block.rect.y as usize+y;
            self.vram.write_block(xx as u16,yy as u16,1,1,bytes);self.owners[yy*1024+xx]=Some(owner);
        }}self.blocks.insert(owner,block);
    }
    fn owner(&self,rect:Rect,clut:bool)->Result<Owner> {
        rect.valid()?;let owner=self.owners[rect.y as usize*1024+rect.x as usize].context("Unresolved external image/palette data")?;
        // CBA addresses raw VRAM words: an image upload may intentionally
        // supply a palette. No guessed CLUT ownership is needed in that case.
        if !clut {export_check!(!owner.clut,"Texture image overwritten by a palette upload");}
        for y in rect.y..rect.y+rect.h {for x in rect.x..rect.x+rect.w {
            export_check!(self.owners[y as usize*1024+x as usize]==Some(owner),"Fragmented image/composite palette requires a native composite-block writer");
        }}Ok(owner)
    }
}

pub(super) struct UploadBlock {pub rect:Rect,pub bytes:Vec<u8>,pub depth:u8,pub source:ResourceId,pub origins:BTreeSet<Origin>,flags:u32}
pub(in crate::app) struct MultiPlan {
    uploads:Vec<UploadBlock>,tmds:BTreeMap<ResourceId,Vec<u8>>,
    source_vram:String,output:Vram,pub evidence:String,
}
impl MultiPlan {
    pub(in crate::app) fn resource_diagnostics(&self)->Vec<String>{
        self.uploads.iter().map(|u|format!("TIM {:?} -> native VRAM {:?}, {} bytes",u.source,u.rect,u.bytes.len()))
            .chain(self.tmds.iter().map(|(id,bytes)|format!("TMD {id:?} relocated SHA256 {} ({} bytes)",super::super::digest(bytes),bytes.len()))).collect()
    }
    pub(in crate::app::native_asset_plan) fn source_resources(&self)->BTreeSet<ResourceId> {
        self.uploads.iter().map(|u|u.source.clone()).chain(self.tmds.keys().cloned()).collect()
    }
    pub fn apply(&self,destination:&Vram)->Result<Vram> {
        export_check!(super::super::super::naming::material_key(destination)==self.source_vram,"Multi-TIM destination changed");Ok(self.output.clone())
    }
}
struct Binding {reference:Reference,image:Owner,palette:Option<Owner>}
impl Planner {
    pub fn plan_multi(&mut self,inventory:&NativeAssetPlan,trace:&UploadTrace,tmd_ids:&[ResourceId])->Result<MultiPlan> {
        let mut staged=self.clone();let result=staged.compile_multi(inventory,trace,tmd_ids)?;*self=staged;Ok(result)
    }
    fn compile_multi(&mut self,inventory:&NativeAssetPlan,trace:&UploadTrace,tmd_ids:&[ResourceId])->Result<MultiPlan> {
        for (id,hash) in &trace.fingerprints {export_check!(super::super::digest(payload(inventory,id,ResourceKind::Tim)?)==*hash,"Upload trace source changed");}
        export_check!(!tmd_ids.is_empty(),"No model dependencies");
        let mut bindings=BTreeMap::new();let mut required=BTreeSet::new();
        for id in tmd_ids {
            export_check!(!bindings.contains_key(id),"Duplicate TMD dependency");
            let mut model=Vec::new();
            for r in references(payload(inventory,id,ResourceKind::Tmd)?)? {
                let depth=((r.tsb>>7)&3) as u8;let divisor=match depth {0=>4,1=>2,_=>1};
                let u0=r.uvs.iter().map(|p|p.0).min().unwrap() as u16;let u1=r.uvs.iter().map(|p|p.0).max().unwrap() as u16;
                let v0=r.uvs.iter().map(|p|p.1).min().unwrap() as u16;let v1=r.uvs.iter().map(|p|p.1).max().unwrap() as u16;
                let rect=Rect{x:(r.tsb&15)*64+u0/divisor,y:((r.tsb>>4)&1)*256+v0,w:u1/divisor-u0/divisor+1,h:v1-v0+1};
                let image=trace.owner(rect,false)?;export_check!(trace.blocks[&image].depth==depth,"Native image depth mismatch");required.insert(image);
                let palette=if depth<2 {let owner=trace.owner(Rect{x:(r.cba&63)*16,y:(r.cba>>6)&511,w:if depth==0{16}else{256},h:1},true)?;required.insert(owner);Some(owner)}else{None};
                model.push(Binding{reference:r,image,palette});
            }bindings.insert(id.clone(),model);
        }
        export_check!(!required.is_empty(),"No textured dependencies");
        let source_vram=super::super::super::naming::material_key(&self.vram);
        let mut targets=BTreeMap::new();let mut uploads=Vec::new();
        for owner in required {
            let block=&trace.blocks[&owner];let refs=bindings.values().flatten().filter(|b|b.image==owner).map(|b|b.reference.clone()).collect::<Vec<_>>();
            let target=self.allocate(block.rect,&block.bytes,block.depth,None,&refs)?;
            targets.insert(owner,target);
            // Merge-zero palettes are not promoted to complete original blocks:
            // any skipped source words remain unresolved outside used windows.
            if owner.clut && trace.events[owner.event].merge_clut_zeros {
                export_check!((block.rect.y..block.rect.y+block.rect.h).all(|y|(block.rect.x..block.rect.x+block.rect.w).all(|x|trace.owners[y as usize*1024+x as usize]==Some(owner))),"Partial merge-zero CLUT cannot be emitted as an authentic complete block");
            }
            let source=trace.events[owner.event].resource.clone();
            uploads.push(UploadBlock{rect:target,bytes:block.bytes.clone(),depth:block.depth,source:source.clone(),origins:inventory.resources[&source].origins.clone(),flags:trace.tim_flags[&source]});
        }
        let mut tmds=BTreeMap::new();let mut samples=0usize;
        for (id,model) in bindings {
            let raw=payload(inventory,&id,ResourceKind::Tmd)?;let mut out=raw.to_vec();let mut allowed=BTreeSet::new();
            for b in &model {
                let r=&b.reference;let src=trace.blocks[&b.image].rect;let dst=targets[&b.image];
                let x=(r.tsb as i32&15)*64+dst.x as i32-src.x as i32;let y=((r.tsb as i32>>4)&1)*256+dst.y as i32-src.y as i32;
                let tsb=(r.tsb&!31)|(x as u16/64)|((y as u16/256)<<4);
                let cba=if let Some(owner)=b.palette {let src=trace.blocks[&owner].rect;let dst=targets[&owner];
                    let x=(r.cba&63)*16-src.x+dst.x;let y=((r.cba>>6)&511)-src.y+dst.y;(r.cba&0x8000)|(x/16)|(y<<6)
                }else{r.cba};
                for (at,value) in [(r.at+2,cba),(r.at+6,tsb)] {out[at..at+2].copy_from_slice(&value.to_le_bytes());allowed.extend([at,at+1]);}
            }
            export_check!(raw.iter().zip(&out).enumerate().all(|(i,(a,b))|a==b||allowed.contains(&i)),"Unrelated TMD bytes changed");
            let readback=references(&out)?;export_check!(readback.len()==model.len(),"Primitive readback count changed");
            for (b,r) in model.iter().zip(readback) {
                let a=&b.reference;export_check!(r.uvs==a.uvs && r.tsb&!31==a.tsb&!31,"UV/depth/blend changed");
                for u in a.uvs.iter().map(|p|p.0).min().unwrap()..=a.uvs.iter().map(|p|p.0).max().unwrap() {for v in a.uvs.iter().map(|p|p.1).min().unwrap()..=a.uvs.iter().map(|p|p.1).max().unwrap() {
                    samples+=1;export_check!(samples<=8_000_000,"Multi-TIM readback sample budget exceeded");
                    export_check!(super::super::super::ps1_export::vram_texel(&trace.vram,a.cba,a.tsb,u as usize,v as usize)==super::super::super::ps1_export::vram_texel(&self.vram,r.cba,r.tsb,u as usize,v as usize),"Multi-TIM sample mismatch");
                }}
            }tmds.insert(id,out);
        }
        Ok(MultiPlan{uploads,tmds,source_vram,output:self.vram.clone(),evidence:trace.replay_evidence.clone()})
    }
}

/// A verified word-offset native resource pack fixture, NOT an installed scene
/// selector table. Slots here are local pack indexes, not runtime mesh IDs.
pub(in crate::app) struct SceneResourcePackage {pub tim_pack:Vec<u8>,pub tmd_pack:Vec<u8>,pub model_slots:BTreeMap<ResourceId,u32>}
fn pack(items:&[Vec<u8>])->Result<Vec<u8>> {
    export_check!(!items.is_empty() && items.len()<=65536,"Native pack count out of range");
    let mut out=vec![0;4+items.len()*4];out[..4].copy_from_slice(&(items.len() as u32).to_le_bytes());
    for (i,item) in items.iter().enumerate() {export_check!(out.len()/4<=u32::MAX as usize,"Native pack offset overflow");let word=(out.len()/4) as u32;out[4+i*4..8+i*4].copy_from_slice(&word.to_le_bytes());out.extend_from_slice(item);while out.len()%4!=0{out.push(0);}}
    let ranges=legaia_container_formats::pack::parse_pack(&out)?;export_check!(ranges.len()==items.len(),"Native pack count readback");
    for (entry,item) in ranges.iter().zip(items) {export_check!(&out[entry.byte_offset..entry.byte_offset+item.len()]==item,"Native pack bytes changed");}
    Ok(out)
}
impl MultiPlan {
    /// Materialize valid native TIM uploads and native word-offset TIM/TMD
    /// packs. Palette runs use a 16-bit image upload envelope, preserving all
    /// raw palette words at their CBA destination (no fabricated image pixels).
    pub fn package(&self,before:&Vram)->Result<SceneResourcePackage> {
        self.apply(before)?;let mut replay=before.clone();let mut tims=Vec::new();let mut emitted=BTreeSet::new();
        for upload in &self.uploads {
            let mut raw=Vec::new();raw.extend_from_slice(&16u32.to_le_bytes());raw.extend_from_slice(&((upload.flags&!0xF)|if upload.depth==3{2}else{upload.depth as u32}).to_le_bytes());
            raw.extend_from_slice(&(12+upload.bytes.len() as u32).to_le_bytes());
            for value in [upload.rect.x,upload.rect.y,upload.rect.w,upload.rect.h] {raw.extend_from_slice(&value.to_le_bytes());}raw.extend_from_slice(&upload.bytes);
            let tim=legaia_tim::parse_native_upload(&raw)?;
            if emitted.insert(raw.clone()) {replay.upload_tim(&tim);tims.push(raw);}
        }
        export_check!(super::super::super::naming::material_key(&replay)==super::super::super::naming::material_key(&self.output),"Native TIM package replay differs");
        let tmds=self.tmds.values().cloned().collect::<Vec<_>>();for raw in &tmds {references(raw)?;}
        Ok(SceneResourcePackage{tim_pack:pack(&tims)?,tmd_pack:pack(&tmds)?,model_slots:self.tmds.keys().enumerate().map(|(i,id)|(id.clone(),i as u32)).collect()})
    }
}

#[cfg(test)]
#[path = "multi_tim_tests.rs"]
mod tests;
