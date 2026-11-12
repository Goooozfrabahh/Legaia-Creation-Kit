//! Expanded field-grid storage and R3000 address calculation.
//!
//! This is a building block, NOT an installed engine patch. In particular,
//! converting a buffer does not update its loader, region boxes, MAN writers,
//! object spawning, or height sampling. No production export calls this module
//! until those consumers and the per-scene allocation lifecycle are integrated.
//! Native coordinates stay unchanged; only storage coordinates gain an origin.

use anyhow::{Result, ensure};
use crate::mips::*;
use std::collections::BTreeMap;

pub const TILE_UNITS: i32 = 128;
pub const RETAIL_RUNTIME_BYTES: usize = 0x14000;
pub const SPARSE_MAGIC: u32 = u32::from_le_bytes(*b"LKG1");
const PAGE_BYTES: usize = 16 * 16 * 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub collision: u8,
    pub object: u16,
}

/// Canonical sparse extension. Original 0..128 cells remain in the original
/// MAP allocation. Each occupied external 16x16 page costs 768 bytes; empty
/// pages cost only a two-byte directory slot. This avoids assuming that a
/// field has the 144 KiB needed by the dense layout conversion below.
pub struct SparseGrid {
    layout: Layout,
    pages: BTreeMap<usize, Vec<u8>>,
}

impl SparseGrid {
    pub fn new(layout: Layout) -> Self { Self { layout, pages: BTreeMap::new() } }
    pub fn page_count(&self) -> usize { self.pages.len() }
    fn directory_len(&self) -> usize { (self.layout.side() / 16).pow(2) }
    pub fn encoded_len(&self) -> usize { 16 + self.directory_len() * 2 + self.pages.len() * PAGE_BYTES }

    fn key(&self, tile: [i32; 2]) -> Option<(usize, usize)> {
        self.layout.index(tile)?;
        if tile.iter().all(|v| (0..128).contains(v)) { return None; }
        let [x, z] = tile.map(|v| (v - self.layout.origin_tile()) as usize);
        Some(((z / 16) * (self.layout.side() / 16) + x / 16, (z % 16) * 16 + x % 16))
    }

    pub fn set(&mut self, tile: [i32; 2], cell: Cell) -> Result<()> {
        let (page, index) = self.key(tile).ok_or_else(|| anyhow::anyhow!(
            "Sparse tile {tile:?} is outside the extension or belongs to the original MAP"))?;
        let bytes = self.pages.entry(page).or_insert_with(|| {
            let mut bytes = vec![0; PAGE_BYTES]; bytes[..256].fill(0xf7); bytes
        });
        bytes[index] = cell.collision;
        bytes[256 + index * 2..258 + index * 2].copy_from_slice(&cell.object.to_le_bytes());
        Ok(())
    }

