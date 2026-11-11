// LCK_R52_CELL_CAMERA
// Per-cell startup camera settings.
//
// Editor-only preferences. These values never modify the
// native PS1 IMG, project contents, transforms or MAP records.
//
// Keep this module when the temporary Save Camera menu item
// is eventually removed.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize,Serialize};

use crate::raster::Camera;
use super::cell_catalog::CellSelection;

const VERSION:u32=1;

#[derive(Clone,Copy,Serialize,Deserialize)]
struct SavedCamera {
    yaw:f32,
    pitch:f32,
    zoom:f32,
    target_offset:[f32;3],
}

impl SavedCamera {
    fn from_camera(camera:Camera)->Self {
        Self {
            yaw:camera.yaw,
            pitch:camera.pitch,
            zoom:camera.zoom,
            target_offset:camera.target_offset,
        }
    }

    fn camera(self)->Camera {
        Camera {
            yaw:self.yaw,
            pitch:self.pitch,
            zoom:self.zoom,
            target_offset:self.target_offset,
        }
    }

    fn valid(self)->bool {
        self.yaw.is_finite()
            && self.pitch.is_finite()
            && self.zoom.is_finite()
            && self.zoom>0.0
            && self.target_offset.iter()
                .all(|value|value.is_finite())
    }
}

#[derive(Default,Serialize,Deserialize)]
struct Bank {
    version:u32,
    cells:BTreeMap<String,SavedCamera>,
}

// Original cells and copied editor cells need independent
// camera records, even when they share the same retail scene.

pub(super) fn key(
    source:&str,
    selection:Option<&CellSelection>
)->String {
    match selection {
        Some(CellSelection::Copy(id))=>
            format!("{source}::copy:{id}"),
        _=>format!("{source}::original"),
    }
}

fn settings_path()->Result<PathBuf,String> {
    let exe=std::env::current_exe()
        .map_err(|error|format!(
            "Cannot locate LCK executable: {error}"
        ))?;

    let parent=exe.parent().ok_or(
        "LCK executable has no parent folder"
    )?;

    Ok(parent
        .join("settings")
        .join("startup_cameras.json"))
}

fn read_bank(path:&Path)->Result<Bank,String> {
    let raw=match fs::read(path) {
        Ok(raw)=>raw,
        Err(error)
            if error.kind()==std::io::ErrorKind::NotFound =>
        {
            return Ok(Bank::default());
        }
        Err(error)=>return Err(format!(
            "Camera settings read failed: {error}"
        )),
    };

    let bank:Bank=serde_json::from_slice(&raw)
        .map_err(|error|format!(
            "Camera settings are invalid: {error}"
        ))?;

    if bank.version>VERSION {
        return Err(
            "Camera settings use a newer unsupported format"
                .to_owned()
        );
    }

    Ok(bank)
}

fn load_at(path:&Path,key:&str)->Result<Option<Camera>,String> {
    let bank=read_bank(path)?;

    Ok(bank.cells.get(key)
        .copied()
        .filter(|camera|camera.valid())
        .map(SavedCamera::camera))
}

fn save_at(
    path:&Path,
    key:&str,
    camera:Camera
)->Result<(),String> {
    if key.is_empty() {
        return Err("Camera cell key is empty".into());
    }

    let saved=SavedCamera::from_camera(camera);

    if !saved.valid() {
        return Err(
            "Camera has invalid position, angle or zoom"
                .to_owned()
        );
    }

    let mut bank=read_bank(path)?;

    bank.version=VERSION;
    bank.cells.insert(key.to_owned(),saved);

    let json=serde_json::to_vec_pretty(&bank)
        .map_err(|error|error.to_string())?;

    let folder=path.parent().ok_or(
        "Camera settings folder not available"
    )?;

    fs::create_dir_all(folder)
        .map_err(|error|error.to_string())?;

    // Preserve the previous settings when replacing
    // an existing bank of cell camera presets.

    if path.exists() {
        fs::copy(
            path,
            path.with_extension("json.bak")
        ).map_err(|error|format!(
            "Camera settings backup failed: {error}"
        ))?;
    }

    let temporary=path.with_extension("json.pending");

    fs::write(&temporary,&json)
        .map_err(|error|format!(
            "Camera settings staging failed: {error}"
        ))?;

    if let Err(rename_error)=fs::rename(&temporary,path) {
        // Some Windows filesystem configurations do not
        // replace an existing destination through rename.
        // The .bak file above is retained for recovery.
        fs::copy(&temporary,path)
            .map_err(|error|format!(
                "Camera settings replacement failed: {rename_error}; {error}"
            ))?;

        let _=fs::remove_file(&temporary);
    }

    let readback=fs::read(path)
        .map_err(|error|error.to_string())?;

    if readback!=json {
        return Err(
            "Camera settings write/readback mismatch".into()
        );
    }

    Ok(())
}

pub(super) fn load(key:&str)->Result<Option<Camera>,String> {
    load_at(&settings_path()?,key)
}

pub(super) fn save(
    key:&str,
    camera:Camera
)->Result<PathBuf,String> {
    let path=settings_path()?;
    save_at(&path,key,camera)?;
    Ok(path)
}

#[cfg(test)]
mod lck_r52_camera_tests {
    use super::*;

    fn test_path()->PathBuf {
        let now=std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        std::env::temp_dir().join(format!(
            "lck_r52_camera_{}_{}.json",
            std::process::id(),now
        ))
    }

    #[test]
    fn save_reload_and_overwrite_preserve_exact_camera() {
        let path=test_path();
        let mut cam=Camera::default();

        cam.yaw=1.23;
        cam.pitch=2.18;
        cam.zoom=0.48;
        cam.target_offset=[512.0,96.0,-1024.0];

        save_at(&path,"town01::original",cam).unwrap();

        assert_eq!(
            load_at(&path,"town01::original").unwrap(),
            Some(cam)
        );

        cam.zoom=0.25;
        cam.target_offset=[700.0,64.0,-900.0];

        save_at(&path,"town01::original",cam).unwrap();

        assert_eq!(
            load_at(&path,"town01::original").unwrap(),
            Some(cam)
        );

        let _=fs::remove_file(&path);
        let _=fs::remove_file(
            path.with_extension("json.bak")
        );
    }

    #[test]
    fn different_cells_do_not_overwrite_each_other() {
        let path=test_path();
        let mut a=Camera::default();
        let mut b=Camera::default();

        a.zoom=0.3;
        b.zoom=2.5;

        save_at(&path,"town01::original",a).unwrap();
        save_at(&path,"town02::original",b).unwrap();

        assert_eq!(
            load_at(&path,"town01::original").unwrap(),
            Some(a)
        );

        assert_eq!(
            load_at(&path,"town02::original").unwrap(),
            Some(b)
        );

        let _=fs::remove_file(&path);
        let _=fs::remove_file(
            path.with_extension("json.bak")
        );
    }

    #[test]
    fn reject_invalid_cameras_and_preserve_copy_keys() {
        let path=test_path();
        let mut camera=Camera::default();

        camera.zoom=f32::NAN;

        assert!(save_at(
            &path,"town01::original",camera
        ).is_err());

        assert_ne!(
            key("town01",None),
            key("town01",Some(&CellSelection::Copy(7)))
        );

        assert!(load_at(
            &path,"town01::original"
        ).unwrap().is_none());
    }
}