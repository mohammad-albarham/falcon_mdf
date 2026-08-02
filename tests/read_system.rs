//! End-to-end tests of the reading system.
//!
//! The other suites each check one thing: `golden` compares decoded values
//! against an independent reference, `robustness` checks malformed input, and
//! the unit tests cover pieces in isolation. This one exercises the features
//! *together* on every corpus file — the combinations an optimisation is most
//! likely to break, and which none of the others would notice.
//!
//! Needs the sample corpus under `test_data/`; skips cleanly when absent.

use falcon_mdf::blocks::ChannelType;
use falcon_mdf::error::Mf4Error;
use falcon_mdf::{Mf4File, OpenOptions, Signal, SignalValues};
use std::path::{Path, PathBuf};

fn corpus() -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect(Path::new("test_data"), &mut found);
    found.sort();
    found
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("MF4") {
            out.push(path);
        }
    }
}

macro_rules! corpus_or_skip {
    () => {{
        let files = corpus();
        if files.is_empty() {
            eprintln!("SKIP: no corpus under test_data/");
            return;
        }
        files
    }};
}

#[test]
fn every_corpus_file_opens_and_reports_a_consistent_structure() {
    for path in corpus_or_skip!() {
        let file = Mf4File::open(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));

        assert!(file.data_group_count() > 0, "{path:?}: no data groups");
        assert!(file.channel_count() > 0, "{path:?}: no channels");

        // The channel count must agree with what iterating actually yields, and
        // every channel must point back at a group that exists.
        let channels: Vec<_> = file.channels().cloned().collect();
        assert_eq!(
            channels.len(),
            file.channel_count(),
            "{path:?}: channel_count disagrees with channels()"
        );

        for ch in &channels {
            let dg = file
                .data_groups()
                .get(ch.data_group_index)
                .unwrap_or_else(|| panic!("{path:?}: {} names a missing data group", ch.name));
            assert!(
                ch.channel_group_index < dg.channel_groups.len(),
                "{path:?}: {} names a missing channel group",
                ch.name
            );
            assert!(!ch.name.is_empty(), "{path:?}: a channel has no name");
        }
    }
}

#[test]
fn every_channel_either_decodes_or_explains_why_not() {
    // The rule this crate holds to: a channel is decoded correctly, or reading
    // it fails with a reason. Never a partial or invented answer.
    for path in corpus_or_skip!() {
        let file = Mf4File::open(&path).expect("corpus file should open");
        let channels: Vec<_> = file.channels().cloned().collect();

        for ch in &channels {
            let signal = file
                .signal(ch)
                .unwrap_or_else(|e| panic!("{path:?}: signal({}) failed: {e}", ch.name));

            match signal.values() {
                Ok(values) => assert_eq!(
                    values.len(),
                    signal.len(),
                    "{path:?}: {} decoded {} values for {} samples",
                    ch.name,
                    values.len(),
                    signal.len()
                ),
                Err(Mf4Error::Unsupported { .. }) => {}
                Err(e) => panic!("{path:?}: {} failed unexpectedly: {e}", ch.name),
            }
        }
    }
}

#[test]
fn decoded_type_matches_the_declared_kind() {
    for path in corpus_or_skip!() {
        let file = Mf4File::open(&path).expect("corpus file should open");
        let channels: Vec<_> = file.channels().cloned().collect();

        for ch in &channels {
            let Ok(signal) = file.signal(ch) else {
                continue;
            };
            let Ok(values) = signal.values() else {
                continue;
            };
            assert_eq!(
                values.kind(),
                ch.value_kind(),
                "{path:?}: {} decoded as {} but declares {}",
                ch.name,
                values.kind().name(),
                ch.value_kind().name()
            );
        }
    }
}

#[test]
fn all_channels_in_a_group_agree_on_sample_count() {
    // Channels of one group share a time axis. A decoder that miscounts one
    // channel would silently misalign it against the master.
    for path in corpus_or_skip!() {
        let file = Mf4File::open(&path).expect("corpus file should open");

        for dg in file.data_groups() {
            for cg in &dg.channel_groups {
                let mut expected: Option<usize> = None;
                for ch in &cg.channels {
                    let Ok(signal) = file.signal(ch) else {
                        continue;
                    };
                    match expected {
                        None => expected = Some(signal.len()),
                        Some(n) => assert_eq!(
                            signal.len(),
                            n,
                            "{path:?}: {} has {} samples, siblings have {n}",
                            ch.name,
                            signal.len()
                        ),
                    }
                }
            }
        }
    }
}

