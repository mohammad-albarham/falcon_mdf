//! Acceptance test for G2's decimation: a synthetic MF4 file carrying a
//! single one-sample spike must still show that spike after the whole signal
//! is squeezed down to a plot a few hundred pixels wide.
//!
//! No corpus file has a spike at a known index and amplitude — that's the
//! point of building one, rather than hunting for a plausible-looking one and
//! hoping. The fixture is built byte-by-byte, the same technique the root
//! crate's `tests/synthetic_blocks.rs` uses (its helpers are private to that
//! crate's test binary, so this duplicates the small subset needed here
//! rather than sharing code across the workspace boundary).
//!
//! This test exercises the real code path: `Mf4File::open` parses the file,
//! `Mf4File::signal` + `Signal::values_f64` decode it exactly as
//! `signal_loader.rs` does, and only then does `decimate_min_max` (the
//! function `gui/src/panels/plot.rs` calls every frame) see the data.

use falcon_mdf::Mf4File;
use falcon_mdf_gui::decimate::decimate_min_max;

const HEADER: usize = 24;

/// Assembles an MF4 file from blocks. See `tests/synthetic_blocks.rs` at the
/// repo root for the fuller version this is trimmed from.
struct FileBuilder {
    bytes: Vec<u8>,
}

impl FileBuilder {
    fn new() -> Self {
        let mut bytes = vec![0u8; 64];
        bytes[0..8].copy_from_slice(b"MDF     ");
        bytes[8..16].copy_from_slice(b"4.11    ");
        bytes[16..24].copy_from_slice(b"falcon  ");
        bytes[28..30].copy_from_slice(&411u16.to_le_bytes());
        FileBuilder { bytes }
    }

