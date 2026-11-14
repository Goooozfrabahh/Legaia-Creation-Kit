//! LCK R6.0 - unified, staged native IMG export.
//! The source retail IMG is read-only.
//! Native serializers retain responsibility for their formats.

use anyhow::{Context,Result,ensure,bail};

use std::{
    fs,
    path::{Path,PathBuf},
    collections::BTreeSet,
    time::{SystemTime,UNIX_EPOCH}
};

use legaia_disc_patch::disc::DiscPatcher;

use legaia_engine_scene::{
    scene::{ProtIndex,Scene},
    scene_assembly::AssembledScene
};

use super::{
    collision_edit,
    door_teleport,
    export_manifest,
    form_bridge,
    ground_edit_ops,
    native_floor_mirror,
    native_ground_delete,
    native_warps,
    native_gameplay_camera,
    ps1_export,
    ps1_original_guard,
    world_view,
    native_extended_manifest
};

#[path = "native128_region_unlock.rs"]
mod native128_region_unlock;
#[path = "native128_r615.rs"]
mod native128_r615;
#[path = "r636_import_relocation.rs"]
mod r636_import_relocation;

pub(super) struct Request<'a> {
    pub source:&'a Path,
    pub output:&'a Path,
    pub scene:&'a AssembledScene,
    pub world:&'a world_view::WorldModel,
    pub index:&'a ProtIndex,
    pub floors:&'a ground_edit_ops::State,
    pub collision:&'a collision_edit::State,
    pub doors:&'a door_teleport::State,
    pub warps:Option<&'a native_warps::SceneWarps>,
    pub project:Option<form_bridge::Session>,
    pub unrestricted:bool,
    pub extension_side:usize,
}

// Staged intermediate images must not remain after an export.
struct Stages(Vec<PathBuf>);

impl Stages {
    fn new()->Self {
        Self(Vec::new())
    }

    fn add(&mut self,p:PathBuf)->Result<PathBuf> {
        ensure!(
            !p.exists(),
            "Staging file already exists: {}",
            p.display()
        );

        self.0.push(p.clone());

        Ok(p)
    }
}

impl Drop for Stages {
    fn drop(&mut self) {
        for p in &self.0 {
            let _=fs::remove_file(p);

            for ext in [
                "json",
                "lckbuilding",
                "lck_w1_building",
                "lckdoorbackup",
                "r57_warp_staging",
                "r57_before_warps",
                "w2_unrestricted_debug.txt"
            ] {
                let _=fs::remove_file(p.with_extension(ext));
            }
        }
    }
}

fn read_map(path:&Path,id:usize)->Result<Vec<u8>> {
    Ok(
        DiscPatcher::open(fs::read(path)?)?
            .read_entry_footprint(id)?
    )
}

// LCK_R624_DISC_GROWTH
// A native writer may expand PROT.DAT and relocate following
// disc sectors. An expanding IMG is valid only after checking
// raw-sector alignment, monotonic stage length, the rebuilt
// ISO/PROT index, and native MAP readability.
//
// Never truncate or pad a stage to the original retail length.
fn r624_validate_length(previous:u64,actual:u64)->Result<()> {
    ensure!(
        actual>=previous,
        "Native IMG stage unexpectedly shrank: previous {previous} bytes, current {actual} bytes"
    );

    ensure!(
        actual>0 && actual%2352==0,
        "Native IMG is not aligned to whole 2352-byte raw sectors: {actual} bytes"
    );

    Ok(())
}

// R626: final-stage native deletion integrity checks.
// The last staged IMG is checked before it is published.

fn r626_grid_word(map:&[u8],cell:(u8,u8))->Result<u16>{
    let at=0x8000usize+(cell.1 as usize*128+cell.0 as usize)*2;
    ensure!(map.len()>=0x12000,"R626: native MAP is truncated");
    Ok(u16::from_le_bytes([map[at],map[at+1]]))
}

fn r626_verify_grid_delete(
    retail:&[u8],
    final_map:&[u8],
    cell:(u8,u8)
)->Result<()>{
    use legaia_scene_formats::field_objects::{
        ObjectRecord,OBJECT_INDEX_MASK,CELL_VISIBLE,CELL_WALK_VISIBLE
    };

    let old=r626_grid_word(retail,cell)?;
    let new=r626_grid_word(final_map,cell)?;
    let old_id=(old&OBJECT_INDEX_MASK) as usize;
    let new_id=(new&OBJECT_INDEX_MASK) as usize;

    let placed=ObjectRecord::parse(retail,old_id)
        .is_some_and(|record|record.is_placed());

    if placed {
        ensure!(
            new_id!=old_id,
            "R626: deleted placed descriptor {old_id} is still linked at {cell:?}"
        );
    }else{
        ensure!(
            new_id!=old_id ||
                new&(CELL_VISIBLE|CELL_WALK_VISIBLE)==0,
            "R626: deleted terrain/draw flags survived at {cell:?}"
        );
    }

    Ok(())
}

