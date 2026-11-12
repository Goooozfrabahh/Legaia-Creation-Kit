// LCK_R50A_EXPORT_MANIFEST
//
// Export companion data for the extended authoring grid.
// Does NOT pretend to enlarge the retail PS1 MAP or patch the
// original engine's floor/visibility/collision routines.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path,PathBuf},
};

use serde::Serialize;

use super::{
    ground_edit_ops::{self,Transform},
    world_view::WorldModel,
};

const TILE:f32=128.0;
const RETAIL:i32=128;

#[derive(Clone,Serialize)]
struct FloorCell {
    tile_x:i32,
    tile_z:i32,
    source:[u8;2],
    editor_uid:Option<u64>,
    offset:[f32;3],
    scale:[f32;3],
    quarter_turns:i32,
    inside_retail:bool,
}

#[derive(Serialize)]
struct ObjectExtent {
    editor_uid:Option<u64>,
    terrain:bool,
    index:usize,
    center:[f32;3],
    min_x:f32,
    max_x:f32,
    min_z:f32,
    max_z:f32,
    outside_retail:bool,
    outside_extended:bool,
}

#[derive(Serialize)]
struct Manifest {
    format:&'static str,
    version:u32,
    scene:String,
    authoring_side:usize,
    origin_tile:i32,
    retail_side:usize,
    native_runtime_extended:bool,
    runtime_rendering_verified:bool,
    runtime_walkability_verified:bool,
    staged_floor_cells:usize,
    staged_beyond_retail:usize,
    clipped_floor_requests:usize,
    overlapping_floor_assignments:usize,
    edited_objects_beyond_retail:usize,
    floor_cells:Vec<FloorCell>,
    edited_objects:Vec<ObjectExtent>,
    limitations:Vec<&'static str>,
}

pub(super) struct Summary {
    pub path:PathBuf,
    pub extended_floors:usize,
    pub clipped:usize,
    pub extended_objects:usize,
}

fn grid_origin(side:usize)->i32 {
    (RETAIL-side as i32)/2
}

fn valid_side(side:usize)->bool {
    matches!(side,128|160|192|224|256)
}

fn footprint(
    source:[u8;2],
    t:Transform,
)->Option<[i32;4]> {
    let odd=t.turns.rem_euclid(2)!=0;

    let sx=if odd {
        t.scale[2].abs()
    } else {
        t.scale[0].abs()
    };

    let sz=if odd {
        t.scale[0].abs()
    } else {
        t.scale[2].abs()
    };

    if !sx.is_finite() || !sz.is_finite()
        || sx<=0.0 || sz<=0.0
        || !t.offset.iter().all(|v|v.is_finite())
    {
        return None;
    }

    let cx=source[0] as f32*TILE
        +TILE*0.5+t.offset[0];

    let cz=source[1] as f32*TILE
        +TILE*0.5+t.offset[2];

    let x0=((cx-64.0*sx)/TILE).floor();
    let x1=((cx+64.0*sx)/TILE).ceil();
    let z0=((cz-64.0*sz)/TILE).floor();
    let z1=((cz+64.0*sz)/TILE).ceil();

    let values=[x0,x1,z0,z1];

    if values.iter().any(|v|
        !v.is_finite() || v.abs()>1_000_000.0
    ) {
        return None;
    }

    Some([x0 as i32,x1 as i32,z0 as i32,z1 as i32])
}

fn stage_floors(
    floors:&ground_edit_ops::State,
    side:usize,
)->(Vec<FloorCell>,usize,usize) {
    let origin=grid_origin(side);
    let end=origin+side as i32;

    let mut cells=BTreeMap::<(i32,i32),FloorCell>::new();
    let mut clipped=0usize;
    let mut overlapping=0usize;

    let mut insert=|
        source:[u8;2],
        t:Transform,
        uid:Option<u64>
    |{
        let Some([x0,x1,z0,z1])=footprint(source,t)
        else {
            clipped+=1;
            return;
        };

        if x0<origin || z0<origin || x1>end || z1>end {
            clipped+=1;
        }

        for z in z0.max(origin)..z1.min(end) {
            for x in x0.max(origin)..x1.min(end) {
                let cell=FloorCell{
                    tile_x:x,
                    tile_z:z,
                    source,
                    editor_uid:uid,
                    offset:t.offset,
                    scale:t.scale,
                    quarter_turns:t.turns.rem_euclid(4),
                    inside_retail:
                        (0..RETAIL).contains(&x)
                        && (0..RETAIL).contains(&z),
                };

                if cells.insert((x,z),cell).is_some() {
                    overlapping+=1;
                }
            }
        }
    };

    for (&source,&t) in &floors.originals {
        if !floors.deleted.contains(&source) {
            insert(source,t,None);
        }
    }

    for copy in &floors.copies {
        insert(
            copy.source,
            copy.transform,
            Some(copy.uid)
        );
    }

    (cells.into_values().collect(),clipped,overlapping)
}

