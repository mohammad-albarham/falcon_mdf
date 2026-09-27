//! Reading through `Mf4File::from_source` with a `RangeSource` — the path a
//! browser worker or an HTTP range reader takes — must decode exactly what
//! `Mf4File::open` decodes, and must not read the whole file to do it.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use falcon_mdf::io::range::{read_seek_source, RangeSource};
use falcon_mdf::Mf4File;

fn reference_files() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_data/reference");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("mf4"))
        })
        .collect();
    files.sort();
    files
}

#[test]
fn every_reference_file_decodes_the_same_through_a_range_source() {
    let files = reference_files();
    if files.is_empty() {
        eprintln!(
            "skipping: no reference files. Run scripts/fetch_reference_files.sh to fetch them."
        );
        return;
    }

    let mut compared_channels = 0usize;
    let mut opened = 0usize;
    for path in &files {
        let Ok(direct) = Mf4File::open(path) else {
            // A file the direct path refuses must be refused the same way.
            let source = read_seek_source(File::open(path).unwrap()).unwrap();
            assert!(
                Mf4File::from_source(Arc::new(source)).is_err(),
                "{}: opens through a range source but not directly",
                path.display()
            );
            continue;
        };
        // Small windows so every file crosses many window boundaries.
        let len = std::fs::metadata(path).unwrap().len();
        let bytes = std::fs::read(path).unwrap();
        let source = RangeSource::with_cache(len, 4096, 8, move |offset, buf: &mut [u8]| {
            let start = offset as usize;
            buf.copy_from_slice(&bytes[start..start + buf.len()]);
            Ok(())
        });
        let ranged = Mf4File::from_source(Arc::new(source))
            .unwrap_or_else(|e| panic!("{}: range source open failed: {e}", path.display()));
        opened += 1;

        let a: Vec<_> = direct.channels().cloned().collect();
        let b: Vec<_> = ranged.channels().cloned().collect();
        assert_eq!(a.len(), b.len(), "{}: channel count", path.display());
        for (ca, cb) in a.iter().zip(&b) {
            assert_eq!(ca.name, cb.name, "{}: channel order", path.display());
            let va = direct.signal(ca).and_then(|s| s.values());
            let vb = ranged.signal(cb).and_then(|s| s.values());
            match (va, vb) {
                (Ok(x), Ok(y)) => {
                    // Through Debug, so a NaN sample compares equal to itself.
                    assert!(
                        format!("{x:?}") == format!("{y:?}"),
                        "{}: channel {} differs",
                        path.display(),
                        ca.name
                    );
                    compared_channels += 1;
                }
                (Err(_), Err(_)) => {}
                (x, y) => panic!(
                    "{}: channel {} decodes one way only: direct {:?}, ranged {:?}",
                    path.display(),
                    ca.name,
                    x.is_ok(),
                    y.is_ok()
                ),
            }
        }
    }
    eprintln!("compared {compared_channels} channels across {opened} files");
    assert!(opened > 0 && compared_channels > 0);
}

/// Opening a file and listing its channels walks blocks, not data. Through a
/// range source that must cost a small fraction of the file — the point of
/// reading a multi-gigabyte log in a browser without loading it. On the
/// 480 MiB fixture it is about 1.6%: one 64 KiB window per data-block header,
/// and the headers are spread through the file.
#[test]
fn opening_a_large_file_fetches_a_small_fraction_of_it() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_data/large/large_uncompressed.mf4");
    if !path.is_file() {
        eprintln!(
            "skipping: no large fixture (see the perf-benchmark skill's make_large_fixture.py)"
        );
        return;
    }
    let len = std::fs::metadata(&path).unwrap().len();
    let fetched = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&fetched);
    let file = std::sync::Mutex::new(File::open(&path).unwrap());
    let source = RangeSource::new(len, move |offset, buf: &mut [u8]| {
        use std::io::{Read, Seek, SeekFrom};
        counter.fetch_add(buf.len() as u64, Ordering::SeqCst);
        let mut f = file.lock().unwrap();
        f.seek(SeekFrom::Start(offset))?;
        f.read_exact(buf)?;
        Ok(())
    });

    let mdf = Mf4File::from_source(Arc::new(source)).expect("open through a range source");
    let channels = mdf.channels().count();
    let read = fetched.load(Ordering::SeqCst);
    eprintln!(
        "{channels} channels listed after fetching {read} of {len} bytes ({:.3}%)",
        read as f64 * 100.0 / len as f64
    );
    assert!(channels > 0);
    assert!(
        read * 20 < len,
        "opening fetched {read} of {len} bytes — more than 5% of the file"
    );
}