fn r626_verify_final_deletions(
    request:&Request<'_>,
    retail:&[u8],
    final_map:&[u8]
)->Result<String>{
    use legaia_scene_formats::field_objects::CELL_WALK_VISIBLE;

    ensure!(
        retail.len()>=0x12000 && final_map.len()>=0x12000,
        "R626: incomplete MAP for deletion verification"
    );

    let mut initialized=final_map.to_vec();

    legaia_engine_field::field_regions::refresh_object_grid_marks(
        &mut initialized
    );

    let mut objects=0usize;

    for draw in request.world.draws.iter()
        .filter(|draw|draw.hidden && draw.editor_uid.is_none())
    {
        if request.world.npc((draw.terrain,draw.index)).is_some(){
            continue;
        }

        let cell=if draw.terrain {
            request.scene.terrain.get(draw.index)
                .map(|native|native.cell)
        }else{
            request.scene.placements.get(draw.index)
                .map(|native|native.cell)
        }.with_context(||format!(
            "R626: hidden native draw has no original MAP identity: terrain={} index={}",
            draw.terrain,draw.index
        ))?;

        r626_verify_grid_delete(retail,&initialized,cell)?;
        objects+=1;
    }

    let mut floors=0usize;

    if !request.floors.deleted.is_empty(){
        let (_,stamps,_)=ground_edit_ops::w46_plan(request.floors)
            .map_err(anyhow::Error::msg)?;

        let replacements=stamps.into_iter()
            .map(|(_,destination)|destination)
            .collect::<BTreeSet<_>>();

        for &cell in &request.floors.deleted {
            if replacements.contains(&cell){
                continue;
            }

            ensure!(
                r626_grid_word(&initialized,(cell[0],cell[1]))?
                    &CELL_WALK_VISIBLE==0,
                "R626: deleted native floor {cell:?} returned after MAP initialization"
            );

            floors+=1;
        }
    }

    let (walls_authored,_)=request.collision.native_edit_snapshot();
    let mut walls=0usize;

    for (x,z,occupied) in walls_authored {
        let (at,bit)=super::collision_wire::wall_offset(x,z)
            .with_context(||format!(
                "R626: invalid collision cell ({x},{z})"
            ))?;

        ensure!(
            (initialized[0x4000+at]&bit!=0)==occupied,
            "R626: authored collision wall ({x},{z}) was overwritten"
        );

        walls+=1;
    }

    Ok(format!(
        "R626 DELETION INTEGRITY PASSED: world objects={objects}; deleted native floors={floors}; collision overrides={walls}; final native MAP verified"
    ))
}

fn verify_stage(
    path:&Path,
    id:usize,
    expected_len:&mut u64,
    expected_entries:usize
)->Result<()> {
    let actual=ps1_export::verify_output_file(
        path,None
    )?;

    r624_validate_length(*expected_len,actual)?;

    let rebuilt=DiscPatcher::open(
        fs::read(path)?
    ).with_context(||format!(
        "Reopen relocated ISO/PROT image: {}",
        path.display()
    ))?;

    ensure!(
        rebuilt.entry_count()==expected_entries,
        "PROT entry count changed after IMG relayout: expected {expected_entries}, found {}",
        rebuilt.entry_count()
    );

    ensure!(
        rebuilt.read_entry_footprint(id)?.len()>=0x12000,
        "Native MAP is truncated after ISO/PROT relayout"
    );

    // Update the expected full-disc size only after
    // successful structural verification.
    *expected_len=actual;

    Ok(())
}

// LCK_R618B_WRITABLE_STAGE
// Source IMG stays immutable. Independent temporary copies
// may need their Windows read-only attribute cleared.
fn copy_stage(src:&Path,dst:&Path)->Result<()> {
    ensure!(
        !dst.exists(),
        "IMG stage already exists: {}",
        dst.display()
    );

    fs::copy(src,dst).with_context(||format!(
        "STAGE COPY FAILED: source={} destination={}",
        src.display(),
        dst.display()
    ))?;

    let metadata=fs::metadata(dst).with_context(||format!(
        "STAGE METADATA FAILED: {}",
        dst.display()
    ))?;

    if metadata.permissions().readonly() {
        let mut permissions=metadata.permissions();
        permissions.set_readonly(false);

        fs::set_permissions(dst,permissions).with_context(||format!(
            "STAGE WRITABLE PERMISSION FAILED: stage={} original={}",
            dst.display(),
            src.display()
        ))?;
    }

    ensure!(
        !fs::metadata(dst)?.permissions().readonly(),
        "STAGE STILL READ-ONLY: {}",
        dst.display()
    );

    Ok(())
}

fn failure(
    skipped:&mut Vec<String>,
    allow:bool,
    which:&str,
    error:anyhow::Error
)->Result<()> {
    if allow {
        skipped.push(format!(
            "FAILED NATIVE {which}: {error:#}; operation was NOT encoded"
        ));

        Ok(())
    } else {
        Err(error.context(format!("Native {which} operation")))
    }
}

