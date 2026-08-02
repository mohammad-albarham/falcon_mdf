//! Robustness against malformed and hostile files.
//!
//! A measurement file arriving from a field logger, a customer, or a network
//! share is untrusted input. Reading one must fail with an error — never panic,
//! never abort the process, never fail to terminate.
//!
//! These tests craft specific malformations from a real file. They need the
//! sample corpus under `test_data/`, which is not checked in, and skip cleanly
//! when it is absent.

use falcon_mdf::error::Mf4Error;
use falcon_mdf::Mf4File;
use std::path::{Path, PathBuf};

/// Smallest corpus file, or `None` when the corpus is not present.
fn corpus_file() -> Option<PathBuf> {
    let mut found: Vec<(u64, PathBuf)> = Vec::new();
    collect(Path::new("test_data"), &mut found);
    found.sort_by_key(|(size, _)| *size);
    found.into_iter().next().map(|(_, p)| p)
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

fn read_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

fn write_u64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

/// Writes `bytes` to a temp file and opens it, returning whatever happens.
fn open_bytes(bytes: &[u8], name: &str) -> falcon_mdf::Result<Mf4File> {
    let path = std::env::temp_dir().join(format!("falcon_mdf_robustness_{name}.mf4"));
    std::fs::write(&path, bytes).expect("failed to write temp file");
    let result = Mf4File::open(&path);
    let _ = std::fs::remove_file(&path);
    result
}

/// Offsets of the block-header links this file's structures hang off.
struct Layout {
    /// Offset of the first data group block.
    dg_first: u64,
}

fn layout(bytes: &[u8]) -> Layout {
    // The HD block sits at a fixed offset 64; its links follow the 24-byte
    // block header, and hd_dg_first is the first of them.
    Layout {
        dg_first: read_u64(bytes, 64 + 24),
    }
}

macro_rules! corpus_or_skip {
    () => {
        match corpus_file() {
            Some(p) => p,
            None => {
                eprintln!("SKIP: no corpus file under test_data/");
                return;
            }
        }
    };
}

#[test]
fn a_self_referential_data_group_link_is_rejected() {
    let path = corpus_or_skip!();
    let mut bytes = std::fs::read(&path).unwrap();
    let dg = layout(&bytes).dg_first as usize;

    // DG links follow its 24-byte header; dg_next is the first.
    write_u64(&mut bytes, dg + 24, dg as u64);

    match open_bytes(&bytes, "dg_cycle") {
        Err(Mf4Error::CyclicLink { chain, .. }) => assert_eq!(chain, "dg_next"),
        Err(other) => panic!("expected a cycle error, got: {other}"),
        Ok(_) => panic!("a self-referential dg_next must not parse successfully"),
    }
}

#[test]
fn a_self_referential_channel_group_link_is_rejected() {
    let path = corpus_or_skip!();
    let mut bytes = std::fs::read(&path).unwrap();
    let dg = layout(&bytes).dg_first as usize;

    // DG links: dg_next, cg_first, data, md_comment.
    let cg = read_u64(&bytes, dg + 24 + 8) as usize;
    if cg == 0 {
        eprintln!("SKIP: corpus file has no channel groups");
        return;
    }
    // CG links: cg_next is the first after its header.
    write_u64(&mut bytes, cg + 24, cg as u64);

    match open_bytes(&bytes, "cg_cycle") {
        Err(Mf4Error::CyclicLink { chain, .. }) => assert_eq!(chain, "cg_next"),
        Err(other) => panic!("expected a cycle error, got: {other}"),
        Ok(_) => panic!("a self-referential cg_next must not parse successfully"),
    }
}

#[test]
fn a_self_referential_channel_link_is_rejected() {
    let path = corpus_or_skip!();
    let mut bytes = std::fs::read(&path).unwrap();
    let dg = layout(&bytes).dg_first as usize;
    let cg = read_u64(&bytes, dg + 24 + 8) as usize;
    if cg == 0 {
        eprintln!("SKIP: corpus file has no channel groups");
        return;
    }
    // CG links: cg_next, cn_first, tx_acq_name, si_acq_source, ...
    let cn = read_u64(&bytes, cg + 24 + 8) as usize;
    if cn == 0 {
        eprintln!("SKIP: corpus file has no channels");
        return;
    }
    write_u64(&mut bytes, cn + 24, cn as u64);

    match open_bytes(&bytes, "cn_cycle") {
        Err(Mf4Error::CyclicLink { .. }) => {}
        Err(other) => panic!("expected a cycle error, got: {other}"),
        Ok(_) => panic!("a self-referential cn_next must not parse successfully"),
    }
}

#[test]
fn truncation_at_any_length_reports_an_error() {
    let path = corpus_or_skip!();
    let bytes = std::fs::read(&path).unwrap();

    for len in [0, 1, 24, 63, 64, 65, 100, 500, 5_000, 50_000] {
        if len > bytes.len() {
            continue;
        }
        let result = open_bytes(&bytes[..len], &format!("trunc_{len}"));
        assert!(
            result.is_err(),
            "a file truncated to {len} bytes must not open successfully"
        );
    }
}

#[test]
fn an_empty_file_reports_an_error() {
    assert!(open_bytes(&[], "empty").is_err());
}

#[test]
fn a_file_of_zeros_reports_an_error() {
    assert!(open_bytes(&[0u8; 4096], "zeros").is_err());
}

#[test]
fn a_file_of_random_looking_bytes_reports_an_error() {
    // Deterministic pseudo-random fill; no valid signature, so this must be
    // rejected at the identification block.
    let mut bytes = vec![0u8; 8192];
    let mut x: u32 = 0x1234_5678;
    for b in bytes.iter_mut() {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *b = (x >> 24) as u8;
    }
    assert!(open_bytes(&bytes, "noise").is_err());
}

/// Deterministic xorshift, so a failure is always reproducible from its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn mutated_files_never_panic() {
    let path = corpus_or_skip!();
    let original = std::fs::read(&path).unwrap();

    // Concentrate on the structural region: that is where lengths, counts and
    // links live, and where a wrong byte does the most damage.
    let region = original.len().min(4096);

    let mut panicked = Vec::new();
    for seed in 1u64..=300 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut bytes = original.clone();
        for _ in 0..=rng.below(8) {
            let at = rng.below(region);
            bytes[at] = (rng.next() & 0xFF) as u8;
        }

        // A panic here would abort the test run, so catch it and report every
        // failing seed at once rather than dying on the first.
        let outcome = std::panic::catch_unwind(|| {
            let _ = open_bytes(&bytes, &format!("mutate_{seed}"));
        });
        if outcome.is_err() {
            panicked.push(seed);
        }
    }

    assert!(
        panicked.is_empty(),
        "reading a malformed file must return an error, not panic. \
         Failing seeds: {panicked:?}"
    );
}

