// LCK_R612A_NATIVE_128_REGIONS
// Experimental native 128x128 MAP region activation.
//
// This uses existing retail region records.
// No new MIPS runtime instructions are installed.

use anyhow::{Context, Result, ensure};
use legaia_disc_patch::disc::DiscPatcher;
use std::{
    collections::BTreeMap,
    fs,
    path::Path
};

const MAP_LENGTH: usize = 0x12000;
const BLOCK: usize = 0x10000;
const BODY: usize = 0x484;
const ORIGINAL_REGIONS: usize = 14;
const STRIDE: usize = 8;
const CELL_COUNT: usize = 128 * 128;
const CHUNK: usize = 32;

const FIRST_UNCLAIMED: usize =
    BLOCK + BODY + ORIGINAL_REGIONS * STRIDE;

const LAST_REGION_BYTE: usize =
    BLOCK + 0x800;

#[derive(Debug, Clone)]
pub(super) struct Summary {
    pub mode: &'static str,
    pub changed_native_tiles: usize,
    pub selected_chunks: usize,
    pub new_regions: usize,
    pub occupied_original_regions: usize,
    pub total_regions: usize,
    pub original_region_bytes_preserved: bool,
}

impl Summary {
    pub fn message(&self) -> String {
        format!(
            "R6.1.2A native region unlock: mode={} changed_tiles={} chunks={} added_regions={} total_regions={}",
            self.mode,
            self.changed_native_tiles,
            self.selected_chunks,
            self.new_regions,
            self.total_regions
        )
    }

    pub fn report(&self) -> String {
        format!(
            concat!(
                "LCK R6.1.2A NATIVE REGION REPORT\n",
                "Mode: {}\n",
                "Changed tiles: {}\n",
                "Selected chunks: {}\n",
                "New region records: {}\n",
                "Original region records: {}\n",
                "Total region records: {}\n",
                "Original records preserved: {}\n",
                "Added region type: 0\n",
                "Original interior records preserved.\n",
                "Native rendering and collision NOT gameplay verified.\n",
                "Region-dependent scripts and camera behavior may change.\n"
            ),
            self.mode,
            self.changed_native_tiles,
            self.selected_chunks,
            self.new_regions,
            self.occupied_original_regions,
            self.total_regions,
            self.original_region_bytes_preserved
        )
    }
}

struct Plan {
    appended: Vec<u8>,
    count: i16,
    summary: Summary,
}

fn read_count(map: &[u8]) -> Result<usize> {
    ensure!(
        map.len() >= MAP_LENGTH,
        "Native MAP shorter than 0x12000 bytes"
    );

    let body = i16::from_le_bytes([
        map[BLOCK + 0x0e],
        map[BLOCK + 0x0f]
    ]);

    let count = i16::from_le_bytes([
        map[BLOCK + 0x10],
        map[BLOCK + 0x11]
    ]);

    ensure!(
        body == BODY as i16 &&
        count == ORIGINAL_REGIONS as i16,
        "Unexpected town01 region table: body={body:#x}, count={count}"
    );

    let main = BLOCK + BODY + 5 * STRIDE;

    ensure!(
        &map[main..main + 5] == &[9, 60, 45, 0, 0],
        "Rim Elm exterior record does not match audited data"
    );

    Ok(count as usize)
}

fn region_covers(rec: &[u8], x: usize, z: usize) -> bool {
    let x0 = i32::from(rec[0]);
    let z0 = i32::from(rec[1]);
    let x1 = i32::from(rec[2]);
    let z1 = i32::from(rec[3]);

    let (xmin, mut xmax) = (x0.min(x1), x0.max(x1));
    let (mut zmin, zmax) = (z0.min(z1), z0.max(z1));

    if xmin == xmax {
        xmax += 2;
    }

    if zmin == zmax {
        zmin -= 2;
    }

    xmin <= x as i32 &&
    (x as i32) < xmax &&
    zmin <= z as i32 &&
    (z as i32) < zmax
}

