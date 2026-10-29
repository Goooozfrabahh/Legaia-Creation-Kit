//! Independent field-loader contract audit. Never a render/gameplay certificate.
//! No calls to mesh_pool, SceneModelBank or the export registration planner.
//! Order follows SCUS FUN_80020224 -> FUN_8001F05C -> FUN_80026B4C,
//! then FUN_8002541C, with the resident five-model bank accounted separately.
use anyhow::{Context,Result,ensure};
use serde::Serialize;
use super::collision_recipes::digest;

#[derive(Debug,Clone,Serialize)]
pub(super) struct Model {
    pub selector:usize,
    pub global_slot:usize,
    pub pointer_table_address:u32,
    pub carrier:String,
    pub member:usize,
    pub sha256:String,
    pub objects:u32,
    pub native_flags:u32,
    #[serde(skip)]
    pub bytes:Vec<u8>,
}
#[derive(Debug,Serialize)]
pub(super) struct Trace {
    pub models:Vec<Model>,
    pub dispatch:Vec<(String,u8,usize)>,
    pub stream_mode:u32,
    pub gaps:Vec<String>,
    pub evidence_class:&'static str,
}
fn word(b:&[u8],at:usize)->Result<u32>{Ok(u32::from_le_bytes(b.get(at..at+4).context("Native loader word outside carrier")?.try_into()?))}
fn packed(b:&[u8])->Result<Vec<Vec<u8>>>{
    let n=word(b,0)? as usize;ensure!(n<=65536 && 4+n*4<=b.len(),"Invalid native pack table");
    let mut out=vec![];
    for i in 0..n{let lo=word(b,4+i*4)? as usize*4;let hi=if i+1<n{word(b,8+i*4)? as usize*4}else{b.len()};
        ensure!(lo>=4+n*4&&lo<hi&&hi<=b.len(),"Invalid native pack member extent");out.push(b[lo..hi].to_vec());}
    Ok(out)
}
fn dispatch(trace:&mut Trace,kind:u8,raw:&[u8],label:String)->Result<u32>{
    trace.dispatch.push((label.clone(),kind,raw.len()));
    let models=match kind{2=>packed(raw)?,9=>vec![raw.to_vec()],_=>vec![]};
    for(member,bytes)in models.into_iter().enumerate(){
        ensure!(word(&bytes,0)?==0x80000002,"Registrar magic mismatch at {label}/{member}");
        let selector=trace.models.len();trace.models.push(Model{selector,global_slot:selector+5,pointer_table_address:0x8007c018+(selector as u32+5)*4,
            carrier:label.clone(),member,sha256:digest(&bytes),objects:word(&bytes,8)?,native_flags:word(&bytes,4)?,bytes});
    }
    Ok(match kind{0|1=>1,2=>2,3..=8=>1<<(kind-1),9=>0,10|15|20=>(kind as u32)<<8,11=>16,_=>{trace.gaps.push(format!("Native dispatcher no-op type {kind:#x} at {label}"));0}})
}
pub(super) fn trace(bundle:&[u8],table_base:usize,pac:Option<&[u8]>)->Result<Trace>{
    let mut out=Trace{models:vec![],dispatch:vec![],stream_mode:0,gaps:vec![],evidence_class:"static native-loader contract; no engine execution/render/interaction verification"};
    let count=word(bundle,table_base)? as usize;ensure!(count>0&&count<=256&&table_base+8+count*8<=bundle.len(),"Invalid descriptor count");
    let mut status=0;
    for i in 0..count{let ts=word(bundle,table_base+8+i*8)?;let kind=(ts>>24)as u8;let len=(ts&0xffffff)as usize;
        let at=table_base.checked_add(word(bundle,table_base+12+i*8)? as usize).context("Descriptor overflow")?;
        let raw=if matches!(kind,10|15|20){vec![]}else{ensure!(len<=16*1024*1024,"Decode budget exceeded");legaia_lzs::decompress(bundle.get(at..).context("Descriptor source outside carrier")?,len)?};
        status|=dispatch(&mut out,kind,&raw,format!("bundle descriptor {i}"))?;
    }
    out.stream_mode=status>>8;
    match out.stream_mode{
        0=>{},10=>{ensure!(pac.is_some(),"Native tim.dat missing");},
        20=>{let p=pac.context("Native PAC missing")?;let mut at=0;
            loop{let ts=word(p,at)?;let size=(ts&0xffffff)as usize;if size==0{break;}
                let body=p.get(at+4..at+4+size).context("Native PAC chunk outside file")?;
                dispatch(&mut out,(ts>>24)as u8,body,format!("PAC chunk {at:#x}"))?;at+=4+(size&!3);
                ensure!(out.dispatch.len()<4096,"Native stream did not terminate");}
        },
        mode=>out.gaps.push(format!("Native stream mode {mode:#x} has no model-registration audit adapter")),
    }
    Ok(out)
}

#[cfg(test)]
#[path="native_loader_trace_tests.rs"]
mod tests;