fn object_extents(
    world:&WorldModel,
    side:usize,
)->Vec<ObjectExtent> {
    let origin=grid_origin(side) as f32*TILE;
    let end=origin+side as f32*TILE;

    let mut result=Vec::new();

    for draw in &world.draws {
        if draw.hidden {continue;}

        let edited=draw.editor_uid.is_some()
            || draw.offset.iter().any(|v|v.abs()>0.01)
            || draw.rotation.iter().any(|v|v.abs()>0.0001)
            || draw.scale.iter().any(|v|(v-1.0).abs()>0.0001);

        if !edited {continue;}

        let [x,_,z]=draw.center;

        let mut min_x=x;
        let mut max_x=x;
        let mut min_z=z;
        let mut max_z=z;

        if let Some(model)=world.model.as_ref() {
            if let Some(vertices)=model.positions.get(
                draw.vertex_start..draw.vertex_end
            ) {
                for p in vertices {
                    min_x=min_x.min(p[0]);
                    max_x=max_x.max(p[0]);
                    min_z=min_z.min(p[2]);
                    max_z=max_z.max(p[2]);
                }
            }
        }

        let outside_retail=
            min_x<0.0 || min_z<0.0 ||
            max_x>=16384.0 || max_z>=16384.0;

        let outside_extended=
            min_x<origin || min_z<origin ||
            max_x>=end || max_z>=end;

        result.push(ObjectExtent{
            editor_uid:draw.editor_uid,
            terrain:draw.terrain,
            index:draw.index,
            center:draw.center,
            min_x,max_x,min_z,max_z,
            outside_retail,
            outside_extended,
        });
    }

    result
}

pub(super) fn write_manifest(
    output:&Path,
    scene:&str,
    world:&WorldModel,
    floors:&ground_edit_ops::State,
    side:usize,
)->Result<Summary,String>{
    if !valid_side(side) {
        return Err(format!(
            "Unsupported authoring grid {side}; expected 128,160,192,224 or 256"
        ));
    }

    if !output.exists() {
        return Err(
            "The corresponding IMG must be published first".into()
        );
    }

    let (floor_cells,clipped,overlapping)=
        stage_floors(floors,side);

    let outside=floor_cells.iter()
        .filter(|cell|!cell.inside_retail)
        .count();

    let edited_objects=object_extents(world,side);

    let objects_outside=edited_objects.iter()
        .filter(|obj|obj.outside_retail)
        .count();

    let manifest=Manifest{
        format:"LCK_EXTENDED_GRID_EXPORT_V1",
        version:1,
        scene:scene.to_owned(),
        authoring_side:side,
        origin_tile:grid_origin(side),
        retail_side:128,
        native_runtime_extended:false,
        runtime_rendering_verified:false,
        runtime_walkability_verified:false,
        staged_floor_cells:floor_cells.len(),
        staged_beyond_retail:outside,
        clipped_floor_requests:clipped,
        overlapping_floor_assignments:overlapping,
        edited_objects_beyond_retail:objects_outside,
        floor_cells,
        edited_objects,
        limitations:vec![
            "The accompanying IMG still uses the 128x128 retail MAP.",
            "Extended floor cells are staged data, not PS1 runtime tiles.",
            "Additional collision, rendering, and loader hooks are required.",
            "Object descriptor allocation and texture VRAM limits remain independent.",
        ],
    };

    let path=output.with_extension("grid_extension.json");

    if path.exists() {
        return Err(format!(
            "Refusing to overwrite existing extension manifest: {}",
            path.display()
        ));
    }

    let json=serde_json::to_vec_pretty(&manifest)
        .map_err(|error|error.to_string())?;

    fs::write(&path,&json)
        .map_err(|error|error.to_string())?;

    let readback=fs::read(&path)
        .map_err(|error|error.to_string())?;

    if json!=readback {
        return Err("Extended manifest readback mismatch".into());
    }

    Ok(Summary{
        path,
        extended_floors:outside,
        clipped,
        extended_objects:objects_outside,
    })
}

