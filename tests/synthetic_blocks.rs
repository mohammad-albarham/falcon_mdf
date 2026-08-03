//! End-to-end tests for blocks no available file contains.
//!
//! Attachments, events and the file-history chain are parsed and surfaced, but
//! the sample corpus has none of them — so until now nothing read one from an
//! actual file. A block parser passing its own unit tests only proves it agrees
//! with the fixture beside it; it does not prove the reader reaches the block,
//! follows its links, or hands back what it found.
//!
//! These tests build complete MF4 files containing those blocks and read them
//! through the public API, which closes that gap without needing a vendor file.

use falcon_mdf::{Mf4File, ReductionKind, SignalValues};

const HEADER: usize = 24;

/// Assembles an MF4 file from blocks, tracking where each lands.
///
/// Blocks reference one another by absolute file offset, so a builder has to
/// know an offset before the block that points at it can be written. Space is
/// reserved first, then filled in.
struct FileBuilder {
    bytes: Vec<u8>,
}

impl FileBuilder {
    /// Starts a file with a valid identification block and nothing else.
    fn new() -> Self {
        let mut bytes = vec![0u8; 64];
        bytes[0..8].copy_from_slice(b"MDF     ");
        bytes[8..16].copy_from_slice(b"4.11    ");
        bytes[16..24].copy_from_slice(b"falcon  ");
        bytes[28..30].copy_from_slice(&411u16.to_le_bytes());
        FileBuilder { bytes }
    }

    /// Returns the offset the next block will start at.
    fn next_offset(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// Appends a block and returns where it began.
    fn push(&mut self, block: &[u8]) -> u64 {
        let at = self.next_offset();
        self.bytes.extend_from_slice(block);
        at
    }

    /// Overwrites eight bytes, for filling in a link once its target is placed.
    fn patch_link(&mut self, at: u64, value: u64) {
        let at = at as usize;
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    /// Writes the file and opens it.
    fn open(&self, name: &str) -> falcon_mdf::Result<Mf4File> {
        let path = std::env::temp_dir().join(format!("falcon_mdf_synth_{name}.mf4"));
        std::fs::write(&path, &self.bytes).expect("write temp file");
        let result = Mf4File::open(&path);
        let _ = std::fs::remove_file(&path);
        result
    }
}

/// Builds a block: four-character id, links, then the data section.
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

/// A text block holding `text`.
fn tx(text: &str) -> Vec<u8> {
    let mut data = text.as_bytes().to_vec();
    data.push(0);
    while data.len() % 8 != 0 {
        data.push(0);
    }
    block(b"##TX", &[], &data)
}

/// A metadata block holding `xml`.
fn md(xml: &str) -> Vec<u8> {
    let mut data = xml.as_bytes().to_vec();
    data.push(0);
    while data.len() % 8 != 0 {
        data.push(0);
    }
    block(b"##MD", &[], &data)
}

/// A minimal header block. Its links are patched once the targets are placed.
///
/// The data section is 32 bytes: start time, timezone and daylight offsets,
/// time flags and class, header flags, a reserved byte, and the start angle and
/// distance.
fn hd() -> Vec<u8> {
    let mut data = vec![0u8; 32];
    data[0..8].copy_from_slice(&1_600_000_000_000_000_000u64.to_le_bytes());
    block(b"##HD", &[0; 6], &data)
}

/// Offset of the header's `n`th link within the file.
///
/// The header's links are, in order: first data group, first file-history
/// entry, first channel-hierarchy node, first attachment, first event, and the
/// comment.
fn hd_link(n: usize) -> u64 {
    (64 + HEADER + n * 8) as u64
}

/// Link indices within the header block.
const HD_FH: usize = 1;
const HD_AT: usize = 3;
const HD_EV: usize = 4;

/// A file-history entry pointing at `comment`.
fn fh(next: u64, comment: u64, time_ns: u64) -> Vec<u8> {
    let mut data = vec![0u8; 16];
    data[0..8].copy_from_slice(&time_ns.to_le_bytes());
    block(b"##FH", &[next, comment], &data)
}

#[test]
fn an_embedded_attachment_is_found_and_its_bytes_read_back() {
    // The payload is what a caller ultimately wants; everything else is
    // bookkeeping that exists to locate it.
    let payload: Vec<u8> = (0u8..=255).collect();

    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("sensor-config.json"));
    let mime = f.push(&tx("application/json"));
    let fh_comment = f.push(&md(
        "<FHcomment><TX>created</TX><tool_id>test</tool_id></FHcomment>",
    ));
    let history = f.push(&fh(0, fh_comment, 1_600_000_000_000_000_000));

    // AT data: flags (embedded), creator, four reserved, 16-byte checksum, then
    // the original and embedded sizes, followed by the bytes themselves.
    let mut at_data = Vec::new();
    at_data.extend_from_slice(&1u16.to_le_bytes()); // embedded
    at_data.extend_from_slice(&0u16.to_le_bytes()); // creator index
    at_data.extend_from_slice(&0u32.to_le_bytes()); // reserved
    at_data.extend_from_slice(&[0u8; 16]); // checksum
    at_data.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    at_data.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    at_data.extend_from_slice(&payload);
    let attachment = f.push(&block(b"##AT", &[0, name, mime, 0], &at_data));

    f.patch_link(hd_link(HD_FH), history);
    f.patch_link(hd_link(HD_AT), attachment);

    let file = f.open("attachment").expect("synthetic file should open");

    let attachments = file.attachments();
    assert_eq!(attachments.len(), 1, "the attachment was not found");

    let at = &attachments[0];
    assert_eq!(at.file_name, "sensor-config.json");
    assert!(at.is_embedded);
    assert_eq!(at.original_size, payload.len() as u64);

    let read_back = file
        .attachment_data(at)
        .expect("reading embedded data should not fail")
        .expect("an embedded attachment should yield its bytes");
    assert_eq!(
        read_back, payload,
        "the bytes read back are not the bytes written"
    );
}

#[test]
fn an_external_attachment_reports_no_embedded_bytes() {
    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("recording.avi"));