    pub fn get(&self, tile: [i32; 2]) -> Option<Cell> {
        let (page, index) = self.key(tile)?;
        let bytes = self.pages.get(&page)?;
        Some(Cell { collision: bytes[index], object: u16::from_le_bytes([bytes[256 + index * 2], bytes[257 + index * 2]]) })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![0; 16 + self.directory_len() * 2];
        out[..4].copy_from_slice(&SPARSE_MAGIC.to_le_bytes());
        out[4..6].copy_from_slice(&1u16.to_le_bytes());
        out[6..8].copy_from_slice(&self.layout.side.to_le_bytes());
        out[8..10].copy_from_slice(&(self.layout.origin_tile() as i16).to_le_bytes());
        out[12..14].copy_from_slice(&(self.pages.len() as u16).to_le_bytes());
        out[14..16].copy_from_slice(&(self.directory_len() as u16).to_le_bytes());
        for (slot, (&page, bytes)) in self.pages.iter().enumerate() {
            out[16 + page * 2..18 + page * 2].copy_from_slice(&((slot + 1) as u16).to_le_bytes());
            out.extend_from_slice(bytes);
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() >= 16 && bytes[..4] == SPARSE_MAGIC.to_le_bytes(), "Invalid sparse grid header");
        let word = |at| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        ensure!(word(4) == 1 && word(10) == 0, "Unsupported sparse grid version/reserved fields");
        let layout = Layout::new(word(6))?;
        let mut grid = Self::new(layout);
        ensure!(word(8) as i16 as i32 == layout.origin_tile(), "Sparse grid origin mismatch");
        ensure!(word(14) as usize == grid.directory_len(), "Sparse grid directory mismatch");
        let count = word(12) as usize;
        let start = 16 + grid.directory_len() * 2;
        ensure!(bytes.len() == start + count * PAGE_BYTES, "Truncated/trailing sparse grid bytes");
        let mut next = 1;
        for page in 0..grid.directory_len() {
            let slot = word(16 + page * 2) as usize;
            if slot == 0 { continue; }
            ensure!(slot == next && slot <= count, "Aliased or noncanonical sparse grid page");
            let width = layout.side() / 16;
            let tile = [(page % width) as i32 * 16 + layout.origin_tile(), (page / width) as i32 * 16 + layout.origin_tile()];
            ensure!(grid.key(tile).is_some(), "Sparse page overlaps original MAP");
            let at = start + (slot - 1) * PAGE_BYTES;
            grid.pages.insert(page, bytes[at..at + PAGE_BYTES].to_vec());
            next += 1;
        }
        ensure!(next == count + 1, "Unreferenced sparse grid page");
        Ok(grid)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    side: u16,
}

impl Layout {
    pub fn new(side: u16) -> Result<Self> {
        ensure!(matches!(side, 128 | 160 | 192 | 224 | 256), "Unsupported field-grid side {side}");
        Ok(Self { side })
    }

    pub fn side(self) -> usize { self.side as usize }
    pub fn origin_tile(self) -> i32 { (128 - self.side as i32) / 2 }
    pub fn collision_offset(self) -> usize { 0x4000 }
    pub fn objects_offset(self) -> usize { 0x4000 + self.side() * self.side() }
    pub fn triggers_offset(self) -> usize { 0x4000 + 3 * self.side() * self.side() }
    pub fn fallback_offset(self) -> usize { self.triggers_offset() + 0x2000 }
    pub fn sound_offset(self) -> usize { self.fallback_offset() + 0x800 }
    pub fn runtime_bytes(self) -> usize { (self.fallback_offset() + 0x2000 + 0x7ff) & !0x7ff }
    pub fn sector_count(self) -> usize { self.runtime_bytes() / 0x800 }

    /// Half-open tile bounds. No narrowing cast or bit mask may precede this
    /// check: both would alias negative/out-of-range coordinates into the map.
    pub fn index(self, tile: [i32; 2]) -> Option<usize> {
        let x = tile[0].checked_sub(self.origin_tile())?;
        let z = tile[1].checked_sub(self.origin_tile())?;
        if !(0..self.side as i32).contains(&x) || !(0..self.side as i32).contains(&z) {
            return None;
        }
        Some(z as usize * self.side() + x as usize)
    }

    pub fn world_index(self, world: [i32; 2]) -> Option<usize> {
        self.index(world.map(|v| v.div_euclid(TILE_UNITS)))
    }

    /// Relocate grids while preserving every original descriptor, cell,
    /// primary trigger byte, fallback byte and sound byte. Input is the actual
    /// 40-sector runtime window, NOT only the 36-sector MAP entry. Trigger
    /// coordinates remain world tiles: their future consumers must translate
    /// them, rather than mutating shared source assets or byte-wrapping them.
    /// New cells have all wall bits set and the native non-walkable tier 7.
    /// Authored floor/geometry compilation must replace that default explicitly.
    pub fn expand_runtime_window(self, retail: &[u8]) -> Result<Vec<u8>> {
        ensure!(retail.len() == RETAIL_RUNTIME_BYTES,
            "Expected complete 0x14000-byte MAP/fallback/sound window, got {:#x}", retail.len());
        let mut out = vec![0; self.runtime_bytes()];
        out[..0x4000].copy_from_slice(&retail[..0x4000]);
        out[self.collision_offset()..self.objects_offset()].fill(0xf7);
        for z in 0..128 {
            let dst = self.index([0, z]).expect("retail fits every supported layout");
            let src = z as usize * 128;
            let c = self.collision_offset() + dst;
            out[c..c + 128].copy_from_slice(&retail[0x4000 + src..0x4000 + src + 128]);
            let o = self.objects_offset() + dst * 2;
            out[o..o + 256].copy_from_slice(&retail[0x8000 + src * 2..0x8000 + src * 2 + 256]);
        }
        out[self.triggers_offset()..self.triggers_offset() + 0x4000].copy_from_slice(&retail[0x10000..]);
        Ok(out)
    }

