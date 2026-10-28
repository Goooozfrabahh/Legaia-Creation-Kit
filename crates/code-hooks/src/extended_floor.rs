//! Experimental NTSC-U hybrid floor redirects. Code lives in an existing
//! guarded SCUS arena; versioned rows live in verified unused MAP padding.
//! No heap allocation, expanded scene load, or retail table stride changes.
use anyhow::{ensure,Context,Result};
use crate::{arenas::{Edit,SCUS_TABLE_RANGES},mips::*};
use std::collections::BTreeMap;

pub const ENTRY:u32=0x80019278;
pub const CODE:u32=0x8007ab80;
pub const CODE_END:u32=0x8007af00;
pub const MAGIC:u32=u32::from_le_bytes(*b"LXF1");
pub const MAX_ROWS:usize=(0x400-8)/12;
pub const TRACE:u32=0x80077728;
pub const TRACE_MAGIC:u32=u32::from_le_bytes(*b"FXD1");

/// Separate observable experimental profile. Original Stage E build/bytes
/// remain unchanged. Slot zero probes the live player position; seven fixed
/// slots provide a controlled comparison independent of wall accessibility.
pub fn plan_observable(exe:&[u8],points:&[[i16;2]])->Result<Vec<Edit>> {
    ensure!(points.len()==7 && points.iter().flatten().all(|&v|(0..16384).contains(&v)),"Observable floor profile needs seven bounded points");
    let mut edits=plan_impl(exe,true)?;
    let base=u32::from_le_bytes(exe[0x18..0x1c].try_into()?);let off=(TRACE-base) as usize+0x800;
    ensure!(exe.get(off..off+256).is_some_and(|b|b.iter().all(|&v|v==0)),"Floor trace arena occupied");
    let mut data=vec![0;256];data[..4].copy_from_slice(&TRACE_MAGIC.to_le_bytes());data[4..8].copy_from_slice(&1u32.to_le_bytes());
    for (i,p) in points.iter().enumerate(){let at=64+(i+1)*24;data[at..at+2].copy_from_slice(&p[0].to_le_bytes());data[at+2..at+4].copy_from_slice(&p[1].to_le_bytes());}
    edits.push(Edit{prot_index:None,file_off:off,bytes:data});Ok(edits)
}

fn emit_observer(a:&mut Asm) {
    // Nested proxy queries must not re-enter the observer. The query marker
    // remains independent: mode 1 = retail, mode 0 = Stage E.
    for w in [lui(S2,0x8007),ori(S2,S2,0x7728),lw(T0,S2,8),0]{a.emit(w);}
    a.branch(bne(T0,ZERO,0),"return");
    a.emit(addiu(T0,ZERO,1));a.emit(sw(T0,S2,8));
    for w in [lw(T0,S2,12),lw(S0,SP,12),addiu(T0,T0,1),sw(T0,S2,12),sw(S0,S2,32),lw(T1,S0,20),lhu(T2,S0,24),sw(T1,S2,36),sh(T2,S2,40),lw(T1,SP,0),sw(S1,S2,48),sw(T1,S2,44),sw(S4,S2,52),lw(T1,SP,4),lui(T2,0x8008),sw(T1,S2,56),lw(S3,T2,0xc364),0,sw(S3,S2,20),srl(T2,S3,20),addiu(T2,T2,0xf800),sltiu(T2,T2,2)]{a.emit(w);}
    a.branch(beq(T2,ZERO,0),"trace_done");
    for w in [lw(T2,S3,20),lhu(T3,S3,24),sw(T2,S2,24),sh(T3,S2,28),andi(T2,T0,31)]{a.emit(w);}
    a.branch(bne(T2,ZERO,0),"trace_done");
    a.branch(beq(T1,ZERO,0),"trace_done");
    for w in [addiu(T1,ZERO,1),sw(T1,S2,8),lw(T2,S2,60),0,addiu(T3,T2,1),andi(T3,T3,7),sw(T3,S2,60),sll(T3,T2,4),sll(T4,T2,3),addu(S4,S2,T3),addu(S4,S4,T4),addiu(S4,S4,64)]{a.emit(w);}
    a.branch(bne(T2,ZERO,0),"probe_fixed");
    for w in [lhu(T2,S3,20),lhu(T3,S3,24),sh(T2,S4,0),sh(T3,S4,2)]{a.emit(w);}
    a.label("probe_fixed");
    for w in [addiu(SP,SP,(-32i16) as u16),lw(T1,S3,16),lhu(T2,S4,0),lhu(T3,S4,2),sw(T1,SP,0),sw(T1,SP,16),sh(T2,SP,20),sh(T3,SP,24),addu(A0,SP,ZERO),addiu(A1,ZERO,1),jal(CODE+24),0,sw(V0,S4,4),lw(T1,SP,16),lw(T2,SP,0),sw(T1,S4,12),sw(T2,SP,16),addu(A0,SP,ZERO),addu(A1,ZERO,ZERO),jal(CODE+24),0,sw(V0,S4,8),lw(T1,SP,16),lw(T2,S2,16),sw(T1,S4,16),addiu(T2,T2,1),sw(T2,S2,16),sw(T2,S4,20),addiu(SP,SP,32)]{a.emit(w);}
    a.label("trace_done");for w in [sw(ZERO,S2,8),lw(S1,S2,48),0,addu(V0,S1,ZERO)]{a.emit(w);}
}
pub fn table_offset(map:&[u8])->Result<usize> {
    ensure!(map.len()==0x12000,"Native MAP length mismatch");
    let end=u16::from_le_bytes(map[0x10000..0x10002].try_into()?) as usize;
    let aligned=(end+3)&!3;
    ensure!(end>=0x12 && aligned<=0x1ff8,"No bounded native primary-table padding");Ok(0x10000+aligned)
}

