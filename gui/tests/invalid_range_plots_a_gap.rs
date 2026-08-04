//! Acceptance test for G3's invalidation requirement: a file whose samples
//! carry an invalid range must plot a *gap*, not a line drawn through the
//! garbage bits the record holds there.
//!
//! Like `spike_survives_decimation.rs`, this exercises the real code path:
//! `Mf4File::open` parses the fixture, `Signal::values_f64` and
//! `Signal::validity` decode it exactly as `signal_loader.rs` does, and only
//! then does `decimate_min_max_gaps` (what `gui/src/panels/plot.rs` calls
//! every frame) see the data. The fixture is built byte-by-byte with the
//! same technique — and the same small builder, duplicated for the reason
//! that file's header gives.
//!
//! Invalidation in MF4: the channel group reserves `cg_inval_bytes` bytes at
//! the end of every record, and a channel whose `cn_flags` bit 1 is set
//! owns the bit at `cn_inval_bit_pos` within that area. A *set* bit marks
//! the sample invalid; `Signal::validity` inverts to `true` = valid.

use falcon_mdf::Mf4File;
use falcon_mdf_gui::decimate::decimate_min_max_gaps;

const HEADER: usize = 24;

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

fn hd_first_dg_link() -> u64 {
    (64 + HEADER) as u64
}

fn dg(cg_first: u64, data: u64) -> Vec<u8> {
    let d = vec![0u8; 8]; // rec_id_size 0: no record id, one channel group
    block(b"##DG", &[0, cg_first, data, 0], &d)
}

/// `inval_bytes` reserves that many invalidation bytes at the end of every
/// record, after the `data_bytes` of channel data.
fn cg(cn_first: u64, cycle_count: u64, data_bytes: u32, inval_bytes: u32) -> Vec<u8> {
    let mut d = vec![0u8; 32];
    d[8..16].copy_from_slice(&cycle_count.to_le_bytes());
    d[24..28].copy_from_slice(&data_bytes.to_le_bytes());
    d[28..32].copy_from_slice(&inval_bytes.to_le_bytes());
    block(b"##CG", &[0, cn_first, 0, 0, 0, 0], &d)
}

/// A float64 channel. `channel_type` 2 is Master, 0 is FixedLength.
/// `flags` bit 1 (0x0002) declares a per-sample invalidation bit, whose
/// position within the record's invalidation area is `inval_bit_pos`.
fn cn_f64(
    next: u64,
    name: u64,
    channel_type: u8,
    byte_offset: u32,
    flags: u32,
    inval_bit_pos: u32,
) -> Vec<u8> {
    let mut d = vec![0u8; 72];
    d[0] = channel_type;
    d[2] = 4; // data_type: FloatLe
    d[4..8].copy_from_slice(&byte_offset.to_le_bytes());
    d[8..12].copy_from_slice(&64u32.to_le_bytes()); // bit_count
    d[12..16].copy_from_slice(&flags.to_le_bytes());
    d[16..20].copy_from_slice(&inval_bit_pos.to_le_bytes());
    block(b"##CN", &[next, 0, name, 0, 0, 0, 0, 0], &d)
}

fn dt(records: &[u8]) -> Vec<u8> {
    block(b"##DT", &[], records)
}

const SAMPLE_COUNT: usize = 1000;
const GAP_START: usize = 400;
const GAP_END: usize = 600;
/// What the record holds where the file says "not measured". Deliberately
/// absurd next to a 0.0..1.0 signal: if a single garbage sample leaks into
/// the decimated output, it is visible in the assertion output, and it
/// would flatten the real data on screen the same way.
const GARBAGE: f64 = 1e9;
const PLOT_PIXEL_COLUMNS: usize = 100;

