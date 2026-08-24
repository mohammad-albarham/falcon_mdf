//! The block map checked against files other tools wrote.
//!
//! `Mf4File::block_map` reads a file as a file rather than as a measurement:
//! the identification block, the header, and every block reachable from them,
//! in address order, each with its links and a line describing its fields.
//! That is what a viewer shows when a file opens and what a debugger reads
//! when one does not, so the map has to be trustworthy in ways that decoding
//! channels never reveals — a walk that lists blocks out of order, double-
//! books a byte, or drops a dangling link without a word would mislead
//! exactly at the moment somebody is relying on it most.
//!
//! This suite pins those ways against the reference corpus — Vector, dSPACE,
//! ETAS and other writers' files — rather than against fixtures built to
//! agree with this walker, which is the same distinction `reference.rs` and
//! `bus_frames.rs` make. Per file, the map must hold:
//!
//! - the blocks are sorted strictly ascending by address and no two overlap;
//! - the first block is the identification block at address 0, 64 bytes long,
//!   and a block exists at address 64, where the header block always sits;
//! - the covered bytes plus the gaps account for the whole file, and no gap
//!   reaches into a block;
//! - every nonzero link either resolves to a listed block or is mentioned in
//!   the warnings — never dropped without a word;
//! - the link labels match the links, and `type_counts` matches the blocks;
//! - `block_at` finds a block exactly at the addresses one starts at.
//!
//! Scanning the same file twice must also give the same map: the walk is what
//! a fix is diffed against, so a map that reshuffles itself between runs is
//! worse than useless.
//!
//! The corpus is not checked in. These tests skip when it is absent, as
//! `reference.rs` does.

use falcon_mdf::{BlockMap, Mf4File};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn reference_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test_data")
        .join("reference")
}

/// Every measurement file under the reference directory, whatever the case of
/// its extension. Walked from the directory, as `reference.rs` does, so a file
/// the fetch script adds is exercised the moment it lands.
fn reference_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("mf4"))
            {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&reference_dir(), &mut files);
    files.sort();
    files
}

fn skip_if_empty(files: &[PathBuf]) -> bool {
    if files.is_empty() {
        eprintln!("SKIP: no reference corpus under test_data/reference");
        return true;
    }
    false
}

#[test]
fn the_block_map_of_every_reference_file_holds_the_invariants() {
    let files = reference_files();
    if skip_if_empty(&files) {
        return;
    }

    for path in &files {
        let file = Mf4File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let map = file.block_map();

        blocks_are_sorted_and_disjoint(path, &map);
        the_id_block_sits_where_the_format_puts_it(path, &map);
        blocks_and_gaps_account_for_every_byte(path, &map);
        every_link_resolves_or_is_warned_about(path, &map);
        labels_and_counts_describe_the_blocks_they_belong_to(path, &map);
        block_at_finds_a_block_exactly_at_its_start(path, &map);
    }

    eprintln!(
        "block map invariants held across {} reference files",
        files.len()
    );
}

/// The blocks list a file's layout, so they come out in the file's order and
/// no two may claim the same byte.
fn blocks_are_sorted_and_disjoint(path: &Path, map: &BlockMap) {
    for pair in map.blocks.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert!(
            a.address < b.address,
            "{}: block addresses not strictly ascending: {:#x} then {:#x}",
            path.display(),
            a.address,
            b.address
        );
        assert!(
            a.address + a.length <= b.address,
            "{}: the block at {:#x} runs {} bytes, into the block at {:#x}",
            path.display(),
            a.address,
            a.length,
            b.address
        );
    }
}

/// The format fixes the identification block at offset 0 and the header block
/// immediately after it, whatever the file contains. An unfinalized writer
/// spells the identification block `UnFinMF` instead of `MDF`, so both
/// identifiers count.
fn the_id_block_sits_where_the_format_puts_it(path: &Path, map: &BlockMap) {
    let first = map
        .blocks
        .first()
        .unwrap_or_else(|| panic!("{}: the map lists no blocks at all", path.display()));
    assert_eq!(
        first.address,
        0,
        "{}: the first block sits at {:#x}, not 0",
        path.display(),
        first.address
    );
    assert_eq!(
        first.length,
        64,
        "{}: the identification block at 0 is {} bytes, not 64",
        path.display(),
        first.length
    );
    assert!(
        first.block_type == "MDF " || first.block_type == "UnFi",
        "{}: the identification block at 0 reports {:?}, not \"MDF \" or \"UnFi\"",
        path.display(),
        first.block_type
    );
    assert!(
        map.block_at(64).is_some(),
        "{}: no block at 64, where the header block always sits",
        path.display()
    );
}

