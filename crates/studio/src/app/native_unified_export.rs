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

    let mut wrote=false;
    let mut patched=Vec::<String>::new();
    let mut skipped=Vec::<String>::new();
    let mut imports=None;
    let mut native128_summary=None;

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

        verify_stage(&destination,entry,source_len)?;

        current=destination;
        wrote=true;
        patched.push(report);
    }
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

        verify_stage(&target,entry,source_len)?;

        current=target;
        patched.push(message);
        r615_wide_used=true;
    }
    // Final validation and atomic publication
    // ========================================================

    ensure!(
        wrote,
        "No native bytes encoded; no empty IMG published"
    );

    verify_stage(&current,entry,source_len)?;

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