// LCK_R626_REGION_ISOLATION
// Changing a MAN destination, MAP warp trigger, or camera
// script is not a request to expand the authored world.
//
// Region expansion is retained for operations that actually
// author placements, floors, or native collision.
fn r626_region_work_requested(
    objects:bool,
    floors:bool,
    collision:bool,
)->bool {
    objects || floors || collision
}
pub(super) fn execute(
    mut request:Request<'_>
)->Result<ps1_export::ExportResult> {

    ps1_original_guard::ensure_new_output(
        request.source,request.output
    )?;

    ensure!(
        !request.output.exists(),
        "Choose a NEW IMG output filename"
    );

    ensure!(
        request.floors.scene.is_empty()
            || request.floors.scene==request.scene.name,
        "Floor changes belong to another cell"
    );

    let source_len=fs::metadata(request.source)?.len();

    ensure!(
        source_len>0 && source_len%2352==0,
        "Source must be a raw 2352-byte PS1 IMG"
    );

    let scene=Scene::load(
        request.index,&request.scene.name
    )?;

    let entry=scene.field_map_index(request.index)
        .context("Selected cell has no native MAP")?
        as usize;

    let retail=request.index.entry_bytes(entry as u32)?;

    ensure!(
        retail.len()>=0x12000,
        "Immutable donor MAP is unavailable"
    );

    // Freeze the editor's pending changes for this export.
    let manifest=export_manifest::capture(
        &request.scene.name,
        request.world,
        request.floors,
        request.collision
    );

    // Floor duplicates must have independent editor identities.
    let mut unique_copies=BTreeSet::new();

    for copy in &request.floors.copies {
        ensure!(
            copy.uid>0 && unique_copies.insert(copy.uid),
            "Duplicate floor UID {}; unique identity required",
            copy.uid
        );
    }

    // LCK_R616D_CAMERA_EXPORT_SELECTION
    let camera_changes=request.project.as_ref()
        .and_then(|s|
            s.project.native_cameras.iter()
                .find(|c|c.scene==request.scene.name)
        ).cloned();

    let has_cameras=camera_changes.as_ref()
        .is_some_and(|c|c.changed());
    let has_objects=
        collision_edit::scene_has_instance_edits(
            &request.scene.name,request.world
        );

    let has_floors=request.floors.has_edits();
    let has_collision=request.collision.has_edits();
    let has_doors=request.doors.has_native_changes();

    let has_warps=match request.warps {
        Some(w)=>{
            ensure!(
                w.scene==request.scene.name,
                "Warp snapshot belongs to another cell"
            );

            native_warps::has_effective_changes(
                request.index,w
            )?
        },
        None=>false,
    };

    ensure!(
        has_objects || has_floors || has_collision || has_cameras
            || has_doors || has_warps,
        "Only editor metadata changed; save project instead"
    );

    // Both door and warp authors may own MAP kind-0.
    // Never silently let the second writer erase the first.
    let do_warps=if has_doors && has_warps {
        if request.unrestricted {
            false
        } else {
            bail!(
                "Doors and standalone warps both modify MAP kind-0"
            )
        }
    } else {
        has_warps
    };

    let dir=request.output.parent()
        .context("Output directory missing")?;

    let stamp=SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_nanos();

    let prefix=format!(
        "LCK_R60_{}_{}",
        std::process::id(),stamp
    );

    let mut temps=Stages::new();

    let mut current=request.source.to_path_buf();

    // LCK_R624_DISC_GROWTH
    // Each stage must preserve or legitimately grow this image.
    let mut staged_len=source_len;

    let mut wrote=false;
    let mut patched=Vec::<String>::new();
    let mut skipped=Vec::<String>::new();
    let mut imports=None;
    let mut native128_summary=None;
    // LCK_R636_IMPORT_INTEGRITY
    let mut r636_object_stage_map:Option<Vec<u8>>=None;

    let r613_full_requested=std::env::current_exe()
        .ok()
        .and_then(|p|p.parent().map(|d|
            d.join("LCK_NATIVE128_FULL.enabled").exists()))
        .unwrap_or(false);

    patched.push(format!(
        "R6.0 manifest: {}",manifest.summary()
    ));

    if has_doors && has_warps {
        skipped.push(
            "FAILED NATIVE WARPS: MAP kind-0 has duplicate ownership; standalone warp edit not encoded".into()
        );
    }

    // ========================================================
    // Stage 1 - Object/native placement changes
    // ========================================================

    if has_objects {
        let target=temps.add(
            dir.join(format!("{prefix}_objects.img"))
        )?;

        let attempt=(||->Result<ps1_export::ExportResult>{
            let plan=if let Some(project)=request.project.as_mut(){
                ps1_export::prepare_project(
                    request.source,
                    request.scene,
                    project,
                    request.index,
                    request.unrestricted
                )?
            }else if request.unrestricted {
                ps1_export::unrestricted::prepare(
                    request.source,
                    request.scene,
                    request.world,
                    request.index
                )?
            }else{
                ps1_export::plan_export(
                    request.source,
                    request.scene,
                    request.world,
                    request.index
                )?
            };

            ps1_export::export(plan,&target)
        })();

        match attempt {
            Ok(result)=>{
                verify_stage(&target,entry,&mut staged_len,request.index.entry_count())?;

                current=target;
                wrote=true;

                patched.extend(result.patched);
                skipped.extend(result.skipped);
                imports=result.imports;
                // R636: preserve all newly registered native grid identities.
                r636_object_stage_map=Some(read_map(&current,entry)?);
            },
            Err(e)=>{
                failure(
                    &mut skipped,
                    request.unrestricted,
                    "OBJECTS",
                    e
                )?;
            }
        }
    }

    // ========================================================
    // Stage 2 - Ground and native texture changes
    // ========================================================

    if has_floors {
        let target=temps.add(
            dir.join(format!("{prefix}_floors.img"))
        )?;

        match native_ground_delete::export_ground(
            &current,
            &target,
            &request.scene.name,
            request.index,
            request.floors,
            request.unrestricted
        ) {
            Ok(message)=>{
                patched.push(message);
                wrote=true;
            },
            Err(e)=>{
                failure(
                    &mut skipped,
                    request.unrestricted,
                    "GROUND",
                    e
                )?;

                let _=fs::remove_file(&target);
                copy_stage(&current,&target)?;
            }
        }

        verify_stage(&target,entry,&mut staged_len,request.index.entry_count())?;

        // An unrestricted failed texture pass cannot corrupt the
        // successful ground changes already in this stage.
        let backup=if request.unrestricted {
            Some(fs::read(&target)?)
        }else{
            None
        };

        match native_floor_mirror::overlay_existing(
            &target,
            &request.scene.name,
            request.index,
            request.floors,
            request.unrestricted
        ) {
            Ok(Some(message))=>{
                patched.push(message);
                wrote=true;
            },
            Ok(None)=>{},
            Err(e)=>{
                if let Some(bytes)=backup {
                    fs::write(&target,bytes)?;
                }

                failure(
                    &mut skipped,
                    request.unrestricted,
                    "FLOOR TEXTURES",
                    e
                )?;
            }
        }

        verify_stage(&target,entry,&mut staged_len,request.index.entry_count())?;

        current=target;
    }

    // ========================================================
    // Stage 3 - Collision
    // ========================================================

    if has_collision {
        let target=temps.add(
            dir.join(format!("{prefix}_collision.img"))
        )?;

        copy_stage(&current,&target)?;

        let backup=if request.unrestricted {
            Some(fs::read(&target)?)
        }else{
            None
        };

        match request.collision.overlay_existing(
            &target,
            &request.scene.name,
            request.index
        ) {
            Ok(message)=>{
                patched.push(message);
                wrote=true;
            },
            Err(e)=>{
                if let Some(bytes)=backup {
                    fs::write(&target,bytes)?;
                }

                failure(
                    &mut skipped,
                    request.unrestricted,
                    "COLLISION",
                    e
                )?;
            }
        }

        verify_stage(&target,entry,&mut staged_len,request.index.entry_count())?;

        current=target;
    }

    // ========================================================
    // Stage 4 - Native doors
    // ========================================================

    if has_doors {
        let target=temps.add(
            dir.join(format!("{prefix}_doors.img"))
        )?;

        copy_stage(&current,&target)?;

        let backup=if request.unrestricted {
            Some(fs::read(&target)?)
        }else{
            None
        };

        match request.doors.overlay_existing(
            &target,request.index
        ) {
            Ok(message)=>{
                patched.push(message);
                wrote=true;
            },
            Err(e)=>{
                if let Some(bytes)=backup {
                    fs::write(&target,bytes)?;
                }

                failure(
                    &mut skipped,
                    request.unrestricted,
                    "DOORS",
                    e
                )?;
            }
        }

        verify_stage(&target,entry,&mut staged_len,request.index.entry_count())?;

        current=target;
    }

    // ========================================================
    // Stage 5 - Standalone MAP warps
    // ========================================================

    if do_warps {
        let target=temps.add(
            dir.join(format!("{prefix}_warps.img"))
        )?;

        copy_stage(&current,&target)?;

        let backup=if request.unrestricted {
            Some(fs::read(&target)?)
        }else{
            None
        };

        match native_warps::overlay_existing(
            &target,
            request.index,
            request.warps.context("Warp edit missing")?
        ) {
            Ok(message)=>{
                patched.push(message);
                wrote=true;
            },
            Err(e)=>{
                if let Some(bytes)=backup {
                    fs::write(&target,bytes)?;
                }

                failure(
                    &mut skipped,
                    request.unrestricted,
                    "WARPS",
                    e
                )?;
            }
        }

        verify_stage(&target,entry,&mut staged_len,request.index.entry_count())?;

        current=target;
    }

    // ========================================================
    // LCK_R616D_NATIVE_MAN_CAMERA_EXPORT
    if let Some(camera)=camera_changes.as_ref()
        .filter(|c|c.changed())
    {
        let destination=temps.add(
            dir.join(format!("{prefix}_camera.img"))
        )?;

        copy_stage(&current,&destination)?;

        // Hard failure: never claim a successful camera export
        // if MAN encoding, compression or readback fails.
        let report=native_gameplay_camera::overlay_existing(
            &destination,
            request.source,
            request.index,
            &request.scene.name,
            camera
        )?;

        verify_stage(&destination,entry,&mut staged_len,request.index.entry_count())?;

        current=destination;
        wrote=true;
        patched.push(report);
    }
    // LCK_R612A_NATIVE_128_REGIONS
    // Modify only an independent export-stage copy.

    // LCK_R626_REGION_ISOLATION
    // Warp-only or MAN-only exports must not force experimental
    // full-native-128 expansion merely because it is enabled.
    let r626_region_needed=r626_region_work_requested(
        has_objects,
        has_floors,
        has_collision
    );

    if wrote && !r626_region_needed &&
        request.scene.name.eq_ignore_ascii_case("town01")
    {
        patched.push(
            "R626: warp/script-only export; experimental region expansion was not requested".into()
        );
    }

    if wrote && r626_region_needed &&
        request.scene.name.eq_ignore_ascii_case("town01")
    {
        let target = temps.add(
            dir.join(format!("{prefix}_native128.img"))
        )?;

        copy_stage(&current, &target)?;

        let full = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d|
                d.join("LCK_NATIVE128_FULL.enabled").exists()
            ))
            .unwrap_or(false);

        match native128_region_unlock::patch_image(
            &target,
            entry,
            &retail,
            full
        ) {
            Ok(Some(summary)) => {
                verify_stage(&target,entry,&mut staged_len,request.index.entry_count())?;

                current = target;
                patched.push(summary.message());
                native128_summary = Some(summary);
            },

            Ok(None) => {
                let _ = fs::remove_file(&target);

                if full {
                    bail!("R613: full native 128 was requested, but no activation regions were created");
                }

                skipped.push(
                    "R6.1.2A: no eligible new native MAP cells; no additional regions needed".into()
                );
            },

            Err(error) => {
                let _ = fs::remove_file(&target);

                if full {
                    return Err(error.context(
                        "R613: full native 128 region activation failed"
                    ));
                }

                skipped.push(format!(
                    "R6.1.2A region stage skipped: {error:#}"
                ));
            }
        }
    }
    // R613: inspect serialized native MAP before publication.
    // A successful audit does not assert PS1 runtime gameplay success.
    let r613_coverage=if wrote &&
        r626_region_needed &&
        request.scene.name.eq_ignore_ascii_case("town01")
    {
        Some(native128_region_unlock::r613_coverage_report(
            &retail,
            &read_map(&current,entry)?,
            r613_full_requested
        )?)
    }else{
        None
    };
    // LCK_R615_NATIVE128_EXPERIMENTAL_WIDE_EXTERIOR
    // The original 128x128 native grid stays unchanged.
    // This option changes the retail region selection window.
    // It does not install MIPS runtime hooks.
    let r615_wide_enabled = std::env::current_exe()
        .ok()
        .and_then(|p|p.parent().map(|d|
            d.join("LCK_R615_WIDE_EXTERIOR.enabled").exists()
        ))
        .unwrap_or(false);
    let mut r615_wide_used = false;

    if r615_wide_enabled &&
        r626_region_needed &&
        request.scene.name.eq_ignore_ascii_case("town01")
    {
        ensure!(native128_summary.is_some(),
            "R615 wide-window mode requires successful R6.1.2A region activation; enable LCK_NATIVE128_FULL.enabled and inspect the region report");

        let target=temps.add(
            dir.join(format!("{prefix}_r615_wide.img"))
        )?;

        copy_stage(&current,&target)?;

                // v2.0.8.5 opt-in native MAP region experiment.
        let unified_requested=std::env::current_exe()
            .ok()
            .and_then(|p|
                p.parent().map(|d|
                    d.join("LCK_V2085_UNIFIED128.enabled").exists()
                )
            )
            .unwrap_or(false);

        let message=if unified_requested {
            native128_r615::patch_unified(
                &target,entry,&retail
            )?
        } else {
            native128_r615::patch_wide(
                &target,entry,&retail
            )?
        };

        verify_stage(&target,entry,&mut staged_len,request.index.entry_count())?;

        current=target;
        patched.push(message);
        r615_wide_used=true;
    }
    // LCK_R636_IMPORT_INTEGRITY
    //
    // After all normal export stages, reconcile every newly
    // imported MAP descriptor against its original object-stage
    // identity. Only detached static decorations may be re-anchored.
    // Never rewrite MAN-bound actor positions as static placements.
    if let Some(ref object_stage)=r636_object_stage_map {
        let final_before=read_map(&current,entry)?;

        let actual_lut=scene.field_floor_height_lut(request.index)?;
        let fallback_lut=[0i16;16];
        let lut=actual_lut.as_ref().unwrap_or(&fallback_lut);

        let (repaired,inspected,moved)=
            r636_import_relocation::reconcile(
                &retail,
                object_stage,
                &final_before,
                lut
            )?;

        ensure!(
            actual_lut.is_some() || repaired==final_before,
            "R636: native floor height LUT absent; cannot safely relocate or vertically compensate imported objects"
        );

        if inspected>0 {
            patched.push(format!(
                "R636 FINAL IMPORT INTEGRITY: {} independent native MAP references checked; {} static references restored using re-anchoring or experimental dual-owner ground/static composition; model identity retained; runtime gameplay not yet verified",
                inspected,moved
            ));
        }

        if repaired!=final_before {
            let target=temps.add(
                dir.join(format!("{prefix}_r636_imports.img"))
            )?;

            copy_stage(&current,&target)?;

            let mut disc=DiscPatcher::open(fs::read(&target)?)?;

            disc.patch_prot_entry(
                entry,0,&repaired
            )?;

            ensure!(
                disc.read_entry_footprint(entry)?==repaired,
                "R636: native MAP patcher readback differs"
            );

            fs::write(
                &target,
                disc.into_image()
            )?;

            verify_stage(
                &target,
                entry,
                &mut staged_len,
                request.index.entry_count()
            )?;

            ensure!(
                read_map(&target,entry)?==repaired,
                "R636: physical IMG native MAP readback differs"
            );

            current=target;
        }
    }

    // LCK_R624_DISC_GROWTH
    if staged_len>source_len {
        patched.push(format!(
            "Native ISO/PROT relayout verified: original {} bytes, rebuilt {} bytes (+{} raw sectors)",
            source_len,
            staged_len,
            (staged_len-source_len)/2352
        ));
    }
    // Final validation and atomic publication
    // ========================================================

    // R638: carry the original stage failure into the error and debug.txt.
    r638_require_native_write(wrote,&skipped)?;

    verify_stage(&current,entry,&mut staged_len,request.index.entry_count())?;

    // LCK_R615_NATIVE_MAP_CONTRACT_AUDIT
    // Report native MAP categories, including zero-byte evidence.
    // Do not misidentify kind-2 elevation overrides (0x0800)
    // as walk ground (0x1000).
    let r615_audit=if request.scene.name.eq_ignore_ascii_case("town01") {
        native128_r615::audit(
            &retail,
            &read_map(&current,entry)?,
            &manifest,
            r615_wide_used
        )?
    } else {
        match native128_r615::audit(
            &retail,
            &read_map(&current,entry)?,
            &manifest,
            false
        ) {
            Ok(report)=>report,
            Err(e)=>format!(
                "R615: no compatible region audit for {}: {e:#}; native export unchanged",
                request.scene.name
            ),
        }
    };
    // R626: check final native MAP after objects, floors, collisions,
    // and runtime-region modifications.
    let final_map=read_map(&current,entry)?;

    let deletion_summary=r626_verify_final_deletions(
        &request,&retail,&final_map
    )?;

    if request.world.draws.iter().any(|draw|
        draw.hidden && draw.editor_uid.is_none()
    ){
        ensure!(
            !skipped.iter().any(|line|
                line.starts_with("FAILED NATIVE OBJECTS:")
                || line.starts_with("FAILED DELETE ")
                || line.starts_with("FAILED OWNED COLLISION RECONCILIATION:")
            ),
            "R626: an object deletion writer failed; incomplete IMG not published"
        );
    }

    patched.push(deletion_summary);

    // Both paths are in the destination directory and therefore
    // on the same volume. An existing output is never replaced.
        // LCK_R618B_PUBLISH_DIAGNOSTICS
    fs::hard_link(
        &current,
        request.output
    ).with_context(||format!(
        "IMG PUBLICATION FAILED: staged={} output={}",
        current.display(),
        request.output.display()
    ))?;

    let verified=(||->Result<()>{
        verify_stage(request.output,entry,&mut staged_len,request.index.entry_count())?;

        ensure!(
            read_map(request.output,entry)?
                ==read_map(&current,entry)?,
            "Published MAP differs from staged native MAP"
        );

        Ok(())
    })();

    if let Err(error)=verified {
        let _=fs::remove_file(request.output);

        return Err(error);
    }

    let r615_path=request.output.with_extension(
        "r615_native_128_audit.txt"
    );

    let r615_detail=format!(
        "{r615_audit}\nENCODED / ATTEMPTED NATIVE EXPORT STAGES\n{}\n\nSKIPPED / FAILED NATIVE OPERATIONS\n{}\n",
        patched.join("\n"),
        skipped.join("\n")
    );

    match fs::write(&r615_path,r615_detail) {
        Ok(())=>patched.push(format!(
            "R615 native map/spawn audit: {}",
            r615_path.display()
        )),
        Err(e)=>skipped.push(format!(
            "R615 audit report could not be saved: {e}"
        )),
    }
    if let Some(summary) = native128_summary {
        let report = request.output.with_extension(
            "r612_native128_regions.txt"
        );

        match fs::write(&report, format!("{}\n{}",summary.report(),r613_coverage.as_deref().unwrap_or(""))) {
            Ok(()) => {
                patched.push(format!(
                    "R6.1.2A report: {}",
                    report.display()
                ));
            },

            Err(e) => {
                skipped.push(format!(
                    "R6.1.2A report write failed: {e}"
                ));
            }
        }
    }
    // Extended authoring data is deliberately identified as
    // metadata, not falsely claimed to be native PS1 runtime data.
    match native_extended_manifest::write_donor_bundle(
        request.output,
        &request.scene.name,
        &retail,
        request.floors,
        request.extension_side
    ) {
        Ok(Some(bundle))=>{
            patched.push(format!(
                "R6.0.8 immutable donor bundle: {} cells, {} source tiles, {}. Runtime 256-grid is NOT enabled.",
                bundle.cells,bundle.sources,bundle.path.display()
            ));
        },
        Ok(None)=>{},
        Err(e)=>{
            skipped.push(format!(
                "R6.0.8 donor compilation unavailable: {e}"
            ));
        }
    }

    match native_extended_manifest::write_manifest(
        request.output,
        &request.scene.name,
        request.world,
        request.floors,
        request.extension_side
    ) {
        Ok(report)=>{
            patched.push(format!(
                "Extended grid metadata: {} floors, {} objects at {}",
                report.extended_floors,
                report.extended_objects,
                report.path.display()
            ));

            if report.extended_floors>0 || report.extended_objects>0 {
                skipped.push(
                    "Extended-grid authoring recorded as metadata; not native engine runtime geometry".into()
                );
            }
        },
        Err(e)=>{
            skipped.push(format!(
                "Extended metadata not written: {e}"
            ));
        }
    }

    let status=format!(
        "LCK R6.0 unified export\nCell: {}\n{}\nUnresolved: {}\nGameplay unverified\n",
        request.scene.name,
        manifest.summary(),
        skipped.join(" | ")
    );

    let _=fs::write(
        request.output.with_extension("r60_export_report.txt"),
        status
    );

    patched.push(
        "Native MAP reread and IMG length verified; gameplay not yet tested".into()
    );

    Ok(ps1_export::ExportResult{
        path:request.output.to_path_buf(),
        patched,
        skipped,
        imports,
    })
}

