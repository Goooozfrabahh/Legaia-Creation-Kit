//! Export-time trigger provenance. A grid bind bit is not a complete binding
//! lookup: FUN_8003A55C calls FUN_801D5630, primary then fallback, without a
//! gate filter. Keep raw record IDs even if MAN decoding is unsupported.
use anyhow::{ensure, Context, Result};
use std::collections::{BTreeMap,BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct Binding {
    pub record: u8,
    pub gate: u8,
    pub block: &'static str,
    pub offset: usize,
}

#[derive(Default, Debug)]
pub(in crate::app) struct NativeBindings {
    bindings:BTreeMap<(u8,u8), Binding>,
    trigger_cells:BTreeSet<(u8,u8)>,
}

impl NativeBindings {
    pub fn parse(primary:&[u8], fallback:&[u8]) -> Result<Self> {
        let mut result=Self::default();
        for (name,bytes) in [("MAP",primary),("fallback",fallback)] {
            if bytes.is_empty() {continue;}
            ensure!(bytes.len()>=18,"Truncated {name} trigger header");
            for kind in 0..3 {
                let h=kind*4;
                let offset=i16::from_le_bytes([bytes[h+2],bytes[h+3]]);
                let count=i16::from_le_bytes([bytes[h+4],bytes[h+5]]);
                // Native signed nonpositive count executes no iterations.
                if count<=0 {continue;}
                ensure!(offset>=18,"Invalid {name} kind-{kind} trigger offset");
                let start=offset as usize;
                let end=start+count as usize*4;
                let rows=bytes.get(start..end).with_context(||format!("Truncated {name} kind-{kind} trigger table"))?;
                for (i,row) in rows.chunks_exact(4).enumerate() {
                    result.trigger_cells.insert((row[0],row[1]));
                    if kind!=1 {continue;}
                    // First match wins, including duplicate primary rows and
                    // primary records that cannot be decoded as partition 0.
                    result.bindings.entry((row[0],row[1])).or_insert(Binding{
                        record:row[2],gate:row[3],block:name,offset:start+i*4,
                    });
                }
            }
        }
        Ok(result)
    }
    pub fn at(&self, cell:(u8,u8))->Option<&Binding> {self.bindings.get(&cell)}
    pub fn has_trigger(&self, cell:(u8,u8))->bool {self.trigger_cells.contains(&cell)}
}

#[cfg(test)]
mod tests {
    use super::*;
    fn block(rows:&[[u8;4]])->Vec<u8> {
        let mut b=vec![0;18];b[6..8].copy_from_slice(&18i16.to_le_bytes());
        b[8..10].copy_from_slice(&(rows.len() as i16).to_le_bytes());
        for row in rows {b.extend_from_slice(row);}b
    }
    #[test]
    fn teleport_and_elevation_cells_are_reserved_without_becoming_bindings() {
        let mut b=vec![0;26];
        b[2..4].copy_from_slice(&18i16.to_le_bytes());b[4..6].copy_from_slice(&1i16.to_le_bytes());
        b[10..12].copy_from_slice(&22i16.to_le_bytes());b[12..14].copy_from_slice(&1i16.to_le_bytes());
        b[18..26].copy_from_slice(&[4,5,0,0,6,7,0,0]);
        let bindings=NativeBindings::parse(&[],&b).unwrap();
        assert!(bindings.has_trigger((4,5))&&bindings.has_trigger((6,7)));
        assert!(bindings.at((4,5)).is_none()&&bindings.at((6,7)).is_none());
    }
    #[test]
    fn primary_first_match_keeps_unknown_man_record_and_nonzero_gate() {
        let binds=NativeBindings::parse(&block(&[[3,4,255,7],[3,4,1,0]]),&block(&[[3,4,2,0],[5,6,8,1]])).unwrap();
        assert_eq!(binds.at((3,4)).unwrap().record,255);
        assert_eq!(binds.at((3,4)).unwrap().gate,7);
        assert_eq!(binds.at((5,6)).unwrap().block,"fallback");
    }
    #[test]
    fn malformed_table_is_not_reported_as_unbound() {
        let mut b=block(&[[3,4,1,0]]);b.pop();
        assert!(NativeBindings::parse(&b,&[]).is_err());
        assert!(NativeBindings::parse(&[],&[0;9]).is_err());
        assert!(NativeBindings::parse(&[0;18],&[0;18]).unwrap().at((0,0)).is_none());
    }
}
