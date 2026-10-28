//! Native kind-2 height representability. This codec does not infer surface
//! ownership, allocate MAP rows or authorize an IMG export.
use std::collections::{BTreeMap,BTreeSet};
use anyhow::{Result,ensure,bail};
use legaia_engine_field::field_elevation::ElevationOverride;

#[derive(Clone,Copy,Debug)]
pub(super) struct Sample {pub subcell:[u8;2],pub height:i32}
#[derive(Clone,Copy,Debug)]
pub(super) struct Destination {
    pub floor_mean:i32,
    /// Real retained native surface heights, not guessed from mesh bounds.
    pub retained:[i32;4],
    /// A bilinear slope cannot be losslessly replaced by four constant steps.
    pub piecewise_constant:bool,
}
pub(super) fn decode(record:&ElevationOverride,floor_mean:i32)->Result<[Sample;4]> {
    ensure!(record.tile_x<128 && record.tile_z<128,"Elevation source outside native grid");
    let mut samples=[Sample{subcell:[0,0],height:0};4];
    for (i,sample) in samples.iter_mut().enumerate() {
        let x=record.tile_x*2+(i%2) as u8;let z=record.tile_z*2+(i/2) as u8;
        *sample=Sample{subcell:[x,z],height:floor_mean.checked_add(record.delta_at(x as i32*64,z as i32*64)).ok_or_else(||anyhow::anyhow!("Floor height overflow"))?};
    }Ok(samples)
}
/// Four unsigned 2-bit steps share one signed coarse byte. Changing the
/// floor mean shifts all four together; it cannot increase their 48-unit span.
pub(super) fn encode(cell:[u8;2],offsets:[i32;4])->Result<ElevationOverride> {
    ensure!(cell.iter().all(|&c|c<128),"Elevation cell outside native grid");
    let low=*offsets.iter().min().unwrap();let high=*offsets.iter().max().unwrap();
    ensure!(i64::from(high)-i64::from(low)<=48,
        "Native kind-2 cell {cell:?} needs height span {} but four 2-bit steps can span at most 48 native units; independent sub-cell coarse heights require additional native floor representation or sampler support",i64::from(high)-i64::from(low));
    ensure!(offsets.iter().all(|h|h%16==0),"Kind-2 offsets must be multiples of 16 native units");
    for coarse in (-128i32..=127).rev() {
        let base=coarse*-32;let mut packed=0u8;let mut valid=true;
        for (i,height) in offsets.iter().enumerate() {
            let delta=i64::from(base)-i64::from(*height);
            if delta%16!=0 || !(0..=48).contains(&delta) {valid=false;break;}
            packed|=((delta/16) as u8)<<(i*2);
        }
        if valid {return Ok(ElevationOverride{tile_x:cell[0],tile_z:cell[1],coarse:coarse as i8,quads:packed});}
    }
    bail!("Cell {cell:?} heights {offsets:?} cannot use a signed coarse byte in 32-unit steps and four 2-bit steps while preserving its floor mean; floor-tier reassignment or native sampler work is required")
}
pub(super) fn serialize(record:&ElevationOverride)->Result<[u8;4]> {
    ensure!(record.tile_x<128 && record.tile_z<128,"Elevation row outside native grid");
    Ok([record.tile_x,record.tile_z,record.coarse as u8,record.quads])
}
/// A native bilinear sampler restricted to one integer X (or Z) is affine,
/// rounded toward zero at 14 fractional bits. If samples at 0 and d round to
/// the same height, this bound proves an observed height at d+1 impossible
/// for ANY corner tiers, not merely the current sixteen ladder entries.
pub(super) fn excludes_bilinear_step(heights:[i32;3],distance:u32)->bool {
    if heights[0]!=heights[1] || distance==0 {return false;}
    let difference=(i128::from(heights[2])-i128::from(heights[0])).abs();
    if difference<=1 {return false;}
    let d=i128::from(distance);let scale=16384i128;
    d*((difference-1)*scale+1)>(d+2)*(scale-1)
}
pub(super) fn repartition(samples:&[Sample],delta_subcells:[i32;2],background:&BTreeMap<[u8;2],Destination>)->Result<Vec<ElevationOverride>> {
    let mut tiles=BTreeMap::<[u8;2],[i32;4]>::new();let mut used=BTreeSet::new();
    for sample in samples {
        let x=sample.subcell[0] as i64+i64::from(delta_subcells[0]);let z=sample.subcell[1] as i64+i64::from(delta_subcells[1]);
        ensure!((0..256).contains(&x)&&(0..256).contains(&z),"Height relocation leaves native grid");
        ensure!(used.insert([x,z]),"Multiple contributions overwrite one destination sub-cell");
        let cell=[(x/2) as u8,(z/2) as u8];
        let frame=background.get(&cell).ok_or_else(||anyhow::anyhow!("Missing retained native floor context for {cell:?}"))?;
        ensure!(frame.piecewise_constant,"Destination {cell:?} has a retained bilinear surface; a kind-2 constant-step replacement would change unrelated floor samples");
        let values=tiles.entry(cell).or_insert(frame.retained);
        values[(z as usize%2)*2+x as usize%2]=sample.height;
    }
    tiles.into_iter().map(|(cell,heights)|{
        let mean=background[&cell].floor_mean;
        let mut offsets=[0;4];
        for i in 0..4 {offsets[i]=heights[i].checked_sub(mean).ok_or_else(||anyhow::anyhow!("Height offset overflow"))?;}
        encode(cell,offsets)
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_native_coarse_and_packed_step_pattern_roundtrips_exact_heights()->Result<()> {
        for coarse in -128i16..=127 {for quads in 0u16..=255 {
            let record=ElevationOverride{tile_x:5,tile_z:9,coarse:coarse as i8,quads:quads as u8};
            let values=decode(&record,0)?.map(|s|s.height);let encoded=encode([5,9],values)?;
            assert_eq!(decode(&encoded,0)?.map(|s|s.height),values);
            let mut block=[0u8;22];block[10..12].copy_from_slice(&18i16.to_le_bytes());block[12..14].copy_from_slice(&1i16.to_le_bytes());
            block[18..22].copy_from_slice(&serialize(&encoded)?);
            let parsed=legaia_engine_field::field_elevation::parse_elevation_overrides(&block);
            assert_eq!(parsed.len(),1);assert_eq!(decode(&parsed[0],0)?.map(|s|s.height),values);
        }}Ok(())
    }
    #[test]
    fn half_tile_translation_can_require_more_than_native_two_bit_steps()->Result<()> {
        let record=ElevationOverride{tile_x:37,tile_z:18,coarse:2,quads:0xcc};
        let samples=decode(&record,0)?;
        let background=[([34,18],Destination{floor_mean:0,retained:[0;4],piecewise_constant:true}),
            ([35,18],Destination{floor_mean:0,retained:[0;4],piecewise_constant:true}),
            ([34,19],Destination{floor_mean:0,retained:[0;4],piecewise_constant:true}),
            ([35,19],Destination{floor_mean:0,retained:[0;4],piecewise_constant:true})].into_iter().collect();
        let error=repartition(&samples,[-5,1],&background).unwrap_err();
        assert!(error.to_string().contains("span 64"));
        let full=repartition(&samples,[-4,0],&background)?;assert_eq!(full.len(),1);
        assert_eq!((full[0].tile_x,full[0].tile_z,full[0].coarse,full[0].quads),(35,18,2,0xcc));Ok(())
    }
    #[test]
    fn unknown_background_bilinear_samples_and_overflow_are_not_silently_replaced() {
        let sample=Sample{subcell:[0,0],height:0};
        assert!(repartition(&[sample],[0,0],&BTreeMap::new()).is_err());
        let background=BTreeMap::from([([0,0],Destination{floor_mean:0,retained:[0;4],piecewise_constant:false})]);
        assert!(repartition(&[sample],[0,0],&background).is_err());
        assert!(repartition(&[sample],[-1,0],&background).is_err());
        assert!(encode([0,0],[0,-16,-32,-64]).is_err());
        assert!(encode([0,0],[1;4]).is_err());
        assert!(encode([0,0],[i32::MIN;4]).is_err());
    }
    #[test]
    fn quadrant_discontinuity_cannot_be_hidden_by_different_native_floor_tiers() {
        assert!(excludes_bilinear_step([0,0,-64],63));
        assert!(excludes_bilinear_step([-192,-192,-304],63));
        // Integer rounding near zero must not be mistaken for a discontinuity.
        assert!(!excludes_bilinear_step([0,0,-1],63));
        for a in [-32768i64,-192,-1,0,1,192,32767] {for b in [-32768i64,-112,-1,0,1,112,32767] {
            for x in [0i64,32,64,96,127] {
                let left=b*x+a*(128-x);let right=a*x+b*(128-x);
                let values=[0i64,63,64].map(|z|((left*(128-z)+right*z)/16384) as i32);
                assert!(!excludes_bilinear_step(values,63));
            }
        }}
    }
}
