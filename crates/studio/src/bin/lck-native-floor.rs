//! Command-line experimental native engine writer. Never starts a GUI or emulator.
use anyhow::{Result,ensure,Context};
use legaia_studio::app::native_extended_floor::{self,Request};
use std::path::PathBuf;
fn main()->Result<()> {
    let args=std::env::args_os().skip(1).collect::<Vec<_>>();
    ensure!((8..=9).contains(&args.len()),"Usage: lck-native-floor SOURCE_IMG ABSOLUTE_EXPERIMENTAL_OUTPUT_IMG CELL TILE_X TILE_Z DX DZ DY [--copy]");
    let parse=|i:usize|->Result<i32>{args[i].to_str().context("Argument is not text")?.parse().context("Invalid integer")};
    let duplicate=if args.len()==9 {ensure!(args[8]=="--copy","Unknown option");true}else{false};
    let request=Request{scene:args[2].to_str().context("Cell is not text")?.to_owned(),source_tile:[u8::try_from(parse(3)?)?,u8::try_from(parse(4)?)?],delta:[parse(5)?,parse(6)?],height_delta:i16::try_from(parse(7)?)?,duplicate};
    println!("{}",native_extended_floor::export(&PathBuf::from(&args[0]),&PathBuf::from(&args[1]),&request)?);Ok(())
}