#[derive(Clone,Debug,PartialEq,Eq)]
pub struct Row {
    pub destination:[u8;2],
    /// 1: sample original retail surface at translated coordinates;
    /// 2: sample the retail bilinear surface, bypassing kind-2 at that point.
    pub mode:u8,
    pub delta:[i16;2],
    pub height_delta:i16,
}
pub fn encode(rows:&[Row])->Result<Vec<u8>> {
    ensure!(!rows.is_empty() && rows.len()<=MAX_ROWS,"Extended floor row budget exceeded");
    let mut keys=std::collections::BTreeSet::new();let mut out=Vec::new();
    out.extend(MAGIC.to_le_bytes());out.extend(1u16.to_le_bytes());out.extend((rows.len() as u16).to_le_bytes());
    for r in rows {
        ensure!(keys.insert(r.destination) && (1..=2).contains(&r.mode),"Duplicate/invalid floor redirect");
        for axis in 0..2 {let low=r.destination[axis] as i32*64+r.delta[axis] as i32;
            ensure!(r.delta[axis]%64==0 && (0..=16320).contains(&low),"Floor redirect outside native grid/alignment");}
        ensure!(r.mode!=2 || r.delta==[0;2],"Bilinear restore must use the original coordinates");
        out.extend([r.destination[0],r.destination[1],r.mode,0]);
        out.extend(r.delta[0].to_le_bytes());out.extend(r.delta[1].to_le_bytes());out.extend(r.height_delta.to_le_bytes());out.extend([0,0]);
    } Ok(out)
}
pub fn decode(bytes:&[u8])->Result<Vec<Row>> {
    ensure!(bytes.len()>=8 && bytes[..4]==MAGIC.to_le_bytes() && bytes[4..6]==1u16.to_le_bytes(),"Unknown extended floor header");
    let count=u16::from_le_bytes(bytes[6..8].try_into()?) as usize;
    ensure!(count>0 && count<=MAX_ROWS && bytes.len()>=8+count*12,"Truncated/oversized extended floor table");
    let mut rows=Vec::new();for b in bytes[8..8+count*12].chunks_exact(12) {
        ensure!(b[3]==0 && b[10..12]==[0,0],"Unknown extended floor row fields");
        rows.push(Row{destination:[b[0],b[1]],mode:b[2],delta:[i16::from_le_bytes(b[4..6].try_into()?),i16::from_le_bytes(b[6..8].try_into()?)],height_delta:i16::from_le_bytes(b[8..10].try_into()?)});
    }
    ensure!(encode(&rows)?==bytes[..8+count*12],"Noncanonical extended floor table");Ok(rows)
}

