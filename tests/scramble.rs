//! Anonymising a file must change every name and no number.
//!
//! Scrambling has two ways to fail and they pull in opposite directions: it
//! can leave identifying text behind, which defeats the point, or it can
//! disturb the file while replacing that text, which destroys the
//! measurement. One comparison catches both — scramble a real file, reopen
//! the output, and hold it against the original sample by sample and name by
//! name.
//!
//! The oracle throughout is the *original file read by the ordinary reader*,
//! never scrambling run backwards. Nothing here asks the implementation to
//! confirm its own work.

use std::path::PathBuf;

use falcon_mdf::{scramble_file, Mf4File};

/// Resolves a corpus file, which is fetched rather than committed and so in a
/// worktree lives in the primary checkout rather than beside the source.
fn corpus(name: &str) -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_data/reference").join(name),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../falcon_mdf/test_data/reference")
            .join(name),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

/// Every channel of a file, in file order, with the numbers it decodes to.
///
/// Read positionally rather than by name: after scrambling there is no name
/// left to look a channel up by, which is the whole point.
struct Snapshot {
    group_count: usize,
    channel_count: usize,
    names: Vec<String>,
    units: Vec<String>,
    /// Sample values as raw bits, so that two NaNs compare equal and no
    /// float comparison can call a changed sample unchanged.
    values: Vec<Vec<u64>>,
}

fn snapshot(path: &PathBuf) -> Snapshot {
    let file = Mf4File::open(path).expect("the file should open");

    let mut names = Vec::new();
    let mut units = Vec::new();
    let mut values = Vec::new();

    for channel in file.channels() {
        names.push(channel.name.clone());
        units.push(channel.unit.clone());
        let bits = match file.signal(channel) {
            Ok(signal) => match signal.values_f64() {
                Ok(v) => v.iter().map(|x| x.to_bits()).collect(),
                Err(_) => Vec::new(),
            },
            Err(_) => Vec::new(),
        };
        values.push(bits);
    }

    Snapshot {
        group_count: file.data_groups().len(),
        channel_count: file.channel_count(),
        names,
        units,
        values,
    }
}

/// The test that matters: every number identical, every name different.
#[test]
fn scrambling_keeps_every_sample_and_changes_every_name() {
    // Files chosen to span the ways text and numbers meet in MF4: plain
    // channels, a value-to-text table whose strings are the text being
    // anonymised, and an algebraic conversion whose formula is text the
    // decoding reads.
    let wanted = [
        "ETAS_SimpleSorted.mf4",
        "dSPACE_LinearConversion.mf4",
        "dSPACE_IntegerTypes.mf4",
        "dSPACE_Value2TextConversion.mf4",
        "dSPACE_AlgebraicConversion.mf4",
        "Vector_AlgebraicConversionQuadratic.mf4",
    ];

    let files: Vec<PathBuf> = wanted.iter().filter_map(|n| corpus(n)).collect();
    assert!(
        !files.is_empty(),
        "no corpus file was found in either location; the corpus is fetched, not committed"
    );

    let dir = tempfile::tempdir().unwrap();

    for src in &files {
        let name = src.file_name().unwrap().to_string_lossy().to_string();
        let before = snapshot(src);

        let dst = dir.path().join(format!("scrambled_{name}"));
        let report = scramble_file(src, &dst, 0x5EED).expect("scrambling should succeed");

        // The output opens, and is the same measurement structurally.
        let after = snapshot(&dst);
        assert_eq!(after.group_count, before.group_count, "{name}: group count");
        assert_eq!(
            after.channel_count, before.channel_count,
            "{name}: channel count"
        );
        assert_eq!(
            after.names.len(),
            before.names.len(),
            "{name}: channels enumerated"
        );

        // Not one number moved.
        for (i, (old, new)) in before.values.iter().zip(&after.values).enumerate() {
            assert_eq!(
                old.len(),
                new.len(),
                "{name}: channel {i} ({}) changed sample count",
                before.names[i]
            );
            assert_eq!(
                old, new,
                "{name}: channel {i} ({}) changed its samples",
                before.names[i]
            );
        }

        // Not one name survived.
        let mut compared = 0;
        for (i, (old, new)) in before.names.iter().zip(&after.names).enumerate() {
            assert_eq!(old.len(), new.len(), "{name}: channel {i} name length");
            if !old.is_empty() {
                assert_ne!(old, new, "{name}: channel {i} kept its name");
                compared += 1;
            }
        }
        assert!(compared > 0, "{name}: no named channel to anonymise");

        // Units too, where the file gives one.
        for (i, (old, new)) in before.units.iter().zip(&after.units).enumerate() {
            assert_eq!(old.len(), new.len(), "{name}: channel {i} unit length");
            if !old.is_empty() {
                assert_ne!(old, new, "{name}: channel {i} kept its unit");
            }
        }

        assert!(
            report.blocks_scrambled > 0,
            "{name}: the report claims nothing was scrambled"
        );
    }
}