#[cfg(test)]
mod r50a_manifest_tests {
    use super::*;

    #[test]
    fn centered_extended_grid_preserves_retail_coordinates(){
        assert_eq!(grid_origin(128),0);
        assert_eq!(grid_origin(160),-16);
        assert_eq!(grid_origin(192),-32);
        assert_eq!(grid_origin(256),-64);
    }

    #[test]
    fn extended_copy_has_a_real_outside_retail_destination(){
        let mut ground=ground_edit_ops::State::default();

        ground.copies.push(
            ground_edit_ops::CopyPiece{
                uid:17,
                source:[127,10],
                transform:Transform{
                    offset:[256.0,0.0,0.0],
                    ..Transform::default()
                },
            }
        );

        let (cells,clipped,_)=stage_floors(&ground,160);

        assert_eq!(clipped,0);
        assert_eq!(cells.len(),1);
        assert_eq!(cells[0].tile_x,129);
        assert!(!cells[0].inside_retail);
    }

    #[test]
    fn smaller_grid_reports_clipping(){
        let mut ground=ground_edit_ops::State::default();

        ground.copies.push(
            ground_edit_ops::CopyPiece{
                uid:23,
                source:[127,10],
                transform:Transform{
                    offset:[2048.0,0.0,0.0],
                    ..Transform::default()
                },
            }
        );

        let (cells,clipped,_)=stage_floors(&ground,128);

        assert!(cells.is_empty());
        assert_eq!(clipped,1);
    }

    #[test]
    fn overlap_uses_the_final_authored_floor(){
        let mut ground=ground_edit_ops::State::default();

        for uid in [1,2] {
            ground.copies.push(
                ground_edit_ops::CopyPiece{
                    uid,
                    source:[18,19],
                    transform:Transform::default(),
                }
            );
        }

        let (cells,_,overlaps)=stage_floors(&ground,160);

        assert_eq!(cells.len(),1);
        assert_eq!(overlaps,1);
        assert_eq!(cells[0].editor_uid,Some(2));
    }
}

// LCK_R608_DONOR_COMPILER
// External authoring cells retain immutable native donor provenance.
// This is an export preparation layer, not runtime expansion.

#[derive(Serialize)]
struct R608Donor {
    source:[u8;2],
    grid_word:u16,
    collision_byte:u8,
    descriptor_id:u16,
    descriptor_flags:u16,
    descriptor_bytes:Vec<u8>,
}

#[derive(Serialize)]
struct R608Cell {
    tile:[i32;2],
    source:[u8;2],
    editor_uid:Option<u64>,
    offset:[f32;3],
    scale:[f32;3],
    quarter_turns:i32,
    descriptor_id:u16,
}

#[derive(Serialize)]
struct R608Bundle {
    format:&'static str,
    version:u32,
    scene:String,
    authoring_side:usize,
    origin_tile:i32,
    runtime_encoded:bool,
    runtime_rendering_verified:bool,
    runtime_walkability_verified:bool,
    external_cells:usize,
    unique_donor_sources:usize,
    unique_descriptors:usize,
    clipped_requests:usize,
    overlapping_assignments:usize,
    donors:Vec<R608Donor>,
    cells:Vec<R608Cell>,
    limitations:Vec<&'static str>,
}

pub(super) struct R608Summary {
    pub path:PathBuf,
    pub cells:usize,
    pub sources:usize,
}

