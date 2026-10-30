//! Optional, reversible BIOS TTY instrumentation. Never an engine compatibility
//! fix: two original call paths print begin/end records around their real work.
//! No allocation, file writes, geometry changes or host/emulator connection.
use anyhow::{Result,ensure,Context};
use crate::{mips::*,arenas::{Edit,SCUS_TABLE_RANGES}};

const PRINT:u32=0x800567a8;
const DISPATCH:u32=0x8001f05c;
const UPLOAD:u32=0x800198e0;
pub const DISPATCH_CODE:u32=0x80077728;
pub const UPLOAD_CODE:u32=0x8007ae00;

fn shim(code:u32,target:u32,gpu:bool)->Result<Vec<u8>>{
    let mut w=vec![addiu(SP,SP,(-48i16)as u16),sw(RA,SP,44)];
    for(r,o)in [(A0,16),(A1,20),(A2,24),(A3,28)]{w.push(sw(r,SP,o));}
    let mut formats=vec![];
    for after in [false,true]{
        formats.push(w.len());w.extend([0,0]);
        if after {w.extend([lw(A1,SP,20),lw(A2,SP,32)]);}
        else if gpu {w.extend([lw(A2,SP,16),0,lw(A1,A2,4)]);}
        else {w.extend([lw(A1,SP,20),lw(A2,SP,16)]);}
        w.extend([lui(T0,0x8008),lw(A3,T0,0xb828),jal(PRINT),0]);
        if !after {
            for(r,o)in [(A0,16),(A1,20),(A2,24),(A3,28)]{w.push(lw(r,SP,o));}
            w.extend([jal(target),0,sw(V0,SP,32),sw(V1,SP,36)]);
        }
    }
    w.extend([lw(V0,SP,32),lw(V1,SP,36),lw(RA,SP,44),addiu(SP,SP,48),jr(RA),0]);
    let mut bytes=w.iter().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>();
    let strings=if gpu { ["LCK GPU BEGIN %08x %08x err=%08x\n\0","LCK GPU DONE %08x %08x err=%08x\n\0"] }else{["LCK LOAD BEGIN %08x %08x err=%08x\n\0","LCK LOAD DONE %08x %08x err=%08x\n\0"]};
    for(i,s)in strings.iter().enumerate(){let address=code+bytes.len()as u32;let pos=formats[i]*4;bytes[pos..pos+4].copy_from_slice(&lui(A0,(address>>16)as u16).to_le_bytes());bytes[pos+4..pos+8].copy_from_slice(&ori(A0,A0,address as u16).to_le_bytes());bytes.extend_from_slice(s.as_bytes());}
    ensure!(bytes.len()<=256,"TTY trace exceeds guarded 256-byte arena");Ok(bytes)
}

pub fn plan(exe:&[u8])->Result<Vec<Edit>>{
    ensure!(exe.starts_with(b"PS-X EXE"),"Expected PS-X executable");let load=u32::from_le_bytes(exe.get(0x18..0x1c).context("EXE load address")?.try_into()?);
    let off=|va:u32|->Result<usize>{Ok(0x800+va.checked_sub(load).context("Trace address below load")? as usize)};
    let mut edits=vec![];
    // The first arena is also used by the OPTIONAL floor observer. Zero guards
    // reject that combination; never overwrite either existing diagnostic.
    for(code,target,gpu)in [(DISPATCH_CODE,DISPATCH,false),(UPLOAD_CODE,UPLOAD,true)]{
        let bytes=shim(code,target,gpu)?;let start=off(code)?;
        ensure!(!SCUS_TABLE_RANGES.iter().any(|&(s,e)|code<e&&s<code+bytes.len()as u32),"Trace intersects a live native table");
        ensure!(exe.get(start..start+bytes.len()).is_some_and(|s|s.iter().all(|b|*b==0)),"Native trace arena occupied at {code:#x}; existing engine patch retained");
        edits.push(Edit{prot_index:None,file_off:start,bytes});
    }
    for (site,target,trace)in [(0x800202b4,DISPATCH,DISPATCH_CODE),(0x80025564,DISPATCH,DISPATCH_CODE),(0x8001f12c,UPLOAD,UPLOAD_CODE),(0x8001f1c8,UPLOAD,UPLOAD_CODE),(0x800254a8,UPLOAD,UPLOAD_CODE)]{
        let start=off(site)?;ensure!(exe.get(start..start+4)==Some(jal(target).to_le_bytes().as_slice()),"Unrecognized native loader call at {site:#x}");
        edits.push(Edit{prot_index:None,file_off:start,bytes:jal(trace).to_le_bytes().to_vec()});
    }Ok(edits)
}

// Trace the existing wrappers' calls into the ORIGINAL allocator/free bodies.
// No heap size, error handling, allocation order or return value is changed.
fn heap_shim(code:u32,free:bool)->Vec<u8>{
    let target=if free{0x8002b584}else{0x8002b468};
    let mut w=vec![addiu(SP,SP,(-32i16)as u16),sw(RA,SP,28),jal(target),0,
        sw(V0,SP,16),sw(V1,SP,20),0,0,
        addu(A1,if free{S0}else{S1},ZERO),addu(A2,V0,ZERO),
        lw(A3,SP,if free{52}else{56}),jal(PRINT),0,
        lw(V0,SP,16),lw(V1,SP,20),lw(RA,SP,28),addiu(SP,SP,32),jr(RA),0];
    let address=code+w.len()as u32*4;
    w[6]=lui(A0,(address>>16)as u16);w[7]=ori(A0,A0,address as u16);
    let mut bytes=w.iter().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>();
    bytes.extend_from_slice(if free{b"LCK FREE %08x %08x RA=%08x\n\0"}else{b"LCK HEAP %08x %08x RA=%08x\n\0"});bytes
}