#[test]
fn a_block_longer_than_the_file_is_rejected() {
    let path = corpus_or_skip!();
    let mut bytes = std::fs::read(&path).unwrap();
    let dg = layout(&bytes).dg_first as usize;

    // A block header declares its own length; claiming more than the file holds
    // used to reach Vec::with_capacity and abort the process.
    write_u64(&mut bytes, dg + 8, u64::MAX / 2);

    assert!(
        open_bytes(&bytes, "huge_block").is_err(),
        "a block claiming to extend past the end of the file must be rejected"
    );
}

#[test]
fn an_absurd_link_count_is_rejected() {
    let path = corpus_or_skip!();
    let mut bytes = std::fs::read(&path).unwrap();
    let dg = layout(&bytes).dg_first as usize;

    // link_count sits at header offset 16. Links live inside the block, so a
    // count that could not fit is corrupt; believing it allocated 8 bytes per
    // claimed link.
    write_u64(&mut bytes, dg + 16, u64::MAX / 8);

    assert!(
        open_bytes(&bytes, "huge_link_count").is_err(),
        "a link count that cannot fit inside the block must be rejected"
    );
}

#[test]
fn an_inflated_cycle_count_cannot_exceed_the_data() {
    let path = corpus_or_skip!();
    let mut bytes = std::fs::read(&path).unwrap();
    let dg = layout(&bytes).dg_first as usize;
    let cg = read_u64(&bytes, dg + 24 + 8) as usize;
    if cg == 0 {
        eprintln!("SKIP: corpus file has no channel groups");
        return;
    }

    // cg_cycle_count follows the CG links; a wild value here used to size every
    // read buffer. Locating it precisely is version-dependent, so overwrite a
    // window of the data section and require that whatever happens, the file
    // either fails to open or reports a sample count the data could hold.
    let data_at = cg + 24 + 6 * 8;
    for i in 0..8 {
        if data_at + 8 + i < bytes.len() {
            bytes[data_at + 8 + i] = 0xFF;
        }
    }

    if let Ok(file) = open_bytes(&bytes, "huge_cycle_count") {
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        for dg in file.data_groups() {
            for cg in &dg.channel_groups {
                assert!(
                    cg.sample_count <= size,
                    "sample count {} exceeds the whole file size {size}",
                    cg.sample_count
                );
            }
        }
    }
}