#[test]
fn reading_the_same_channel_twice_gives_the_same_answer() {
    // Records and payloads are cached between reads. A cache keyed wrongly
    // would return one channel's data for another.
    for path in corpus_or_skip!() {
        let file = Mf4File::open(&path).expect("corpus file should open");
        let channels: Vec<_> = file.channels().cloned().collect();

        // Read every channel, then read them all again in reverse, which forces
        // the single-entry caches to be rebuilt in a different order.
        let first: Vec<Option<SignalValues>> = channels
            .iter()
            .map(|ch| file.signal(ch).ok().and_then(|s| s.values().ok()))
            .collect();

        let second: Vec<Option<SignalValues>> = channels
            .iter()
            .rev()
            .map(|ch| file.signal(ch).ok().and_then(|s| s.values().ok()))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        for (i, (a, b)) in first.iter().zip(second.iter()).enumerate() {
            assert_eq!(
                a, b,
                "{path:?}: {} differs between reads — a cache is keyed wrongly",
                channels[i].name
            );
        }
    }
}

#[test]
fn the_buffered_backend_reads_identically_to_the_memory_mapped_one() {
    // Two I/O paths that must not disagree. mmap is the default; buffered is
    // what untrusted input is meant to use.
    for path in corpus_or_skip!() {
        let mapped = Mf4File::open(&path).expect("mmap open");
        let buffered = Mf4File::open_buffered(&path).expect("buffered open");

        assert_eq!(mapped.channel_count(), buffered.channel_count(), "{path:?}");
        assert_eq!(mapped.version(), buffered.version(), "{path:?}");

        let a: Vec<_> = mapped.channels().cloned().collect();
        let b: Vec<_> = buffered.channels().cloned().collect();

        for (ca, cb) in a.iter().zip(b.iter()) {
            assert_eq!(ca.name, cb.name, "{path:?}: channel names differ");
            let va = mapped.signal(ca).ok().and_then(|s| s.values().ok());
            let vb = buffered.signal(cb).ok().and_then(|s| s.values().ok());
            assert_eq!(va, vb, "{path:?}: {} differs between backends", ca.name);
        }
    }
}

#[test]
fn disabling_the_channel_index_does_not_change_what_is_read() {
    for path in corpus_or_skip!() {
        let default = Mf4File::open(&path).expect("default open");
        let no_index = Mf4File::open_with_options(
            &path,
            OpenOptions {
                build_channels_db: false,
                ..Default::default()
            },
        )
        .expect("open without the channel index");

        // The index is a lookup accelerator. Switching it off must not change
        // what the file reports about itself, nor which channels can be found —
        // it once made channel_count() report zero.
        assert_eq!(
            default.channel_count(),
            no_index.channel_count(),
            "{path:?}"
        );
        assert_eq!(
            no_index.channel_count(),
            no_index.channels().count(),
            "{path:?}: channel_count disagrees with channels() without the index"
        );
        assert_eq!(
            default.channel_names().len(),
            no_index.channel_names().len(),
            "{path:?}: name list differs without the index"
        );

        for ch in default.channels() {
            assert!(
                no_index.has_channel(&ch.name),
                "{path:?}: {} not findable without the index",
                ch.name
            );
            assert_eq!(
                no_index.find_channel(&ch.name).map(|c| &c.name),
                Some(&ch.name),
                "{path:?}: find_channel({}) differs without the index",
                ch.name
            );
            assert_eq!(
                default.find_channels(&ch.name).len(),
                no_index.find_channels(&ch.name).len(),
                "{path:?}: find_channels({}) differs without the index",
                ch.name
            );
        }

        let a: Vec<_> = default.channels().cloned().collect();
        let b: Vec<_> = no_index.channels().cloned().collect();
        for (ca, cb) in a.iter().zip(b.iter()) {
            let va = default.signal(ca).ok().and_then(|s| s.values().ok());
            let vb = no_index.signal(cb).ok().and_then(|s| s.values().ok());
            assert_eq!(va, vb, "{path:?}: {} differs", ca.name);
        }
    }
}

#[test]
fn lookup_by_name_finds_the_same_channel_iteration_yields() {
    for path in corpus_or_skip!() {
        let file = Mf4File::open(&path).expect("corpus file should open");
        let channels: Vec<_> = file.channels().cloned().collect();

        for ch in &channels {
            let found = file
                .find_channel(&ch.name)
                .unwrap_or_else(|| panic!("{path:?}: find_channel({}) found nothing", ch.name));
            assert_eq!(found.name, ch.name, "{path:?}");
            assert!(file.has_channel(&ch.name), "{path:?}: {}", ch.name);
        }

        assert!(file.find_channel("!!definitely not a channel!!").is_none());
    }
}