    let mut at_data = Vec::new();
    at_data.extend_from_slice(&0u16.to_le_bytes()); // not embedded
    at_data.extend_from_slice(&0u16.to_le_bytes());
    at_data.extend_from_slice(&0u32.to_le_bytes());
    at_data.extend_from_slice(&[0u8; 16]);
    at_data.extend_from_slice(&4096u64.to_le_bytes()); // original size
    at_data.extend_from_slice(&0u64.to_le_bytes()); // nothing embedded
    let attachment = f.push(&block(b"##AT", &[0, name, 0, 0], &at_data));

    f.patch_link(hd_link(HD_AT), attachment);

    let file = f.open("attachment_external").expect("should open");
    let at = &file.attachments()[0];

    assert!(!at.is_embedded);
    assert_eq!(at.original_size, 4096);
    assert_eq!(
        file.attachment_data(at).unwrap(),
        None,
        "an external attachment has no bytes in this file"
    );
}

#[test]
fn a_chain_of_attachments_is_followed_to_the_end() {
    let mut f = FileBuilder::new();
    f.push(&hd());

    let names: Vec<u64> = ["first.bin", "second.bin", "third.bin"]
        .iter()
        .map(|n| f.push(&tx(n)))
        .collect();

    // Built back to front, so each block knows the offset of its successor.
    let mut next = 0u64;
    for name in names.iter().rev() {
        let mut data = vec![0u8; 40];
        data[24..32].copy_from_slice(&8u64.to_le_bytes());
        next = f.push(&block(b"##AT", &[next, *name, 0, 0], &data));
    }
    f.patch_link(hd_link(HD_AT), next);

    let file = f.open("attachment_chain").expect("should open");
    let found: Vec<&str> = file
        .attachments()
        .iter()
        .map(|a| a.file_name.as_str())
        .collect();

    assert_eq!(
        found,
        ["first.bin", "second.bin", "third.bin"],
        "the whole chain should be walked, in order"
    );
}

#[test]
fn events_are_read_with_their_position_and_comment() {
    let mut f = FileBuilder::new();
    f.push(&hd());

    let comment = f.push(&md("<EVcomment><TX>brake applied</TX></EVcomment>"));

    // EV data: type, sync type, range type, cause, flags, three reserved, a
    // scope count, two counts, then the base value and factor.
    let mut ev_data = Vec::new();
    ev_data.extend_from_slice(&[4u8, 0, 0, 4, 0, 0, 0, 0]);
    ev_data.extend_from_slice(&0u32.to_le_bytes()); // scope count
    ev_data.extend_from_slice(&0u16.to_le_bytes()); // attachments
    ev_data.extend_from_slice(&0u16.to_le_bytes()); // creator
    ev_data.extend_from_slice(&2_500_000_000i64.to_le_bytes());
    ev_data.extend_from_slice(&1e-9f64.to_le_bytes());
    let event = f.push(&block(b"##EV", &[0, 0, 0, comment], &ev_data));

    f.patch_link(hd_link(HD_EV), event);

    let file = f.open("event").expect("should open");
    let events = file.events();
    assert_eq!(events.len(), 1, "the event was not found");

    let ev = &events[0];
    assert_eq!(ev.comment, "brake applied");
    assert_eq!(ev.sync_base_value, 2_500_000_000);
    assert_eq!(ev.sync_factor, 1e-9);
    assert!(
        (ev.position() - 2.5).abs() < 1e-12,
        "position should be base times factor, got {}",
        ev.position()
    );
}

#[test]
fn the_file_history_chain_is_read_in_order() {
    let mut f = FileBuilder::new();
    f.push(&hd());

    let created = f.push(&md(
        "<FHcomment><TX>created</TX><tool_id>writer</tool_id></FHcomment>",
    ));
    let edited = f.push(&md(
        "<FHcomment><TX>edited</TX><tool_id>editor</tool_id></FHcomment>",
    ));

    let second = f.push(&fh(0, edited, 2_000_000_000_000_000_000));
    let first = f.push(&fh(second, created, 1_000_000_000_000_000_000));
    f.patch_link(hd_link(HD_FH), first);

    let file = f.open("history").expect("should open");
    let history = file.file_history();

    assert_eq!(history.len(), 2);
    assert_eq!(history[0].comment, "created");
    assert_eq!(history[0].tool_id(), Some("writer"));
    assert_eq!(history[0].time.timestamp_ns, 1_000_000_000_000_000_000);
    assert_eq!(history[1].comment, "edited");
    assert_eq!(history[1].tool_id(), Some("editor"));
}

#[test]
fn a_cycle_in_an_attachment_chain_is_rejected() {
    // The same guard that protects the data-group and channel chains has to
    // cover these too, or a crafted file loops forever here instead.
    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("looping.bin"));

    let mut data = vec![0u8; 40];
    data[24..32].copy_from_slice(&8u64.to_le_bytes());
    let at = f.push(&block(b"##AT", &[0, name, 0, 0], &data));

    // Point the attachment's next link at itself.
    f.patch_link(at + HEADER as u64, at);
    f.patch_link(hd_link(HD_AT), at);

    assert!(
        f.open("attachment_cycle").is_err(),
        "a self-referential attachment chain must be rejected"
    );
}

/// A data group block. Links: next, first channel group, data, comment.
fn dg(next: u64, cg_first: u64, data: u64, rec_id_size: u8) -> Vec<u8> {
    let mut d = vec![0u8; 8];
    d[0] = rec_id_size;
    block(b"##DG", &[next, cg_first, data, 0], &d)
}

/// A channel group block. Links: next, first channel, acquisition name,
/// acquisition source, first sample reduction, comment.
fn cg(cn_first: u64, cycle_count: u64, data_bytes: u32) -> Vec<u8> {
    let mut d = vec![0u8; 32];
    d[8..16].copy_from_slice(&cycle_count.to_le_bytes());
    d[24..28].copy_from_slice(&data_bytes.to_le_bytes());
    block(b"##CG", &[0, cn_first, 0, 0, 0, 0], &d)
}

