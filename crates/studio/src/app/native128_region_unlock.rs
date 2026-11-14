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
const BODY: usize = 0x488;
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
    table_start: usize,
    append_offset: usize,
    appended: Vec<u8>,
    count: i16,
    summary: Summary,
}

#[derive(Debug, Clone, Copy)]
struct RegionLayout {
    body: usize,
    count: usize,
}

// Read the actual region-table metadata from this MAP.
// No retail-specific offset or fixed record count.
fn read_layout(map: &[u8]) -> Result<RegionLayout> {
    ensure!(
        map.len() >= MAP_LENGTH,
        "Native MAP shorter than expected footprint"
    );

    let raw_body = i16::from_le_bytes([
        map[BLOCK + 0x0e],
        map[BLOCK + 0x0f],
    ]);

    let raw_count = i16::from_le_bytes([
        map[BLOCK + 0x10],
        map[BLOCK + 0x11],
    ]);

    ensure!(
        raw_body >= 0x20 && raw_body < 0x800,
        "Invalid native region offset: {raw_body:#x}"
    );

    ensure!(
        raw_count > 0 && raw_count < 256,
        "Invalid native region count: {raw_count}"
    );

    let body = raw_body as usize;
    let count = raw_count as usize;

    let end = BLOCK + body + count * STRIDE;

    ensure!(
        end <= LAST_REGION_BYTE && end <= map.len(),
        "Region table exceeds audited native region storage: body={body:#x}, count={count}"
    );

    Ok(RegionLayout { body, count })
}