#[test]
fn variable_length_channels_resolve_their_payloads() {
    let mut seen = 0usize;
    for path in corpus_or_skip!() {
        let file = Mf4File::open(&path).expect("corpus file should open");
        let channels: Vec<_> = file.channels().cloned().collect();

        for ch in channels
            .iter()
            .filter(|c| c.channel_type == ChannelType::VariableLength && c.unreadable().is_none())
        {
            let signal = file.signal(ch).expect("signal");
            if signal.is_empty() {
                continue;
            }
            let values = signal
                .values()
                .unwrap_or_else(|e| panic!("{path:?}: {} failed: {e}", ch.name));
            seen += 1;

            assert!(
                matches!(
                    values,
                    SignalValues::Bytes { .. } | SignalValues::VarBytes { .. }
                ),
                "{path:?}: {} should decode to bytes, got {}",
                ch.name,
                values.kind().name()
            );
            assert!(
                values.bytes_at(0).is_some(),
                "{path:?}: {} resolved no payload for its first sample",
                ch.name
            );
            // Payloads resolving to nothing for every sample would be the
            // byte-order bug (B15) coming back.
            let non_empty = (0..values.len().min(64))
                .filter(|&i| values.bytes_at(i).is_some_and(|b| !b.is_empty()))
                .count();
            assert!(
                non_empty > 0,
                "{path:?}: {} resolved no payloads at all",
                ch.name
            );
        }
    }
    assert!(seen > 0, "expected the corpus to exercise VLSD channels");
}

#[test]
fn validity_is_reported_consistently() {
    for path in corpus_or_skip!() {
        let file = Mf4File::open(&path).expect("corpus file should open");
        let channels: Vec<_> = file.channels().cloned().collect();

        for ch in &channels {
            let Ok(signal) = file.signal(ch) else {
                continue;
            };
            match signal.validity() {
                Some(v) => {
                    assert_eq!(v.len(), signal.len(), "{path:?}: {}", ch.name);
                    assert_eq!(
                        signal.valid_count(),
                        v.iter().filter(|ok| **ok).count(),
                        "{path:?}: {} valid_count disagrees with validity()",
                        ch.name
                    );
                    for (i, &ok) in v.iter().enumerate().take(32) {
                        assert_eq!(signal.is_valid(i), ok, "{path:?}: {}[{i}]", ch.name,);
                    }
                }
                None => assert_eq!(
                    signal.valid_count(),
                    signal.len(),
                    "{path:?}: {} without invalidation bits must be wholly valid",
                    ch.name
                ),
            }
        }
    }
}

#[test]
fn numeric_values_survive_the_round_trip_through_f64() {
    for path in corpus_or_skip!() {
        let file = Mf4File::open(&path).expect("corpus file should open");
        let channels: Vec<_> = file.channels().cloned().collect();

        for ch in &channels {
            let Ok(signal) = file.signal(ch) else {
                continue;
            };
            let Ok(values) = signal.values() else {
                continue;
            };
            if !values.kind().is_numeric() {
                continue;
            }
            let as_f64 = signal.values_f64().expect("f64 view of a numeric channel");
            assert_eq!(as_f64.len(), values.len(), "{path:?}: {}", ch.name);
            assert_eq!(as_f64, values.to_f64(), "{path:?}: {}", ch.name);
        }
    }
}

#[test]
fn metadata_and_timestamps_are_present_and_sane() {
    for path in corpus_or_skip!() {
        let file = Mf4File::open(&path).expect("corpus file should open");

        assert!(
            !file.comment().contains('<'),
            "{path:?}: comment contains markup"
        );
        assert!(
            file.version().to_string().starts_with('4'),
            "{path:?}: unexpected version {}",
            file.version()
        );
        assert!(file.file_size() > 0, "{path:?}");

        // A master channel's timestamps must not go backwards.
        for dg in file.data_groups() {
            for cg in &dg.channel_groups {
                let Some(master) = cg.master_channel() else {
                    continue;
                };
                let Ok(signal) = file.signal(master) else {
                    continue;
                };
                let Ok(times) = signal.values_f64() else {
                    continue;
                };
                for pair in times.windows(2).take(4096) {
                    assert!(
                        pair[1] >= pair[0],
                        "{path:?}: {} goes backwards: {} then {}",
                        master.name,
                        pair[0],
                        pair[1]
                    );
                }
            }
        }
    }
}

