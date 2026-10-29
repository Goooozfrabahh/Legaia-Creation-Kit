//! Fixture-only TIM/TMD relocation. No scene packing or IMG export capability.
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
use anyhow::{Result,Context,ensure,bail};
use legaia_tim::{Vram,Tim,PixelMode};
use super::{NativeAssetPlan,ResourceId,ResourceKind,Origin};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(in crate::app) struct Rect {pub x:u16,pub y:u16,pub w:u16,pub h:u16}
/// Proven field display/transition use, not a universal free-VRAM map.
/// See render-kernels/vram_capture.rs, including the native clear strip.
pub(in crate::app) fn field_runtime_reservations()->[Rect;5] {
    [Rect{x:0,y:0,w:320,h:240},Rect{x:0,y:240,w:320,h:240},
     Rect{x:320,y:0,w:320,h:240},Rect{x:320,y:256,w:320,h:256},Rect{x:640,y:256,w:16,h:256}]
}
impl Rect {
    fn valid(self)->Result<Self> {anyhow::ensure!(self.w>0 && self.h>0 && self.x as u32+self.w as u32<=1024 && self.y as u32+self.h as u32<=512,"VRAM rectangle out of bounds");Ok(self)}
    fn contains(self,x:usize,y:usize)->bool {x>=self.x as usize && y>=self.y as usize && x<(self.x+self.w) as usize && y<(self.y+self.h) as usize}
    fn overlaps(self,b:Self)->bool {self.x<b.x+b.w && b.x<self.x+self.w && self.y<b.y+b.h && b.y<self.y+self.h}
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(super) struct Placement {pub image:[u16;2],pub clut:Option<[u16;2]>}
#[derive(Clone)]
struct Block {rect:Rect,bytes:Vec<u8>,role:u8,align:[u16;2]}
#[derive(Clone)]
pub(in crate::app) struct Planner {vram:Vram,protected:Vec<bool>,blocks:Vec<Block>}
pub(super) struct RelocationPlan {
    pub donor_tim:ResourceId,
    pub origins:BTreeSet<Origin>,
    pub tim:Vec<u8>,
    pub tmds:BTreeMap<ResourceId,Vec<u8>>,
    pub placement:Placement,
    source_vram:String,
    output:Vram,
}
impl RelocationPlan {
    pub fn apply(&self,source:&Vram)->Result<Vram> {
        export_check!(super::super::naming::material_key(source)==self.source_vram,"Texture plan destination VRAM changed");
        Ok(self.output.clone())
    }
}
#[derive(Clone)]
struct Reference {at:usize,cba:u16,tsb:u16,uvs:Vec<(u8,u8)>}

#[cfg(test)]
pub(in crate::app) fn texture_sample_changes(a:&[u8],av:&Vram,b:&[u8],bv:&Vram)->Result<(usize,usize)>{
    let ar=references(a)?;let br=references(b)?;anyhow::ensure!(ar.len()==br.len(),"Primitive counts differ");
    let(mut samples,mut changed)=(0,0);
    for (a,b) in ar.iter().zip(br.iter()){
        anyhow::ensure!(a.uvs==b.uvs,"Texture UVs changed");
        let min=a.uvs.iter().map(|p|p.0).min().context("No UVs")?;let max=a.uvs.iter().map(|p|p.0).max().unwrap();
        let vmin=a.uvs.iter().map(|p|p.1).min().unwrap();let vmax=a.uvs.iter().map(|p|p.1).max().unwrap();
        for u in min..=max {for v in vmin..=vmax {samples+=1;if super::super::ps1_export::vram_texel(av,a.cba,a.tsb,u as usize,v as usize)!=super::super::ps1_export::vram_texel(bv,b.cba,b.tsb,u as usize,v as usize){changed+=1;}}}
    }Ok((samples,changed))
}

fn references(raw:&[u8])->Result<Vec<Reference>> {
    let tmd=legaia_tmd::parse(raw)?;
    let table_end=legaia_tmd::HEADER_SIZE+tmd.objects.len()*legaia_tmd::OBJECT_SIZE;
    let mut geometry=Vec::new();
    for object in &tmd.objects {
        for (top,count) in [(object.header.vert_top,object.vertices.len()),(object.header.normal_top,object.normals.len())] {
            if count>0 {let start=legaia_tmd::HEADER_SIZE+top as usize;geometry.push(start..start+count*legaia_tmd::VECTOR_SIZE);}
        }
    }
    let mut out=Vec::new();
    for object in &tmd.objects {
        let groups=legaia_tmd::legaia_prims::iter_groups(raw,object.primitives_byte_offset,object.primitives_byte_size)?;
        export_check!(groups.iter().map(|g|g.prims.len()).sum::<usize>()==object.claimed_n_primitive as usize,"Incomplete native primitive walk");
        for group in groups {
            let descriptor=legaia_tmd::descriptor::Descriptor::for_flags(group.header.flags).context("Unsupported native primitive descriptor")?;
            for prim in group.prims {
                export_check!(prim.vertex_indices_raw.len()==group.header.n_vertices() && prim.vertex_indices_raw.iter().all(|i|i%8==0 && (*i as usize/8)<object.vertices.len()),"Malformed primitive vertex references");
                if !descriptor.packet_shape.is_textured(){continue;}
                let offset=descriptor.texture_block_offset.context("Unknown native texture block")?;
                export_check!(offset+4+group.header.n_vertices()*2<=prim.bytes_size && prim.uvs.len()==group.header.n_vertices(),"Truncated primitive texture block");
                export_check!((prim.tsb>>7)&3!=3,"Unsupported reserved GPU texture depth");
                out.push(Reference{at:prim.bytes_offset+offset,cba:prim.cba,tsb:prim.tsb,uvs:prim.uvs});
            }
        }
    }
    // Aliased primitive storage must not cause a second patch interpretation.
    let mut ranges=BTreeSet::new();for r in &out {for at in [r.at+2,r.at+3,r.at+6,r.at+7] {
        export_check!(at>=table_end && !geometry.iter().any(|range|range.contains(&at)),"Texture fields alias native geometry/header data");
        export_check!(ranges.insert(at),"Aliased native texture reference fields");
    }}
    Ok(out)
}
fn rectangles(tim:&Tim)->Result<(Rect,Option<Rect>,u8)> {
    let depth=match tim.mode {PixelMode::Bpp4=>0,PixelMode::Bpp8=>1,PixelMode::Bpp16=>2,_=>bail!("Unsupported TIM texture mode (24-bit/mixed)")};
    let image=Rect{x:tim.image.fb_x,y:tim.image.fb_y,w:tim.image.fb_w,h:tim.image.h}.valid()?;
    // The verified field uploader linearizes w*h CLUT entries on one row.
    let clut=tim.clut.as_ref().map(|c| {
        let width=c.w.checked_mul(c.h).context("Linear CLUT length overflow")?;
        Rect{x:c.fb_x,y:c.fb_y,w:width,h:1}.valid()
    }).transpose()?;
    export_check!((depth<2)==clut.is_some(),"Indexed TIM requires a CLUT; direct-colour CLUT is unsupported");
    if let Some(c)=clut {export_check!(c.x%16==0 && !c.overlaps(image),"Unaligned or overlapping source CLUT/image");}
    Ok((image,clut,depth))
}
fn page_fits(refs:&[Reference],dx:i32,dy:i32)->bool {
    refs.iter().all(|r| {
        let x=(r.tsb as i32&15)*64+dx;let y=((r.tsb as i32>>4)&1)*256+dy;
        (0..=960).contains(&x) && x%64==0 && (y==0||y==256)
    })
}
fn verify_source(refs:&[Reference],image:Rect,clut:Option<Rect>,depth:u8)->Result<()> {
    for r in refs {
        export_check!(((r.tsb>>7)&3) as u8==depth,"TIM/primitive texture depth mismatch");
        let px=(r.tsb as usize&15)*64;let py=((r.tsb as usize>>4)&1)*256;
        let divisor=match depth {0=>4,1=>2,_=>1};
        let u0=r.uvs.iter().map(|v|v.0).min().context("Empty UVs")? as usize;
        let u1=r.uvs.iter().map(|v|v.0).max().unwrap() as usize;
        let v0=r.uvs.iter().map(|v|v.1).min().unwrap() as usize;
        let v1=r.uvs.iter().map(|v|v.1).max().unwrap() as usize;
        export_check!(image.contains(px+u0/divisor,py+v0) && image.contains(px+u1/divisor,py+v1),"Primitive requires image data outside this TIM; multi-TIM closure required");
        if let Some(c)=clut {
            let x=(r.cba as usize&63)*16;let y=(r.cba as usize>>6)&511;let width=if depth==0 {16}else{256};
            export_check!(c.contains(x,y) && c.contains(x+width-1,y),"Primitive requires an external or incomplete CLUT");
        }
    }Ok(())
}
impl Planner {
    pub(in crate::app) fn reserve_observations(&mut self,observations:&super::runtime_evidence::VramObservations)->Result<()> {
        export_check!(self.blocks.is_empty(),"Runtime observations must be reserved before any allocation");
        for (target,observed) in self.protected.iter_mut().zip(observations.mask()){*target|=*observed;}Ok(())
    }
    /// Reserve a verified immutable/shared upload footprint, including zero
    /// words, without inventing missing pixels in the destination replay.
    pub(in crate::app) fn reserve_written(&mut self,known:&Vram) {
        for i in 0..1024*512 {if known.is_written(i%1024,i/1024){self.protected[i]=true;}}
    }
    /// All destination uploads are protected, including uploaded zero words.
    /// Caller must additionally reserve framebuffer/shared/runtime-use regions.
    pub fn new(destination:&Vram,reservations:&[Rect])->Result<Self> {
        let mut protected=(0..1024*512).map(|i|destination.is_written(i%1024,i/1024)).collect::<Vec<_>>();
        for &rect in reservations {let r=rect.valid()?;for y in r.y..r.y+r.h {for x in r.x..r.x+r.w {protected[y as usize*1024+x as usize]=true;}}}
        Ok(Self{vram:destination.clone(),protected,blocks:Vec::new()})
    }
    fn available(&self,rect:Rect,bytes:&[u8],role:u8,align:[u16;2])->bool {
        if rect.valid().is_err(){return false;}
        if self.blocks.iter().any(|b|b.rect==rect && b.bytes==bytes && b.role==role && b.align==align){return true;}
        if self.blocks.iter().any(|b|b.rect.overlaps(rect)){return false;}
        (rect.y..rect.y+rect.h).all(|y|(rect.x..rect.x+rect.w).all(|x|!self.protected[y as usize*1024+x as usize]))
    }
    fn allocate(&mut self,source:Rect,bytes:&[u8],role:u8,explicit:Option<[u16;2]>,refs:&[Reference])->Result<Rect> {
        let align=if role==3 {[source.x%16,0]}else{[source.x%64,source.y%256]};
        let candidate=|x,y|Rect{x,y,w:source.w,h:source.h};
        let fits=|r:Rect|r.w==source.w && r.h==source.h && self.available(r,bytes,role,align) && if role==3 {r.x%16==align[0]}else{
            r.x%64==align[0] && r.y%256==align[1] && page_fits(refs,r.x as i32-source.x as i32,r.y as i32-source.y as i32)};
        let chosen=if let Some([x,y])=explicit {let r=candidate(x,y);anyhow::ensure!(fits(r),"Conflicting, out-of-bounds or unencodable VRAM destination");r}
        else if let Some(b)=self.blocks.iter().find(|b|b.bytes==bytes && b.role==role && b.align==align && fits(b.rect)) {b.rect}
        else {
            let mut found=None;
            'rows:for y in 0..512u16 {for x in 0..1024u16 {
                if (role==3 && x%16!=align[0]) || (role!=3 && (x%64!=align[0]||y%256!=align[1])){continue;}
                let r=candidate(x,y);if fits(r){found=Some(r);break 'rows;}
            }}found.context("No compatible VRAM allocation remains")?
        };
        if !self.blocks.iter().any(|b|b.rect==chosen && b.bytes==bytes && b.role==role && b.align==align) {
            self.vram.write_block(chosen.x,chosen.y,chosen.w,chosen.h,bytes);
            self.blocks.push(Block{rect:chosen,bytes:bytes.to_vec(),role,align});
        }Ok(chosen)
    }
    /// Atomic in-memory allocation; failures preserve all prior reservations.
    pub fn plan(&mut self,inventory:&NativeAssetPlan,tim_id:&ResourceId,tmd_ids:&[ResourceId],placement:Option<Placement>)->Result<RelocationPlan> {
        let mut staged=self.clone();let plan=staged.compile(inventory,tim_id,tmd_ids,placement)?;*self=staged;Ok(plan)
    }
    fn compile(&mut self,inventory:&NativeAssetPlan,tim_id:&ResourceId,tmd_ids:&[ResourceId],placement:Option<Placement>)->Result<RelocationPlan> {
        let payload=|id:&ResourceId,kind:ResourceKind|->Result<&[u8]>{
            export_check!(id.kind==kind,"Wrong native resource kind");let r=inventory.resources.get(id).context("Native resource absent")?;
            let bytes=r.payload.as_deref().context("Native resource bytes absent")?;
            export_check!(super::digest(bytes)==id.sha256,"Stale native resource fingerprint");Ok(bytes)
        };
        let raw=payload(tim_id,ResourceKind::Tim)?;let tim=legaia_tim::parse_strict(raw)?;
        export_check!(tim.byte_extent()==raw.len(),"Trailing TIM bytes are not part of a single texture package");
        let (image,clut,depth)=rectangles(&tim)?;
        export_check!(!tmd_ids.is_empty(),"No native TMD dependency supplied");
        let mut decoded=BTreeMap::new();let mut all=Vec::new();
        for id in tmd_ids {export_check!(!decoded.contains_key(id),"Duplicate TMD dependency");let refs=references(payload(id,ResourceKind::Tmd)?)?;verify_source(&refs,image,clut,depth)?;all.extend(refs.clone());decoded.insert(id.clone(),refs);}
        export_check!(!all.is_empty(),"No textured native primitives supplied");
        let before=super::super::naming::material_key(&self.vram);
        let target_image=self.allocate(image,&tim.image.data,depth,placement.map(|p|p.image),&all)?;
        let clut_bytes=tim.clut.as_ref().map(|c|c.entries.iter().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>());
        if let Some(p)=placement {export_check!(p.clut.is_some()==clut.is_some(),"Destination CLUT presence mismatch");}
        let target_clut=if let Some(c)=clut {Some(self.allocate(c,clut_bytes.as_deref().unwrap(),3,placement.and_then(|p|p.clut),&[])?) }else{None};
        let dx=target_image.x as i32-image.x as i32;let dy=target_image.y as i32-image.y as i32;
        let mut rewritten=raw.to_vec();
        let image_header=if let Some(c)=&tim.clut {8+12+c.entries.len()*2}else{8};
        rewritten[image_header+4..image_header+6].copy_from_slice(&target_image.x.to_le_bytes());rewritten[image_header+6..image_header+8].copy_from_slice(&target_image.y.to_le_bytes());
        if let Some(c)=target_clut {rewritten[12..14].copy_from_slice(&c.x.to_le_bytes());rewritten[14..16].copy_from_slice(&c.y.to_le_bytes());}
        let relocated=legaia_tim::parse_strict(&rewritten)?;
        export_check!(relocated.flags==tim.flags && relocated.image.data==tim.image.data && relocated.clut.as_ref().map(|c|&c.entries)==tim.clut.as_ref().map(|c|&c.entries),"TIM pixel/palette roundtrip changed");
        let mut donor=Vram::new();donor.upload_tim(&tim);
        let mut tmds=BTreeMap::new();let mut sample_count=0usize;
        for (id,refs) in decoded {
            let original=payload(&id,ResourceKind::Tmd)?;let mut out=original.to_vec();let mut allowed=BTreeSet::new();
            for r in &refs {
                let page_x=(r.tsb as i32&15)*64+dx;let page_y=((r.tsb as i32>>4)&1)*256+dy;
                let tsb=(r.tsb&!31)|((page_x/64) as u16)|((page_y/256) as u16)<<4;
                let cba=if let (Some(source),Some(target))=(clut,target_clut) {
                    let x=(r.cba&63)*16-source.x+target.x;let y=((r.cba>>6)&511)-source.y+target.y;
                    export_check!(x%16==0 && x<=1008 && y<=511,"CLUT reference not encodable");(r.cba&0x8000)|(x/16)|(y<<6)
                }else{r.cba};
                for (at,value) in [(r.at+2,cba),(r.at+6,tsb)] {out[at..at+2].copy_from_slice(&value.to_le_bytes());allowed.extend([at,at+1]);}
            }
            export_check!(original.iter().zip(&out).enumerate().all(|(i,(a,b))|a==b||allowed.contains(&i)),"Unrelated native primitive bytes changed");
            let parsed=references(&out)?;export_check!(parsed.len()==refs.len(),"Primitive reference count changed");
            for (a,b) in refs.iter().zip(&parsed) {
                export_check!(a.uvs==b.uvs && (a.tsb&!31)==(b.tsb&!31),"UV/depth/blend mode changed");
                sample_count+=(a.uvs.iter().map(|p|p.0).max().unwrap() as usize-a.uvs.iter().map(|p|p.0).min().unwrap() as usize+1)
                    *(a.uvs.iter().map(|p|p.1).max().unwrap() as usize-a.uvs.iter().map(|p|p.1).min().unwrap() as usize+1);
                export_check!(sample_count<=8_000_000,"Texture semantic readback exceeds bounded sample budget");
                for u in a.uvs.iter().map(|p|p.0).min().unwrap()..=a.uvs.iter().map(|p|p.0).max().unwrap() {
                    for v in a.uvs.iter().map(|p|p.1).min().unwrap()..=a.uvs.iter().map(|p|p.1).max().unwrap() {
                        export_check!(super::super::ps1_export::vram_texel(&donor,a.cba,a.tsb,u as usize,v as usize)==super::super::ps1_export::vram_texel(&self.vram,b.cba,b.tsb,u as usize,v as usize),"Native texture sample roundtrip differs");
                    }
                }
            }tmds.insert(id,out);
        }
        Ok(RelocationPlan{donor_tim:tim_id.clone(),origins:inventory.resources[tim_id].origins.clone(),tim:rewritten,tmds,
            placement:Placement{image:[target_image.x,target_image.y],clut:target_clut.map(|c|[c.x,c.y])},source_vram:before,output:self.vram.clone()})
    }
}

#[cfg(test)]
#[path = "texture_relocation_tests.rs"]
pub(in crate::app::native_asset_plan) mod tests;

#[path = "multi_tim.rs"]
pub(in crate::app) mod multi_tim;