struct Asm {w:Vec<u32>,labels:BTreeMap<&'static str,usize>,fix:Vec<(usize,&'static str,bool)>}
impl Asm {
    fn emit(&mut self,w:u32){self.w.push(w);}
    fn label(&mut self,l:&'static str){assert!(self.labels.insert(l,self.w.len()).is_none());}
    fn branch(&mut self,w:u32,l:&'static str){self.fix.push((self.w.len(),l,false));self.emit(w);self.emit(0);}
    fn jump(&mut self,l:&'static str){self.fix.push((self.w.len(),l,true));self.emit(0);self.emit(0);}
    fn finish(mut self)->Result<(Vec<u8>,BTreeMap<&'static str,u32>)>{
        for (i,l,jump) in &self.fix {let target=*self.labels.get(l).context("Missing assembler label")?;
            self.w[*i]=if *jump {j(CODE+target as u32*4)}else{self.w[*i]|(i16::try_from(target as isize-*i as isize-1)? as u16 as u32)};}
        Ok((self.w.iter().flat_map(|w|w.to_le_bytes()).collect(),self.labels.into_iter().map(|(k,v)|(k,CODE+v as u32*4)).collect()))
    }
}

pub fn build()->Result<(Vec<u8>,BTreeMap<&'static str,u32>)> {build_impl(false)}
fn build_impl(observable:bool)->Result<(Vec<u8>,BTreeMap<&'static str,u32>)> {
    let mut a=Asm{w:Vec::new(),labels:BTreeMap::new(),fix:Vec::new()};
    // The retail frame has unused argument-home slots +8/+12. Every entry
    // initializes them; nested queries bypass only our redirect lookup.
    for (name,mode) in [("entry",false),("query",true)] {
        a.label(name);for w in [addiu(SP,SP,(-40i16) as u16),addu(A3,A0,ZERO),sw(A0,SP,12),sw(if mode {A1}else{ZERO},SP,8),j(ENTRY+8),0]{a.emit(w);}
    }
    a.label("grid");for w in [lhu(A0,A0,0),lw(T0,SP,8),addiu(T1,ZERO,2)]{a.emit(w);}
    a.branch(bne(T0,T1,0),"grid_branch");a.emit(andi(A0,A0,0xe7ff));
    a.label("grid_branch");a.emit(bltz(V1,3));a.emit(ori(S3,S3,0x35c));a.emit(j(0x80019308));a.emit(0);a.emit(j(0x80019348));a.emit(0);
    a.label("result");for w in [addu(V0,S1,ZERO),lw(T0,SP,8),0]{a.emit(w);}
    a.branch(bne(T0,ZERO,0),"finish");
    if observable {for w in [sw(S1,SP,0),sw(ZERO,SP,4),addu(S4,ZERO,ZERO)]{a.emit(w);}}
    for w in [lw(S0,SP,12),lui(T0,0x1f80),lw(T0,T0,0x3ec),lui(T1,1),addu(T0,T0,T1),lhu(T1,T0,0),0,addiu(T1,T1,3),andi(T1,T1,0xfffc),sltiu(T2,T1,0x1ff9)]{a.emit(w);}
    a.branch(beq(T2,ZERO,0),"finish");
    for w in [addiu(T9,ZERO,0x2000),subu(T9,T9,T1),addu(T0,T0,T1),lw(T1,T0,0),lui(T2,(MAGIC>>16) as u16),ori(T2,T2,MAGIC as u16)]{a.emit(w);}
    a.branch(bne(T1,T2,0),"finish");for w in [lhu(T1,T0,4),addiu(T2,ZERO,1)]{a.emit(w);}
    a.branch(bne(T1,T2,0),"finish");for w in [lhu(T1,T0,6),0,sltiu(T2,T1,(MAX_ROWS+1) as u16)]{a.emit(w);}
    a.branch(beq(T2,ZERO,0),"finish");
    for w in [sll(T2,T1,3),sll(T6,T1,2),addu(T2,T2,T6),addiu(T2,T2,if observable {12}else{8})]{a.emit(w);}
    if observable {a.emit(addu(T6,T0,T2));}
    a.emit(sltu(T2,T9,T2));
    a.branch(bne(T2,ZERO,0),"finish");
    if observable {for w in [sw(T0,SP,4),lhu(T6,T6,0xfffc),0,sll(S4,T6,16)]{a.emit(w);}}
    for w in [lhu(T2,S0,0x14),lhu(T3,S0,0x18),srl(T4,T2,6),srl(T5,T3,6),addiu(T0,T0,8)]{a.emit(w);}
    a.label("loop");a.branch(beq(T1,ZERO,0),"finish");a.emit(lbu(T6,T0,0));a.emit(lbu(T7,T0,1));
    a.branch(bne(T6,T4,0),"next");a.branch(bne(T7,T5,0),"next");
    for w in [lbu(A1,T0,2),0,addiu(T6,A1,(-1i16) as u16),sltiu(T6,T6,2)]{a.emit(w);}
    a.branch(beq(T6,ZERO,0),"finish");
    if observable {a.emit(or(S4,S4,A1));}
    for w in [addu(S2,T0,ZERO),lw(T6,S0,0x10),lh(T7,S2,4),lh(T8,S2,6),addu(T2,T2,T7),addu(T3,T3,T8),sltiu(T9,T2,0x4000)]{a.emit(w);}
    a.branch(beq(T9,ZERO,0),"finish");a.emit(sltiu(T9,T3,0x4000));a.branch(beq(T9,ZERO,0),"finish");
    for w in [addiu(SP,SP,(-32i16) as u16),sw(T6,SP,0x10),sh(T2,SP,0x14),sh(T3,SP,0x18),addu(A0,SP,ZERO)]{a.emit(w);}
    // Query address is already bound. A real jal and a non-load delay slot.
    a.emit(jal(CODE+6*4));a.emit(0);
    for w in [addu(S1,V0,ZERO),lh(T0,S2,8),lw(T1,SP,0x10),addu(S1,S1,T0),sw(T1,S0,0x10),addiu(SP,SP,32),addu(V0,S1,ZERO)]{a.emit(w);}
    a.jump("finish");a.label("next");a.emit(addiu(T1,T1,(-1i16) as u16));a.emit(addiu(T0,T0,12));a.jump("loop");
    a.label("finish");
    if observable {emit_observer(&mut a);}
    a.label("return");a.emit(lw(RA,SP,0x24));a.emit(lw(S4,SP,0x20));a.emit(j(0x80019590));a.emit(0);
    let result=a.finish()?;ensure!(CODE+result.0.len() as u32<=CODE_END,"Extended floor code exceeds guarded arena");Ok(result)
}

/// Caller also fingerprints the entire retail SCUS/overlay profile. Every
/// overwritten instruction and allocated byte is independently guarded here.
pub fn plan(exe:&[u8])->Result<Vec<Edit>> {plan_impl(exe,false)}
fn plan_impl(exe:&[u8],observable:bool)->Result<Vec<Edit>> {
    ensure!(exe.starts_with(b"PS-X EXE"),"Not a PS-X executable");
    let base=u32::from_le_bytes(exe.get(0x18..0x1c).context("Truncated EXE")?.try_into()?);
    let offset=|va:u32|->Result<usize>{Ok(usize::try_from(va.checked_sub(base).context("Native address below executable")?)?+0x800)};
    let (code,labels)=build_impl(observable)?;let start=offset(CODE)?;
    ensure!(exe.get(start..start+code.len()).is_some_and(|b|b.iter().all(|&v|v==0)),"Native floor arena occupied; incompatible engine mods");
    ensure!(!SCUS_TABLE_RANGES.iter().any(|&(s,e)|CODE<e&&s<CODE+code.len() as u32),"Native floor code overlaps live table");
    let mut edits=vec![Edit{prot_index:None,file_off:start,bytes:code}];
    for (va,expected,label) in [(ENTRY,[0x27bdffd8u32,0x00803821],"entry"),(0x800192fc,[0x94840000,0x04600011],"grid"),(0x80019588,[0x8fbf0024,0x8fb40020],"result")] {
        let off=offset(va)?;let old:Vec<u8>=expected.iter().flat_map(|w|w.to_le_bytes()).collect();
        ensure!(exe.get(off..off+8)==Some(old.as_slice()),"Extended floor hook mismatch at {va:#x}");
        edits.push(Edit{prot_index:None,file_off:off,bytes:[j(labels[label]),0].iter().flat_map(|w|w.to_le_bytes()).collect()});
    }Ok(edits)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn versioned_floor_rows_roundtrip_and_reject_invalid_mappings(){
        let rows=[Row{destination:[10,20],mode:1,delta:[64,-64],height_delta:-128},Row{destination:[11,20],mode:2,delta:[0;2],height_delta:0}];
        assert_eq!(decode(&encode(&rows).unwrap()).unwrap(),rows);
        assert!(encode(&[rows[0].clone(),rows[0].clone()]).is_err());
        let mut bad=rows[0].clone();bad.delta[0]=1;assert!(encode(&[bad]).is_err());
        let mut bytes=encode(&rows).unwrap();bytes[4]=2;assert!(decode(&bytes).is_err());
    }
    #[test] fn observable_code_fits_arena(){let (code,_)=build_impl(true).unwrap();println!("Observable code bytes {}",code.len());}
    #[test] fn code_fits_existing_guarded_ram_without_allocation(){let (code,labels)=build().unwrap();assert!(code.len()<896);assert_eq!(labels["entry"],CODE);assert_eq!(labels["query"],CODE+24);}
}