/// Timestamps are samples like any other and must survive untouched.
#[test]
fn scrambling_leaves_the_time_base_alone() {
    let Some(src) = corpus("ETAS_SimpleSorted.mf4") else {
        eprintln!("corpus file missing; skipping");
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("scrambled.mf4");
    scramble_file(&src, &dst, 1).expect("scrambling should succeed");

    let before = Mf4File::open(&src).unwrap();
    let after = Mf4File::open(&dst).unwrap();

    // The time base is the master channel of each group, read as samples.
    let mut checked = 0;
    for (dg, group) in before.data_groups().iter().enumerate() {
        for cg in 0..group.channel_groups.len() {
            let (Some(old_master), Some(new_master)) =
                (before.master_channel(dg, cg), after.master_channel(dg, cg))
            else {
                continue;
            };
            let a: Vec<u64> = before
                .signal(old_master)
                .and_then(|s| s.values_f64())
                .expect("the original master channel should read")
                .iter()
                .map(|x| x.to_bits())
                .collect();
            let b: Vec<u64> = after
                .signal(new_master)
                .and_then(|s| s.values_f64())
                .expect("the scrambled master channel should read")
                .iter()
                .map(|x| x.to_bits())
                .collect();
            assert_eq!(a, b, "group {dg}/{cg}: the time base moved");
            assert!(!a.is_empty(), "group {dg}/{cg}: no timestamps to compare");
            checked += 1;
        }
    }
    assert!(checked > 0, "no group had a master channel to compare");
}

/// The file must be byte-identical everywhere the text is not.
#[test]
fn scrambling_moves_no_block_and_changes_no_length() {
    let Some(src) = corpus("dSPACE_Value2TextConversion.mf4") else {
        eprintln!("corpus file missing; skipping");
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("scrambled.mf4");
    scramble_file(&src, &dst, 99).expect("scrambling should succeed");

    let original = std::fs::read(&src).unwrap();
    let scrambled = std::fs::read(&dst).unwrap();
    assert_eq!(original.len(), scrambled.len(), "the file changed size");

    // Every block sits at the same address, carries the same type, the same
    // length and the same links as before.
    let before = Mf4File::open(&src).unwrap().block_map();
    let after = Mf4File::open(&dst).unwrap().block_map();

    assert_eq!(before.blocks.len(), after.blocks.len(), "block count");
    for (a, b) in before.blocks.iter().zip(&after.blocks) {
        assert_eq!(a.address, b.address, "a block moved");
        assert_eq!(a.block_type, b.block_type, "a block changed type");
        assert_eq!(a.length, b.length, "a block changed length");
        assert_eq!(a.links, b.links, "a block's links changed");
    }
    assert!(after.warnings.is_empty(), "walk warned: {:?}", after.warnings);

    // And the bytes that did change are only ever inside a text block.
    let text_ranges: Vec<(u64, u64)> = before
        .blocks
        .iter()
        .filter(|b| b.block_type == "##TX" || b.block_type == "##MD")
        .map(|b| (b.address, b.address + b.length))
        .collect();

    let mut differing = 0;
    for (i, (x, y)) in original.iter().zip(&scrambled).enumerate() {
        if x != y {
            differing += 1;
            let at = i as u64;
            assert!(
                text_ranges.iter().any(|(s, e)| at >= *s && at < *e),
                "byte {i:#x} changed outside any text block"
            );
        }
    }
    assert!(differing > 0, "nothing was scrambled at all");
}

/// A value-to-text table's strings are identifying text and must be replaced.
#[test]
fn the_strings_of_a_text_table_are_anonymised() {
    let Some(src) = corpus("dSPACE_Value2TextConversion.mf4") else {
        eprintln!("corpus file missing; skipping");
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("scrambled.mf4");
    scramble_file(&src, &dst, 4).expect("scrambling should succeed");

    // The table's strings are what the channel decodes to, so read them as
    // text rather than reaching into the conversion.
    let before = Mf4File::open(&src).unwrap();
    let after = Mf4File::open(&dst).unwrap();

    let mut found_text = false;
    for (old, new) in before.channels().zip(after.channels()) {
        let (Ok(a), Ok(b)) = (before.signal(old), after.signal(new)) else {
            continue;
        };
        let (Ok(av), Ok(bv)) = (a.values(), b.values()) else {
            continue;
        };
        if let (falcon_mdf::SignalValues::Str(old_text), falcon_mdf::SignalValues::Str(new_text)) =
            (av, bv)
        {
            for (o, n) in old_text.iter().zip(&new_text) {
                assert_eq!(o.len(), n.len(), "a table string changed length");
                if !o.is_empty() {
                    assert_ne!(o, n, "a table string survived scrambling");
                    found_text = true;
                }
            }
        }
    }
    assert!(found_text, "this file was chosen for its text table");
}

/// An algebraic conversion's formula is arithmetic, not a name: scrambling it
/// would change the numbers, so it is left alone. The numbers prove it.
#[test]
fn an_algebraic_formula_still_evaluates_after_scrambling() {
    let files: Vec<PathBuf> = [
        "dSPACE_AlgebraicConversion.mf4",
        "Vector_AlgebraicConversionQuadratic.mf4",
        "Vector_AlgebraicConversionRational.mf4",
        "Vector_AlgebraicConversionSinus.mf4",
    ]
    .iter()
    .filter_map(|n| corpus(n))
    .collect();

    if files.is_empty() {
        eprintln!("corpus files missing; skipping");
        return;
    }

    let dir = tempfile::tempdir().unwrap();

    for src in &files {
        let name = src.file_name().unwrap().to_string_lossy().to_string();
        let dst = dir.path().join(format!("scrambled_{name}"));
        let report = scramble_file(src, &dst, 11).expect("scrambling should succeed");

        assert!(
            report.blocks_preserved > 0,
            "{name}: the formula block should have been preserved"
        );

        let before = Mf4File::open(src).unwrap();
        let after = Mf4File::open(&dst).unwrap();

        // A conversion that stopped working would not error: it would quietly
        // produce different physical values. Compare them.
        let mut checked = 0;
        for (old, new) in before.channels().zip(after.channels()) {
            let (Ok(a), Ok(b)) = (before.signal(old), after.signal(new)) else {
                continue;
            };
            let (Ok(av), Ok(bv)) = (a.values_f64(), b.values_f64()) else {
                continue;
            };
            let ab: Vec<u64> = av.iter().map(|x| x.to_bits()).collect();
            let bb: Vec<u64> = bv.iter().map(|x| x.to_bits()).collect();
            assert_eq!(ab, bb, "{name}: channel '{}' converted differently", old.name);
            checked += 1;
        }
        assert!(checked > 0, "{name}: no channel compared");
    }
}

/// An MDF 3.x file has a different block layout, so it is refused by name
/// rather than copied through unscrambled.
#[test]
fn an_mdf3_file_is_refused_by_name() {
    let Some(src) = corpus("multiple.MF4").or_else(|| corpus("simple.mf4")) else {
        eprintln!("corpus file missing; skipping");
        return;
    };

    // Build a file that declares MDF 3.00 in its ID block.
    let mut bytes = std::fs::read(&src).unwrap();
    bytes[28..30].copy_from_slice(&300u16.to_le_bytes());

    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("v3.mf4");
    std::fs::write(&fake, &bytes).unwrap();

    let err = scramble_file(&fake, dir.path().join("out.mf4"), 0)
        .expect_err("an MDF 3 file must be refused");
    let text = err.to_string();
    assert!(
        text.contains("300") && text.to_lowercase().contains("scramble"),
        "the error should name the version and the operation: {text}"
    );
}