fn plan(
    original: &[u8],
    exported: &[u8],
    full: bool
) -> Result<Option<Plan>> {

    let count = read_count(original)?;

    ensure!(
        read_count(exported)? == count,
        "Exported region header was already modified"
    );

    let old_end = BLOCK + BODY + count * STRIDE;

    ensure!(
        original[BLOCK + BODY..old_end]
            == exported[BLOCK + BODY..old_end],
        "Original region records were already changed"
    );

    // Tiles already inside retail type-0 or type-1
    // regions are excluded from newly added regions.

    let mut protected = [false; CELL_COUNT];

    for z in 0..128usize {
        for x in 0..128usize {
            for i in 0..count {
                let o = BLOCK + BODY + i * STRIDE;
                let rec = &original[o..o + STRIDE];

                if rec[4] <= 1 && region_covers(rec, x, z) {
                    protected[z * 128 + x] = true;
                    break;
                }
            }
        }
    }

    let mut selected = [[false; 4]; 4];
    let mut changed = 0usize;

    // Detect actual serialized MAP changes,
    // not just editor-only placed geometry.

    for z in 0..128usize {
        for x in 0..128usize {
            let i = z * 128 + x;
            let w = 0x8000 + i * 2;
            let c = 0x4000 + i;

            if original[w..w + 2] != exported[w..w + 2]
                || original[c] != exported[c]
            {
                changed += 1;

                if !protected[i] {
                    selected[z / CHUNK][x / CHUNK] = true;
                }
            }
        }
    }

    if full {
        selected = [[true; 4]; 4];
    }

    let chunk_count = selected
        .iter()
        .flatten()
        .filter(|v| **v)
        .count();

    if chunk_count == 0 {
        return Ok(None);
    }

    // Generate non-overlapping activation rectangles.
    // Each is contained within a 32x32 tile chunk.

    let mut rects = Vec::<[u8; 4]>::new();

    for bz in (0..128usize).step_by(CHUNK) {
        for bx in (0..128usize).step_by(CHUNK) {

            if !selected[bz / CHUNK][bx / CHUNK] {
                continue;
            }

            let mut active =
                BTreeMap::<(u8, u8), usize>::new();

            for z in bz..bz + CHUNK {
                let mut next =
                    BTreeMap::<(u8, u8), usize>::new();

                let mut x = bx;

                while x < bx + CHUNK {

                    if protected[z * 128 + x] {
                        x += 1;
                        continue;
                    }

                    let x0 = x;

                    while x < bx + CHUNK
                        && !protected[z * 128 + x]
                    {
                        x += 1;
                    }

                    let key = (x0 as u8, x as u8);

                    if let Some(&idx) = active.get(&key) {
                        rects[idx][3] = (z + 1) as u8;
                        next.insert(key, idx);
                    } else {
                        let idx = rects.len();

                        rects.push([
                            x0 as u8,
                            z as u8,
                            x as u8,
                            (z + 1) as u8
                        ]);

                        next.insert(key, idx);
                    }
                }

                active = next;
            }
        }
    }

    if rects.is_empty() {
        return Ok(None);
    }

    let total = count + rects.len();

    ensure!(
        total < 256,
        "Native region count exceeds byte-sized runtime budget"
    );

    let new_end = BLOCK + BODY + total * STRIDE;

    ensure!(
        new_end <= LAST_REGION_BYTE,
        "New region records would overlap another native table"
    );

    ensure!(
        original[old_end..new_end]
            .iter()
            .all(|b| *b == 0)
        &&
        exported[old_end..new_end]
            .iter()
            .all(|b| *b == 0),
        "MAP region padding is occupied"
    );

    let mut appended =
        Vec::with_capacity(rects.len() * STRIDE);

    for [x0, z0, x1, z1] in &rects {
        appended.extend_from_slice(&[
            *x0, *z0, *x1, *z1,
            0, 0, 0, 0
        ]);
    }

    Ok(Some(Plan {
        appended,
        count: total as i16,
        summary: Summary {
            mode: if full {
                "full-native-128"
            } else {
                "authored-tile-chunks"
            },
            changed_native_tiles: changed,
            selected_chunks: chunk_count,
            new_regions: rects.len(),
            occupied_original_regions: count,
            total_regions: total,
            original_region_bytes_preserved: true,
        },
    }))
}