/// One channel group: a time master and a data channel whose samples
/// `GAP_START..GAP_END` are marked invalid. Records are 16 data bytes plus
/// one invalidation byte; the data channel owns bit 0 of it.
fn build_fixture() -> Mf4File {
    let mut records = Vec::with_capacity(SAMPLE_COUNT * 17);
    for i in 0..SAMPLE_COUNT {
        let t = i as f64 * 0.001;
        let invalid = (GAP_START..GAP_END).contains(&i);
        let v = if invalid { GARBAGE } else { t.fract() };
        records.extend_from_slice(&t.to_le_bytes());
        records.extend_from_slice(&v.to_le_bytes());
        records.push(u8::from(invalid)); // set bit = invalid
    }

    let mut f = FileBuilder::new();
    f.push(&hd());

    let master_name = f.push(&tx("Time"));
    let data_name = f.push(&tx("Gapped"));

    // Channels are pushed in reverse link order: the last channel in the
    // group's list is written first (with `next = 0`).
    let data_channel = f.push(&cn_f64(0, data_name, 0, 8, 0x0002, 0));
    let master_channel = f.push(&cn_f64(data_channel, master_name, 2, 0, 0, 0));

    let group = f.push(&cg(master_channel, SAMPLE_COUNT as u64, 16, 1));
    let data_block = f.push(&dt(&records));
    let group_block = f.push(&dg(group, data_block));
    f.patch_link(hd_first_dg_link(), group_block);

    f.open("gapped").expect("synthetic fixture should open")
}

#[test]
fn an_invalid_range_plots_a_gap_not_a_line() {
    let file = build_fixture();

    let cg = &file.data_groups()[0].channel_groups[0];
    let data_channel = cg
        .channels
        .iter()
        .find(|c| c.name == "Gapped")
        .expect("data channel is present");
    let master = file.master_channel(0, 0).expect("master channel is found");

    let signal = file.signal(data_channel).expect("signal");
    let values = signal.values_f64().expect("decode");
    let valid = signal
        .validity()
        .expect("a channel with an invalidation bit reports validity");
    let times = file
        .signal(master)
        .expect("signal")
        .values_f64()
        .expect("decode");

    assert_eq!(values.len(), SAMPLE_COUNT);
    assert_eq!(valid.len(), SAMPLE_COUNT);
    assert_eq!(values[GAP_START], GARBAGE, "fixture built correctly");
    // The file's polarity, inverted by `validity()`: valid outside the gap,
    // invalid inside it, right at both boundaries.
    assert!(valid[GAP_START - 1]);
    assert!(!valid[GAP_START]);
    assert!(!valid[GAP_END - 1]);
    assert!(valid[GAP_END]);
    assert!(
        file.signal(master).expect("signal").validity().is_none(),
        "the master has no invalidation bit, so it reports no validity info"
    );

    let x_range = (times[0], times[times.len() - 1]);
    let segments =
        decimate_min_max_gaps(&times, &values, Some(&valid), x_range, PLOT_PIXEL_COLUMNS);

    assert_eq!(segments.len(), 2, "one invalid range, two drawn segments");
    assert!(
        segments[0]
            .iter()
            .all(|p| p[0] < times[GAP_START] - f64::EPSILON),
        "first segment must end before the gap: {:?}",
        segments[0]
    );
    assert!(
        segments[1].iter().all(|p| p[0] >= times[GAP_END]),
        "second segment must start after the gap: {:?}",
        segments[1]
    );
    assert!(
        segments.iter().flatten().all(|p| p[1] != GARBAGE),
        "garbage bits must never be drawn: {segments:?}"
    );

    // The teeth check: the same samples with validity ignored come back as
    // ONE segment containing the garbage — so the assertions above really
    // are distinguishing the gapped path from a plot that draws through
    // invalid samples as if they were measured.
    let no_gaps = decimate_min_max_gaps(&times, &values, None, x_range, PLOT_PIXEL_COLUMNS);
    assert_eq!(no_gaps.len(), 1);
    assert!(
        no_gaps[0].iter().any(|p| p[1] == GARBAGE),
        "without validity the garbage is drawn, which is what the gapped \
         path must prevent"
    );
}
