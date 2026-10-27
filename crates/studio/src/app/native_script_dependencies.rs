//! Structured observations, not a proof of script control-flow closure.
use legaia_scene_formats::{field_disasm::{InsnInfo, LinearWalker, MenuCtrlKind}, man_section};

#[derive(Clone, Debug)]
pub(super) struct WallWrite {
    pub pc: usize,
    pub sub: u8,
    pub x0: u8, pub z0: u8, pub x1: u8, pub z1: u8, pub mask: u8,
}
impl WallWrite {
    /// Match the native tile loop and its four-bit sub-cell mask exactly.
    /// Wrapped end bounds do not imply a wrapping iteration range.
    pub fn touches(&self, [sx, sz]: [u8; 2]) -> bool {
        let bit = 1 << ((sx & 1) + 2 * (sz & 1));
        let mask = match self.sub { 0 | 1 => 15, 2 | 3 => self.mask & 15, _ => 0 };
        mask & bit != 0 && (self.x0..self.x1.wrapping_add(1)).contains(&(sx / 2)) &&
            (self.z0.wrapping_add(1)..self.z1.wrapping_add(2)).contains(&(sz / 2))
    }
    pub fn description(&self) -> String {
        let effect = match self.sub {0 => "clear all", 1 => "set all", 2 => "clear mask", 3 => "set mask", _ => "unsupported sub-op (halts)"};
        format!("sub: {}, X [{},{}) Z [{},{}) (half-open, u8 wrapping); {effect}, mask: {}",
            self.sub, self.x0, self.x1.wrapping_add(1), self.z0.wrapping_add(1), self.z1.wrapping_add(2), self.mask & 15)
    }
}

#[derive(Default, Debug)]
pub(super) struct ScriptScan {
    pub instructions: Vec<String>,
    pub targets: Vec<(usize, u8)>,
    pub wall_writes: Vec<WallWrite>,
    pub error: Option<String>,
}
pub(super) fn scan_script(bytes: &[u8], pc0: usize) -> ScriptScan {
    let mut scan = ScriptScan::default();
    if pc0 >= bytes.len() { scan.error = Some("Invalid/empty script range".into()); return scan; }
    for decoded in LinearWalker::new(bytes, pc0) {
        match decoded {
            Ok(insn) => {
                if scan.instructions.len() >= 10000 { scan.error = Some("Instruction report limit reached".into()); break; }
                if let Some(target) = insn.extended { scan.targets.push((insn.pc, target)); }
                if let InsnInfo::MenuCtrl { kind: MenuCtrlKind::Nibble7Tile {sub, x0, z0, x1, z1, mask}, .. } = &insn.info {
                    scan.wall_writes.push(WallWrite {pc: insn.pc, sub: *sub, x0: *x0, z0: *z0, x1: *x1, z1: *z1, mask: *mask});
                }
                scan.instructions.push(format!("+{:#x}: {:?}", insn.pc, insn));
            }
            Err((pc, error)) => {scan.error = Some(format!("Decode stopped at +{pc:#x}: {error:?}")); break;}
        }
    }
    scan
}

#[derive(Debug)]
pub(super) struct RecordScan {
    pub partition: usize, pub record: usize, pub flat: usize,
    pub start: usize, pub pc0: usize, pub len: usize,
    pub scan: ScriptScan,
}
#[derive(Debug)]
pub(super) struct SceneInventory {
    pub man_sha256: String,
    pub partition_counts: [i16; 3],
    pub records: Vec<RecordScan>,
    pub gaps: Vec<String>,
}
impl SceneInventory {
    pub fn read(man: &[u8]) -> Result<Self, String> {
        let file = man_section::parse(man).map_err(|e| e.to_string())?;
        let mut result = Self {man_sha256: super::collision_recipes::digest(man),
            partition_counts: file.header.partition_counts, records: Vec::new(), gaps: Vec::new()};
        let mut base = 0;
        for (partition, records) in file.partitions.iter().enumerate() {
            for record in 0..records.len() {
                let Some((start, pc0, len)) = legaia_engine_field::man_field_scripts::partition_record_span(&file, man, partition, record) else {
                    result.gaps.push(format!("P{partition}[{record}] flat {}: invalid record span/header", base + record)); continue;
                };
                let scan = scan_script(&man[start..start + len], pc0);
                if let Some(error) = &scan.error {result.gaps.push(format!("P{partition}[{record}] flat {}: {error}", base + record));}
                result.records.push(RecordScan {partition, record, flat: base + record, start, pc0, len, scan});
            }
            base += records.len();
        }
        Ok(result)
    }
    pub fn incoming(&self, target: usize) -> Vec<(usize, usize, usize)> {
        self.records.iter().flat_map(|r| r.scan.targets.iter().filter_map(move |&(pc, id)|
            (id as usize == target).then_some((r.flat, r.start + pc, pc)))).collect()
    }
    pub fn writers_at(&self, cell: [u8; 2]) -> Vec<(usize, usize)> {
        self.records.iter().flat_map(|r| r.scan.wall_writes.iter().filter_map(move |w|
            w.touches(cell).then_some((r.flat, r.start + w.pc)))).collect()
    }
}

#[cfg(test)]
pub(super) fn fixture_man(scripts: &[&[u8]]) -> Vec<u8> {
    // P1 records have one locals-count byte and a four-byte actor header.
    let mut man = vec![0; 0x2b + scripts.len() * 3];
    man[0x24..0x26].copy_from_slice(&(scripts.len() as i16).to_le_bytes());
    let base = man.len();
    for (i, script) in scripts.iter().enumerate() {
        let rel = (man.len() - base) as u32;
        man[0x2b + i*3..0x2e + i*3].copy_from_slice(&rel.to_le_bytes()[..3]);
        man.extend_from_slice(&[0; 5]); man.extend_from_slice(script);
    }
    let rel = (man.len() - base) as u32;
    man[0x28..0x2b].copy_from_slice(&rel.to_le_bytes()[..3]);
    man.extend_from_slice(&[0; 18]); man
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn masks_row_bias_and_wrapped_empty_ranges_match_native_writes() {
        let w = WallWrite {pc: 0, sub: 2, x0: 3, x1: 3, z0: 4, z1: 4, mask: 0x81};
        assert!(w.touches([6, 10]));
        assert!(!w.touches([7, 10]));
        assert!(!w.touches([6, 8])); assert!(!w.touches([8, 10]));
        let all = WallWrite {sub: 0, ..w.clone()};
        assert!(all.touches([7, 11]));
        let wrapped = WallWrite {x0: 255, x1: 255, ..all};
        assert!(!wrapped.touches([254, 10]));
    }
    #[test]
    fn inventory_preserves_incoming_sites_writers_and_decode_gaps() {
        let man = fixture_man(&[&[0xa1, 0, 0x21], &[0x4c, 0x73, 3, 4, 3, 4, 1, 0x21], &[0x4c, 0x73]]);
        let inventory = SceneInventory::read(&man).unwrap();
        assert_eq!(inventory.records.len(), 3); assert_eq!(inventory.gaps.len(), 1);
        assert_eq!(inventory.incoming(0).len(), 1);
        assert_eq!(inventory.writers_at([6, 10]), vec![(1, inventory.records[1].start + 5)]);
        assert!(inventory.writers_at([7, 10]).is_empty());
        assert_eq!(inventory.records[2].scan.wall_writes.len(), 0);
    }
}
