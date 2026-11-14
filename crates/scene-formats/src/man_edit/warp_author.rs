// LCK_R620_INDEPENDENT_CROSS_CELL
// Append independently addressable partition-2 records for walk-on scene changes.
// New MAN data goes before section 0; existing record bodies and section bytes
// remain byte-identical, with count/table/section pointers adjusted.
use crate::{field_disasm, man_section};

#[derive(Debug,Clone,PartialEq,Eq)]
pub struct IndependentWarp {
    pub slot:u8,                    // P2-relative gate-1 MAP record number
    pub name:String,                // destination CDNAME
    pub index:i16,                  // native 0x3F scene index operand
    pub entry_x:u8,
    pub entry_z:u8,
    pub dir:u8,
}

fn put_u24(out:&mut Vec<u8>, value:usize) -> Result<(),String> {
    if value > 0xFF_FFFF { return Err("MAN 24-bit offset overflow".into()); }
    out.push(value as u8);
    out.push((value>>8) as u8);
    out.push((value>>16) as u8);
    Ok(())
}

fn p2_record(w:&IndependentWarp) -> Result<(Vec<u8>,usize),String> {
    if w.name.is_empty() || w.name.len()>255 ||
        !w.name.bytes().all(|b|b.is_ascii_lowercase() ||
                                 b.is_ascii_digit() || b==b'_') {
        return Err(format!("Invalid destination CDNAME '{}'",w.name));
    }
    if w.dir>7 { return Err("Cross-cell facing exceeds 7".into()); }
    // Uniform 2-byte ASCII pairs are a validated MAN named-record encoding.
    let title=format!("LCKW{:03}",w.slot);
    let mut out=Vec::new();
    out.push(title.len() as u8);
    for b in title.bytes(){out.push(b);out.push(0);}
    // Empty P2 condition lists: unconditional activation.
    out.extend_from_slice(&[0,0,0]);
    let opcode_offset=out.len();
    out.push(0x3F);
    out.extend_from_slice(&w.index.to_le_bytes());
    out.push(w.name.len() as u8);
    out.extend_from_slice(w.name.as_bytes());
    out.extend_from_slice(&[w.entry_x,w.entry_z,w.dir]);
    out.push(0x21); // native NOP after scene-change instruction
    Ok((out,opcode_offset))
}

pub fn append_independent_warps(man:&[u8], warps:&[IndependentWarp])
    -> Result<Vec<u8>,String> {
    if warps.is_empty(){return Ok(man.to_vec());}
    let original=man_section::parse(man).map_err(|e|e.to_string())?;
    let flat=original.header.total_records();
    let n2=original.header.partition_counts[2] as usize;
    if n2+warps.len()>256 || n2+warps.len()>i16::MAX as usize {
        return Err("Native MAN P2 record table exceeds gate-1 trigger index capacity".into());
    }
    let data=original.data_region_offset;
    let sec0=original.sections[0].offset;
    let relative=original.header.u24_at_28 as usize;
    if sec0<data || relative!=sec0-data {
        return Err("MAN data/section relationship is invalid".into());
    }
    for part in &original.partitions {
        if part.iter().any(|&offset|offset as usize>=relative) {
            return Err("A native MAN record points into the section chain".into());
        }
    }
    let mut generated=Vec::new();
    let mut offsets=Vec::new();
    let mut opcode_offsets=Vec::new();
    for (i,w) in warps.iter().enumerate() {
        if w.slot as usize!=n2+i {
            return Err(format!("Cross-cell warp slot {} must be {}",w.slot,n2+i));
        }
        let position=relative+generated.len();
        offsets.push(position);
        let (rec,pc)=p2_record(w)?;
        opcode_offsets.push(position+pc);
        generated.extend_from_slice(&rec);
    }
    let new_relative=relative.checked_add(generated.len())
        .ok_or("MAN section offset overflow")?;
    if new_relative>0xFF_FFFF {
        return Err("MAN section pointer exceeds 24 bits".into());
    }
    let mut out=Vec::with_capacity(man.len()+3*warps.len()+generated.len());
    out.extend_from_slice(&man[..data]);
    for offset in &offsets {put_u24(&mut out,*offset)?;}
    out.extend_from_slice(&man[data..sec0]);
    out.extend_from_slice(&generated);
    out.extend_from_slice(&man[sec0..]);
    out[0x26..0x28].copy_from_slice(&((n2+warps.len()) as i16).to_le_bytes());
    out[0x28]=(new_relative&255) as u8;
    out[0x29]=((new_relative>>8)&255) as u8;
    out[0x2A]=((new_relative>>16)&255) as u8;
    let checked=man_section::parse(&out).map_err(|e|e.to_string())?;
    if checked.header.total_records()!=flat+warps.len() ||
       checked.sections[0].offset!=sec0+3*warps.len()+generated.len() ||
       checked.partitions[0]!=original.partitions[0] ||
       checked.partitions[1]!=original.partitions[1] ||
       checked.partitions[2][..n2]!=original.partitions[2][..] ||
       out[checked.sections[0].offset..]!=man[sec0..] {
        return Err("Cross-cell MAN reconstruction failed structural verification".into());
    }
    for (i,w) in warps.iter().enumerate() {
        let pc=checked.data_region_offset+opcode_offsets[i];
        let instruction=field_disasm::decode(&out,pc)
            .map_err(|e|format!("New warp decode: {e}"))?;
        let Some(name)=field_disasm::scene_change_name(&out,&instruction) else {
            return Err("New MAN script has no decodable 0x3F scene name".into());
        };
        match instruction.info {
            field_disasm::InsnInfo::SceneChange {index,entry_x,entry_z,dir,..}
                if index==w.index && name==w.name &&
                   entry_x==w.entry_x && entry_z==w.entry_z && dir==w.dir => {},
            _=>return Err("Cross-cell MAN operand verification failed".into()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod lck_r620_independent_warp_tests {
    use super::*;
    fn blank_man()->Vec<u8> {
        let mut result=vec![0u8;0x2B];
        result.extend_from_slice(&[0;18]); // six empty native sections
        result
    }
    fn warp(slot:u8,x:u8)->IndependentWarp {
        IndependentWarp{slot,name:"map01".into(),index:7,
            entry_x:x,entry_z:20,dir:2}
    }
    #[test]
    fn two_new_warps_have_independent_partition_two_scripts() {
        let input=blank_man();
        let output=append_independent_warps(&input,&[warp(0,12),warp(1,35)]).unwrap();
        let parsed=man_section::parse(&output).unwrap();
        assert_eq!(parsed.header.partition_counts[2],2);
        assert_ne!(parsed.partitions[2][0],parsed.partitions[2][1]);
        assert_eq!(&output[parsed.sections[0].offset..],&input[0x2B..]);
        let sites=super::super::scene_change_sites(&output);
        assert_eq!(sites.len(),2);
        assert_eq!(sites[0].entry_x,12);
        assert_eq!(sites[1].entry_x,35);
    }
    #[test]
    fn invalid_partition_two_slot_is_rejected() {
        assert!(append_independent_warps(&blank_man(),&[warp(3,8)]).is_err());
    }
}