    fn next_offset(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn push(&mut self, block: &[u8]) -> u64 {
        let at = self.next_offset();
        self.bytes.extend_from_slice(block);
        at
    }

    fn patch_link(&mut self, at: u64, value: u64) {
        let at = at as usize;
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn open(&self, name: &str) -> falcon_mdf::Result<Mf4File> {
        let path = std::env::temp_dir().join(format!("falcon_mdf_gui_synth_{name}.mf4"));
        std::fs::write(&path, &self.bytes).expect("write temp file");
        let result = Mf4File::open(&path);
        let _ = std::fs::remove_file(&path);
        result
    }
}

fn block(id: &[u8; 4], links: &[u64], data: &[u8]) -> Vec<u8> {
    let total = HEADER + links.len() * 8 + data.len();
    let mut out = vec![0u8; HEADER];
    out[0..4].copy_from_slice(id);
    out[8..16].copy_from_slice(&(total as u64).to_le_bytes());
    out[16..24].copy_from_slice(&(links.len() as u64).to_le_bytes());
    for link in links {
        out.extend_from_slice(&link.to_le_bytes());
    }
    out.extend_from_slice(data);
    out
}

fn tx(text: &str) -> Vec<u8> {
    let mut data = text.as_bytes().to_vec();
    data.push(0);
    while !data.len().is_multiple_of(8) {
        data.push(0);
    }
    block(b"##TX", &[], &data)
}

fn hd() -> Vec<u8> {
    let mut data = vec![0u8; 32];
    data[0..8].copy_from_slice(&1_600_000_000_000_000_000u64.to_le_bytes());
    block(b"##HD", &[0; 6], &data)
}

/// Offset of the header's first link (first data group) within the file.
fn hd_first_dg_link() -> u64 {
    (64 + HEADER) as u64
}

fn dg(cg_first: u64, data: u64) -> Vec<u8> {
    let d = vec![0u8; 8]; // rec_id_size 0: no record id, one channel group
    block(b"##DG", &[0, cg_first, data, 0], &d)
}

fn cg(cn_first: u64, cycle_count: u64, data_bytes: u32) -> Vec<u8> {
    let mut d = vec![0u8; 32];
    d[8..16].copy_from_slice(&cycle_count.to_le_bytes());
    d[24..28].copy_from_slice(&data_bytes.to_le_bytes());
    block(b"##CG", &[0, cn_first, 0, 0, 0, 0], &d)
}

/// A float64 channel. `channel_type` 2 is Master, 0 is FixedLength.
fn cn_f64(next: u64, name: u64, channel_type: u8, byte_offset: u32) -> Vec<u8> {
    let mut d = vec![0u8; 72];
    d[0] = channel_type;
    d[2] = 4; // data_type: FloatLe
    d[4..8].copy_from_slice(&byte_offset.to_le_bytes());
    d[8..12].copy_from_slice(&64u32.to_le_bytes()); // bit_count
    block(b"##CN", &[next, 0, name, 0, 0, 0, 0, 0], &d)
}

fn dt(records: &[u8]) -> Vec<u8> {
    block(b"##DT", &[], records)
}

// Prime, so a stride sampler picking every Nth sample (N = count / columns,
// for any small integer column count) can't land on the spike by
// coincidence — the point of this test is that min/max decimation finds it
// regardless of where it falls, not that it got lucky.
const SAMPLE_COUNT: usize = 9973;
const SPIKE_INDEX: usize = 5000;
const SPIKE_VALUE: f64 = 999.0;
const PLOT_PIXEL_COLUMNS: usize = 200;

/// One channel group: a time master and a data channel, `SAMPLE_COUNT`
/// samples, flat at 0.0 except one sample forced to `SPIKE_VALUE`.
fn build_fixture() -> Mf4File {
    let mut records = Vec::with_capacity(SAMPLE_COUNT * 16);
    for i in 0..SAMPLE_COUNT {
        let t = i as f64 * 0.001;
        let v = if i == SPIKE_INDEX { SPIKE_VALUE } else { 0.0 };
        records.extend_from_slice(&t.to_le_bytes());
        records.extend_from_slice(&v.to_le_bytes());
    }

    let mut f = FileBuilder::new();
    f.push(&hd());

    let master_name = f.push(&tx("Time"));
    let data_name = f.push(&tx("Spike"));

    // Channels are pushed in reverse link order: the last channel in the
    // group's list is written first (with `next = 0`), so the one before it
    // can point at an offset that already exists.
    let data_channel = f.push(&cn_f64(0, data_name, 0, 8));
    let master_channel = f.push(&cn_f64(data_channel, master_name, 2, 0));

    let group = f.push(&cg(master_channel, SAMPLE_COUNT as u64, 16));
    let data_block = f.push(&dt(&records));
    let group_block = f.push(&dg(group, data_block));
    f.patch_link(hd_first_dg_link(), group_block);

    f.open("spike").expect("synthetic fixture should open")
}

#[test]
fn a_single_sample_spike_survives_full_zoom_out() {
    let file = build_fixture();

    let cg = &file.data_groups()[0].channel_groups[0];
    let data_channel = cg
        .channels
        .iter()
        .find(|c| c.name == "Spike")
        .expect("data channel is present");
    let master = file.master_channel(0, 0).expect("master channel is found");

    let values = file
        .signal(data_channel)
        .expect("signal")
        .values_f64()
        .expect("decode");
    let times = file
        .signal(master)
        .expect("signal")
        .values_f64()
        .expect("decode");

    assert_eq!(values.len(), SAMPLE_COUNT);
    assert_eq!(times.len(), SAMPLE_COUNT);
    assert_eq!(values[SPIKE_INDEX], SPIKE_VALUE, "fixture built correctly");

    // Full zoom-out: the whole time span in one view, decimated to roughly
    // what a plot a couple hundred pixels wide would ask for.
    let x_range = (times[0], times[times.len() - 1]);
    let decimated = decimate_min_max(&times, &values, x_range, PLOT_PIXEL_COLUMNS);

    assert!(
        decimated.len() < SAMPLE_COUNT,
        "decimation should actually reduce the point count: {} points from {SAMPLE_COUNT} samples",
        decimated.len()
    );
    assert!(
        decimated.iter().any(|p| p[1] == SPIKE_VALUE),
        "the spike at index {SPIKE_INDEX} (t={}, v={SPIKE_VALUE}) must survive full zoom-out into \
         {PLOT_PIXEL_COLUMNS} columns, but it's missing from {decimated:?}",
        times[SPIKE_INDEX],
    );
}