pub(super) fn patch_image(
    staged_path: &Path,
    entry: usize,
    retail_map: &[u8],
    full: bool,
) -> Result<Option<Summary>> {

    let mut disc = DiscPatcher::open(
        fs::read(staged_path)
            .with_context(|| format!(
                "Read experimental IMG {}",
                staged_path.display()
            ))?
    )?;

    let staged_map = disc.read_entry_footprint(entry)?;

    let Some(p) = plan(retail_map, &staged_map, full)?
    else {
        return Ok(None);
    };

    disc.patch_prot_entry(
        entry,
        FIRST_UNCLAIMED as u64,
        &p.appended
    )?;

    disc.patch_prot_entry(
        entry,
        (BLOCK + 0x10) as u64,
        &p.count.to_le_bytes()
    )?;

    fs::write(
        staged_path,
        disc.into_image()
    ).context("Write experimental staged IMG")?;

    let checked = DiscPatcher::open(
        fs::read(staged_path)?
    )?.read_entry_footprint(entry)?;

    ensure!(
        i16::from_le_bytes([
            checked[BLOCK + 0x10],
            checked[BLOCK + 0x11]
        ]) == p.count,
        "Region header readback failed"
    );

    ensure!(
        checked[
            FIRST_UNCLAIMED..
            FIRST_UNCLAIMED + p.appended.len()
        ] == p.appended[..],
        "Native region payload readback failed"
    );

    ensure!(
        checked[BLOCK + BODY..FIRST_UNCLAIMED]
            == staged_map[BLOCK + BODY..FIRST_UNCLAIMED],
        "Original region records changed"
    );

    Ok(Some(p.summary))
}

#[cfg(test)]
mod r612_native128_tests {
    use super::*;

    fn fixture() -> (Vec<u8>, Vec<u8>) {
        let mut original = vec![0u8; MAP_LENGTH];

        original[
            BLOCK + 0x0e..BLOCK + 0x10
        ].copy_from_slice(
            &(BODY as i16).to_le_bytes()
        );

        original[
            BLOCK + 0x10..BLOCK + 0x12
        ].copy_from_slice(
            &(ORIGINAL_REGIONS as i16).to_le_bytes()
        );

        let rec = BLOCK + BODY + 5 * STRIDE;

        original[rec..rec + 8].copy_from_slice(
            &[9, 60, 45, 0, 0, 0, 0, 0]
        );

        (original.clone(), original)
    }

    #[test]
    fn external_floor_adds_region() {
        let (original, mut staged) = fixture();

        staged[0x4000 + 33 * 128 + 64] = 0x71;

        let p = plan(&original, &staged, false)
            .unwrap()
            .unwrap();

        assert_eq!(p.summary.changed_native_tiles, 1);
        assert_eq!(p.summary.selected_chunks, 1);
        assert_eq!(p.appended.len(), STRIDE);
        assert_eq!(&p.appended[..5], &[64, 32, 96, 64, 0]);
        assert_eq!(p.count, 15);
    }

    #[test]
    fn unchanged_map_adds_nothing() {
        let (original, staged) = fixture();

        assert!(
            plan(&original, &staged, false)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn interior_region_is_protected() {
        let (mut original, _) = fixture();

        let at = BLOCK + BODY + 7 * STRIDE;

        original[at..at + 8].copy_from_slice(
            &[60, 35, 72, 45, 0, 0, 0, 0]
        );

        let mut staged = original.clone();

        staged[0x4000 + 36 * 128 + 67] ^= 1;

        assert!(
            plan(&original, &staged, false)
                .unwrap()
                .is_none()
        );

        staged[0x4000 + 33 * 128 + 64] ^= 1;

        let p = plan(&original, &staged, false)
            .unwrap()
            .unwrap();

        for r in p.appended.chunks_exact(8) {
            for z in r[1]..r[3] {
                for x in r[0]..r[2] {
                    assert!(
                        !(60..72).contains(&x)
                        || !(35..45).contains(&z)
                    );
                }
            }
        }
    }

    #[test]
    fn full_mode_covers_unclaimed_tiles() {
        let (original, staged) = fixture();

        let p = plan(&original, &staged, true)
            .unwrap()
            .unwrap();

        let mut covered = [false; CELL_COUNT];

        for r in p.appended.chunks_exact(8) {
            for z in r[1]..r[3] {
                for x in r[0]..r[2] {
                    let i = z as usize * 128 + x as usize;
                    assert!(!covered[i]);
                    covered[i] = true;
                }
            }
        }

        for z in 0..128 {
            for x in 0..128 {
                let rec = &original[
                    BLOCK + BODY + 5 * STRIDE..
                    BLOCK + BODY + 6 * STRIDE
                ];

                assert_eq!(
                    covered[z * 128 + x],
                    !region_covers(rec, x, z)
                );
            }
        }
    }

    #[test]
    fn occupied_padding_is_rejected() {
        let (original, mut staged) = fixture();

        staged[0x4000 + 10 * 128 + 64] = 1;
        staged[FIRST_UNCLAIMED] = 0x77;

        assert!(
            plan(&original, &staged, false).is_err()
        );
    }
}