fn r608_extract_donor(
    map:&[u8],
    source:[u8;2],
)->Result<R608Donor,String>{
    if map.len()<0x12000 {
        return Err("Immutable retail MAP is truncated".into());
    }

    let x=usize::from(source[0]);
    let z=usize::from(source[1]);

    if x>=128 || z>=128 {
        return Err(format!(
            "Source floor {:?} is outside the retail MAP",source
        ));
    }

    let index=z*128+x;
    let word_at=0x8000+index*2;
    let collision_at=0x4000+index;

    let word=u16::from_le_bytes([
        map[word_at],map[word_at+1]
    ]);

    let descriptor_id=word&0x01ff;
    let record_at=usize::from(descriptor_id)*32;

    if record_at+32>map.len() {
        return Err("Floor donor descriptor truncated".into());
    }

    let flags=u16::from_le_bytes([
        map[record_at+0x12],
        map[record_at+0x13],
    ]);

    Ok(R608Donor{
        source,
        grid_word:word,
        collision_byte:map[collision_at],
        descriptor_id,
        descriptor_flags:flags,
        descriptor_bytes:map[record_at..record_at+32].to_vec(),
    })
}

fn r608_build_bundle(
    scene:&str,
    map:&[u8],
    floors:&ground_edit_ops::State,
    side:usize,
)->Result<Option<R608Bundle>,String>{
    if !valid_side(side) {
        return Err(format!("Unsupported grid side {side}"));
    }

    if map.len()<0x12000 {
        return Err("Immutable retail MAP missing".into());
    }

    if !floors.scene.is_empty() && floors.scene!=scene {
        return Err("Floor edits belong to another scene".into());
    }

    let (staged,clipped,overlaps)=stage_floors(floors,side);

    if clipped!=0 {
        return Err(format!(
            "{clipped} floor requests exceed the selected grid"
        ));
    }

    let mut donors=BTreeMap::<[u8;2],R608Donor>::new();
    let mut cells=Vec::<R608Cell>::new();

    for cell in staged.into_iter().filter(|c|!c.inside_retail) {
        if !donors.contains_key(&cell.source) {
            donors.insert(
                cell.source,
                r608_extract_donor(map,cell.source)?
            );
        }

        let descriptor_id=donors
            .get(&cell.source)
            .ok_or("Missing donor after insertion")?
            .descriptor_id;

        cells.push(R608Cell{
            tile:[cell.tile_x,cell.tile_z],
            source:cell.source,
            editor_uid:cell.editor_uid,
            offset:cell.offset,
            scale:cell.scale,
            quarter_turns:cell.quarter_turns,
            descriptor_id,
        });
    }

    if cells.is_empty() {
        return Ok(None);
    }

    let unique_descriptors=donors.values()
        .map(|d|d.descriptor_id)
        .collect::<std::collections::BTreeSet<_>>()
        .len();

    let unique_donor_sources=donors.len();
    let external_cells=cells.len();

    Ok(Some(R608Bundle{
        format:"LCK_R608_IMMUTABLE_NATIVE_DONORS",
        version:1,
        scene:scene.to_owned(),
        authoring_side:side,
        origin_tile:grid_origin(side),
        runtime_encoded:false,
        runtime_rendering_verified:false,
        runtime_walkability_verified:false,
        external_cells,
        unique_donor_sources,
        unique_descriptors,
        clipped_requests:clipped,
        overlapping_assignments:overlaps,
        donors:donors.into_values().collect(),
        cells,
        limitations:vec![
            "Donor data is preserved but not installed as PS1 runtime pages.",
            "LKG1 runtime loader and allocation remain unimplemented.",
            "Native external visibility and height sampling remain unimplemented.",
            "Wall, collision and MAN write hooks remain unimplemented.",
            "Extended scene-region and camera handling remain unimplemented.",
        ],
    }))
}

