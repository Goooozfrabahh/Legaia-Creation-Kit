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