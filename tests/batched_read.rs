//! Tests for batched multi-channel reading (`Mf4File::signals`).
//!
//! Asserts that reading a batch of channels produces results identical to
//! looping `Mf4File::signal()` over each channel — values, typed representations,
//! invalidation masks, units, and error behaviour.

use falcon_mdf::{Mf4File, Mf4Writer, Signal, SignalValues};
use std::path::{Path, PathBuf};
use std::time::Instant;

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
        } else if path
            .extension()
            .and_then(|s| s.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("mf4"))
            == Some(true)
        {
            out.push(path);
        }
    }
}

/// Asserts two signals are identical in all public observable properties.
fn assert_signals_identical(batched: &Signal, sequential: &Signal, ctx: &str) {
    assert_eq!(
        batched.channel().name,
        sequential.channel().name,
        "{ctx}: channel name mismatch"
    );
    assert_eq!(
        batched.unit(),
        sequential.unit(),
        "{ctx}: channel unit mismatch"
    );
    assert_eq!(
        batched.len(),
        sequential.len(),
        "{ctx}: sample count mismatch"
    );
    assert_eq!(
        batched.validity(),
        sequential.validity(),
        "{ctx}: validity mismatch"
    );

    let batched_val = batched.values();
    let seq_val = sequential.values();

    match (batched_val, seq_val) {
        (Ok(b_vals), Ok(s_vals)) => {
            assert_eq!(
                b_vals.len(),
                s_vals.len(),
                "{ctx}: signal values len mismatch"
            );
            assert_eq!(
                b_vals.kind(),
                s_vals.kind(),
                "{ctx}: signal values kind mismatch"
            );
            assert_signals_values_eq(&b_vals, &s_vals, ctx);
        }
        (Err(b_err), Err(s_err)) => {
            assert_eq!(
                b_err.to_string(),
                s_err.to_string(),
                "{ctx}: error mismatch"
            );
        }
        (Ok(_), Err(e)) => panic!("{ctx}: batched succeeded but sequential errored: {e}"),
        (Err(e), Ok(_)) => panic!("{ctx}: sequential succeeded but batched errored: {e}"),
    }
}

fn assert_signals_values_eq(a: &SignalValues, b: &SignalValues, ctx: &str) {
    match (a, b) {
        (SignalValues::F32(va), SignalValues::F32(vb)) => {
            for (i, (x, y)) in va.iter().zip(vb).enumerate() {
                if x.is_nan() && y.is_nan() {
                    continue;
                }
                assert_eq!(x.to_bits(), y.to_bits(), "{ctx} sample {i}");
            }
        }
        (SignalValues::F64(va), SignalValues::F64(vb)) => {
            for (i, (x, y)) in va.iter().zip(vb).enumerate() {
                if x.is_nan() && y.is_nan() {
                    continue;
                }
                assert_eq!(x.to_bits(), y.to_bits(), "{ctx} sample {i}");
            }
        }
        (other_a, other_b) => {
            assert_eq!(other_a, other_b, "{ctx}");
        }
    }
}

