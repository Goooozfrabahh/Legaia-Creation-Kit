//! Retail CPU-routine tests only. Never starts the LCK application/emulator,
//! modifies executable instructions, or produces an exportable game image.
use anyhow::{Result,Context,ensure};
use std::path::PathBuf;

#[test]
#[ignore="explicit original-disc SCUS floor-routine investigation; no GUI or IMG writes"]
fn retail_floor_instruction_probe()->Result<()> {
    let path=PathBuf::from(std::env::var_os("LCK_RETAIL_SOURCE").context("LCK_RETAIL_SOURCE required")?);
    let library=crate::library::Library::open(&path)?;
    let exe=library.iso_files.iter().find(|f|f.path.eq_ignore_ascii_case("SCUS_942.54")).context("NTSC-U executable unavailable; no address assumptions for other releases")?;
    let bytes=library.iso_file_bytes(exe)?;
    ensure!(super::collision_recipes::digest(&bytes)=="292256e2e66db42727f613406785e444254d3f699569e611f65fcf1c6d2f3482","Unknown SCUS profile; native addresses must be re-established");
    ensure!(bytes.starts_with(b"PS-X EXE"),"Not a native PS-X EXE");
    let base=u32::from_le_bytes(bytes[0x18..0x1c].try_into()?);
    let gp=u32::from_le_bytes(bytes[0x14..0x18].try_into()?);
    let address=0x80019278u32;let offset=(address-base) as usize+0x800;
    println!("SCUS SHA256 {}; base {base:#x}; GP {gp:#x}",super::collision_recipes::digest(&bytes));
    for (i,word) in bytes[offset..offset+0x340].chunks_exact(4).enumerate() {
        let w=u32::from_le_bytes(word.try_into()?);
        println!("{:08x}: {w:08x} op={:02x} rs={} rt={} rd={} sa={} imm={:04x}",address+i as u32*4,w>>26,(w>>21)&31,(w>>16)&31,(w>>11)&31,(w>>6)&31,w&65535);
    }
    let overlay=library.index.entry_bytes(897)?;
    ensure!(super::collision_recipes::digest(&overlay)=="0b39d892d783ffcc89dac06ee395540be0a7392c5cbc79fb9ba3442daccc09f6","Unknown field-overlay profile");
    let overlay_base=0x801ce818u32;
    println!("Overlay 897 SHA256 {}; bytes {}",super::collision_recipes::digest(&overlay),overlay.len());
    for va in [0x801d5630u32,0x801d5ae0] {
        let at=(va-overlay_base) as usize;
        println!("Overlay function {va:#x} first words: {:02x?}",&overlay[at..at+32]);
    }
    let mut cpu=legaia_code_hooks::mips_sim::Cpu::new();
    cpu.load(base,&bytes[0x800..]);cpu.load(overlay_base,&overlay);
    const MAP:u32=0x80100000;const ACTOR:u32=0x801b0000;const STOP:u32=0x80000080;
    cpu.load(MAP,&vec![0;0x14000]);cpu.wr32(0x1f8003ec,MAP);
    cpu.load(0x8007b318,&[4,4,4,8]);
    cpu.wr16(0x1f80035c,0);cpu.wr16(0x1f80035e,(-192i16) as u16);
    cpu.wr16(MAP+0x1000a,18);cpu.wr16(MAP+0x1000c,1);
    cpu.load(MAP+0x10012,&[37,18,2,0xcc]);
    let grid=MAP+0x8000+(18*128+37)*2;cpu.wr16(grid,0x800);
    for (x,z,expected) in [(37*128+32,18*128+32,-64),(37*128+96,18*128+32,-112),(37*128+32,18*128+96,-64),(37*128+96,18*128+96,-112)] {
        cpu.r=[0;32];cpu.r[4]=ACTOR;cpu.r[28]=0x8007b318;cpu.r[29]=0x801bff00;cpu.r[31]=STOP;
        cpu.wr16(ACTOR+0x14,x);cpu.wr16(ACTOR+0x18,z);cpu.pc=address;cpu.steps=0;
        cpu.run_until(&[STOP]);
        println!("Retail routine x{x} z{z}: result {}, expected {expected}, {} CPU steps",cpu.r[2] as i32,cpu.steps);
        ensure!(cpu.r[2] as i32==expected,"Retail sampler differs from encoding assumptions");
    }
    // A second row at the SAME key cannot subdivide the native tile. Native
    // lookup returns the first row, irrespective of the sampled quadrant.
    cpu.wr16(MAP+0x1000c,2);cpu.load(MAP+0x10016,&[37,18,8,0]);
    for (dx,dz,expected) in [(32,32,-64),(96,96,-112)] {
        cpu.r=[0;32];cpu.r[4]=ACTOR;cpu.r[29]=0x801bff00;cpu.r[31]=STOP;
        cpu.wr16(ACTOR+0x14,37*128+dx);cpu.wr16(ACTOR+0x18,18*128+dz);cpu.pc=address;cpu.steps=0;cpu.run_until(&[STOP]);
        ensure!(cpu.r[2] as i32==expected,"Native lookup unexpectedly split duplicate-key height rows");
    }
    println!("Retail duplicate-key lookup: first record wins; no per-subcell row subdivision");
    // Exercise the real data-only serializer, including growth of kind-2 and
    // relocation of kind-3 table bytes. The code instructions remain untouched.
    use super::native_floor_patch::{self,Edit};
    for (duplicate,destination,turns) in [(false,Some([35,18]),0),(true,Some([35,18]),1),(false,None,0)] {
        let source=native_floor_patch::fixture();
        let edit=Edit{expected:[37,18,2,0xcc],destination,duplicate,quarter_turns:turns,height_delta:0};
        let plan=native_floor_patch::compile(&source,&[0;16],&[edit])?;let transformed=plan.apply(&source)?;
        cpu.load(MAP,&transformed);
        for cell in [[37u16,18u16],[35,18]] {for (i,(dx,dz)) in [(32,32),(96,32),(32,96),(96,96)].into_iter().enumerate() {
            cpu.r=[0;32];cpu.r[4]=ACTOR;cpu.r[29]=0x801bff00;cpu.r[31]=STOP;
            cpu.wr16(ACTOR+0x14,cell[0]*128+dx);cpu.wr16(ACTOR+0x18,cell[1]*128+dz);cpu.pc=address;cpu.steps=0;cpu.run_until(&[STOP]);
            let source_kept=duplicate && cell==[37,18];let target=destination==Some([cell[0] as u8,cell[1] as u8]);
            let expected=if source_kept {[-64,-112,-64,-112][i]}else if target {if turns==1 {[-112,-112,-64,-64][i]}else{[-64,-112,-64,-112][i]}}else{0};
            ensure!(cpu.r[2] as i32==expected,"Serialized native floor move/copy/rotate/delete disagrees with retail CPU routine");
        }}
        cpu.r=[0;32];cpu.r[4]=3;cpu.r[5]=1;cpu.r[6]=2;cpu.r[29]=0x801bff00;cpu.r[31]=STOP;cpu.pc=0x801d5630;cpu.steps=0;
        cpu.run_until(&[STOP]);let ptr=cpu.r[2];
        ensure!(ptr!=0 && (0..8).map(|i|cpu.rd8(ptr+i)).collect::<Vec<_>>()==[1,2,3,4,5,6,7,8],"Native region lookup changed after floor-table relocation");
    }
    println!("Retail CPU routine accepted serialized native full-tile move, duplicate, quarter rotation and deletion; retained region lookup unchanged");
    let at=MAP+0x4000+19*128+34;
    cpu.load(at,&[0,0]);cpu.load(at+128,&[1,1]);
    cpu.wr16(MAP+0x8000+(19*128+34)*2,0x1ff);
    cpu.load(MAP+511*32,&[0xab;32]);
    for (dz,expected) in [(32,-48),(96,-144)] {
        cpu.r=[0;32];cpu.r[4]=ACTOR;cpu.r[29]=0x801bff00;cpu.r[31]=STOP;cpu.wr32(ACTOR+0x10,0);
        cpu.wr16(ACTOR+0x14,34*128+96);cpu.wr16(ACTOR+0x18,19*128+dz);cpu.pc=address;cpu.steps=0;cpu.run_until(&[STOP]);
        ensure!(cpu.r[2] as i32==expected,"Retail bilinear branch differs from retained slope model");
    }
    println!("Retail bilinear branch returned -48/-144 on retained slope; changing geometry descriptor bytes did not turn them into floor collision");
    Ok(())
}