#[cfg(test)]
mod lck_r626_deletion_integrity_tests {
    use super::*;

    #[test]
    fn deleted_native_grid_reference_cannot_survive()->Result<()> {
        let mut retail=vec![0u8;0x12000];
        let at=0x8000+(10*128+9)*2;

        retail[at..at+2].copy_from_slice(
            &(0x2000u16|17).to_le_bytes()
        );

        assert!(
            r626_verify_grid_delete(&retail,&retail,(9,10)).is_err()
        );

        let mut removed=retail.clone();
        removed[at..at+2].fill(0);

        r626_verify_grid_delete(
            &retail,&removed,(9,10)
        )?;

        let mut replacement=retail.clone();

        replacement[at..at+2].copy_from_slice(
            &(0x1000u16|18).to_le_bytes()
        );

        r626_verify_grid_delete(
            &retail,&replacement,(9,10)
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod lck_r60_coordinator_tests {
    use super::*;

    #[test]
    fn stage_copy_has_independent_bytes()->Result<()> {
        let dir=tempfile::tempdir()?;

        let source=dir.path().join("original.img");
        let copy=dir.path().join("copy.img");

        fs::write(&source,[1u8,2,3,4])?;

        copy_stage(&source,&copy)?;

        fs::write(&copy,[4u8,3,2,1])?;

        assert_eq!(
            fs::read(&source)?,
            [1,2,3,4]
        );

        Ok(())
    }

    #[test]
    fn stage_rejects_overwriting_existing_file()->Result<()> {
        let dir=tempfile::tempdir()?;

        let source=dir.path().join("source.img");
        let copy=dir.path().join("copy.img");

        fs::write(&source,[1u8])?;
        fs::write(&copy,[2u8])?;

        assert!(
            copy_stage(&source,&copy).is_err()
        );

        Ok(())
    }
}

// LCK_R618B_STAGE_REGRESSION
#[cfg(test)]
mod r618b_stage_tests {
    use super::*;

    #[test]
    fn readonly_master_produces_writable_stage()->Result<()> {
        let dir=tempfile::tempdir()?;
        let original=dir.path().join("source.img");
        let stage=dir.path().join("stage.img");

        fs::write(&original,[1u8,2,3,4,5,6])?;

        let mut p=fs::metadata(&original)?.permissions();
        p.set_readonly(true);
        fs::set_permissions(&original,p)?;

        copy_stage(&original,&stage)?;

        assert_eq!(
            fs::read(&stage)?,
            vec![1u8,2,3,4,5,6]
        );

        assert!(
            !fs::metadata(&stage)?.permissions().readonly()
        );

        assert!(
            fs::metadata(&original)?.permissions().readonly()
        );

        let mut p=fs::metadata(&original)?.permissions();
        p.set_readonly(false);
        fs::set_permissions(&original,p)?;

        Ok(())
    }

    #[test]
    fn readonly_master_supports_new_img_publication()->Result<()> {
        let dir=tempfile::tempdir()?;
        let original=dir.path().join("master.img");
        let stage=dir.path().join("stage.img");
        let output=dir.path().join("output.img");

        let master=vec![0x33u8;4096];
        let updated=vec![0x55u8;4096];

        fs::write(&original,&master)?;

        let mut p=fs::metadata(&original)?.permissions();
        p.set_readonly(true);
        fs::set_permissions(&original,p)?;

        copy_stage(&original,&stage)?;

        fs::write(&stage,&updated)?;
        fs::hard_link(&stage,&output)?;

        assert_eq!(fs::read(&output)?,updated);
        assert_eq!(fs::read(&original)?,master);

        assert!(
            fs::metadata(&original)?.permissions().readonly()
        );

        let mut p=fs::metadata(&original)?.permissions();
        p.set_readonly(false);
        fs::set_permissions(&original,p)?;

        Ok(())
    }

    #[test]
    fn staging_never_overwrites_existing_files()->Result<()> {
        let dir=tempfile::tempdir()?;
        let source=dir.path().join("source.img");
        let destination=dir.path().join("existing.img");

        fs::write(&source,[1u8,2,3])?;
        fs::write(&destination,[9u8,8,7])?;

        assert!(copy_stage(&source,&destination).is_err());

        assert_eq!(
            fs::read(&destination)?,
            vec![9u8,8,7]
        );

        Ok(())
    }
}


#[cfg(test)]
mod lck_r624_disc_growth_tests {
    use super::*;

    #[test]
    fn accepts_two_sector_relayout(){
        let original=466_714_416u64;
        let rebuilt=466_719_120u64;

        assert_eq!(
            rebuilt-original,
            2352*2
        );

        assert!(
            r624_validate_length(
                original,rebuilt
            ).is_ok()
        );
    }

    #[test]
    fn rejects_truncated_stages(){
        let original=466_719_120u64;
        let truncated=466_714_416u64;

        assert!(
            r624_validate_length(
                original,truncated
            ).is_err()
        );
    }

    #[test]
    fn rejects_unaligned_growth(){
        let original=466_714_416u64;

        assert!(
            r624_validate_length(
                original,original+1
            ).is_err()
        );
    }

    #[test]
    fn accepts_unchanged_sector_aligned_stage(){
        let original=466_714_416u64;

        assert!(
            r624_validate_length(
                original,original
            ).is_ok()
        );
    }
}


#[cfg(test)]
mod lck_r626_region_isolation_tests {
    use super::*;

    #[test]
    fn warp_only_does_not_request_region_expansion(){
        assert!(
            !r626_region_work_requested(
                false,false,false
            )
        );
    }

    #[test]
    fn authored_geometry_still_requests_regions(){
        assert!(r626_region_work_requested(
            true,false,false
        ));

        assert!(r626_region_work_requested(
            false,true,false
        ));

        assert!(r626_region_work_requested(
            false,false,true
        ));
    }
}


// R638_EXPORT_FAILURE_CAUSE
fn r638_require_native_write(wrote:bool,failures:&[String])->Result<()> {
    if !wrote {
        let detail=if failures.is_empty() {
            "No native stage failure was recorded".to_owned()
        } else {
            failures.join("\n")
        };
        bail!("No native bytes encoded; no empty IMG published\nNative stage failures:\n{detail}");
    }
    Ok(())
}

#[cfg(test)]
mod r638_export_error_tests {
    use super::*;

    #[test]
    fn unrestricted_warp_rejection_reaches_the_final_error() {
        let mut failures=Vec::new();
        failure(&mut failures,true,"DOORS",anyhow::anyhow!("test MAN rejection")).unwrap();
        let error=r638_require_native_write(false,&failures).unwrap_err();
        let report=format!("{error:#}");
        assert!(report.contains("FAILED NATIVE DOORS"));
        assert!(report.contains("test MAN rejection"));
        assert!(report.contains("no empty IMG published"));
    }

    #[test]
    fn successful_write_and_missing_failure_details_are_distinct() {
        assert!(r638_require_native_write(true,&[]).is_ok());
        let error=r638_require_native_write(false,&[]).unwrap_err();
        assert!(error.to_string().contains("No native stage failure was recorded"));
    }
}