#[test]
fn batched_read_on_synthetic_multi_group_file() {
    let mut writer = Mf4Writer::new();

    // Group 1: 5 channels with invalidation
    let g1 = writer.add_group(&[0.0, 0.1, 0.2, 0.3, 0.4]).unwrap();
    g1.add_channel_with_validity(
        "Ch1_1",
        "m/s",
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        Some(&[true, true, false, true, true]),
    )
    .unwrap();
    g1.add_channel("Ch1_2", "deg", &[10.0, 20.0, 30.0, 40.0, 50.0])
        .unwrap();
    g1.add_channel_with_validity(
        "Ch1_3",
        "bar",
        &[0.1, 0.2, 0.3, 0.4, 0.5],
        Some(&[false, true, true, true, false]),
    )
    .unwrap();

    // Group 2: 3 channels
    let g2 = writer.add_group(&[0.0, 0.5, 1.0]).unwrap();
    g2.add_channel("Ch2_1", "rpm", &[1000.0, 2000.0, 3000.0])
        .unwrap();
    g2.add_channel("Ch2_2", "Nm", &[100.0, 200.0, 300.0])
        .unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();

    // 1. Read all channels across both groups in single batch
    let all_channels: Vec<_> = file.channels().collect();
    let batched_signals = file.signals(&all_channels).unwrap();
    let seq_signals: Vec<_> = all_channels
        .iter()
        .map(|c| file.signal(c).unwrap())
        .collect();

    assert_eq!(batched_signals.len(), seq_signals.len());
    for (i, (b, s)) in batched_signals.iter().zip(&seq_signals).enumerate() {
        assert_signals_identical(b, s, &format!("all_channels[{i}]"));
    }

    // 2. Read arbitrary interleaved subset in reverse order
    let mixed_subset = vec![all_channels[3], all_channels[0], all_channels[4]];
    let batched_mixed = file.signals(&mixed_subset).unwrap();
    assert_eq!(batched_mixed.len(), 3);
    assert_eq!(batched_mixed[0].channel().name, all_channels[3].name);
    assert_eq!(batched_mixed[1].channel().name, all_channels[0].name);
    assert_eq!(batched_mixed[2].channel().name, all_channels[4].name);

    for (i, (b, ch)) in batched_mixed.iter().zip(&mixed_subset).enumerate() {
        let seq = file.signal(ch).unwrap();
        assert_signals_identical(b, &seq, &format!("mixed_subset[{i}]"));
    }

    // 3. Empty input returns empty output
    let empty: Vec<&falcon_mdf::Channel> = Vec::new();
    let empty_signals = file.signals(&empty).unwrap();
    assert!(empty_signals.is_empty());
}

#[test]
fn batched_read_matches_sequential_across_all_corpus_files() {
    let files = corpus();
    if files.is_empty() {
        eprintln!("SKIP: no test corpus found under test_data/");
        return;
    }

    let mut total_files = 0;
    let mut total_groups = 0;
    let mut total_channels_checked = 0;

    for path in &files {
        let Ok(file) = Mf4File::open(path) else {
            continue;
        };
        total_files += 1;

        for (dg_idx, dg) in file.data_groups().iter().enumerate() {
            for (cg_idx, cg) in dg.channel_groups.iter().enumerate() {
                total_groups += 1;
                let group_channels: Vec<_> = cg.channels.iter().collect();
                if group_channels.is_empty() {
                    continue;
                }

                // Batch read all channels in group
                let batch_res = file.signals(&group_channels);
                let seq_results: Vec<_> = group_channels.iter().map(|c| file.signal(c)).collect();

                match batch_res {
                    Ok(batch_signals) => {
                        assert_eq!(batch_signals.len(), group_channels.len());
                        for (i, (b_sig, seq_res)) in
                            batch_signals.iter().zip(&seq_results).enumerate()
                        {
                            total_channels_checked += 1;
                            let s_sig = seq_res
                                .as_ref()
                                .expect("sequential signal succeeded when batch succeeded");
                            assert_signals_identical(
                                b_sig,
                                s_sig,
                                &format!("{path:?} dg={dg_idx} cg={cg_idx} ch={i}"),
                            );
                        }
                    }
                    Err(batch_err) => {
                        let has_seq_err = seq_results.iter().any(|r| r.is_err());
                        assert!(
                            has_seq_err,
                            "{path:?} dg={dg_idx} cg={cg_idx}: batch returned Err({batch_err}) but all sequential succeeded"
                        );
                    }
                }
            }
        }
    }

    println!(
        "Verified batched reads across {total_files} corpus files, {total_groups} groups, {total_channels_checked} channels."
    );
    assert!(total_channels_checked > 0);
}