    /// Leaf function ABI: a0 = allocated map base; a1/a2 = signed world X/Z.
    /// Returns v0 = collision-byte address, v1 = object-word address; both zero
    /// when outside this layout. Clobbers only v0/v1/t0/t1/t2. No absolute code
    /// addresses or guessed RAM caves. Caller chooses a verified code arena.
    /// This helper does NOT perform the wall probe's separate +2 Z bias.
    pub fn address_routine(self) -> Vec<u32> {
        let bias = (-self.origin_tile()) as u16;
        let mut w = vec![
            sra(T0, A1, 7), sra(T1, A2, 7),
            addiu(T0, T0, bias), addiu(T1, T1, bias),
            sltiu(T2, T0, self.side), 0, 0,
            sltiu(T2, T1, self.side), 0, 0,
        ];
        match self.side {
            128 => w.push(sll(T1, T1, 7)),
            160 => w.extend([sll(T2, T1, 5), sll(T1, T1, 7), addu(T1, T1, T2)]),
            192 => w.extend([sll(T2, T1, 6), sll(T1, T1, 7), addu(T1, T1, T2)]),
            224 => w.extend([sll(T2, T1, 5), sll(T1, T1, 8), subu(T1, T1, T2)]),
            256 => w.push(sll(T1, T1, 8)),
            _ => unreachable!(),
        }
        let offset = self.objects_offset() as u32;
        w.extend([
            addu(T0, T0, T1), addu(V0, A0, T0), addiu(V0, V0, 0x4000),
            sll(T0, T0, 1), lui(T1, (offset >> 16) as u16), ori(T1, T1, offset as u16),
            addu(V1, A0, T0), jr(RA), addu(V1, V1, T1),
        ]);
        let invalid = w.len();
        w.extend([addu(V0, ZERO, ZERO), jr(RA), addu(V1, ZERO, ZERO)]);
        for at in [5, 8] { w[at] = beq(T2, ZERO, (invalid - at - 1) as i16); }
        w
    }

