//! Offline original-SCUS allocator replay on a historical RAM snapshot.
//! Build with rustc --edition=2024 scripts/replay-native-heap.rs -o <new-tool.exe>.
//! Args: SCUS RAM requests.tsv output.tsv. TSV: A <synthetic-id> <bytes>, F <id>.
//! This is a bounded historical-state forecast, NOT a current runtime capture.
#[path="../crates/code-hooks/src/mips_sim.rs"]
mod mips_sim;
use mips_sim::Cpu;
use std::{collections::BTreeMap, error::Error};
fn call(c:&mut Cpu,entry:u32,a0:u32,a1:u32)->u32 {
    c.r[4]=a0;c.r[5]=a1;c.r[28]=0x8007b318;c.r[29]=0x801ff000;c.r[31]=0x8ffffffc;c.pc=entry;
    let start=c.steps;
    while c.pc!=0x8ffffffc {
        assert!(c.steps-start<100000,"allocator instruction budget");
        if c.pc==0x800567a8 {c.r[2]=0;c.pc=c.r[31];continue;}
        assert!((0x80010000..0x8007b318).contains(&c.pc),"allocator escaped SCUS");
        let w=c.rd32(c.pc);c.exec(w);
    }c.r[2]
}
fn free(c:&Cpu)->(usize,usize) {
    let desc=c.rd32(0x8007bb58);assert!((0x80000000..0x80200000).contains(&desc));
    let top=c.rd32(desc);let sentinel=top+12;let mut p=c.rd32(top+16);
    let(mut total,mut largest,mut n)=(0,0,0);
    while p!=sentinel {assert!((0x80000000..0x801ffff4).contains(&p)&&n<4096);let size=c.rd32(p+8)as usize;total+=size;largest=largest.max(size);p=c.rd32(p+4);n+=1;}
    (total,largest)
}
fn main()->Result<(),Box<dyn Error>> {
    let a=std::env::args().collect::<Vec<_>>();assert_eq!(a.len(),5,"SCUS RAM requests.tsv new-output.tsv");
    let exe=std::fs::read(&a[1])?;let ram=std::fs::read(&a[2])?;assert_eq!(ram.len(),0x200000);
    let mut c=Cpu::new();c.load_delays=true;c.strict_alignment=true;c.load(0x80000000,&ram);
    let base=u32::from_le_bytes(exe[0x18..0x1c].try_into()?);
    for(lo,hi)in[(0x80017888u32,0x800178f0),(0x80017b94,0x80017bec),(0x8002b468,0x8002b688)]{
        let at=0x800+(lo-base)as usize;c.load(lo,&exe[at..at+(hi-lo)as usize]);
    }
    let mut pointers=BTreeMap::new();let mut out=String::from("action\tid\tbytes\treturned\tfree\tlargest\n");
    let initial=free(&c);out+=&format!("INITIAL\t0\t0\t0\t{}\t{}\n",initial.0,initial.1);
    for line in std::fs::read_to_string(&a[3])?.lines() {
        let fields=line.split_whitespace().collect::<Vec<_>>();let id=fields[1].parse::<u64>()?;
        if fields[0]=="A" {
            let bytes=fields[2].parse::<u32>()?;let p=call(&mut c,0x80017888,0,bytes);let f=free(&c);
            out+=&format!("A\t{id}\t{bytes}\t{p}\t{}\t{}\n",f.0,f.1);
            if p==0 {break;}pointers.insert(id,p);
        }else {assert_eq!(fields[0],"F");let p=pointers.remove(&id).ok_or("free without allocation")?;call(&mut c,0x80017b94,p,0);let f=free(&c);out+=&format!("F\t{id}\t0\t{p}\t{}\t{}\n",f.0,f.1);}
    }
    use std::io::Write;
    std::fs::OpenOptions::new().write(true).create_new(true).open(&a[4])?.write_all(out.as_bytes())?;
    Ok(())
}
