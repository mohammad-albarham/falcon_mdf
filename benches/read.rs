//! Read-path benchmarks.
//!
//! Three things are measured separately, because they regress for different
//! reasons: opening a file (block traversal), decoding every channel into its
//! native type (record striding and bit extraction), and decoding into `f64`
//! (the extra pass that conversion costs).
//!
//! These need the sample corpus under `test_data/`, which is not checked in.
//! Without it the benchmarks report nothing rather than failing.
//!
//! Run with `cargo bench`; compare against a previous run with
//! `cargo bench -- --baseline <name>`.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use falcon_mdf::Mf4File;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Corpus files, smallest first, or empty when the corpus is absent.
fn corpus() -> Vec<PathBuf> {
    let mut found: Vec<(u64, PathBuf)> = Vec::new();
    collect(Path::new("test_data"), &mut found);
    found.sort_by_key(|(size, _)| *size);
    found.into_iter().map(|(_, p)| p).collect()
}

fn collect(dir: &Path, out: &mut Vec<(u64, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("MF4") {
            if let Ok(meta) = entry.metadata() {
                out.push((meta.len(), path));
            }
        }
    }
}

fn label(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".into())
}

/// Total samples in a file, used as the throughput unit so a change in what is
/// decoded cannot be mistaken for a change in speed.
fn sample_count(path: &Path) -> u64 {
    let Ok(file) = Mf4File::open(path) else {
        return 0;
    };
    let channels: Vec<_> = file.channels().cloned().collect();
    channels
        .iter()
        .filter_map(|ch| file.signal(ch).ok())
        .map(|s| s.len() as u64)
        .sum()
}

fn bench_open(c: &mut Criterion) {
    let files = corpus();
    if files.is_empty() {
        eprintln!("SKIP: no corpus under test_data/");
        return;
    }

    let mut group = c.benchmark_group("open");
    group.measurement_time(Duration::from_secs(5));
    for path in files.iter().take(3) {
        group.bench_with_input(BenchmarkId::from_parameter(label(path)), path, |b, path| {
            b.iter(|| Mf4File::open(path).map(|f| f.channel_count()));
        });
    }
    group.finish();
}

fn bench_read(c: &mut Criterion) {
    let files = corpus();
    if files.is_empty() {
        return;
    }

    let mut group = c.benchmark_group("read_all_native");
    group.measurement_time(Duration::from_secs(5));
    for path in files.iter().take(3) {
        let samples = sample_count(path);
        if samples == 0 {
            continue;
        }
        group.throughput(Throughput::Elements(samples));
        group.bench_with_input(BenchmarkId::from_parameter(label(path)), path, |b, path| {
            // Opening is inside the timed section deliberately: reading a file
            // once end to end is the operation being measured, and it is what a
            // caller actually does.
            b.iter(|| {
                let file = Mf4File::open(path).expect("corpus file should open");
                let channels: Vec<_> = file.channels().cloned().collect();
                let mut total = 0usize;
                for ch in &channels {
                    if let Ok(signal) = file.signal(ch) {
                        if let Ok(values) = signal.values() {
                            total += values.len();
                        }
                    }
                }
                total
            });
        });
    }
    group.finish();
}

fn bench_decode_only(c: &mut Criterion) {
    let files = corpus();
    if files.is_empty() {
        return;
    }

    // Decoding with the file already open and its records cached, which isolates
    // the extraction loop from I/O and decompression.
    let mut group = c.benchmark_group("decode_cached");
    group.measurement_time(Duration::from_secs(5));
    for path in files.iter().take(3) {
        let Ok(file) = Mf4File::open(path) else {
            continue;
        };
        let channels: Vec<_> = file.channels().cloned().collect();
        let samples = sample_count(path);
        if samples == 0 {
            continue;
        }
        group.throughput(Throughput::Elements(samples));
        group.bench_with_input(
            BenchmarkId::from_parameter(label(path)),
            &channels,
            |b, channels| {
                b.iter(|| {
                    let mut total = 0usize;
                    for ch in channels {
                        if let Ok(signal) = file.signal(ch) {
                            if let Ok(values) = signal.values() {
                                total += values.len();
                            }
                        }
                    }
                    total
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_open, bench_read, bench_decode_only);
criterion_main!(benches);