    /// Sparse resolver ABI: a0 = original MAP, a1/a2 = signed world X/Z,
    /// a3 = validated resident LKG1 buffer, or zero for an unextended scene.
    /// Returns the same address pair as `address_routine`; zero means absent.
    /// Original tiles always resolve through the unmodified MAP, including
    /// when a3 is zero. Uses only v0/v1 and t0..t7; no stack or allocation.
    /// Header/slot guards supplement (do not replace) loader length validation.
    pub fn sparse_address_routine(self) -> Vec<u32> {
        let mut a = Branches::default();
        a.emit(&[sra(T0, A1, 7), sra(T1, A2, 7), sltiu(T2, T0, 128), sltiu(T3, T1, 128), and(T2, T2, T3)]);
        a.branch(beq(T2, ZERO, 0), "extension");
        a.emit(&[sll(T1, T1, 7), addu(T0, T0, T1), addu(V0, A0, T0), addiu(V0, V0, 0x4000),
            sll(T0, T0, 1), addu(V1, A0, T0), ori(T1, ZERO, 0x8000), jr(RA), addu(V1, V1, T1)]);
        a.label("extension");
        a.branch(beq(A3, ZERO, 0), "absent");
        let bias = (-self.origin_tile()) as u16;
        a.emit(&[addiu(T0, T0, bias), addiu(T1, T1, bias), sltiu(T2, T0, self.side), sltiu(T3, T1, self.side), and(T2, T2, T3)]);
        a.branch(beq(T2, ZERO, 0), "absent");
        a.emit(&[lw(T2, A3, 0), lui(T3, (SPARSE_MAGIC >> 16) as u16), ori(T3, T3, SPARSE_MAGIC as u16)]);
        a.branch(bne(T2, T3, 0), "absent");
        a.emit(&[lw(T2, A3, 4), lui(T3, self.side), ori(T3, T3, 1)]);
        a.branch(bne(T2, T3, 0), "absent");
        a.emit(&[lw(T2, A3, 8), ori(T3, ZERO, self.origin_tile() as i16 as u16)]);
        a.branch(bne(T2, T3, 0), "absent");
        let width = self.side / 16;
        let directory = width * width;
        a.emit(&[lhu(T2, A3, 14), ori(T3, ZERO, directory)]);
        a.branch(bne(T2, T3, 0), "absent");
        a.emit(&[srl(T2, T1, 4), addu(T3, ZERO, ZERO)]);
        for bit in 0..5 {
            if width & (1 << bit) != 0 { a.emit(&[sll(T4, T2, bit), addu(T3, T3, T4)]); }
        }
        a.emit(&[srl(T2, T0, 4), addu(T3, T3, T2), sll(T3, T3, 1), addu(T3, A3, T3),
            lhu(T5, T3, 16), lhu(T6, A3, 12)]);
        a.branch(beq(T5, ZERO, 0), "absent");
        a.emit(&[sltiu(T7, T6, directory + 1)]);
        a.branch(beq(T7, ZERO, 0), "absent");
        a.emit(&[sltu(T7, T6, T5)]);
        a.branch(bne(T7, ZERO, 0), "absent");
        a.emit(&[addiu(T5, T5, (-1i16) as u16), sll(T4, T5, 9), sll(T5, T5, 8),
            addu(T4, T4, T5), addu(T4, A3, T4), addiu(T4, T4, 16 + directory * 2),
            andi(T0, T0, 15), andi(T1, T1, 15), sll(T1, T1, 4), addu(T0, T0, T1),
            addu(V0, T4, T0), sll(T0, T0, 1), addu(V1, T4, T0), jr(RA), addiu(V1, V1, 256)]);
        a.label("absent");
        a.emit(&[addu(V0, ZERO, ZERO), jr(RA), addu(V1, ZERO, ZERO)]);
        a.finish()
    }
}

#[derive(Default)]
struct Branches {
    words: Vec<u32>,
    labels: BTreeMap<&'static str, usize>,
    fixups: Vec<(usize, &'static str)>,
}
impl Branches {
    fn emit(&mut self, words: &[u32]) { self.words.extend_from_slice(words); }
    fn label(&mut self, label: &'static str) { assert!(self.labels.insert(label, self.words.len()).is_none()); }
    fn branch(&mut self, word: u32, label: &'static str) {
        self.fixups.push((self.words.len(), label)); self.emit(&[word, 0]);
    }
    fn finish(mut self) -> Vec<u32> {
        for (at, label) in self.fixups {
            let relative = self.labels[label] as isize - at as isize - 1;
            self.words[at] |= i16::try_from(relative).expect("short resolver branch") as u16 as u32;
        }
        self.words
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mips_sim::Cpu;
    const CODE: u32 = 0x80010000;
    const STOP: u32 = 0x80000080;
    const MAP: u32 = 0x800a0000;

    #[test]
    fn expanded_grid_native_windows_preserve_original_data_and_sidecars() {
        let retail: Vec<u8> = (0..RETAIL_RUNTIME_BYTES).map(|i| ((i * 17 + i / 127) % 256) as u8).collect();
        for side in [128, 160, 192, 224, 256] {
            let l = Layout::new(side).unwrap();
            let expanded = l.expand_runtime_window(&retail).unwrap();
            assert_eq!(expanded.len(), l.sector_count() * 0x800);
            assert_eq!(&expanded[..0x4000], &retail[..0x4000]);
            assert_eq!(&expanded[l.triggers_offset()..l.triggers_offset() + 0x4000], &retail[0x10000..]);
            for z in 0..128 { for x in 0..128 {
                let src = (z * 128 + x) as usize;
                let dst = l.index([x, z]).unwrap();
                assert_eq!(expanded[0x4000 + dst], retail[0x4000 + src]);
                assert_eq!(&expanded[l.objects_offset() + dst * 2..l.objects_offset() + dst * 2 + 2],
                    &retail[0x8000 + src * 2..0x8000 + src * 2 + 2]);
            }}
            if side == 128 { assert_eq!(expanded, retail); }
            else { assert_eq!(expanded[l.collision_offset()], 0xf7); assert_eq!(expanded[l.objects_offset()], 0); }
        }
        let l = Layout::new(256).unwrap();
        assert_eq!(l.runtime_bytes(), 0x38000);
        assert_eq!(l.runtime_bytes() - RETAIL_RUNTIME_BYTES, 144 * 1024);
        assert!(l.expand_runtime_window(&retail[..0x12000]).is_err());
        assert!(Layout::new(257).is_err());
    }

    fn query(c: &mut Cpu, world: [i32; 2]) -> [u32; 2] {
        query_with_extension(c, world, 0, 10)
    }

    fn query_with_extension(c: &mut Cpu, world: [i32; 2], extension: u32, last_scratch: usize) -> [u32; 2] {
        for (i, v) in c.r.iter_mut().enumerate() { *v = 0x12340000 + i as u32; }
        c.r[0] = 0; c.r[A0 as usize] = MAP;
        c.r[A1 as usize] = world[0] as u32; c.r[A2 as usize] = world[1] as u32;
        c.r[A3 as usize] = extension;
        c.r[RA as usize] = STOP; c.pc = CODE;
        let original = c.r;
        let start = c.steps;
        while c.pc != STOP {
            assert!(c.steps - start < 120, "address routine failed to return");
            let w = c.rd32(c.pc); c.exec(w);
        }
        for i in 0..32 {
            if i != 2 && i != 3 && !(8..=last_scratch).contains(&i) { assert_eq!(c.r[i], original[i], "register {i} clobbered"); }
        }
        [c.r[2], c.r[3]]
    }

    #[test]
    fn expanded_grid_sparse_codec_is_bounded_and_does_not_replace_native_cells() {
        let l = Layout::new(256).unwrap();
        let mut grid = SparseGrid::new(l);
        assert_eq!(grid.encoded_len(), 528);
        for (tile, cell) in [([-64, -64], Cell { collision: 0x12, object: 0x1234 }),
            ([191, 191], Cell { collision: 0x34, object: 0x5678 }),
            ([-1, 0], Cell { collision: 0x56, object: 0x9abc })] { grid.set(tile, cell).unwrap(); }
        assert_eq!(grid.page_count(), 3);
        assert_eq!(grid.encoded_len(), 528 + 3 * 768);
        let bytes = grid.encode();
        let decoded = SparseGrid::decode(&bytes).unwrap();
        assert_eq!(decoded.encode(), bytes);
        assert_eq!(decoded.get([-64, -64]), Some(Cell { collision: 0x12, object: 0x1234 }));
        assert_eq!(decoded.get([-63, -64]), Some(Cell { collision: 0xf7, object: 0 }));
        for tile in [[0, 0], [127, 127], [-65, 0], [192, 0], [0, -65], [0, 192]] {
            assert!(grid.set(tile, Cell { collision: 0, object: 0 }).is_err());
        }
        for len in [0, 15, 16, bytes.len() - 1] { assert!(SparseGrid::decode(&bytes[..len]).is_err()); }
        for (at, value) in [(0, 0), (4, 2), (6, 0xff), (8, 0), (10, 1), (12, 0xff), (14, 0xff), (16, 4)] {
            let mut bad = bytes.clone(); bad[at] = value;
            assert!(SparseGrid::decode(&bad).is_err(), "accepted corrupted offset {at}");
        }
    }

    #[test]
    fn expanded_grid_sparse_r3000_keeps_retail_and_resolves_external_pages() {
        const EXT: u32 = 0x80100000;
        for side in [128, 160, 192, 224, 256] {
            let l = Layout::new(side).unwrap();
            let mut grid = SparseGrid::new(l);
            let origin = l.origin_tile();
            // Allocate alternating external pages; leave the others absent.
            for z in origin..origin + side as i32 { for x in origin..origin + side as i32 {
                if (0..128).contains(&x) && (0..128).contains(&z) { continue; }
                if ((x - origin) / 16 + (z - origin) / 16) % 2 == 0 {
                    grid.set([x, z], Cell { collision: (x ^ z) as u8, object: (x * 257 + z) as u16 }).unwrap();
                }
            }}
            let bytes = grid.encode();
            let mut c = Cpu::new(); c.load_delays = true; c.strict_alignment = true;
            c.load_words(CODE, &l.sparse_address_routine()); c.load(EXT, &bytes);
            for z in origin..origin + side as i32 { for x in origin..origin + side as i32 {
                let result = query_with_extension(&mut c, [x * 128 + 63, z * 128 + 127], EXT, 15);
                if (0..128).contains(&x) && (0..128).contains(&z) {
                    let index = (z * 128 + x) as u32;
                    assert_eq!(result, [MAP + 0x4000 + index, MAP + 0x8000 + index * 2]);
                } else if let Some(cell) = grid.get([x, z]) {
                    assert!((EXT..EXT + bytes.len() as u32).contains(&result[0]));
                    assert!((EXT..EXT + bytes.len() as u32 - 1).contains(&result[1]));
                    assert_eq!(c.rd8(result[0]), cell.collision);
                    assert_eq!(c.rd16(result[1]), cell.object);
                } else { assert_eq!(result, [0, 0], "absent page at {x},{z} wrapped"); }
            }}
            for world in [[origin * 128 - 1, 0], [0, (origin + side as i32) * 128], [i32::MIN, 0], [0, i32::MAX]] {
                assert_eq!(query_with_extension(&mut c, world, EXT, 15), [0, 0]);
            }
            assert_eq!(query_with_extension(&mut c, [-1, 0], 0, 15), [0, 0]);
            assert_eq!(query_with_extension(&mut c, [64, 64], 0, 15), [MAP + 0x4000, MAP + 0x8000]);
            c.wr32(EXT, 0);
            assert_eq!(query_with_extension(&mut c, [-1, 0], EXT, 15), [0, 0]);
        }
    }

    #[test]
    fn expanded_grid_r3000_every_tile_and_edges_match_signed_host_addressing() {
        for side in [128, 160, 192, 224, 256] {
            let l = Layout::new(side).unwrap();
            let mut c = Cpu::new(); c.load_delays = true; c.strict_alignment = true;
            c.load_words(CODE, &l.address_routine());
            let origin = l.origin_tile();
            for z in origin..origin + side as i32 { for x in origin..origin + side as i32 {
                // Opposite tile corners catch floor-vs-truncation at negative coordinates.
                for delta in [0, 127] {
                    let world = [x * 128 + delta, z * 128 + delta];
                    let index = l.world_index(world).unwrap() as u32;
                    assert_eq!(query(&mut c, world), [MAP + 0x4000 + index, MAP + l.objects_offset() as u32 + index * 2]);
                }
            }}
            let low = origin * 128; let end = (origin + side as i32) * 128;
            for world in [[low - 1, low], [low, low - 1], [end, low], [low, end],
                [i32::MIN, 0], [0, i32::MIN], [i32::MAX, 0], [0, i32::MAX]] {
                assert_eq!(l.world_index(world), None);
                assert_eq!(query(&mut c, world), [0, 0], "outside coordinate {world:?} wrapped");
            }
        }
    }
}