pub(super) fn write_donor_bundle(
    output:&Path,
    scene:&str,
    immutable_map:&[u8],
    floors:&ground_edit_ops::State,
    side:usize,
)->Result<Option<R608Summary>,String>{
    if !output.is_file() {
        return Err("IMG must be published before donor compilation".into());
    }

    let Some(bundle)=r608_build_bundle(
        scene,immutable_map,floors,side
    )? else {
        return Ok(None);
    };

    let path=output.with_extension("r608_native_donors.json");

    if path.exists() {
        return Err(format!(
            "Refusing to overwrite existing donor bundle: {}",
            path.display()
        ));
    }

    let sparse_payload=r609_compile(&bundle)?;

    let sparse_path=output.with_extension("r610_lkg1_candidate.bin");

    if sparse_path.exists() {
        return Err(format!(
            "Refusing to overwrite existing sparse payload: {}",
            sparse_path.display()
        ));
    }

    let cells=bundle.external_cells;
    let sources=bundle.unique_donor_sources;

    let bytes=serde_json::to_vec_pretty(&bundle)
        .map_err(|e|e.to_string())?;

    fs::write(&path,&bytes).map_err(|e|e.to_string())?;

    let verified=fs::read(&path).map_err(|e|e.to_string())?;

    if verified!=bytes {
        let _=fs::remove_file(&path);
        return Err("Donor bundle readback failed".into());
    }

    // Publish the sparse candidate separately from the retail IMG.
    // Do not set native_runtime_extended or gameplay verification flags.
    if let Err(error)=fs::write(&sparse_path,&sparse_payload) {
        let _=fs::remove_file(&path);
        return Err(format!("Sparse candidate write failed: {error}"));
    }

    match fs::read(&sparse_path) {
        Ok(readback) if readback==sparse_payload => {},
        _ => {
            let _=fs::remove_file(&sparse_path);
            let _=fs::remove_file(&path);
            return Err("Sparse candidate readback mismatch".into());
        }
    }

    Ok(Some(R608Summary{path,cells,sources}))
}

#[cfg(test)]
mod r608_donor_plan_tests {
    use super::*;

    fn test_map()->Vec<u8>{
        let mut map=vec![0u8;0x12000];

        let at=0x8000+(10*128+127)*2;
        map[at..at+2].copy_from_slice(&0x1122u16.to_le_bytes());

        let desc=0x122usize*32;
        map[desc+0x12..desc+0x14]
            .copy_from_slice(&0x0017u16.to_le_bytes());

        map[0x4000+10*128+127]=0x37;
        map
    }

    #[test]
    fn immutable_donor_retains_descriptor_and_collision(){
        let map=test_map();
        let donor=r608_extract_donor(&map,[127,10]).unwrap();

        assert_eq!(donor.grid_word,0x1122);
        assert_eq!(donor.descriptor_id,0x122);
        assert_eq!(donor.descriptor_flags,0x17);
        assert_eq!(donor.collision_byte,0x37);
        assert_eq!(donor.descriptor_bytes.len(),32);
    }

    #[test]
    fn external_tile_keeps_original_world_coordinates(){
        let map=test_map();
        let mut floors=ground_edit_ops::State::default();

        floors.scene="town01".into();

        floors.copies.push(ground_edit_ops::CopyPiece{
            uid:105,
            source:[127,10],
            transform:Transform{
                offset:[256.0,0.0,0.0],
                ..Transform::default()
            }
        });

        let bundle=r608_build_bundle(
            "town01",&map,&floors,256
        ).unwrap().unwrap();

        assert_eq!(bundle.cells.len(),1);
        assert_eq!(bundle.cells[0].tile,[129,10]);
        assert_eq!(bundle.cells[0].descriptor_id,0x122);
        assert_eq!(bundle.unique_donor_sources,1);
        assert!(!bundle.runtime_encoded);
    }

    #[test]
    fn invalid_source_and_truncated_map_are_rejected(){
        let map=test_map();

        assert!(r608_extract_donor(&map,[255,10]).is_err());
        assert!(r608_extract_donor(&[],[127,10]).is_err());
    }
}

// LCK_R609_SPARSE_COMPILER
//
// Binary sparse-grid candidate for native external cells.
// The 128x128 retail MAP remains authoritative for original tiles.
//
// 16-byte header
// 2-byte page directory entries
// 16x16 cells per allocated page
// 256 collision bytes + 512 object-word bytes per page
//
// The executable runtime loader has not yet been installed.
// This file is an implementation candidate, not gameplay proof.

// LCK_R610_CANONICAL_CODEC
// R6.0.9's independent header wrote 16 at 0x0a where the native
// runtime codec requires reserved zero. Never maintain two LKG1 codecs.
use legaia_code_hooks::field_grid::{
    Cell as R610Cell, Layout as R610Layout, SparseGrid as R610SparseGrid
};

fn r609_validate(bytes:&[u8])->Result<(usize,usize,i32),String>{
    let grid=R610SparseGrid::decode(bytes).map_err(|e|e.to_string())?;
    if grid.encode()!=bytes {
        return Err("Noncanonical LKG1 encoding".into());
    }
    let side=usize::from(u16::from_le_bytes([bytes[6],bytes[7]]));
    let layout=R610Layout::new(side as u16).map_err(|e|e.to_string())?;
    Ok((side,(side/16)*(side/16),layout.origin_tile()))
}