/// A channel block. Links: next, composition, name, source, conversion, data,
/// unit, comment.
#[allow(clippy::too_many_arguments)]
fn cn(
    next: u64,
    composition: u64,
    name: u64,
    channel_type: u8,
    data_type: u8,
    byte_offset: u32,
    bit_count: u32,
) -> Vec<u8> {
    let mut d = vec![0u8; 72];
    d[0] = channel_type;
    d[2] = data_type;
    d[4..8].copy_from_slice(&byte_offset.to_le_bytes());
    d[8..12].copy_from_slice(&bit_count.to_le_bytes());
    block(b"##CN", &[next, composition, name, 0, 0, 0, 0, 0], &d)
}

/// A channel array block describing a one-dimensional array of `len` elements.
fn ca(template_cn: u64, len: u64, element_bytes: i32) -> Vec<u8> {
    let mut d = Vec::new();
    d.push(0u8); // ca_type = Array
    d.push(0u8); // ca_storage = CN template: elements adjacent in the record
    d.extend_from_slice(&1u16.to_le_bytes()); // one dimension
    d.extend_from_slice(&0u32.to_le_bytes()); // flags
    d.extend_from_slice(&element_bytes.to_le_bytes()); // byte offset base
    d.extend_from_slice(&0u32.to_le_bytes()); // invalidation bit base
    d.extend_from_slice(&len.to_le_bytes()); // ca_dim_size[0]
                                             // Links: composition, then one scale axis per dimension.
    block(b"##CA", &[template_cn, 0], &d)
}

/// A data block holding raw records.
fn dt(records: &[u8]) -> Vec<u8> {
    block(b"##DT", &[], records)
}

#[test]
fn an_array_channel_decodes_to_its_elements() {
    // Three f64 elements per sample, two samples — the shape a vector-valued
    // signal such as a three-axis accelerometer takes.
    let samples: [[f64; 3]; 2] = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]];
    let mut records = Vec::new();
    for s in &samples {
        for v in s {
            records.extend_from_slice(&v.to_le_bytes());
        }
    }

    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Acceleration"));
    let template = f.push(&cn(0, 0, 0, 0, 4, 0, 64)); // one f64 element
    let array = f.push(&ca(template, 3, 8));
    let channel = f.push(&cn(0, array, name, 0, 4, 0, 64));
    let group = f.push(&cg(channel, samples.len() as u64, 24));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));

    f.patch_link(hd_link(0), group_block);

    let file = f.open("array").expect("synthetic file should open");
    let ch = file
        .find_channel("Acceleration")
        .expect("the array channel should be listed");

    assert_eq!(
        ch.array_shape(),
        Some(&[3u64][..]),
        "the array's shape should come from the CA block"
    );
    assert!(
        ch.unreadable().is_none(),
        "an array channel with a template should be readable"
    );

    let values = file
        .signal(ch)
        .expect("signal")
        .values_f64()
        .expect("an array channel should decode");

    assert_eq!(
        values,
        vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        "elements should come back flattened, sample by sample"
    );
}