#[test]
fn batched_vs_sequential_wide_group_benchmark() {
    // Construct a wide group with 100 channels, 10,000 samples.
    let mut writer = Mf4Writer::new();
    let num_channels = 100;
    let num_samples = 10_000;
    let time_axis: Vec<f64> = (0..num_samples).map(|i| i as f64 * 0.001).collect();

    let g = writer.add_group(&time_axis).unwrap();
    for ch_idx in 0..num_channels {
        let data: Vec<f64> = (0..num_samples)
            .map(|s| (ch_idx * 1000 + s) as f64)
            .collect();
        g.add_channel(&format!("Channel_{ch_idx:03}"), "rpm", &data)
            .unwrap();
    }

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();

    let channels: Vec<_> = file.channels().collect();
    assert_eq!(channels.len(), num_channels + 1); // +1 for Time master

    let iters = 20;

    // 1. Signal creation timing (signals() vs looping signal())
    let t0 = Instant::now();
    for _ in 0..iters {
        let mut sigs = Vec::with_capacity(channels.len());
        for ch in &channels {
            sigs.push(file.signal(ch).unwrap());
        }
    }
    let seq_signal_dur = t0.elapsed();

    let t1 = Instant::now();
    for _ in 0..iters {
        let _sigs = file.signals(&channels).unwrap();
    }
    let batch_signal_dur = t1.elapsed();

    // 2. Full read & decode timing (open + signals() + values() vs open + loop signal() + values())
    let t2 = Instant::now();
    for _ in 0..iters {
        let f = Mf4File::open(temp.path()).unwrap();
        let chs: Vec<_> = f.channels().collect();
        for ch in &chs {
            let sig = f.signal(ch).unwrap();
            let _ = sig.values().unwrap();
        }
    }
    let seq_full_dur = t2.elapsed();

    let t3 = Instant::now();
    for _ in 0..iters {
        let f = Mf4File::open(temp.path()).unwrap();
        let chs: Vec<_> = f.channels().collect();
        let sigs = f.signals(&chs).unwrap();
        for sig in &sigs {
            let _ = sig.values().unwrap();
        }
    }
    let batch_full_dur = t3.elapsed();

    println!(
        "\n--- WIDE GROUP BENCHMARK (1 group, {} channels, {} samples, {} iters) ---",
        channels.len(),
        num_samples,
        iters
    );
    println!(
        "Signal creation:  loop signal() = {:?}, batch signals() = {:?}",
        seq_signal_dur, batch_signal_dur
    );
    if batch_signal_dur.as_nanos() > 0 {
        println!(
            "Signal creation speedup: {:.2}x",
            seq_signal_dur.as_secs_f64() / batch_signal_dur.as_secs_f64()
        );
    }
    println!(
        "Full read+decode: loop signal() = {:?}, batch signals() = {:?}",
        seq_full_dur, batch_full_dur
    );
    if batch_full_dur.as_nanos() > 0 {
        println!(
            "Full read+decode speedup: {:.2}x",
            seq_full_dur.as_secs_f64() / batch_full_dur.as_secs_f64()
        );
    }
}

#[test]
fn batched_vs_sequential_cache_thrashing_benchmark() {
    let mut writer = Mf4Writer::new();
    let num_groups = 10;
    let samples: Vec<f64> = (0..5000).map(|i| i as f64 * 0.001).collect();

    for g_idx in 0..num_groups {
        let g = writer.add_group(&samples).unwrap();
        for ch_idx in 0..5 {
            let data: Vec<f64> = (0..5000)
                .map(|s| (g_idx * 100 + ch_idx * 10 + s) as f64)
                .collect();
            g.add_channel(&format!("G{g_idx}_Ch{ch_idx}"), "unit", &data)
                .unwrap();
        }
    }

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();
    let file = Mf4File::open(temp.path()).unwrap();

    let mut interleaved_channels = Vec::new();
    for ch_idx in 0..5 {
        for g_idx in 0..num_groups {
            let name = format!("G{g_idx}_Ch{ch_idx}");
            if let Some(ch) = file.find_channel(&name) {
                interleaved_channels.push(ch);
            }
        }
    }

    assert_eq!(interleaved_channels.len(), 50);

    let iters = 10;
    let t0 = Instant::now();
    for _ in 0..iters {
        for ch in &interleaved_channels {
            let sig = file.signal(ch).unwrap();
            let _ = sig.values().unwrap();
        }
    }
    let seq_duration = t0.elapsed();

    let t1 = Instant::now();
    for _ in 0..iters {
        let sigs = file.signals(&interleaved_channels).unwrap();
        for sig in &sigs {
            let _ = sig.values().unwrap();
        }
    }
    let batch_duration = t1.elapsed();

    println!(
        "\n--- CACHE-THRASHING MULTI-GROUP BENCHMARK (10 groups, 50 channels total, {} iters) ---",
        iters
    );
    println!(
        "Sequential loop signal() (LRU thrashing): {:?}",
        seq_duration
    );
    println!(
        "Batched signals() (grouped internally):  {:?}",
        batch_duration
    );
    if batch_duration.as_nanos() > 0 {
        let ratio = seq_duration.as_secs_f64() / batch_duration.as_secs_f64();
        println!("Batched Speedup:                         {:.2}x", ratio);
    }
}
