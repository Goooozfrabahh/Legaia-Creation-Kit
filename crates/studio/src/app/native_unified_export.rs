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
    ps1_export,
    ps1_original_guard,
    world_view,
    native_extended_manifest
};

#[path = "native128_region_unlock.rs"]
mod native128_region_unlock;

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

fn verify_stage(path:&Path,id:usize,len:u64)->Result<()> {
    ps1_export::verify_output_file(path,Some(len))?;

    ensure!(
        read_map(path,id)?.len()>=0x12000,
        "Native MAP is truncated"
    );

    Ok(())
}

fn copy_stage(src:&Path,dst:&Path)->Result<()> {
    ensure!(
        !dst.exists(),
        "Staging path already exists: {}",
        dst.display()
    );

    fs::copy(src,dst)?;

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
        has_objects || has_floors || has_collision
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

    let mut wrote=false;
    let mut patched=Vec::<String>::new();
    let mut skipped=Vec::<String>::new();
    let mut imports=None;
    let mut native128_summary=None;

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
                verify_stage(&target,entry,source_len)?;

                current=target;
                wrote=true;

                patched.extend(result.patched);
                skipped.extend(result.skipped);
                imports=result.imports;
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

        verify_stage(&target,entry,source_len)?;

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
            request.floors
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

        verify_stage(&target,entry,source_len)?;

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

        verify_stage(&target,entry,source_len)?;

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

        verify_stage(&target,entry,source_len)?;

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

        verify_stage(&target,entry,source_len)?;

        current=target;
    }

    // ========================================================
    // LCK_R612A_NATIVE_128_REGIONS
    // Modify only an independent export-stage copy.

    if wrote && request.scene.name.eq_ignore_ascii_case("town01") {
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
                verify_stage(&target, entry, source_len)?;

                current = target;
                patched.push(summary.message());
                native128_summary = Some(summary);
            },

            Ok(None) => {
                let _ = fs::remove_file(&target);

                skipped.push(
                    "R6.1.2A: no eligible new native MAP cells; no additional regions needed".into()
                );
            },

            Err(error) => {
                let _ = fs::remove_file(&target);

                skipped.push(format!(
                    "R6.1.2A region stage skipped: {error:#}"
                ));
            }
        }
    }
    // Final validation and atomic publication
    // ========================================================

    ensure!(
        wrote,
        "No native bytes encoded; no empty IMG published"
    );

    verify_stage(&current,entry,source_len)?;

    // Both paths are in the destination directory and therefore
    // on the same volume. An existing output is never replaced.
    fs::hard_link(
        &current,
        request.output
    ).context("Publish staged IMG atomically")?;

    let verified=(||->Result<()>{
        verify_stage(
            request.output,
            entry,
            source_len
        )?;

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

    if let Some(summary) = native128_summary {
        let report = request.output.with_extension(
            "r612_native128_regions.txt"
        );

        match fs::write(&report, summary.report()) {
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