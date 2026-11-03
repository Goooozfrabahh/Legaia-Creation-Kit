//! LCK master disc protection.
#![allow(dead_code)]

use anyhow::{Result,Context,ensure,bail};
use std::{
    fs,
    io::ErrorKind,
    path::{Path,PathBuf},
};

pub const ORIGINAL_IMG:&str="D:\\Emulators\\Games\\Legend of Legaia\\LegendOfLegaia.img";

pub fn original_path()->PathBuf{
    PathBuf::from(ORIGINAL_IMG)
}

fn equal_paths(a:&Path,b:&Path)->bool{
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}

fn protected_path()->PathBuf{
    let original=original_path();
    fs::canonicalize(&original).unwrap_or(original)
}

fn destination_path(output:&Path)->Result<PathBuf>{
    let name=output.file_name().context("Export filename missing")?;
    let parent=output.parent().context("Export directory missing")?;

    let parent=if parent.as_os_str().is_empty(){
        Path::new(".")
    }else{
        parent
    };

    Ok(fs::canonicalize(parent)?.join(name))
}

/// Enforces create-only IMG export, irrespective of whether the
/// editor is currently displaying the retail IMG or another disc.
pub(super) fn ensure_new_output(
    source:&Path,
    output:&Path,
)->Result<()>{
    let source=fs::canonicalize(source)
        .context("Source IMG is unavailable")?;

    let target=destination_path(output)?;
    let master=protected_path();

    ensure!(
        !equal_paths(&target,&source),
        "PROTECTED: Cannot export over the loaded source IMG"
    );

    ensure!(
        !equal_paths(&target,&master),
        "PROTECTED: Cannot overwrite the original retail Legend of Legaia IMG"
    );

    match fs::symlink_metadata(output){
        Ok(_)=>{
            bail!(
                "PROTECTED: Export destination already exists. \
                 Choose a new IMG filename: {}",
                output.display()
            );
        }
        Err(e) if e.kind()==ErrorKind::NotFound=>{},
        Err(e)=>return Err(e.into()),
    }

    Ok(())
}

/// Collision overlays are allowed to modify newly generated
/// working IMGs, but never the original retail source.
pub(super) fn ensure_existing_overlay(target:&Path)->Result<()>{
    let actual=fs::canonicalize(target)?;
    let master=protected_path();

    ensure!(
        !equal_paths(&actual,&master),
        "PROTECTED: Cannot apply collision overlay to the original retail IMG"
    );

    ensure!(
        fs::metadata(target)?.is_file(),
        "Collision overlay target is not a file"
    );

    Ok(())
}

#[cfg(test)]
mod tests{
    use super::*;

    #[test]
    fn existing_output_is_never_accepted(){
        let dir=tempfile::tempdir().unwrap();
        let source=dir.path().join("source.img");
        let output=dir.path().join("existing.img");

        fs::write(&source,b"source").unwrap();
        fs::write(&output,b"existing").unwrap();

        assert!(ensure_new_output(&source,&output).is_err());
    }

    #[test]
    fn same_source_is_never_accepted(){
        let dir=tempfile::tempdir().unwrap();
        let source=dir.path().join("source.img");

        fs::write(&source,b"source").unwrap();

        assert!(ensure_new_output(&source,&source).is_err());
    }

    #[test]
    fn new_destination_is_allowed(){
        let dir=tempfile::tempdir().unwrap();
        let source=dir.path().join("source.img");
        let destination=dir.path().join("new_export.img");

        fs::write(&source,b"source").unwrap();

        assert!(ensure_new_output(&source,&destination).is_ok());
    }

    #[test]
    fn retail_path_is_protected(){
        let original=original_path();
        if original.exists(){
            assert!(ensure_new_output(&original,&original).is_err());
            assert!(ensure_existing_overlay(&original).is_err());
        }
    }
}