#[test]
fn both_backends_agree_channel_for_channel() {
    // Compared by position, not by name. These files carry 73 groups that each
    // contain a channel called `Timestamp`, so looking one up by name returns
    // the first of them — comparing that against the fortieth reports a
    // difference that is not there.
    for path in corpus_or_skip!() {
        let mapped = Mf4File::open(&path).expect("mmap open");
        let buffered = Mf4File::open_buffered(&path).expect("buffered open");

        for (dg_a, dg_b) in mapped.data_groups().iter().zip(buffered.data_groups()) {
            for (cg_a, cg_b) in dg_a.channel_groups.iter().zip(&dg_b.channel_groups) {
                for (ch_a, ch_b) in cg_a.channels.iter().zip(&cg_b.channels) {
                    assert_eq!(ch_a.name, ch_b.name, "{path:?}: channel order differs");
                    let a = mapped.signal(ch_a).ok().and_then(|s| s.values().ok());
                    let b = buffered.signal(ch_b).ok().and_then(|s| s.values().ok());
                    assert_eq!(
                        a, b,
                        "{path:?}: dg{} cg{} {} differs between backends",
                        dg_a.index, cg_a.index, ch_a.name
                    );
                }
            }
        }
    }
}

/// Writes bytes to a temporary file and opens them.
fn open_bytes(bytes: &[u8], name: &str) -> falcon_mdf::Result<Mf4File> {
    let path = std::env::temp_dir().join(format!("falcon_mdf_system_{name}.mf4"));
    std::fs::write(&path, bytes).expect("write temp file");
    let result = Mf4File::open(&path);
    let _ = std::fs::remove_file(&path);
    result
}

/// Rewrites a file's declared version, leaving everything else untouched.
///
/// The version lives in the identification block as an ASCII string at byte 8
/// and a number at byte 28. Both have to agree, or the file contradicts itself.
fn with_version(bytes: &[u8], text: &str, number: u16) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let mut label = [b' '; 8];
    label[..text.len().min(8)].copy_from_slice(&text.as_bytes()[..text.len().min(8)]);
    out[8..16].copy_from_slice(&label);
    out[28..30].copy_from_slice(&number.to_le_bytes());
    out
}

#[test]
fn versions_4_0_and_4_2_are_read_like_4_11() {
    // Every corpus file is 4.11, so the other two versions the crate advertises
    // have never been exercised. The block layouts this reader touches are
    // unchanged across 4.0, 4.1 and 4.2 — later versions add blocks rather than
    // altering these — so relabelling a real file is a faithful test of the
    // version handling itself, which is the part that was untested.
    for path in corpus_or_skip!() {
        let original = std::fs::read(&path).expect("read corpus file");
        let reference = Mf4File::open(&path).expect("open original");
        let expected: Vec<_> = reference.channels().map(|c| c.name.clone()).collect();

        for (text, number, shown) in [("4.00    ", 400u16, "4.00"), ("4.20    ", 420, "4.20")] {
            let patched = with_version(&original, text, number);
            let file = open_bytes(&patched, &format!("version_{number}"))
                .unwrap_or_else(|e| panic!("{path:?} relabelled {shown}: {e}"));

            assert_eq!(
                file.version().to_string(),
                shown,
                "{path:?}: version not reported as {shown}"
            );

            let names: Vec<_> = file.channels().map(|c| c.name.clone()).collect();
            assert_eq!(
                names, expected,
                "{path:?}: relabelling as {shown} changed which channels were found"
            );

            // And the data still decodes to the same values.
            for (a, b) in file.channels().zip(reference.channels()) {
                let x = file.signal(a).ok().and_then(|s: Signal| s.values().ok());
                let y = reference
                    .signal(b)
                    .ok()
                    .and_then(|s: Signal| s.values().ok());
                assert_eq!(x, y, "{path:?}: {} differs when labelled {shown}", a.name);
            }
        }
    }
}

#[test]
fn an_unsupported_major_version_is_rejected() {
    // A version this crate does not implement must be refused rather than read
    // as though it were MDF 4.
    // One file is enough: this exercises the version gate, not the data.
    let files = corpus_or_skip!();
    let path = &files[0];
    let original = std::fs::read(path).expect("read corpus file");

    for (text, number) in [("3.30    ", 330u16), ("5.00    ", 500)] {
        let patched = with_version(&original, text, number);
        assert!(
            open_bytes(&patched, &format!("bad_version_{number}")).is_err(),
            "{path:?}: a file declaring version {number} must not open"
        );
    }
}