fn r609_compile(bundle:&R608Bundle)->Result<Vec<u8>,String>{
    let side=u16::try_from(bundle.authoring_side)
        .map_err(|_| "Grid side overflows native field".to_string())?;
    let layout=R610Layout::new(side).map_err(|e|e.to_string())?;
    if bundle.origin_tile!=layout.origin_tile() {
        return Err("Authoring origin disagrees with native runtime layout".into());
    }
    let mut sparse=R610SparseGrid::new(layout);
    let mut donors=BTreeMap::<[u8;2],&R608Donor>::new();
    let mut occupied=std::collections::BTreeSet::new();
    for donor in &bundle.donors {
        if donors.insert(donor.source,donor).is_some() {
            return Err("Duplicated immutable donor source".into());
        }
    }
    for cell in &bundle.cells {
        if !occupied.insert(cell.tile) {
            return Err(format!("Duplicate external tile {:?}",cell.tile));
        }
        let donor=donors.get(&cell.source)
            .ok_or("External tile references missing donor")?;
        if cell.descriptor_id!=donor.descriptor_id
            || (donor.grid_word&0x01ff)!=cell.descriptor_id {
            return Err("Native donor descriptor mismatch".into());
        }
        sparse.set(cell.tile,R610Cell{
            collision:donor.collision_byte,
            object:donor.grid_word
        }).map_err(|e|e.to_string())?;
    }
    let bytes=sparse.encode();
    let decoded=R610SparseGrid::decode(&bytes).map_err(|e|e.to_string())?;
    if decoded.encode()!=bytes {
        return Err("Native sparse codec readback mismatch".into());
    }
    for cell in &bundle.cells {
        let donor=donors.get(&cell.source)
            .ok_or("Lost donor during codec verification")?;
        if decoded.get(cell.tile)!=Some(R610Cell{
            collision:donor.collision_byte,
            object:donor.grid_word
        }) {
            return Err(format!("Sparse cell readback mismatch at {:?}",cell.tile));
        }
    }
    Ok(bytes)
}

fn r609_lookup(bytes:&[u8],cell:[i32;2])
    ->Result<Option<(u8,u16)>,String>{
    let (side,_,origin)=r609_validate(bytes)?;
    if (0..128).contains(&cell[0]) && (0..128).contains(&cell[1]) {
        return Err("Retail cells resolve from original MAP".into());
    }
    if cell.iter().any(|&v|v<origin || v>=origin+side as i32) {
        return Ok(None);
    }
    let decoded=R610SparseGrid::decode(bytes).map_err(|e|e.to_string())?;
    Ok(decoded.get(cell).map(|c|(c.collision,c.object)))
}

#[cfg(test)]
mod r610_cross_codec_tests {
    use super::*;

    fn fixture()->(Vec<u8>,ground_edit_ops::State){
        let mut map=vec![0u8;0x12000];
        let at=0x8000+(10*128+127)*2;
        map[at..at+2].copy_from_slice(&0x1122u16.to_le_bytes());
        let desc=0x122usize*32;
        map[desc+0x12..desc+0x14].copy_from_slice(&0x17u16.to_le_bytes());
        map[0x4000+10*128+127]=0x37;
        let mut ground=ground_edit_ops::State::default();
        ground.scene="town01".into();
        for (uid,dest) in [(1u64,-1i32),(2,191)] {
            ground.copies.push(ground_edit_ops::CopyPiece{
                uid,source:[127,10],
                transform:Transform{
                    offset:[((dest-127)*128) as f32,0.0,0.0],
                    ..Transform::default()
                },
            });
        }
        (map,ground)
    }