#[test]
fn an_array_without_a_template_stays_unreadable() {
    // Without the template channel describing one element, there is nothing to
    // say how wide an element is — decoding would be guesswork.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Mystery"));
    let array = f.push(&ca(0, 4, 8)); // no template
    let channel = f.push(&cn(0, array, name, 0, 4, 0, 64));
    let group = f.push(&cg(channel, 1, 32));
    let data = f.push(&dt(&[0u8; 32]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("array_no_template").expect("should open");
    let ch = file
        .find_channel("Mystery")
        .expect("channel should be listed");

    assert!(
        ch.unreadable().is_some(),
        "an array with no element template must not be decoded"
    );
    assert!(
        file.signal(ch).and_then(|s| s.values()).is_err(),
        "reading it must fail rather than return part of the data"
    );
}

/// A sample-reduction block. Two links: next level, and the reduction data.
fn sr(next: u64, data: u64, cycle_count: u64, interval: f64, sync_type: u8) -> Vec<u8> {
    let mut d = vec![0u8; 24];
    d[0..8].copy_from_slice(&cycle_count.to_le_bytes());
    d[8..16].copy_from_slice(&interval.to_le_bytes());
    d[16] = sync_type;
    block(b"##SR", &[next, data], &d)
}

/// A channel group with a sample-reduction chain.
fn cg_with_reductions(cn_first: u64, sr_first: u64, cycle_count: u64, data_bytes: u32) -> Vec<u8> {
    let mut d = vec![0u8; 32];
    d[8..16].copy_from_slice(&cycle_count.to_le_bytes());
    d[24..28].copy_from_slice(&data_bytes.to_le_bytes());
    block(b"##CG", &[0, cn_first, 0, 0, sr_first, 0], &d)
}

#[test]
fn sample_reduction_levels_are_listed_with_their_parameters() {
    // A group may carry several reductions, each condensing a longer interval
    // than the last. The descriptors are readable; the reduced values are not,
    // and that distinction is the point of this test.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Speed"));
    let channel = f.push(&cn(0, 0, name, 0, 4, 0, 64));

    // Built back to front so each level knows its successor's offset.
    let coarse = f.push(&sr(0, 0, 10, 1.0, 0));
    let fine = f.push(&sr(coarse, 0, 100, 0.1, 0));

    let group = f.push(&cg_with_reductions(channel, fine, 1000, 8));
    let data = f.push(&dt(&[0u8; 8000]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("sample_reduction").expect("should open");
    let cg = &file.data_groups()[0].channel_groups[0];
    let levels = cg.sample_reductions();

    assert_eq!(levels.len(), 2, "both reduction levels should be found");

    assert_eq!(levels[0].cycle_count, 100);
    assert_eq!(levels[0].interval, 0.1);
    assert_eq!(levels[1].cycle_count, 10);
    assert_eq!(levels[1].interval, 1.0);

    // The group's own data is unaffected by the presence of reductions.
    assert_eq!(cg.sample_count, 1000);
}

#[test]
fn a_group_without_reductions_reports_none() {
    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("Speed"));
    let channel = f.push(&cn(0, 0, name, 0, 4, 0, 64));
    let group = f.push(&cg(channel, 2, 8));
    let data = f.push(&dt(&[0u8; 16]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("no_reduction").expect("should open");
    assert!(file.data_groups()[0].channel_groups[0]
        .sample_reductions()
        .is_empty());
}

#[test]
fn a_cycle_in_a_reduction_chain_is_rejected() {
    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("Speed"));
    let channel = f.push(&cn(0, 0, name, 0, 4, 0, 64));

    let level = f.push(&sr(0, 0, 10, 1.0, 0));
    // Point the level's next link at itself.
    f.patch_link(level + HEADER as u64, level);

    let group = f.push(&cg_with_reductions(channel, level, 10, 8));
    let data = f.push(&dt(&[0u8; 80]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    assert!(
        f.open("reduction_cycle").is_err(),
        "a self-referential reduction chain must be rejected"
    );
}

#[test]
fn reduced_values_are_read_from_the_right_third_of_each_record() {
    // A reduced record holds three copies of the group's normal record: the
    // means, then the minima, then the maxima. Reading the wrong third returns
    // real numbers from the wrong series, which is worse than failing.
    let means = [10.0f64, 20.0];
    let mins = [1.0f64, 2.0];
    let maxes = [100.0f64, 200.0];

    let mut reduced = Vec::new();
    for i in 0..2 {
        reduced.extend_from_slice(&means[i].to_le_bytes());
        reduced.extend_from_slice(&mins[i].to_le_bytes());
        reduced.extend_from_slice(&maxes[i].to_le_bytes());
    }

    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Speed"));
    let channel = f.push(&cn(0, 0, name, 0, 4, 0, 64));
    let reduction_data = f.push(&block(b"##RD", &[], &reduced));
    let level = f.push(&sr(0, reduction_data, 2, 1.0, 0));
    let group = f.push(&cg_with_reductions(channel, level, 100, 8));
    let data = f.push(&dt(&[0u8; 800]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("reduced_values").expect("should open");
    let ch = file.find_channel("Speed").expect("channel");
    let cg = &file.data_groups()[0].channel_groups[0];
    let level = &cg.sample_reductions()[0];

    let read = |kind| {
        file.reduced_signal(ch, level, kind)
            .expect("reduced signal")
            .values_f64()
            .expect("values")
    };

    assert_eq!(read(ReductionKind::Mean), vec![10.0, 20.0]);
    assert_eq!(read(ReductionKind::Min), vec![1.0, 2.0]);
    assert_eq!(read(ReductionKind::Max), vec![100.0, 200.0]);
}

#[test]
fn a_reduction_naming_no_data_fails_rather_than_returning_nothing() {
    let mut f = FileBuilder::new();
    f.push(&hd());
    let name = f.push(&tx("Speed"));
    let channel = f.push(&cn(0, 0, name, 0, 4, 0, 64));
    let level = f.push(&sr(0, 0, 4, 1.0, 0)); // no data link
    let group = f.push(&cg_with_reductions(channel, level, 10, 8));
    let data = f.push(&dt(&[0u8; 80]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("reduction_no_data").expect("should open");
    let ch = file.find_channel("Speed").unwrap();
    let level = &file.data_groups()[0].channel_groups[0].sample_reductions()[0];

    assert!(
        file.reduced_signal(ch, level, ReductionKind::Mean).is_err(),
        "a reduction with no data block must fail, not report zero samples"
    );
}

/// A channel block carrying a conversion link, which `cn` leaves empty.
#[allow(clippy::too_many_arguments)]
fn cn_converted(
    next: u64,
    name: u64,
    conversion: u64,
    channel_type: u8,
    data_type: u8,
    byte_offset: u32,
    bit_count: u32,
) -> Vec<u8> {
    let mut d = vec![0u8; 72];
    d[0] = channel_type;
    d[2] = data_type;
    d[4..8].copy_from_slice(&byte_offset.to_le_bytes());
    d[8..12].copy_from_slice(&bit_count.to_le_bytes());
    block(b"##CN", &[next, 0, name, 0, conversion, 0, 0, 0], &d)
}

/// A conversion block. Links after the four fixed ones are `cc_ref`; `values`
/// are the `cc_val` parameters.
fn cc(conversion_type: u8, references: &[u64], values: &[f64]) -> Vec<u8> {
    let mut d = Vec::new();
    d.push(conversion_type);
    d.push(0); // precision
    d.extend_from_slice(&0u16.to_le_bytes()); // flags
    d.extend_from_slice(&(references.len() as u16).to_le_bytes()); // cc_ref_count
    d.extend_from_slice(&(values.len() as u16).to_le_bytes()); // cc_val_count
    d.extend_from_slice(&0f64.to_le_bytes()); // min physical
    d.extend_from_slice(&0f64.to_le_bytes()); // max physical
    for v in values {
        d.extend_from_slice(&v.to_le_bytes());
    }

    let mut links = vec![0u64; 4];
    links.extend_from_slice(references);
    block(b"##CC", &links, &d)
}

/// Packs each string into a fixed-width, NUL-padded record field.
fn text_records(samples: &[&str], width: usize) -> Vec<u8> {
    let mut out = vec![0u8; samples.len() * width];
    for (slot, s) in out.chunks_exact_mut(width).zip(samples) {
        let bytes = s.as_bytes();
        slot[..bytes.len()].copy_from_slice(bytes);
    }
    out
}

#[test]
fn a_text_to_value_conversion_maps_each_string_to_its_number() {
    // MF4 type 9. Every cc_ref is a key; the default is the *last cc_val*,
    // there being no default link — which is what distinguishes its layout
    // from the value-to-text table of type 7.
    let width = 8;
    let records = text_records(&["off", "on", "off", "unknown"], width);

    let mut f = FileBuilder::new();
    f.push(&hd());

    let key_off = f.push(&tx("off"));
    let key_on = f.push(&tx("on"));
    let conv = f.push(&cc(9, &[key_off, key_on], &[0.0, 1.0, -1.0]));

    let name = f.push(&tx("Ignition"));
    let channel = f.push(&cn_converted(0, name, conv, 0, 6, 0, (width * 8) as u32));
    let group = f.push(&cg(channel, 4, width as u32));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("ttab").expect("synthetic file should open");
    let ch = file
        .find_channel("Ignition")
        .expect("channel should be listed");

    let values = file
        .signal(ch)
        .expect("signal")
        .values_f64()
        .expect("a text-to-value channel should decode");

    // The fourth sample matches no key and takes the trailing default.
    assert_eq!(values, vec![0.0, 1.0, 0.0, -1.0]);
}

#[test]
fn a_text_to_value_table_without_a_default_yields_nan_for_an_unknown_key() {
    let width = 8;
    let records = text_records(&["on", "missing"], width);

    let mut f = FileBuilder::new();
    f.push(&hd());

    let key_on = f.push(&tx("on"));
    let conv = f.push(&cc(9, &[key_on], &[1.0]));

    let name = f.push(&tx("Ignition"));
    let channel = f.push(&cn_converted(0, name, conv, 0, 6, 0, (width * 8) as u32));
    let group = f.push(&cg(channel, 2, width as u32));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f
        .open("ttab_nodefault")
        .expect("synthetic file should open");
    let ch = file
        .find_channel("Ignition")
        .expect("channel should be listed");
    let values = file
        .signal(ch)
        .expect("signal")
        .values_f64()
        .expect("decode");

    assert_eq!(values[0], 1.0);
    assert!(
        values[1].is_nan(),
        "an unmatched key with no default has no value, not zero: {}",
        values[1]
    );
}

#[test]
fn a_text_to_text_conversion_translates_each_string() {
    // MF4 type 10. References alternate key, replacement, key, replacement,
    // with one default at the end — so the count is always odd.
    let width = 8;
    let records = text_records(&["ok", "err", "other"], width);

    let mut f = FileBuilder::new();
    f.push(&hd());

    let in_ok = f.push(&tx("ok"));
    let out_ok = f.push(&tx("Healthy"));
    let in_err = f.push(&tx("err"));
    let out_err = f.push(&tx("Faulted"));
    let default = f.push(&tx("Unrecognised"));
    let conv = f.push(&cc(10, &[in_ok, out_ok, in_err, out_err, default], &[]));

    let name = f.push(&tx("Status"));
    let channel = f.push(&cn_converted(0, name, conv, 0, 6, 0, (width * 8) as u32));
    let group = f.push(&cg(channel, 3, width as u32));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("trans").expect("synthetic file should open");
    let ch = file
        .find_channel("Status")
        .expect("channel should be listed");

    assert_eq!(
        ch.value_kind(),
        falcon_mdf::ValueKind::Str,
        "a text-to-text conversion still produces text"
    );

    match file.signal(ch).expect("signal").values().expect("decode") {
        falcon_mdf::SignalValues::Str(v) => {
            assert_eq!(v, vec!["Healthy", "Faulted", "Unrecognised"]);
        }
        other => panic!("expected strings, got {}", other.kind()),
    }
}

#[test]
fn a_text_to_text_table_with_an_even_reference_count_is_rejected() {
    // Pairs plus one default is always odd. An even count means a replacement
    // or the default is missing, and guessing which would silently mistranslate
    // every sample after the gap.
    let width = 8;
    let mut f = FileBuilder::new();
    f.push(&hd());

    let in_ok = f.push(&tx("ok"));
    let out_ok = f.push(&tx("Healthy"));
    let conv = f.push(&cc(10, &[in_ok, out_ok], &[]));

    let name = f.push(&tx("Status"));
    let channel = f.push(&cn_converted(0, name, conv, 0, 6, 0, (width * 8) as u32));
    let group = f.push(&cg(channel, 1, width as u32));
    let data = f.push(&dt(&text_records(&["ok"], width)));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("trans_even").expect("synthetic file should open");
    let ch = file
        .find_channel("Status")
        .expect("channel should be listed");
    assert!(
        file.signal(ch).expect("signal").values().is_err(),
        "a malformed text-to-text table must fail rather than translate half of it"
    );
}

/// A bitfield conversion block. Unlike every other conversion type, its
/// `cc_val` parameters are unsigned integers rather than doubles.
fn cc_bitfield(references: &[u64], masks: &[u64]) -> Vec<u8> {
    let mut d = Vec::new();
    d.push(11u8);
    d.push(0); // precision
    d.extend_from_slice(&0u16.to_le_bytes()); // flags
    d.extend_from_slice(&(references.len() as u16).to_le_bytes());
    d.extend_from_slice(&(masks.len() as u16).to_le_bytes());
    d.extend_from_slice(&0f64.to_le_bytes());
    d.extend_from_slice(&0f64.to_le_bytes());
    for m in masks {
        d.extend_from_slice(&m.to_le_bytes());
    }

    let mut links = vec![0u64; 4];
    links.extend_from_slice(references);
    block(b"##CC", &links, &d)
}

/// A conversion block carrying a name, which `cc` leaves empty.
fn cc_named(conversion_type: u8, name: u64, references: &[u64], values: &[f64]) -> Vec<u8> {
    let mut b = cc(conversion_type, references, values);
    // The name is the first of the four fixed links, immediately after the
    // 24-byte block header.
    b[24..32].copy_from_slice(&name.to_le_bytes());
    b
}

#[test]
fn a_bitfield_conversion_renders_each_field_of_a_status_word() {
    // MF4 type 11: each entry masks the raw value and renders the result, and
    // a nested table's name labels its part. A gearbox status word packing the
    // gear in the low nibble and a clutch flag above it is the shape this
    // conversion exists for.
    let samples: [u16; 3] = [0x0011, 0x0002, 0x0005];
    let mut records = Vec::new();
    for v in &samples {
        records.extend_from_slice(&v.to_le_bytes());
    }

    let mut f = FileBuilder::new();
    f.push(&hd());

    // Nested value-to-text table, keyed by the *masked* value.
    let first = f.push(&tx("first"));
    let second = f.push(&tx("second"));
    let unknown = f.push(&tx("unknown"));
    let gear_name = f.push(&tx("gear"));
    let gear_table = f.push(&cc_named(
        7,
        gear_name,
        &[first, second, unknown],
        &[1.0, 2.0],
    ));

    // A bare text reference is a flag's label.
    let clutch = f.push(&tx("clutch"));

    let conv = f.push(&cc_bitfield(&[gear_table, clutch], &[0x000F, 0x0010]));

    let name = f.push(&tx("GearboxStatus"));
    let channel = f.push(&cn_converted(0, name, conv, 0, 0, 0, 16));
    let group = f.push(&cg(channel, samples.len() as u64, 2));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("bitfield").expect("synthetic file should open");
    let ch = file
        .find_channel("GearboxStatus")
        .expect("channel should be listed");

    match file.signal(ch).expect("signal").values().expect("decode") {
        falcon_mdf::SignalValues::Str(v) => {
            assert_eq!(
                v,
                vec![
                    // gear 1 and the clutch bit set
                    "gear = first | clutch",
                    // gear 2, clutch clear: the flag contributes nothing
                    "gear = second",
                    // gear 5 matches no key, so the nested table's default applies
                    "gear = unknown",
                ]
            );
        }
        other => panic!("expected strings, got {}", other.kind()),
    }
}

#[test]
fn a_bitfield_referencing_itself_is_rejected_rather_than_recursed() {
    // A `cc_ref` pointing back at its own block would recurse until the stack
    // ran out. The depth limit turns that into an error.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let conv = f.next_offset();
    f.push(&cc_bitfield(&[conv], &[0xFF]));

    let name = f.push(&tx("Status"));
    let channel = f.push(&cn_converted(0, name, conv, 0, 0, 0, 16));
    let group = f.push(&cg(channel, 1, 2));
    let data = f.push(&dt(&[0u8; 2]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    // Either the file refuses to open or the channel refuses to decode; what
    // matters is that neither hangs nor overflows the stack.
    if let Ok(file) = f.open("bitfield_cycle") {
        if let Some(ch) = file.find_channel("Status") {
            let _ = file.signal(ch).and_then(|s| s.values());
        }
    }
}

/// A channel array block using a storage form whose elements live outside the
/// record — one channel group per element.
fn ca_cg_template(template_cn: u64, len: u64, element_bytes: i32) -> Vec<u8> {
    let mut d = Vec::new();
    d.push(0u8); // ca_type = Array
    d.push(1u8); // ca_storage = CG template
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&0u32.to_le_bytes());
    d.extend_from_slice(&element_bytes.to_le_bytes());
    d.extend_from_slice(&0u32.to_le_bytes());
    d.extend_from_slice(&len.to_le_bytes());
    block(b"##CA", &[template_cn, 0], &d)
}

#[test]
fn an_array_stored_one_group_per_element_stays_unreadable() {
    // Only the CN-template form keeps a sample's elements adjacent in the
    // record. Decoding a CG-template array with the same striding would return
    // whatever bytes happen to follow the channel — plausible-looking numbers
    // that are not the array.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Acceleration"));
    let template = f.push(&cn(0, 0, 0, 0, 4, 0, 64));
    let array = f.push(&ca_cg_template(template, 3, 8));
    let channel = f.push(&cn(0, array, name, 0, 4, 0, 64));
    let group = f.push(&cg(channel, 2, 24));
    let data = f.push(&dt(&[0u8; 48]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("array_cg").expect("synthetic file should open");
    let ch = file
        .find_channel("Acceleration")
        .expect("the array channel should still be listed");

    assert!(
        ch.unreadable().is_some(),
        "an array whose elements are in other groups cannot be read from this record"
    );
    assert!(file.signal(ch).expect("signal").values().is_err());
}

/// A channel with no conversion link, for pairing a stored channel with a
/// virtual one in the same group.
fn cn_named(next: u64, name: u64, channel_type: u8, byte_offset: u32, bit_count: u32) -> Vec<u8> {
    cn(next, 0, name, channel_type, 0, byte_offset, bit_count)
}

#[test]
fn a_virtual_master_channel_builds_a_time_base_from_its_factor() {
    // B21. Field values here are taken from the standard, not from the reader:
    // cn_type 3 is a virtual master, and its cn_bit_count *must* be 0 because
    // the channel occupies no bytes in the record. Its raw value is the
    // zero-based sample index, which the conversion scales — that is how a file
    // stores a regularly-spaced time base without writing one sample of it.
    //
    // The factor is deliberately non-zero. Every virtual channel in the corpus
    // has a factor of 0, which collapses the index to a constant and makes a
    // reader that ignores the index indistinguishable from a correct one.
    let mut f = FileBuilder::new();
    f.push(&hd());

    // phys = cc_val[0] + cc_val[1] * raw — 10 ms per sample, starting at 0.
    let conv = f.push(&cc(1, &[], &[0.0, 0.01]));

    // Records the virtual channel must not read: two bytes per sample, all
    // non-zero, so striding them would give 0x0707 rather than a ramp.
    let records = vec![7u8; 8];

    let temp_name = f.push(&tx("Temperature"));
    let time_name = f.push(&tx("t"));
    let time = f.push(&cn_converted(0, time_name, conv, 3, 0, 0, 0));
    let temp = f.push(&cn_named(time, temp_name, 0, 0, 16));
    let group = f.push(&cg(temp, 4, 2));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f
        .open("virtual_master")
        .expect("synthetic file should open");

    let time_ch = file.find_channel("t").expect("the master should be listed");
    let values = file
        .signal(time_ch)
        .expect("signal")
        .values_f64()
        .expect("a virtual master should decode");
    assert_eq!(values, vec![0.0, 0.01, 0.02, 0.03]);

    // The stored channel sharing the group is unaffected: the virtual rule must
    // not change how an ordinary field is read.
    let temp_ch = file
        .find_channel("Temperature")
        .expect("the stored channel should be listed");
    assert_eq!(
        file.signal(temp_ch)
            .expect("signal")
            .values_f64()
            .expect("a stored channel should decode"),
        vec![1799.0; 4],
        "0x0707 little-endian"
    );
}

#[test]
fn a_virtual_data_channel_counts_samples_when_it_has_no_conversion() {
    // Without a conversion the raw index is the value, and it must be reported
    // at a width that can hold it — cn_bit_count is 0, so sizing the value from
    // the field would wrap the count at 256.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Index"));
    let channel = f.push(&cn_named(0, name, 6, 0, 0));
    let group = f.push(&cg(channel, 300, 1));
    let data = f.push(&dt(&vec![0xFFu8; 300]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("virtual_data").expect("synthetic file should open");
    let ch = file
        .find_channel("Index")
        .expect("channel should be listed");
    let values = file
        .signal(ch)
        .expect("signal")
        .values_f64()
        .expect("a virtual data channel should decode");

    assert_eq!(values.len(), 300);
    assert_eq!(values[0], 0.0);
    assert_eq!(values[299], 299.0, "the index must not wrap at 256");
}

/// A channel group that declares invalidation bytes after its data bytes.
///
/// The record is `[record id][cg_data_bytes][cg_inval_bytes]`, so the
/// invalidation area begins where the data ends.
fn cg_with_inval(cn_first: u64, cycle_count: u64, data_bytes: u32, inval_bytes: u32) -> Vec<u8> {
    let mut d = vec![0u8; 32];
    d[8..16].copy_from_slice(&cycle_count.to_le_bytes());
    d[24..28].copy_from_slice(&data_bytes.to_le_bytes());
    d[28..32].copy_from_slice(&inval_bytes.to_le_bytes());
    block(b"##CG", &[0, cn_first, 0, 0, 0, 0], &d)
}

/// A one-byte unsigned channel carrying a per-sample invalidation bit.
///
/// `cn_flags` bit 1 marks the bit as present, and `cn_inval_bit_pos` locates it
/// within the group's invalidation bytes.
fn cn_invalidated(next: u64, name: u64, byte_offset: u32, inval_bit_pos: u32) -> Vec<u8> {
    let mut d = vec![0u8; 72];
    d[2] = 0; // cn_data_type: unsigned, little-endian
    d[4..8].copy_from_slice(&byte_offset.to_le_bytes());
    d[8..12].copy_from_slice(&8u32.to_le_bytes()); // cn_bit_count
    d[12..16].copy_from_slice(&0x0002u32.to_le_bytes()); // cn_flags: invalidation bit
    d[16..20].copy_from_slice(&inval_bit_pos.to_le_bytes());
    block(b"##CN", &[next, 0, name, 0, 0, 0, 0, 0], &d)
}

#[test]
fn invalidation_bits_mark_the_samples_the_file_says_are_not_measurements() {
    // 4.9.2. Polarity is the fact worth getting from the standard rather than
    // from the reader: a *set* cn_inval_bit_pos bit means the sample is
    // INVALID. `validity()` inverts that, so `true` there means valid.
    //
    // Two channels share one invalidation byte at different bit positions, so
    // the test fails if a channel reads its neighbour's bit rather than its own.
    let mut f = FileBuilder::new();
    f.push(&hd());

    // [a][b][inval] per sample. Bit 0 invalidates `a`, bit 3 invalidates `b`.
    let records: Vec<u8> = vec![
        10,
        20,
        0b0000_0000, // both valid
        11,
        21,
        0b0000_0001, // a invalid
        12,
        22,
        0b0000_1000, // b invalid
        13,
        23,
        0b0000_1001, // both invalid
    ];

    let name_a = f.push(&tx("a"));
    let name_b = f.push(&tx("b"));
    let ch_b = f.push(&cn_invalidated(0, name_b, 1, 3));
    let ch_a = f.push(&cn_invalidated(ch_b, name_a, 0, 0));
    let group = f.push(&cg_with_inval(ch_a, 4, 2, 1));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("invalidation").expect("synthetic file should open");

    let a = file
        .signal(file.find_channel("a").expect("a"))
        .expect("signal");
    assert_eq!(
        a.validity(),
        Some(vec![true, false, true, false]),
        "channel a must follow bit 0"
    );
    assert_eq!(a.valid_count(), 2);

    let b = file
        .signal(file.find_channel("b").expect("b"))
        .expect("signal");
    assert_eq!(
        b.validity(),
        Some(vec![true, true, false, false]),
        "channel b must follow bit 3"
    );
    assert_eq!(b.valid_count(), 2);

    // Invalid samples are still returned — the documented contract is that
    // `validity()` says which of them are measurements, not that the rest are
    // removed or zeroed.
    assert_eq!(
        a.values_f64().expect("a should decode"),
        vec![10.0, 11.0, 12.0, 13.0]
    );
    assert!(a.is_valid(0) && !a.is_valid(1) && a.is_valid(2) && !a.is_valid(3));
}

#[test]
fn a_channel_without_the_flag_reports_no_validity_even_beside_one_that_has_it() {
    // Invalidation bytes belong to the group, the flag to the channel. A
    // channel without the flag has no bit in that area, and must not be given
    // one by position — reporting `None` is what says "every sample is valid".
    let mut f = FileBuilder::new();
    f.push(&hd());

    // Bit 0 is set on every sample; the unflagged channel must ignore it.
    let records: Vec<u8> = vec![10, 20, 1, 11, 21, 1];

    let name_plain = f.push(&tx("plain"));
    let name_flagged = f.push(&tx("flagged"));
    let plain_ch = f.push(&cn_named(0, name_plain, 0, 1, 8));
    let flagged = f.push(&cn_invalidated(plain_ch, name_flagged, 0, 0));
    let group = f.push(&cg_with_inval(flagged, 2, 2, 1));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("invalidation_mixed").expect("file should open");

    let plain = file
        .signal(file.find_channel("plain").expect("plain"))
        .expect("signal");
    assert_eq!(plain.validity(), None);
    assert_eq!(plain.valid_count(), 2);

    let flagged = file
        .signal(file.find_channel("flagged").expect("flagged"))
        .expect("signal");
    assert_eq!(flagged.validity(), Some(vec![false, false]));
    assert_eq!(flagged.valid_count(), 0);
}

#[test]
fn a_canopen_date_channel_decodes_each_field_from_its_own_bits() {
    // 4.9.3. Every field but the milliseconds shares a byte with reserved or
    // flag bits, so the fixture deliberately *sets* those neighbours: a decoder
    // that reads a byte whole rather than masking it will fold them in and
    // produce a month of 0x83 or an hour with the summer-time bit added.
    //
    // 2026-08-03T12:34:56.789, Monday, summer time.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let ms: u16 = 56_789; // seconds live inside the minute's ms field
    let mut sample = Vec::new();
    sample.extend_from_slice(&ms.to_le_bytes()); // bytes 0-1
    sample.push(34 | 0b1100_0000); // minutes, reserved bits 6-7 set
    sample.push(12 | 0b1000_0000); // hours + summer time
    sample.push(3 | (1 << 5)); // day of month + day of week (Monday)
    sample.push(8 | 0b1100_0000); // month, reserved bits 6-7 set
    sample.push(42 | 0b1000_0000); // 1984 + 42 = 2026, reserved bit 7 set
    assert_eq!(sample.len(), 7);

    let name = f.push(&tx("Timestamp"));
    // cn_data_type 12 = CANopen date; cn_bit_count 56 = seven bytes.
    let channel = f.push(&cn(0, 0, name, 0, 12, 0, 56));
    let group = f.push(&cg(channel, 1, 7));
    let data = f.push(&dt(&sample));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("canopen_date").expect("synthetic file should open");
    let ch = file.find_channel("Timestamp").expect("channel");
    let values = file.signal(ch).expect("signal").values().expect("decode");

    let SignalValues::CanopenDate(dates) = values else {
        panic!("a CANopen date channel must decode as dates, got {values:?}");
    };
    let d = dates[0];
    assert_eq!(d.year, 2026, "year counts from 1984, masked to seven bits");
    assert_eq!(d.month, 8);
    assert_eq!(d.day, 3);
    assert_eq!(d.hour, 12);
    assert_eq!(d.minute, 34);
    assert_eq!(d.ms, 56_789);
    assert_eq!(d.day_of_week, 1);
    assert!(d.summer_time);

    let expected = (20_668i64 * 86_400 + 12 * 3_600 + 34 * 60) * 1_000_000_000 + 56_789 * 1_000_000;
    assert_eq!(d.to_unix_nanos(), expected);
}

#[test]
fn a_canopen_time_channel_masks_its_reserved_bits() {
    // The ms field is 28 bits inside a 32-bit word; the top four are reserved
    // and set here, so a decoder taking the word whole reports a wildly wrong
    // time rather than 12:34:56.789.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let ms: u32 = (12 * 3_600 + 34 * 60) * 1_000 + 56_789;
    let days: u16 = (20_668 - 5_113) as u16; // 2026-08-03, days since 1984
    let mut sample = Vec::new();
    sample.extend_from_slice(&(ms | 0xF000_0000).to_le_bytes());
    sample.extend_from_slice(&days.to_le_bytes());
    assert_eq!(sample.len(), 6);

    let name = f.push(&tx("Elapsed"));
    // cn_data_type 13 = CANopen time; cn_bit_count 48 = six bytes.
    let channel = f.push(&cn(0, 0, name, 0, 13, 0, 48));
    let group = f.push(&cg(channel, 1, 6));
    let data = f.push(&dt(&sample));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("canopen_time").expect("synthetic file should open");
    let ch = file.find_channel("Elapsed").expect("channel");
    let values = file.signal(ch).expect("signal").values().expect("decode");

    let SignalValues::CanopenTime(times) = values else {
        panic!("a CANopen time channel must decode as times, got {values:?}");
    };
    assert_eq!(
        times[0].ms_since_midnight, ms,
        "the top four bits are reserved"
    );
    assert_eq!(times[0].days_since_1984, days);

    let expected = (20_668i64 * 86_400 + 12 * 3_600 + 34 * 60) * 1_000_000_000 + 56_789 * 1_000_000;
    assert_eq!(times[0].to_unix_nanos(), expected);
}

#[test]
fn a_complex_channel_splits_each_sample_into_its_two_parts() {
    // cn_bit_count covers the pair, so 128 bits is two f64. Reading it as one
    // number would give the real part alone and drop the imaginary half.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let mut records = Vec::new();
    for (re, im) in [(1.5f64, -2.5f64), (0.0, 7.25)] {
        records.extend_from_slice(&re.to_le_bytes());
        records.extend_from_slice(&im.to_le_bytes());
    }

    let name = f.push(&tx("Impedance"));
    // cn_data_type 14 = complex, little-endian.
    let channel = f.push(&cn(0, 0, name, 0, 14, 0, 128));
    let group = f.push(&cg(channel, 2, 16));
    let data = f.push(&dt(&records));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("complex").expect("synthetic file should open");
    let ch = file.find_channel("Impedance").expect("channel");
    let values = file.signal(ch).expect("signal").values().expect("decode");

    assert_eq!(
        values,
        SignalValues::Complex {
            re: vec![1.5, 0.0],
            im: vec![-2.5, 7.25],
        }
    );
}

#[test]
fn a_complex_channel_of_an_impossible_width_is_refused() {
    // A complex sample is two floats. Anything but 64 or 128 bits is not one,
    // and guessing which half is which would be inventing the layout.
    let mut f = FileBuilder::new();
    f.push(&hd());

    let name = f.push(&tx("Bad"));
    let channel = f.push(&cn(0, 0, name, 0, 14, 0, 96));
    let group = f.push(&cg(channel, 1, 12));
    let data = f.push(&dt(&[0u8; 12]));
    let group_block = f.push(&dg(0, group, data, 0));
    f.patch_link(hd_link(0), group_block);

    let file = f.open("complex_bad").expect("synthetic file should open");
    let ch = file.find_channel("Bad").expect("channel");
    assert!(file.signal(ch).expect("signal").values().is_err());
}