/// Separate diagnostic on an uninstrumented source: log every wrapper malloc
/// size/result/caller and free pointer/result/caller, including first-frame work.
pub fn allocation_plan(exe:&[u8])->Result<Vec<Edit>>{
    ensure!(exe.starts_with(b"PS-X EXE"),"Expected PS-X executable");
    let load=u32::from_le_bytes(exe.get(0x18..0x1c).context("EXE load address")?.try_into()?);
    let off=|va:u32|->Result<usize>{Ok(0x800+va.checked_sub(load).context("Trace below EXE")?as usize)};
    let mut edits=vec![];
    for(code,free,site,target)in [(DISPATCH_CODE,false,0x800178a4,0x8002b468),(UPLOAD_CODE,true,0x80017bd4,0x8002b584)]{
        let bytes=heap_shim(code,free);ensure!(bytes.len()<=256,"Heap trace exceeds reserved gap");
        ensure!(!SCUS_TABLE_RANGES.iter().any(|&(s,e)|code<e&&s<code+bytes.len()as u32),"Heap trace intersects live table");
        let at=off(code)?;ensure!(exe.get(at..at+bytes.len()).is_some_and(|b|b.iter().all(|v|*v==0)),"Heap trace gap occupied; existing patch retained");
        let call=off(site)?;ensure!(exe.get(call..call+4)==Some(jal(target).to_le_bytes().as_slice()),"Unrecognized native heap call");
        edits.push(Edit{prot_index:None,file_off:at,bytes});
        edits.push(Edit{prot_index:None,file_off:call,bytes:jal(code).to_le_bytes().to_vec()});
    }Ok(edits)
}

#[cfg(test)]
mod tests{
    use super::*;use crate::mips_sim::Cpu;
    #[test]
    fn heap_trace_preserves_native_call_and_returns_and_reports_original_caller(){
        for free in [false,true]{
            let code=if free{UPLOAD_CODE}else{DISPATCH_CODE};let target=if free{0x8002b584}else{0x8002b468};
            let mut cpu=Cpu::new();cpu.load_delays=true;cpu.strict_alignment=true;cpu.load(code,&heap_shim(code,free));cpu.pc=code;
            cpu.r[29]=0x801ff000;cpu.r[31]=0x8ffffffc;cpu.r[4]=0x80100000;cpu.r[5]=8368;cpu.r[16]=0x80100000;cpu.r[17]=8368;
            cpu.wr32(0x801ff000+if free{20}else{24},0x80020f00);
            let mut calls=0;let mut prints=0;
            while cpu.pc!=0x8ffffffc{assert!(cpu.steps<500);
                if cpu.pc==target{calls+=1;assert_eq!((cpu.r[4],cpu.r[5]),(0x80100000,8368));cpu.r[2]=0x80120000;cpu.r[3]=17;cpu.pc=cpu.r[31];}
                else if cpu.pc==PRINT{prints+=1;assert_eq!((cpu.r[5],cpu.r[6],cpu.r[7]),(if free{0x80100000}else{8368},0x80120000,0x80020f00));for i in 2..16{cpu.r[i]=0xdead0000+i as u32;}cpu.pc=cpu.r[31];}
                else{cpu.exec(cpu.rd32(cpu.pc));}}
            assert_eq!((calls,prints),(1,1));assert_eq!((cpu.r[2],cpu.r[3],cpu.r[29]),(0x80120000,17,0x801ff000));assert_eq!((cpu.r[16],cpu.r[17]),(0x80100000,8368));
        }
    }
    #[test]
    fn tracing_retains_real_arguments_returns_stack_and_saved_registers(){
        for gpu in [false,true]{let code=if gpu{UPLOAD_CODE}else{DISPATCH_CODE};let target=if gpu{UPLOAD}else{DISPATCH};let mut cpu=Cpu::new();cpu.load_delays=true;cpu.strict_alignment=true;cpu.load(code,&shim(code,target,gpu).unwrap());cpu.pc=code;cpu.r[29]=0x801ff000;cpu.r[31]=0x8ffffffc;cpu.r[4]=0x80100000;cpu.r[5]=0x02123456;cpu.r[6]=9;cpu.r[7]=1;cpu.wr32(0x80100004,8);for i in 16..24{cpu.r[i]=0x12340000+i as u32;}let mut ran=0;let mut prints=0;
            while cpu.pc!=0x8ffffffc{assert!(cpu.steps<500);if cpu.pc==PRINT{prints+=1;for i in 4..16{cpu.r[i]=0xf0000000+i as u32;}cpu.r[2]=42;cpu.pc=cpu.r[31];}else if cpu.pc==target{assert_eq!(&cpu.r[4..8],&[0x80100000,0x02123456,9,1]);ran+=1;cpu.r[2]=0xaabbccdd;cpu.r[3]=0x99887766;cpu.pc=cpu.r[31];}else{cpu.exec(cpu.rd32(cpu.pc));}}
            assert_eq!((ran,prints),(1,2));assert_eq!((cpu.r[2],cpu.r[3],cpu.r[29]),(0xaabbccdd,0x99887766,0x801ff000));for i in 16..24{assert_eq!(cpu.r[i],0x12340000+i as u32);}
        }
    }
}
