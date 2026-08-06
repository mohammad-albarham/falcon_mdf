//! Fuzzes the write path end to end: input bytes become little-endian f64
//! samples, `Mf4Writer` turns them into a file, and the reader — audited
//! block by block against the standard — must decode back exactly what went
//! in. `parse` covers reading hostile input; this covers the other half of
//! the symmetry: what the writer emits, the reader must reproduce.
//!
//! Any panic, abort or hang here is a bug. So is a value, a timestamp or a
//! validity flag that comes back different from what was written: the two
//! halves of the crate disagreeing means one of them misreads the format.
//!
//! Run with:
//!
//! ```text
//! cargo +nightly fuzz run roundtrip
//! ```
//!
//! No corpus seeding needed: the input is synthetic samples, not MF4 bytes,
//! so the fuzzer starts from valid structures by construction.

#![no_main]

use libfuzzer_sys::fuzz_target;

/// Cap on total samples (time axis plus channel columns) per input, so one
/// iteration stays cheap enough for millions of runs.
const MAX_SAMPLES: usize = 4096;

fuzz_target!(|data: &[u8]| {
    // The input becomes little-endian f64 samples; a trailing partial sample
    // is ignored.
    let samples: Vec<f64> = data
        .chunks_exact(8)
        .map(|chunk| f64::from_le_bytes(chunk.try_into().unwrap()))
        .collect();
    if samples.is_empty() {
        return;
    }

    // The first byte picks the shape: one or two channels, and per channel
    // whether it carries validity flags.
    let selector = data[0];
    let n_channels = usize::from(selector & 1 == 1) + 1;
    let stride = 1 + n_channels; // one time column plus one column per channel
    let n = (samples.len() / stride).min(MAX_SAMPLES / stride);
    if n == 0 {
        return;
    }

    let times = &samples[..n];
    let columns: &[&[f64]] = match n_channels {
        1 => &[&samples[n..2 * n]],
        _ => &[&samples[n..2 * n], &samples[2 * n..3 * n]],
    };
    let names = ["chan0", "chan1"];
    let has_validity = [selector & 2 != 0, selector & 4 != 0];

    let mut writer = falcon_mdf::Mf4Writer::with_start_time_ns(0);

    // Records are sorted by time on write, so a time axis has to be
    // orderable; NaN is not. The writer must refuse it — pin that contract
    // rather than avoiding such inputs.
    if times.iter().any(|t| t.is_nan()) {
        assert!(writer.add_group(times).is_err());
        return;
    }

    let group = writer.add_group(times).unwrap();
    for (index, values) in columns.iter().enumerate() {
        if has_validity[index] {
            // Validity derives from each sample's low bit, so roughly half of
            // all written samples carry the invalidation bit and the packing
            // of those bits gets exercised on both single- and two-channel
            // groups.
            let valid: Vec<bool> = values.iter().map(|v| v.to_bits() & 1 == 1).collect();
            group
                .add_channel_with_validity(names[index], "V", values, Some(&valid))
                .unwrap();
        } else {
            group.add_channel(names[index], "V", values).unwrap();
        }
    }

    // The reader is path-based, so the file has to reach it as a file. Each
    // process gets its own path; libFuzzer runs one input at a time per
    // process. Same arrangement as the parse target.
    let path =
        std::env::temp_dir().join(format!("falcon_mdf_fuzz_roundtrip_{}.mf4", std::process::id()));
    if writer.write_to_file(&path).is_err() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let file = match falcon_mdf::Mf4File::open(&path) {
        Ok(file) => file,
        Err(err) => panic!("a file the writer just produced failed to open: {err}"),
    };

    // Records come back sorted by time; apply the writer's stable `total_cmp`
    // ordering to the inputs before comparing.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| times[a].total_cmp(&times[b]));

    let time_signal = file
        .signal(
            file.find_channel("Time")
                .expect("the master channel the writer always emits"),
        )
        .unwrap();
    let got_times = time_signal.values_f64().unwrap();
    assert_eq!(got_times.len(), n);
    for (got, &i) in got_times.iter().zip(&order) {
        // Bit equality: the writer stores IEEE754 doubles verbatim, so this
        // holds for infinities, subnormals and NaN payloads alike.
        assert_eq!(got.to_bits(), times[i].to_bits());
    }

    for (index, values) in columns.iter().enumerate() {
        let signal = file
            .signal(
                file.find_channel(names[index])
                    .expect("a channel just written"),
            )
            .unwrap();
        let got = signal.values_f64().unwrap();
        assert_eq!(got.len(), n);
        for (got, &i) in got.iter().zip(&order) {
            assert_eq!(got.to_bits(), values[i].to_bits());
        }
        match has_validity[index] {
            // Invalid samples stay in the value column — the reader reports
            // them flagged, not dropped — so the expectation is the flags in
            // sorted order.
            true => {
                let expected: Vec<bool> = order
                    .iter()
                    .map(|&i| values[i].to_bits() & 1 == 1)
                    .collect();
                assert_eq!(signal.validity(), Some(expected));
            }
            false => assert_eq!(signal.validity(), None),
        }
    }

    let _ = std::fs::remove_file(&path);
});