/// The blocks and the gaps divide the file between them: covered bytes plus
/// gap bytes is the file size, and a gap — which is what the name says —
/// contains no block.
fn blocks_and_gaps_account_for_every_byte(path: &Path, map: &BlockMap) {
    let gap_bytes: u64 = map.gaps.iter().map(|gap| gap.length).sum();
    assert_eq!(
        map.covered_bytes + gap_bytes,
        map.file_size,
        "{}: {} covered bytes + {} gap bytes do not make the {}-byte file",
        path.display(),
        map.covered_bytes,
        gap_bytes,
        map.file_size
    );

    // Blocks and gaps are both in address order, so one sweep compares them.
    let mut block = 0usize;
    for gap in &map.gaps {
        let gap_end = gap.address + gap.length;
        while block < map.blocks.len()
            && map.blocks[block].address + map.blocks[block].length <= gap.address
        {
            block += 1;
        }
        if block < map.blocks.len() {
            assert!(
                map.blocks[block].address >= gap_end,
                "{}: the gap at {:#x}..{:#x} overlaps the block at {:#x}",
                path.display(),
                gap.address,
                gap_end,
                map.blocks[block].address
            );
        }
    }
}

/// A link that does not point at a block is the whole point of the warnings:
/// it must be explained there, never silently dropped.
fn every_link_resolves_or_is_warned_about(path: &Path, map: &BlockMap) {
    for block in &map.blocks {
        for (index, &link) in block.links.iter().enumerate() {
            if link == 0 {
                continue;
            }
            let resolves = map.block_at(link).is_some();
            let warned = map
                .warnings
                .iter()
                .any(|warning| warning.contains(&format!("{link:#x}")));
            let label = block.link_labels.get(index).map_or("?", String::as_str);
            assert!(
                resolves || warned,
                "{}: link {index} ({label}) of the block at {:#x} points to {link:#x}, \
                 which is neither a listed block nor a warning",
                path.display(),
                block.address
            );
        }
    }
}

fn labels_and_counts_describe_the_blocks_they_belong_to(path: &Path, map: &BlockMap) {
    for block in &map.blocks {
        assert_eq!(
            block.link_labels.len(),
            block.links.len(),
            "{}: the block at {:#x} has {} links but {} labels",
            path.display(),
            block.address,
            block.links.len(),
            block.link_labels.len()
        );
    }

    let counts = map.type_counts();
    let counted: usize = counts.iter().map(|(_, n)| *n).sum();
    assert_eq!(
        counted,
        map.blocks.len(),
        "{}: type_counts lists {counted} blocks, the map holds {}",
        path.display(),
        map.blocks.len()
    );
    for (block_type, count) in &counts {
        let actual = map
            .blocks
            .iter()
            .filter(|b| &b.block_type == block_type)
            .count();
        assert_eq!(
            actual,
            *count,
            "{}: type_counts says {count} {block_type} blocks, the map holds {actual}",
            path.display()
        );
    }
}

/// `block_at` is the map's lookup, so it resolves exactly at the addresses a
/// block starts at: inside a block, inside a gap, and past the end of the
/// file it returns nothing.
fn block_at_finds_a_block_exactly_at_its_start(path: &Path, map: &BlockMap) {
    let starts: HashSet<u64> = map.blocks.iter().map(|b| b.address).collect();

    let mut probes = Vec::new();
    for block in &map.blocks {
        probes.push(block.address);
        probes.push(block.address + 1);
    }
    for gap in &map.gaps {
        probes.push(gap.address);
        probes.push(gap.address + gap.length - 1);
    }
    probes.push(map.file_size);
    probes.push(map.file_size + 1);

    for address in probes {
        assert_eq!(
            map.block_at(address).is_some(),
            starts.contains(&address),
            "{}: block_at({address:#x}) disagrees with the block list",
            path.display()
        );
    }
}

/// Scanning the same bytes twice must give the same map, or diffing a file
/// before and after a fix against it is meaningless.
#[test]
fn scanning_a_file_twice_gives_the_same_map() {
    let files = reference_files();
    if skip_if_empty(&files) {
        return;
    }

    for path in &files {
        let file = Mf4File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let first = file.block_map();
        let second = file.block_map();

        assert_eq!(
            first.blocks.len(),
            second.blocks.len(),
            "{}: two scans found different block counts ({} vs {})",
            path.display(),
            first.blocks.len(),
            second.blocks.len()
        );
        assert_eq!(
            first.covered_bytes,
            second.covered_bytes,
            "{}: two scans covered different byte counts ({} vs {})",
            path.display(),
            first.covered_bytes,
            second.covered_bytes
        );
        for (a, b) in first.blocks.iter().zip(&second.blocks) {
            assert_eq!(
                a.address,
                b.address,
                "{}: the two scans disagree at {:#x}",
                path.display(),
                a.address
            );
        }
    }
}