    #[test]
    fn editor_payload_is_byte_identical_to_native_codec(){
        let (map,ground)=fixture();
        let bundle=r608_build_bundle("town01",&map,&ground,256)
            .unwrap().unwrap();
        let bytes=r609_compile(&bundle).unwrap();
        assert_eq!(&bytes[..4],b"LKG1");
        assert_eq!(&bytes[10..12],&[0,0]);
        let native=R610SparseGrid::decode(&bytes).unwrap();
        assert_eq!(native.encode(),bytes);
        assert_eq!(native.get([-1,10]),Some(R610Cell{
            collision:0x37,object:0x1122
        }));
        assert_eq!(native.get([191,10]),Some(R610Cell{
            collision:0x37,object:0x1122
        }));
        assert_eq!(native.get([0,10]),None);
        let mut independently=R610SparseGrid::new(R610Layout::new(256).unwrap());
        independently.set([-1,10],R610Cell{collision:0x37,object:0x1122}).unwrap();
        independently.set([191,10],R610Cell{collision:0x37,object:0x1122}).unwrap();
        assert_eq!(independently.encode(),bytes);
    }

    #[test]
    fn previous_r609_reserved_header_is_rejected(){
        let (map,ground)=fixture();
        let bundle=r608_build_bundle("town01",&map,&ground,256)
            .unwrap().unwrap();
        let mut bytes=r609_compile(&bundle).unwrap();
        bytes[10]=16;
        assert!(R610SparseGrid::decode(&bytes).is_err());
        assert!(r609_validate(&bytes).is_err());
    }
}
#[cfg(test)]
mod r609_sparse_tests {
    use super::*;

    fn sample()->(Vec<u8>,ground_edit_ops::State){
        let mut map=vec![0u8;0x12000];

        let source=0x8000+(10*128+127)*2;
        map[source..source+2]
            .copy_from_slice(&0x1122u16.to_le_bytes());

        let descriptor=0x122usize*32;
        map[descriptor+0x12..descriptor+0x14]
            .copy_from_slice(&0x17u16.to_le_bytes());

        map[0x4000+10*128+127]=0x37;

        let mut ground=ground_edit_ops::State::default();
        ground.scene="town01".into();

        ground.copies.push(ground_edit_ops::CopyPiece{
            uid:109,
            source:[127,10],
            transform:Transform{
                offset:[256.0,0.0,0.0],
                ..Transform::default()
            }
        });

        (map,ground)
    }

    #[test]
    fn binary_has_correct_header_directory_and_page(){
        let (map,ground)=sample();

        let bundle=r608_build_bundle(
            "town01",&map,&ground,256
        ).unwrap().unwrap();

        let bytes=r609_compile(&bundle).unwrap();

        assert_eq!(&bytes[0..4],b"LKG1");
        assert_eq!(bytes.len(),16+512+768);

        let (side,count,origin)=r609_validate(&bytes).unwrap();

        assert_eq!(side,256);
        assert_eq!(count,256);
        assert_eq!(origin,-64);
    }

    #[test]
    fn runtime_candidate_resolves_external_donor(){
        let (map,ground)=sample();

        let bundle=r608_build_bundle(
            "town01",&map,&ground,256
        ).unwrap().unwrap();

        let bytes=r609_compile(&bundle).unwrap();

        assert_eq!(
            r609_lookup(&bytes,[129,10]).unwrap(),
            Some((0x37,0x1122))
        );

        assert_eq!(
            r609_lookup(&bytes,[128,10]).unwrap(),
            Some((0xF7,0))
        );
    }

    #[test]
    fn negative_coordinates_are_not_wrapped(){
        let (map,mut ground)=sample();

        ground.copies[0].transform.offset[0]=-16384.0;

        let bundle=r608_build_bundle(
            "town01",&map,&ground,256
        ).unwrap().unwrap();

        let bytes=r609_compile(&bundle).unwrap();

        assert_eq!(
            r609_lookup(&bytes,[-1,10]).unwrap(),
            Some((0x37,0x1122))
        );

        assert_eq!(
            r609_lookup(&bytes,[-65,10]).unwrap(),
            None
        );

        assert!(r609_lookup(&bytes,[10,10]).is_err());
    }

    #[test]
    fn malformed_payload_is_rejected(){
        let (map,ground)=sample();

        let bundle=r608_build_bundle(
            "town01",&map,&ground,256
        ).unwrap().unwrap();

        let bytes=r609_compile(&bundle).unwrap();

        let mut truncated=bytes.clone();
        truncated.pop();

        assert!(r609_validate(&truncated).is_err());

        let mut bad=bytes.clone();
        bad[0]=0;

        assert!(r609_validate(&bad).is_err());

        let mut bad=bytes.clone();
        bad[6]=127;

        assert!(r609_validate(&bad).is_err());
    }
}