fn read_count(map: &[u8]) -> Result<usize> {
    Ok(read_layout(map)?.count)
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

    // LCK_R625B_REGION_RELOCATION
    // An earlier native export stage may relocate or extend
    // the region table. Verify the retail prefix through
    // the already-established adaptive R613B audit.
    let original_layout = read_layout(original)?;
    let exported_layout = read_layout(exported)?;

    let (_retail_body, body, _retail_count, count) =
        r613b_verified_region_layouts(original, exported)?;

    let old_end = BLOCK + body + count * STRIDE;

    // Tiles already inside retail type-0 or type-1
    // regions are excluded from newly added regions.

    let mut protected = [false; CELL_COUNT];

    for z in 0..128usize {
        for x in 0..128usize {
            for i in 0..count {
                let o = BLOCK + body + i * STRIDE;
                let rec = &exported[o..o + STRIDE];

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

    let new_end = BLOCK + body + total * STRIDE;

    ensure!(
        new_end <= LAST_REGION_BYTE,
        "New region records would overlap another native table"
    );

    ensure!(
        exported[old_end..new_end]
            .iter()
            .all(|b| *b == 0)
        &&
        (
            original_layout.body != exported_layout.body
            || original_layout.count != exported_layout.count
            || original[old_end..new_end]
                .iter()
                .all(|b| *b == 0)
        ),
        "R632 REGION CONFLICT - EXPORT BLOCKED, NO OVERWRITE: retail_body=0x{:X}, staged_body=0x{:X}, retail_regions={}, staged_regions={}, adding_regions={}, append_MAP_offsets=0x{:X}..0x{:X}, retail_nonzero={}, staged_nonzero={}, first_retail_bytes={:?}, first_staged_bytes={:?}. CAUSE: an earlier IMG export stage occupies bytes required for additional native region records. SUGGESTED FIX: identify the stage that owns the occupied offsets and relocate the region table into verified free storage while preserving both operations. For warp-only exports, avoid including unnecessary geometry edits that request full-region expansion.",
        original_layout.body,
        body,
        original_layout.count,
        count,
        rects.len(),
        old_end,
        new_end,
        original[old_end..new_end]
            .iter()
            .filter(|&&b|b!=0)
            .count(),
        exported[old_end..new_end]
            .iter()
            .filter(|&&b|b!=0)
            .count(),
        original[old_end..new_end]
            .iter()
            .enumerate()
            .filter(|(_,b)|**b!=0)
            .take(12)
            .map(|(i,&b)|(old_end+i,b))
            .collect::<Vec<_>>(),
        exported[old_end..new_end]
            .iter()
            .enumerate()
            .filter(|(_,b)|**b!=0)
            .take(12)
            .map(|(i,&b)|(old_end+i,b))
            .collect::<Vec<_>>()
    );

    let mut appended =
        Vec::with_capacity(rects.len() * STRIDE);

    for [x0, z0, x1, z1] in &rects {
        appended.extend_from_slice(&[
            *x0, *z1, *x1, *z0,
            0, 0, 0, 0
        ]);
    }

    Ok(Some(Plan {
        table_start: BLOCK + body,
        append_offset: old_end,
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
        p.append_offset as u64,
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
            p.append_offset..
            p.append_offset + p.appended.len()
        ] == p.appended[..],
        "Native region payload readback failed"
    );

    ensure!(
        checked[p.table_start..p.append_offset]
            == staged_map[p.table_start..p.append_offset],
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
        assert_eq!(&p.appended[..5], &[64, 64, 96, 32, 0]);
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
            for z in r[1].min(r[3])..r[1].max(r[3]) {
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
            for z in r[1].min(r[3])..r[1].max(r[3]) {
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
// R613_NATIVE128_COVERAGE
// Validate native MAP region coverage.
// This does not replace runtime render/physics hooks.

#[derive(Clone, Debug, PartialEq, Eq)]
struct R613Coverage {
    final_regions:usize,
    original_covered:usize,
    added_covered:usize,
    uncovered:usize,
    changed_outside:usize,
    changed_without_region:usize,
    walk_flagged_outside:usize,
    walk_flagged_without_region:usize,
}

fn r613b_verified_region_layouts(
    original: &[u8],
    exported: &[u8],
) -> Result<(usize, usize, usize, usize)> {

    let retail = read_layout(original)?;
    let staged = read_layout(exported)?;

    let original_body = retail.body;
    let exported_body = staged.body;
    let old_count = retail.count;
    let count = staged.count;

    ensure!(
        count >= old_count,
        "R613B: exported region count decreased"
    );

    let original_start = BLOCK + original_body;
    let original_end = original_start + old_count * STRIDE;

    let exported_start = BLOCK + exported_body;
    let preserved_end = exported_start + old_count * STRIDE;

    let final_end = exported_start + count * STRIDE;

    ensure!(
        original_end <= LAST_REGION_BYTE &&
        final_end <= LAST_REGION_BYTE,
        "R613B: native region storage boundary exceeded"
    );

    ensure!(
        original[original_start..original_end]
            == exported[exported_start..preserved_end],
        "R613B: original region records differ across layouts (retail={original_body:#x}, exported={exported_body:#x})"
    );

    Ok((
        original_body,
        exported_body,
        old_count,
        count,
    ))
}
fn r613_audit(
    original:&[u8],
    exported:&[u8],
    require_full:bool
)->Result<R613Coverage> {
    let (
        original_body,
        exported_body,
        old_count,
        count,
    ) = r613b_verified_region_layouts(original, exported)?;

    let mut result=R613Coverage{
        final_regions:count,
        original_covered:0,
        added_covered:0,
        uncovered:0,
        changed_outside:0,
        changed_without_region:0,
        walk_flagged_outside:0,
        walk_flagged_without_region:0,
    };

    for z in 0..128usize {
        for x in 0..128usize {
            let original_region=(0..old_count).any(|n| {
                let p=BLOCK+original_body+n*STRIDE;
                original[p+4]<=1 &&
                    region_covers(
                        &original[p..p+STRIDE],x,z
                    )
            });

            let final_region=(0..count).any(|n| {
                let p=BLOCK+exported_body+n*STRIDE;
                exported[p+4]<=1 &&
                    region_covers(
                        &exported[p..p+STRIDE],x,z
                    )
            });

            if original_region {
                result.original_covered+=1;
                continue;
            }

            if final_region {
                result.added_covered+=1;
            }else{
                result.uncovered+=1;
            }

            let at=0x8000+(z*128+x)*2;
            let co=0x4000+z*128+x;

            let changed=
                original[at..at+2]!=exported[at..at+2] ||
                original[co]!=exported[co];

            if changed {
                result.changed_outside+=1;

                if !final_region {
                    result.changed_without_region+=1;
                }
            }

            let word=u16::from_le_bytes([
                exported[at],exported[at+1]
            ]);

            if word & 0x0800 != 0 {
                result.walk_flagged_outside+=1;

                if !final_region {
                    result.walk_flagged_without_region+=1;
                }
            }
        }
    }

    ensure!(
        result.original_covered+
        result.added_covered+
        result.uncovered==CELL_COUNT,
        "R613: native tile-accounting mismatch"
    );

    if require_full {
        ensure!(
            result.uncovered==0,
            "R613: full 128 requested, but {} native tiles lack type-0/1 region coverage",
            result.uncovered
        );

        ensure!(
            result.changed_without_region==0,
            "R613: {} authored native tiles lack region coverage",
            result.changed_without_region
        );
    }

    Ok(result)
}

pub(super) fn r613_coverage_report(
    original:&[u8],
    exported:&[u8],
    full:bool
)->Result<String> {
    let a=r613_audit(original,exported,full)?;

    Ok(format!(concat!(
        "R6.1.3 NATIVE MAP COVERAGE AUDIT\n",
        "Region records: {}\n",
        "Retail-covered tiles: {}\n",
        "Newly covered native tiles: {}\n",
        "Uncovered native tiles: {}\n",
        "Changed native tiles outside retail regions: {}\n",
        "Changed native tiles without active region: {}\n",
        "Walk-flagged native tiles outside retail regions: {}\n",
        "Walk-flagged native tiles without active region: {}\n",
        "Full 128 coverage requested: {}\n",
        "Coverage verifies MAP data only. ",
        "Retail renderer, walk physics, scripts and camera ",
        "are NOT gameplay verified.\n"
    ),
        a.final_regions,
        a.original_covered,
        a.added_covered,
        a.uncovered,
        a.changed_outside,
        a.changed_without_region,
        a.walk_flagged_outside,
        a.walk_flagged_without_region,
        full
    ))
}

#[cfg(test)]
mod r613_coverage_tests {
    use super::*;

    fn base()->Vec<u8> {
        let mut out=vec![0u8;MAP_LENGTH];

        out[BLOCK+0x0e..BLOCK+0x10]
            .copy_from_slice(&(BODY as i16).to_le_bytes());

        out[BLOCK+0x10..BLOCK+0x12]
            .copy_from_slice(&(ORIGINAL_REGIONS as i16).to_le_bytes());

        let i=BLOCK+BODY+5*STRIDE;

        out[i..i+8].copy_from_slice(
            &[9,60,45,0,0,0,0,0]
        );

        out
    }

    fn with_plan(mut out:Vec<u8>,p:&Plan)->Vec<u8> {
        out[BLOCK+0x10..BLOCK+0x12]
            .copy_from_slice(&p.count.to_le_bytes());

        out[FIRST_UNCLAIMED..FIRST_UNCLAIMED+p.appended.len()]
            .copy_from_slice(&p.appended);

        out
    }

    #[test]
    fn full_mode_has_coverage_for_all_16384_cells() {
        let old=base();
        let p=plan(&old,&old,true).unwrap().unwrap();
        let next=with_plan(old.clone(),&p);

        let a=r613_audit(&old,&next,true).unwrap();

        assert_eq!(a.uncovered,0);

        assert_eq!(
            a.original_covered+a.added_covered,
            CELL_COUNT
        );
    }

    #[test]
    fn changed_floor_without_region_is_detected() {
        let old=base();
        let mut next=old.clone();

        next[0x4000+33*128+64]=0x71;

        assert!(r613_audit(&old,&next,true).is_err());

        let a=r613_audit(&old,&next,false).unwrap();

        assert_eq!(a.changed_without_region,1);
    }

    #[test]
    fn authored_region_activates_changed_native_tile() {
        let old=base();
        let mut next=old.clone();

        next[0x4000+33*128+64]=0x71;

        let p=plan(&old,&next,false).unwrap().unwrap();
        let fixed=with_plan(next,&p);

        assert_eq!(
            r613_audit(&old,&fixed,false)
                .unwrap()
                .changed_without_region,
            0
        );
    }

    #[test]
    fn retail_region_tampering_is_rejected() {
        let old=base();
        let mut changed=old.clone();

        changed[BLOCK+BODY+5*STRIDE]=10;

        assert!(
            r613_audit(&old,&changed,false).is_err()
        );
    }
}




#[cfg(test)]
#[test]
fn r614_dynamic_region_layouts() {
    // Different valid tables must all be accepted.
    for &(body, count) in &[
        (0x484usize, 14usize),
        (0x488usize, 14usize),
        (0x580usize, 16usize),
    ] {
        let mut map = vec![0u8; MAP_LENGTH];

        map[BLOCK + 0x0e..BLOCK + 0x10]
            .copy_from_slice(&(body as i16).to_le_bytes());

        map[BLOCK + 0x10..BLOCK + 0x12]
            .copy_from_slice(&(count as i16).to_le_bytes());

        let main = BLOCK + body + 5 * STRIDE;

        map[main..main + 5]
            .copy_from_slice(&[9, 60, 45, 0, 0]);

        let layout = read_layout(&map).unwrap();

        assert_eq!(layout.body, body);
        assert_eq!(layout.count, count);

        let plan = plan(&map, &map, true)
            .unwrap()
            .unwrap();

        assert_eq!(plan.table_start, BLOCK + body);
        assert_eq!(
            plan.append_offset,
            BLOCK + body + count * STRIDE
        );

        assert!(plan.count as usize > count);
        assert!(!plan.appended.is_empty());
    }
}

#[cfg(test)]
#[test]
fn r614_invalid_table_bounds_rejected() {
    let mut map = vec![0u8; MAP_LENGTH];

    map[BLOCK + 0x0e..BLOCK + 0x10]
        .copy_from_slice(&0x7f8_i16.to_le_bytes());

    map[BLOCK + 0x10..BLOCK + 0x12]
        .copy_from_slice(&14_i16.to_le_bytes());

    assert!(read_layout(&map).is_err());
}


#[cfg(test)]
mod r613b_adaptive_audit_tests {
    use super::*;

    fn region_map(body: usize, count: usize) -> Vec<u8> {
        let mut map = vec![0u8; MAP_LENGTH];

        map[BLOCK + 0x0e..BLOCK + 0x10]
            .copy_from_slice(&(body as i16).to_le_bytes());

        map[BLOCK + 0x10..BLOCK + 0x12]
            .copy_from_slice(&(count as i16).to_le_bytes());

        let exterior = BLOCK + body + 5 * STRIDE;

        map[exterior..exterior + 5]
            .copy_from_slice(&[9, 60, 45, 0, 0]);

        if count > 14 {
            let extra = BLOCK + body + 14 * STRIDE;

            map[extra..extra + STRIDE]
                .copy_from_slice(&[0, 0, 32, 32, 0, 0, 0, 0]);
        }

        map
    }

    #[test]
    fn accepts_original_and_relocated_tables() {
        for &(retail_body, staged_body) in &[
            (0x484usize, 0x484usize),
            (0x488usize, 0x488usize),
            (0x484usize, 0x488usize),
            (0x488usize, 0x484usize),
        ] {
            let original = region_map(retail_body, 14);
            let staged = region_map(staged_body, 15);

            let actual = r613b_verified_region_layouts(
                &original,
                &staged,
            ).unwrap();

            assert_eq!(
                actual,
                (retail_body, staged_body, 14, 15)
            );
        }
    }

    #[test]
    fn rejects_corrupted_or_lost_original_regions() {
        let original = region_map(0x484, 14);
        let mut staged = region_map(0x488, 15);

        // Changing a retail region must remain an error.
        staged[BLOCK + 0x488 + 5 * STRIDE] ^= 1;

        assert!(
            r613b_verified_region_layouts(
                &original, &staged
            ).is_err()
        );

        // Losing original records must also remain an error.
        let smaller = region_map(0x488, 13);

        assert!(
            r613b_verified_region_layouts(
                &original, &smaller
            ).is_err()
        );
    }
}


// LCK_R625B_REGION_RELOCATION
#[cfg(test)]
mod lck_r625b_region_tests {
    use super::*;

    fn test_map()->Vec<u8> {
        let mut map=vec![0u8;MAP_LENGTH];

        map[BLOCK+0x0e..BLOCK+0x10]
            .copy_from_slice(&0x484i16.to_le_bytes());

        map[BLOCK+0x10..BLOCK+0x12]
            .copy_from_slice(&14i16.to_le_bytes());

        let row=BLOCK+0x484+5*STRIDE;

        map[row..row+8]
            .copy_from_slice(&[9,60,45,0,0,0,0,0]);

        map
    }

    #[test]
    fn accepts_relocated_extended_region_table(){
        let original=test_map();
        let mut staged=original.clone();

        let old_start=BLOCK+0x484;
        let new_start=BLOCK+0x488;
        let length=14*STRIDE;

        staged[new_start..new_start+length]
            .copy_from_slice(
                &original[old_start..old_start+length]
            );

        staged[BLOCK+0x0e..BLOCK+0x10]
            .copy_from_slice(&0x488i16.to_le_bytes());

        staged[BLOCK+0x10..BLOCK+0x12]
            .copy_from_slice(&15i16.to_le_bytes());

        staged[new_start+length..new_start+length+8]
            .copy_from_slice(&[0,0,16,16,0,0,0,0]);

        staged[0x4000+88*128+88]=1;

        let p=plan(&original,&staged,false)
            .unwrap()
            .expect("New region required");

        assert_eq!(p.table_start,new_start);
        assert_eq!(
            p.append_offset,
            new_start+15*STRIDE
        );
        assert!(p.count>15);
    }

    #[test]
    fn rejects_corrupted_retail_region(){
        let original=test_map();
        let mut staged=original.clone();

        let old_start=BLOCK+0x484;
        let new_start=BLOCK+0x488;
        let length=14*STRIDE;

        staged[new_start..new_start+length]
            .copy_from_slice(
                &original[old_start..old_start+length]
            );

        staged[BLOCK+0x0e..BLOCK+0x10]
            .copy_from_slice(&0x488i16.to_le_bytes());

        staged[new_start+5*STRIDE]^=1;

        assert!(plan(&original,&staged,false).is_err());
    }
}
