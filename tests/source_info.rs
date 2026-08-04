//! Source information (SI block) regression tests.
//!
//! `ChannelGroup.source` and `Channel.source` are populated from the file's SI
//! blocks. These values were read by hand from the corpus files (see
//! `examples/list_channels.rs`-style dumps) before being hardcoded here, so a
//! failure means the SI plumbing regressed, not that the fixture was written
//! from the implementation's own output.
//!
//! The corpus lives under `test_data/`, which is gitignored; these tests skip
//! rather than fail when it is absent, matching `tests/golden.rs` and
//! `tests/reference.rs`.

use falcon_mdf::blocks::{BusType, SourceType};
use falcon_mdf::Mf4File;
use std::path::{Path, PathBuf};

fn reference_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test_data")
        .join("reference")
}

/// `Vector_CANape.MF4` has two channel groups, each with an XCPsim ECU/CAN
/// acquisition source, and a master ("t") channel whose own `si_source` link
/// is null. Every non-master channel shares one SI block per group.
#[test]
fn vector_canape_channel_and_group_sources_match_the_file() {
    let path = reference_dir().join("Vector_CANape.MF4");
    if !path.exists() {
        eprintln!("SKIP: no reference corpus. Run scripts/fetch_reference_files.sh");
        return;
    }
    let file = Mf4File::open(&path).expect("Vector_CANape.MF4 should open");

    assert_eq!(file.data_group_count(), 2);

    for dg in file.data_groups() {
        assert_eq!(dg.channel_groups.len(), 1);
        let cg = &dg.channel_groups[0];

        let src = cg
            .source
            .as_ref()
            .unwrap_or_else(|| panic!("dg {} cg 0 should have a source", dg.index));
        assert_eq!(src.name, "");
        assert_eq!(src.path, "XCPsim");
        assert_eq!(src.source_type, Some(SourceType::Ecu));
        assert_eq!(src.bus_type, Some(BusType::Can));
        assert!(!src.simulated);

        let mut checked_master = false;
        let mut checked_data = false;
        for ch in &cg.channels {
            if ch.name == "t" {
                assert!(
                    ch.source.is_none(),
                    "master channel 't' has no si_source link in this file"
                );
                checked_master = true;
            } else {
                let src = ch
                    .source
                    .as_ref()
                    .unwrap_or_else(|| panic!("channel {} should have a source", ch.name));
                assert_eq!(src.name, "");
                assert_eq!(src.path, "XCPsim");
                assert_eq!(src.source_type, Some(SourceType::Ecu));
                assert_eq!(src.bus_type, Some(BusType::Can));
                assert!(!src.simulated);
                checked_data = true;
            }
        }
        assert!(checked_master, "expected a 't' master channel");
        assert!(checked_data, "expected at least one non-master channel");
    }
}

/// `ETAS_IntegerTypes.mf4` carries no SI blocks at all: its channel groups'
/// `si_acq_source` links are null, so every source should come back `None`
/// rather than some fallback value.
#[test]
fn etas_integer_types_has_no_source_information() {
    let path = reference_dir().join("ETAS_IntegerTypes.mf4");
    if !path.exists() {
        eprintln!("SKIP: no reference corpus. Run scripts/fetch_reference_files.sh");
        return;
    }
    let file = Mf4File::open(&path).expect("ETAS_IntegerTypes.mf4 should open");

    let mut checked = 0usize;
    for dg in file.data_groups() {
        for cg in &dg.channel_groups {
            assert!(
                cg.source.is_none(),
                "dg {} cg {} should have no source",
                dg.index,
                cg.index
            );
            for ch in &cg.channels {
                assert!(
                    ch.source.is_none(),
                    "channel {} should have no source",
                    ch.name
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "expected at least one channel to check");
}

/// `test_metadata.mf4` has ~25 channels across two channel groups that all
/// share one "ETK test device:1" SI block, so opening it should hit the SI
/// cache instead of re-parsing that block for every channel. (Not every
/// vendor writer shares one SI block this way — `Vector_CANape.MF4` above
/// gives each channel its own SI block with identical content, so that file
/// would show all misses; this test picks a file that actually exercises
/// reuse.)
#[test]
fn shared_source_lookups_hit_the_cache() {
    let path = reference_dir().join("test_metadata.mf4");
    if !path.exists() {
        eprintln!("SKIP: no reference corpus. Run scripts/fetch_reference_files.sh");
        return;
    }
    let file = Mf4File::open(&path).expect("test_metadata.mf4 should open");

    let stats = file.cache_stats();
    assert!(
        stats.si_hits > 0,
        "expected repeated SI lookups against a shared SI block to hit the cache, got {stats:?}"
    );
    assert!(
        stats.si_misses > 0,
        "expected at least one SI block to be parsed"
    );
}
